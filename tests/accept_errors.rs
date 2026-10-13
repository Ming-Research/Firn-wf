//! Accept failures must leave the listening socket available for later peers.

use super::*;
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

const EMFILE_LOG: &[u8] = b"Accepting client connection: Too many open files\n";
const INITIAL_DESCRIPTOR_LIMIT: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct DescriptorLimit {
    soft: u64,
    hard: u64,
}

unsafe extern "C" {
    fn prlimit64(pid: i32, resource: i32, new: *const DescriptorLimit,
                 old: *mut DescriptorLimit) -> i32;
}

struct DescriptorSnapshot {
    number: u64,
    target: String,
    identity: Option<(u64, u64)>,
    info: String,
}

/// Snapshot descriptor identity and access metadata, never file contents.
/// A parallel test may close a parent descriptor while this snapshot is read;
/// retain the read error as evidence instead of failing the accept case.
fn descriptor_snapshot(pid: u32) -> std::io::Result<Vec<DescriptorSnapshot>> {
    let mut numbers = std::fs::read_dir(format!("/proc/{pid}/fd"))?
        .map(|entry| entry?.file_name().to_str().and_then(|name| name.parse::<u64>().ok())
            .ok_or_else(|| std::io::Error::other("nonnumeric Linux descriptor")))
        .collect::<std::io::Result<Vec<_>>>()?;
    numbers.sort_unstable();
    Ok(numbers.into_iter().map(|number| {
        let path = format!("/proc/{pid}/fd/{number}");
        let target = std::fs::read_link(&path).map(|target| target.display().to_string())
            .unwrap_or_else(|error| format!("[readlink failed: {error}]"));
        let identity = std::fs::metadata(&path).ok().map(|meta| (meta.dev(), meta.ino()));
        let info = match std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{number}")) {
            Ok(text) => {
                let fields = text.lines().filter(|line| ["flags:", "mnt_id:", "ino:"]
                    .iter().any(|prefix| line.starts_with(prefix))).collect::<Vec<_>>().join(", ");
                let flags = text.lines().find_map(|line| line.strip_prefix("flags:"))
                    .and_then(|flags| u64::from_str_radix(flags.trim(), 8).ok());
                match flags {
                    Some(flags) => {
                        let access = match flags & 3 { 0 => "read", 1 => "write", 2 => "read/write", _ => "unknown" };
                        format!("{fields}, access={access}, cloexec={}", flags & 0o2000000 != 0)
                    }
                    None => format!("{fields}, access=unknown"),
                }
            }
            Err(error) => format!("[fdinfo failed: {error}]"),
        };
        DescriptorSnapshot { number, target, identity, info }
    }).collect())
}

fn jobserver_tokens() -> String {
    ["CARGO_MAKEFLAGS", "MAKEFLAGS"].into_iter().map(|name| {
        let value = std::env::var(name).unwrap_or_default();
        let tokens = value.split_whitespace().filter(|token|
            token.starts_with("--jobserver-auth=") || token.starts_with("--jobserver-fds="))
            .collect::<Vec<_>>();
        format!("{name} jobserver tokens: {tokens:?}")
    }).collect::<Vec<_>>().join("\n")
}

/// Keep candidate parent endpoints beside each child FD. Matching targets or
/// device/inode pairs support attribution; shared anon_inode identities alone
/// do not prove inheritance. Flags and jobserver tokens distinguish endpoints.
fn fd_diagnostics(child: &ProgramChild, native: bool) -> String {
    let mut limits = DescriptorLimit { soft: 0, hard: 0 };
    let limit_result = unsafe { prlimit64(child.id() as i32, 7, std::ptr::null(), &mut limits) };
    let limit_text = if limit_result == 0 { format!("{limits:?}") }
        else { format!("[prlimit failed: {}]", std::io::Error::last_os_error()) };
    let mut output = format!("native={native}, child PID={}, parent PID={}, current limits={limit_text}\n{}\n",
        child.id(), std::process::id(), jobserver_tokens());
    let child_fds = match descriptor_snapshot(child.id()) {
        Ok(fds) => fds,
        Err(error) => {
            output.push_str(&format!("child descriptor snapshot failed: {error}\n"));
            return output;
        }
    };
    let below_limit = child_fds.iter().filter(|fd| fd.number < limits.soft).count();
    output.push_str(&format!("current FD count={}, highest FD={:?}, free native slots={}\n",
        child_fds.len(), child_fds.last().map(|fd| fd.number), limits.soft.saturating_sub(below_limit as u64)));
    let parent_fds = descriptor_snapshot(std::process::id()).unwrap_or_else(|error| {
        output.push_str(&format!("parent descriptor snapshot failed: {error}\n"));
        Vec::new()
    });
    for fd in child_fds {
        output.push_str(&format!("child fd {} target={:?}, dev/inode={:?}, {}\n",
            fd.number, fd.target, fd.identity, fd.info));
        let mut matched = false;
        for parent in &parent_fds {
            if (!fd.target.starts_with('[') && parent.target == fd.target)
                || (fd.identity.is_some() && parent.identity == fd.identity) {
                matched = true;
                output.push_str(&format!("  candidate parent fd {} target={:?}, dev/inode={:?}, {}\n",
                    parent.number, parent.target, parent.identity, parent.info));
            }
        }
        if !matched { output.push_str("  no matching parent FD in this snapshot\n"); }
    }
    output
}

fn failure_diagnostics(child: &ProgramChild, native: bool, port: u16) -> String {
    // diagnostic_block stops the process; read /proc before invoking it.
    let fds = fd_diagnostics(child, native);
    let process = child.monitor().diagnostic_block(Some(port));
    format!("\n{fds}{process}")
}

/// Linux RLIMIT_NOFILE is 7. Read back every change so a missing limit cannot
/// turn this into an ordinary connection test; the parent keeps its own limit.
fn descriptor_limit(child: &ProgramChild, soft: Option<u64>, native: bool, port: u16) -> DescriptorLimit {
    let mut current = DescriptorLimit { soft: 0, hard: 0 };
    let result = unsafe { prlimit64(child.id() as i32, 7, std::ptr::null(), &mut current) };
    assert_eq!(result, 0, "read child descriptor limit: {}{}",
        std::io::Error::last_os_error(), failure_diagnostics(child, native, port));
    if let Some(soft) = soft {
        assert!(soft <= current.hard, "requested descriptor limit exceeds hard limit{}",
            failure_diagnostics(child, native, port));
        let next = DescriptorLimit { soft, hard: current.hard };
        let result = unsafe { prlimit64(child.id() as i32, 7, &next, std::ptr::null_mut()) };
        assert_eq!(result, 0, "set child descriptor limit: {}{}",
            std::io::Error::last_os_error(), failure_diagnostics(child, native, port));
        let actual = descriptor_limit(child, None, native, port);
        assert_eq!(actual.soft, soft, "child applied RLIMIT_NOFILE{}",
            failure_diagnostics(child, native, port));
        assert_eq!(actual.hard, current.hard, "child retained hard RLIMIT_NOFILE{}",
            failure_diagnostics(child, native, port));
        actual
    } else {
        current
    }
}

fn descriptors(child: &ProgramChild, native: bool, port: u16) -> BTreeSet<u64> {
    let result = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap_or_else(|error| panic!("read child descriptors: {error}{}",
            failure_diagnostics(child, native, port)))
        .map(|entry| entry?.file_name().to_str().and_then(|name| name.parse().ok())
            .ok_or_else(|| std::io::Error::other("nonnumeric Linux descriptor")))
        .collect::<std::io::Result<BTreeSet<u64>>>();
    result.unwrap_or_else(|error| panic!("read child descriptor entries: {error}{}",
        failure_diagnostics(child, native, port)))
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
/// are needed. Native free slots equal the tightened soft limit minus the
/// actual startup FD count, including holes below high inherited descriptors.
/// The factory needs one credit each for the stop listener, TCP listener and
/// readiness client, then one per free slot and one for the failing accept:
/// free_slots + 4 must fit its startup budget. Initial cwd/stdio handles do not
/// take this budget, and this no-AOF server opens no other factory handles.
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
        let reply = ping_before(&mut first, admission_deadline).unwrap_or_else(|error|
            panic!("readiness PING failed: {error}{}", failure_diagnostics(&child, native, port)));
        assert_eq!(reply, *b"+PONG\r\n", "readiness reply{}",
            failure_diagnostics(&child, native, port));
        let initial_limit = descriptor_limit(&child, None, native, port);
        let startup = descriptors(&child, native, port);
        let highest = *startup.last().unwrap_or_else(|| panic!(
            "firn has no descriptors after readiness{}", failure_diagnostics(&child, native, port)));
        let soft = highest + 1 + 8;
        let free_slots = soft - startup.len() as u64;
        let factory_budget = initial_limit.soft.min(1 << 20).saturating_sub(64);
        let required_credits = free_slots + 4;
        let budget = format!("native={native}, startup limits={initial_limit:?}, \
            startup FD count={}, highest FD={highest}, new soft={soft}, \
            free native slots={free_slots}, maximum live clients={}, \
            required factory credits={required_credits}, startup factory budget={factory_budget}",
            startup.len(), free_slots + 1);
        // Bypass libtest's print-macro capture so passing CI runs retain the
        // descriptor identities needed to explain high inherited descriptors.
        std::io::stderr().lock().write_all(format!("\nAccept EMFILE startup: {budget}\n{}",
            fd_diagnostics(&child, native)).as_bytes()).expect("write startup FD diagnostic");
        assert_eq!((initial_limit.soft, initial_limit.hard),
            (INITIAL_DESCRIPTOR_LIMIT, INITIAL_DESCRIPTOR_LIMIT), "startup limit: {budget}{}",
            failure_diagnostics(&child, native, port));
        assert!(soft < initial_limit.soft,
            "startup descriptor range leaves no room to tighten: {budget}{}",
            failure_diagnostics(&child, native, port));
        assert!(required_credits <= factory_budget,
            "native EMFILE witness needs more factory credits than startup permits: {budget}{}",
            failure_diagnostics(&child, native, port));
        descriptor_limit(&child, Some(soft), native, port);
        let mut admitted = vec![first];
        let mut previous = startup.clone();
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        while previous.len() < soft as usize {
            let remaining = admission_deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "descriptor saturation exceeded 15 seconds: {budget}{}",
                failure_diagnostics(&child, native, port));
            let mut peer = TcpStream::connect_timeout(&address, remaining).unwrap_or_else(|error|
                panic!("connect peer before saturation: {error}; {budget}{}",
                    failure_diagnostics(&child, native, port)));
            let reply = ping_before(&mut peer, admission_deadline)
                .unwrap_or_else(|error| panic!("admit peer before saturation: {error}; \
                    {budget}, previous={previous:?}{}",
                    failure_diagnostics(&child, native, port)));
            assert_eq!(reply, *b"+PONG\r\n", "admission PONG: {budget}{}",
                failure_diagnostics(&child, native, port));
            admitted.push(peer);
            let current = descriptors(&child, native, port);
            assert!(previous.is_subset(&current) && current.len() == previous.len() + 1,
                "each retained client must add one descriptor: before={previous:?}, after={current:?}; {budget}{}",
                failure_diagnostics(&child, native, port));
            previous = current;
        }
        assert_eq!(previous, (0..soft).collect::<BTreeSet<_>>(),
            "all descriptors below the soft limit must be occupied: {budget}{}",
            failure_diagnostics(&child, native, port));

        // connect can finish in the kernel's backlog without accept succeeding.
        // A full descriptor table and an unserved PING together demonstrate the
        // failure inside firn, rather than exhausting the harness's descriptors.
        let mut blocked = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .unwrap_or_else(|error| panic!("queue a peer while firn's descriptors are full: {error}; {budget}{}",
                failure_diagnostics(&child, native, port)));
        let result = ping_before(&mut blocked, Instant::now() + Duration::from_millis(500));
        match result {
            Err(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut |
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::UnexpectedEof |
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) => {}
            result => panic!("a saturated firn must not serve another peer: {result:?}; {budget}{}",
                failure_diagnostics(&child, native, port)),
        }
        // Wait for an actual accept attempt before freeing slots, even when a
        // loaded runner delays main past the unserved peer's read timeout.
        // Fixed firn logs EMFILE; the old behavior instead closes its listener,
        // observable as a lost descriptor while all admitted peers stay open.
        let attempt_deadline = Instant::now() + Duration::from_secs(5);
        while !child.stderr_contains(EMFILE_LOG) && descriptors(&child, native, port) == previous {
            assert!(Instant::now() < attempt_deadline,
                "firn neither logged EMFILE nor closed its listener within five seconds: {budget}{}",
                failure_diagnostics(&child, native, port));
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(blocked);
        for _ in 0..4 {
            drop(admitted.pop().expect("at least eight extra peers were admitted"));
        }

        let recovery_deadline = Instant::now() + Duration::from_secs(10);
        let mut recovered = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .unwrap_or_else(|error| panic!("connect after releasing descriptor slots: {error}; {budget}{}",
                failure_diagnostics(&child, native, port)));
        let reply = ping_before(&mut recovered, recovery_deadline)
            .unwrap_or_else(|error| panic!("firn did not resume accepting within ten seconds: {error}; {budget}{}",
                failure_diagnostics(&child, native, port)));
        assert_eq!(reply, *b"+PONG\r\n", "fresh peer after native EMFILE: {budget}{}",
            failure_diagnostics(&child, native, port));

        // Keep the other admitted peers connected while SHUTDOWN cancels both
        // their receives and the renewed accept wait; do not hide broken drain
        // behavior by dropping them before the process exits.
        let stop_deadline = Instant::now() + Duration::from_secs(5);
        write_before(&mut recovered, b"SHUTDOWN NOSAVE\r\n", stop_deadline)
            .unwrap_or_else(|error| panic!("request orderly stop after recovery: {error}; {budget}{}",
                failure_diagnostics(&child, native, port)));
        loop {
            if child.try_wait().expect("poll orderly stop after recovery").is_some() {
                break;
            }
            assert!(Instant::now() < stop_deadline,
                "orderly stop after accept failure exceeded five seconds: {budget}{}",
                failure_diagnostics(&child, native, port));
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
