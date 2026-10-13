//! Accept failures must leave the listening socket available for later peers.

use super::*;
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};

const EMFILE_LOG: &[u8] = b"Accepting client connection: Too many open files\n";
const ACCEPT_LOG_PREFIX: &[u8] = b"Accepting client connection: ";
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

fn clock_ticks_per_second() -> u64 {
    static TICKS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *TICKS.get_or_init(|| {
        // This command runs inside the Linux CI test, through the same bounded
        // subprocess harness as the other external helpers, never on the Mac.
        let output = support::run_command(Command::new("getconf").arg("CLK_TCK"));
        assert!(output.status.success(), "getconf CLK_TCK failed: {output:?}");
        let ticks: u64 = std::str::from_utf8(&output.stdout).expect("getconf numeric output")
            .trim().parse().expect("getconf CLK_TCK is a positive integer");
        assert!(ticks > 0, "getconf CLK_TCK must be positive");
        ticks
    })
}

fn cpu_ticks(child: &ProgramChild, native: bool, port: u16) -> u64 {
    let result = (|| -> std::io::Result<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id()))?;
        // comm (field 2) may contain spaces or ')'. The final ')' precedes
        // state (field 3); utime/stime (fields 14/15) are then indices 11/12.
        let end = stat.rfind(')').ok_or_else(|| std::io::Error::other("missing process comm terminator"))?;
        let fields = stat[end + 1..].split_whitespace().collect::<Vec<_>>();
        let tick = |index: usize| fields.get(index).and_then(|field| field.parse::<u64>().ok())
            .ok_or_else(|| std::io::Error::other("missing process CPU ticks"));
        let user = tick(11)?;
        let system = tick(12)?;
        user.checked_add(system).ok_or_else(|| std::io::Error::other("process CPU ticks overflow"))
    })();
    result.unwrap_or_else(|error| panic!("read child CPU ticks: {error}{}",
        failure_diagnostics(child, native, port)))
}

fn accept_log_count(child: &ProgramChild, native: bool, port: u16) -> usize {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(count) = child.stderr_occurrences(ACCEPT_LOG_PREFIX) {
            return count;
        }
        assert!(Instant::now() < deadline, "captured stderr buffer stayed busy for two seconds{}",
            failure_diagnostics(child, native, port));
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A coarse functional guard against an immediate-error retry loop, not a
/// performance comparison. Expiry still wakes ten times a second; 25% of one
/// core leaves broad headroom for that work and CI scheduling noise. A busy
/// loop may receive little CPU on a contended runner, so bounded log growth is
/// an independent guard. Account for the actual wall interval and round the
/// CPU allowance up to one USER_HZ tick. Logs allow one line per wall second
/// plus two for limiter boundaries and a late output-reader observation.
fn assert_saturated_quiet(child: &ProgramChild, native: bool, port: u16, budget: &str) {
    let ticks_per_second = clock_ticks_per_second();
    let before_logs = accept_log_count(child, native, port);
    let before_cpu = cpu_ticks(child, native, port);
    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(1100));
    let after_logs = accept_log_count(child, native, port);
    let after_cpu = cpu_ticks(child, native, port);
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_secs(1), "quiet observation must cover at least one second");
    let used_ticks = after_cpu.checked_sub(before_cpu).expect("process CPU ticks cannot decrease");
    let quarter_core_tick_ns = 4_000_000_000_u128;
    let allowed_ticks = (elapsed.as_nanos() * u128::from(ticks_per_second)
        + quarter_core_tick_ns - 1) / quarter_core_tick_ns;
    let new_logs = after_logs.checked_sub(before_logs).expect("captured stderr cannot shrink");
    let allowed_logs = elapsed.as_secs() + 2;
    let observation = format!("elapsed={elapsed:?}, USER_HZ={ticks_per_second}, CPU ticks={used_ticks}, \
        allowed CPU ticks={allowed_ticks} (25% of one core), accept log lines={new_logs}, \
        allowed log lines={allowed_logs}; {budget}");
    std::io::stderr().lock().write_all(format!("Accept EMFILE quiet: {observation}\n").as_bytes())
        .expect("write quiet observation diagnostic");
    assert!(u128::from(used_ticks) <= allowed_ticks,
        "saturated accept retries consumed more than 25% of one core: {observation}{}",
        failure_diagnostics(child, native, port));
    assert!(new_logs as u64 <= allowed_logs,
        "saturated accept diagnostics exceeded one per second plus two boundary lines: {observation}{}",
        failure_diagnostics(child, native, port));
}

fn stop_after_emfile(
    mut child: ProgramChild, caller: &mut TcpStream, native: bool, port: u16,
    budget: &str, timeout: Duration, fault_began: Instant,
) {
    // Keep all other admitted peers alive through this wait. The saturated
    // case bounds prompt overall exit; it does not alone prove the timer was
    // interrupted, since the resource pause is shorter than the exit bound.
    let stop_deadline = Instant::now() + timeout;
    write_before(caller, b"SHUTDOWN NOSAVE\r\n", stop_deadline)
        .unwrap_or_else(|error| panic!("request orderly stop after EMFILE: {error}; {budget}{}",
            failure_diagnostics(&child, native, port)));
    loop {
        if child.try_wait().expect("poll orderly stop after EMFILE").is_some() {
            break;
        }
        assert!(Instant::now() < stop_deadline,
            "orderly stop after EMFILE exceeded {timeout:?}: {budget}{}",
            failure_diagnostics(&child, native, port));
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().expect("wait for firn to stop after EMFILE");
    assert_eq!(output.status.code(), Some(0), "orderly stop after EMFILE: {output:?}");
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(output.stderr.windows(EMFILE_LOG.len()).any(|window| window == EMFILE_LOG),
        "the server must log native EMFILE, not just an unserved peer: {errors}");
    // Include the later queued peer and recovery, not only the empty-backlog
    // quiet phase. The last admission is the earliest possible native EMFILE.
    let fault_elapsed = fault_began.elapsed();
    let total_logs = output.stderr.windows(ACCEPT_LOG_PREFIX.len())
        .filter(|window| *window == ACCEPT_LOG_PREFIX).count();
    let allowed_logs = fault_elapsed.as_secs() + 2;
    assert!(total_logs as u64 <= allowed_logs,
        "accept diagnostics exceeded one per second plus two boundary lines before exit: \
        logs={total_logs}, allowed={allowed_logs}, elapsed={fault_elapsed:?}; {budget}; stderr={errors}");
}

#[test]
fn firn_accept_recovers_after_emfile_on_adapter() {
    accept_emfile_case(false, false);
}

#[test]
fn firn_accept_recovers_after_emfile_on_native_route() {
    accept_emfile_case(true, false);
}

#[test]
fn firn_shutdown_cancels_emfile_pause_on_adapter() {
    accept_emfile_case(false, true);
}

#[test]
fn firn_shutdown_cancels_emfile_pause_on_native_route() {
    accept_emfile_case(true, true);
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
fn accept_emfile_case(native: bool, shutdown_while_saturated: bool) {
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
    let child = ProgramChild::spawn(&mut command)
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
    let mut fault_began = None;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    while previous.len() < soft as usize {
        let remaining = admission_deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "descriptor saturation exceeded 15 seconds: {budget}{}",
            failure_diagnostics(&child, native, port));
        if previous.len() + 1 == soft as usize {
            fault_began = Some(Instant::now());
        }
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
    let fault_began = fault_began.expect("the final one-FD admission filled the table");

    // Linux accept reserves an FD before waiting on an empty backlog.
    // The last admission already fills the table; open no additional peer
    // until EMFILE was observed and the quiet saturated interval finished.
    let attempt_deadline = Instant::now() + Duration::from_secs(5);
    while !child.stderr_contains(EMFILE_LOG) {
        assert_eq!(descriptors(&child, native, port), previous,
            "listener and retained clients must survive the first EMFILE: {budget}{}",
            failure_diagnostics(&child, native, port));
        assert!(Instant::now() < attempt_deadline,
            "firn did not report EMFILE with a full FD table and empty backlog within five seconds: {budget}{}",
            failure_diagnostics(&child, native, port));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_saturated_quiet(&child, native, port, &budget);
    assert_eq!(descriptors(&child, native, port), previous,
        "all saturated descriptors must remain open during the quiet phase: {budget}{}",
        failure_diagnostics(&child, native, port));
    if shutdown_while_saturated {
        stop_after_emfile(child, &mut admitted[0], native, port, &budget, Duration::from_secs(1), fault_began);
        return;
    }

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

    stop_after_emfile(child, &mut recovered, native, port, &budget, Duration::from_secs(5), fault_began);
    drop(admitted);
}
