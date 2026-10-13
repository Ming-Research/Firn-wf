//! Accept failures must leave the listening socket available for later peers.

use super::*;
use std::collections::BTreeSet;
use std::process::{Command, Stdio};

const EMFILE_LOG: &[u8] = b"Accepting client connection: Too many open files\n";

#[repr(C)]
#[derive(Clone, Copy)]
struct DescriptorLimit {
    soft: u64,
    hard: u64,
}

unsafe extern "C" {
    fn prlimit64(pid: i32, resource: i32, new: *const DescriptorLimit,
                 old: *mut DescriptorLimit) -> i32;
}

/// Linux RLIMIT_NOFILE is 7. Read back every change so a missing limit cannot
/// turn this into an ordinary connection test; the parent keeps its own limit.
fn descriptor_limit(child: &ProgramChild, soft: Option<u64>) -> DescriptorLimit {
    let mut current = DescriptorLimit { soft: 0, hard: 0 };
    let result = unsafe { prlimit64(child.id() as i32, 7, std::ptr::null(), &mut current) };
    assert_eq!(result, 0, "read child descriptor limit: {}",
        std::io::Error::last_os_error());
    if let Some(soft) = soft {
        assert!(soft <= current.hard, "requested descriptor limit exceeds hard limit");
        let next = DescriptorLimit { soft, hard: current.hard };
        let result = unsafe { prlimit64(child.id() as i32, 7, &next, std::ptr::null_mut()) };
        assert_eq!(result, 0, "set child descriptor limit: {}",
            std::io::Error::last_os_error());
        let actual = descriptor_limit(child, None);
        assert_eq!(actual.soft, soft, "child applied RLIMIT_NOFILE");
        assert_eq!(actual.hard, current.hard, "child retained hard RLIMIT_NOFILE");
        actual
    } else {
        current
    }
}

fn descriptors(child: &ProgramChild, port: u16) -> BTreeSet<u64> {
    std::fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap_or_else(|error| panic!("read child descriptors: {error}{}",
            child.monitor().diagnostic_block(Some(port))))
        .map(|entry| entry.expect("child descriptor directory entry")
            .file_name().to_str().expect("numeric descriptor name")
            .parse().expect("numeric Linux descriptor"))
        .collect()
}

fn time_left(deadline: Instant) -> std::io::Result<Duration> {
    let time = deadline.saturating_duration_since(Instant::now());
    if time.is_zero() {
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "peer I/O deadline"))
    } else {
        Ok(time)
    }
}

fn write_before(stream: &mut TcpStream, request: &[u8], deadline: Instant) -> std::io::Result<()> {
    let mut sent = 0;
    while sent < request.len() {
        stream.set_write_timeout(Some(time_left(deadline)?))?;
        match stream.write(&request[sent..]) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "write request")),
            Ok(count) => sent += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    time_left(deadline)?;
    Ok(())
}

/// One absolute deadline covers both the request write and its PONG read.
fn ping_before(stream: &mut TcpStream, deadline: Instant) -> std::io::Result<[u8; 7]> {
    write_before(stream, b"PING\r\n", deadline)?;
    let mut reply = [0; 7];
    let mut received = 0;
    while received < reply.len() {
        stream.set_read_timeout(Some(time_left(deadline)?))?;
        match stream.read(&mut reply[received..]) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "read PONG")),
            Ok(count) => received += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    time_left(deadline)?;
    Ok(reply)
}

/// Ubuntu's util-linux prlimit execs firn in place, retaining the monitored
/// PID and process group. Start with 256 descriptors: Whitefoot derives handle
/// credits from the initial limit minus a reserve of 64, so a tiny initial
/// limit would test the runtime's admission limit rather than accept(2).
/// After readiness, /proc supplies the actual descriptor range; tightening
/// only the soft limit leaves ample runtime credits and provokes native EMFILE.
/// Pin one worker and driver, disable AOF and keep every admitted socket open,
/// so each PONG must add exactly one descriptor and no background file opens
/// are needed. The case checks these assumptions instead of guessing overhead.
#[test]
fn firn_accept_recovers_after_emfile_on_both_routes() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let lease = support::lease_port();
        let port = lease.port();
        let executable = std::env::var_os("FIRN").expect("validated FIRN executable");
        let mut command = Command::new("prlimit");
        command.args(["--nofile=256:256", "--"]).arg(executable)
            .args([port.to_string(), "0".to_owned(), "-".to_owned()])
            .current_dir(program.working_directory())
            .env("WF_WORKERS", "1").env("WF_DRIVERS", "1")
            .env_remove("WF_REQUIRE_WINDOWS_IOCP")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if native {
            command.env_remove("WF_IO_NO_NATIVE_RING");
        } else {
            command.env("WF_IO_NO_NATIVE_RING", "1");
        }
        let mut child = ProgramChild::spawn(&mut command)
            .expect("spawn firn through Ubuntu util-linux prlimit").with_port(&lease);
        let mut first = connect_when_ready(port, &child);
        let admission_deadline = Instant::now() + Duration::from_secs(15);
        assert_eq!(ping_before(&mut first, admission_deadline).unwrap(), *b"+PONG\r\n");
        let initial_limit = descriptor_limit(&child, None);
        assert_eq!((initial_limit.soft, initial_limit.hard), (256, 256));
        let startup = descriptors(&child, port);
        let highest = *startup.last().expect("firn has descriptors after its first PONG");
        let soft = highest + 1 + 8;
        assert!(soft < initial_limit.soft,
            "startup descriptor use leaves no room to tighten: {startup:?}");
        assert!(soft <= 64,
            "this witness needs at most 64 descriptors to stay well below \
             Whitefoot's 192 startup handle credits: startup={startup:?}, soft={soft}");
        descriptor_limit(&child, Some(soft));
        let mut admitted = vec![first];
        let mut previous = startup.clone();
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        while previous.len() < soft as usize {
            let remaining = admission_deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "descriptor saturation exceeded 15 seconds{}",
                child.monitor().diagnostic_block(Some(port)));
            let mut peer = TcpStream::connect_timeout(&address, remaining).unwrap();
            let reply = ping_before(&mut peer, admission_deadline)
                .unwrap_or_else(|error| panic!("admit peer before saturation: {error}; \
                    startup={startup:?}, previous={previous:?}, soft={soft}{}",
                    child.monitor().diagnostic_block(Some(port))));
            assert_eq!(reply, *b"+PONG\r\n");
            admitted.push(peer);
            let current = descriptors(&child, port);
            assert!(previous.is_subset(&current) && current.len() == previous.len() + 1,
                "each retained client must add one descriptor: before={previous:?}, after={current:?}");
            previous = current;
        }
        assert_eq!(previous, (0..soft).collect::<BTreeSet<_>>(),
            "all descriptors below the soft limit are occupied before accept must fail");

        // connect can finish in the kernel's backlog without accept succeeding.
        // A full descriptor table and an unserved PING together demonstrate the
        // failure inside firn, rather than exhausting the harness's descriptors.
        let mut blocked = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .expect("queue a peer while firn's descriptors are full");
        let result = ping_before(&mut blocked, Instant::now() + Duration::from_millis(500));
        match result {
            Err(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut |
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::UnexpectedEof |
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) => {}
            result => panic!("a saturated firn must not serve another peer: {result:?}"),
        }
        // Wait for an actual accept attempt before freeing slots, even when a
        // loaded runner delays main past the unserved peer's read timeout.
        // Fixed firn logs EMFILE; the old behavior instead closes its listener,
        // observable as a lost descriptor while all admitted peers stay open.
        let attempt_deadline = Instant::now() + Duration::from_secs(5);
        while !child.stderr_contains(EMFILE_LOG) && descriptors(&child, port) == previous {
            assert!(Instant::now() < attempt_deadline,
                "firn neither logged EMFILE nor closed its listener within five seconds{}",
                child.monitor().diagnostic_block(Some(port)));
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(blocked);
        for _ in 0..4 {
            drop(admitted.pop().expect("at least eight extra peers were admitted"));
        }

        let recovery_deadline = Instant::now() + Duration::from_secs(10);
        let mut recovered = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .unwrap_or_else(|error| panic!("connect after releasing descriptor slots: {error}{}",
                child.monitor().diagnostic_block(Some(port))));
        let reply = ping_before(&mut recovered, recovery_deadline)
            .unwrap_or_else(|error| panic!("firn did not resume accepting within ten seconds: {error}{}",
                child.monitor().diagnostic_block(Some(port))));
        assert_eq!(reply, *b"+PONG\r\n", "fresh peer after native EMFILE");

        // Keep the other admitted peers connected while SHUTDOWN cancels both
        // their receives and the renewed accept wait; do not hide broken drain
        // behavior by dropping them before the process exits.
        let stop_deadline = Instant::now() + Duration::from_secs(5);
        write_before(&mut recovered, b"SHUTDOWN NOSAVE\r\n", stop_deadline)
            .expect("request orderly stop after recovery");
        loop {
            if child.try_wait().expect("poll orderly stop after recovery").is_some() {
                break;
            }
            assert!(Instant::now() < stop_deadline,
                "orderly stop after accept failure exceeded five seconds{}",
                child.monitor().diagnostic_block(Some(port)));
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().expect("wait for recovered firn to stop");
        assert_eq!(output.status.code(), Some(0), "orderly stop after accept failure: {output:?}");
        let errors = String::from_utf8_lossy(&output.stderr);
        assert!(output.stderr.windows(EMFILE_LOG.len()).any(|window| window == EMFILE_LOG),
            "the recovered server must log native EMFILE, not just an unserved peer: {errors}");
        drop(admitted);
    }
}
