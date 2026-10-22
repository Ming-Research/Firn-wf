//! Crash real rewrites, on Redis 7.2.16 and Firn, at witnessed file states.
//! The rename cases use strace's syscall-exit delay on the file-adapter
//! route: it stretches an existing state without changing a syscall result.
//! An atomic rename has no observable interior; its two sides are separate
//! cases. Native-ring coverage here is the untraced early-scan case. One
//! more case delays the temporary base's sync, with strace's syscall-entry
//! delay, to hold open the window between a rewrite's cut and its completed
//! base.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SEEDS: usize = 4000;
const MANIFEST: &str = "appendonly.aof.manifest";
const OLD_BASE: &str = "appendonly.aof.1.base.aof";
const NEW_BASE: &str = "appendonly.aof.2.base.aof";

#[derive(Clone, Copy, Debug)]
enum Point { EarlyScan, BeforeManifest, AfterManifest }

#[derive(Clone, Copy, Debug)]
enum Server { Redis, Firn(bool) }

fn start(server: Server, oracle: &Path, directory: &Path, traced: bool) -> (ProgramChild, TcpStream) {
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let binary = match server {
        Server::Redis => oracle.to_path_buf(),
        Server::Firn(_) => PathBuf::from(std::env::var_os("FIRN").expect("FIRN")),
    };
    let mut command = if traced {
        let mut command = Command::new("strace");
        command.args(["-f", "-qq", "-o"]).arg(directory.join("rename.trace"))
            .args(["-e", "trace=rename,renameat,renameat2", "-e",
                "inject=rename,renameat,renameat2:delay_exit=1s", "--"]).arg(&binary);
        command
    } else { Command::new(&binary) };
    command.current_dir(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    match server {
        Server::Redis => {
            command.args(["--port", &port, "--bind", "127.0.0.1", "--save", "", "--appendonly", "yes",
                "--appendfilename", "appendonly.aof", "--appendfsync", "everysec",
                "--aof-use-rdb-preamble", "no", "--auto-aof-rewrite-percentage", "0", "--dir"])
                .arg(directory);
        }
        Server::Firn(native) => {
            command.args([&port, "0", "appendonly.aof", "--auto-aof-rewrite-percentage", "0"])
                .env("WF_WORKERS", "2").env("WF_DRIVERS", "2").env_remove("WF_REQUIRE_WINDOWS_IOCP");
            if native { command.env_remove("WF_IO_NO_NATIVE_RING"); }
            else { command.env("WF_IO_NO_NATIVE_RING", "1"); }
        }
    }
    let child = ProgramChild::spawn(&mut command).expect("spawn rewrite crash subject").with_port(&lease);
    let mut client = connect_when_ready(lease.port(), &child);
    loaded(&mut client, server);
    (child, client)
}

/// Waits until the server answers PING with +PONG. Redis 7.2.16 admits
/// connections while it replays its append-only file and answers -LOADING
/// until the replay ends (firn listens only after it); that reply alone is
/// "not ready yet" and is retried on the same connection. Any other reply,
/// a closed connection or no +PONG within 60 s fails the test.
fn loaded(client: &mut TcpStream, server: Server) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        client.write_all(&resp(&["PING"])).expect("send the readiness PING");
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];
        while !line.ends_with(b"\r\n") {
            assert!(Instant::now() < deadline, "{server:?}: no PING reply within 60 s");
            assert_eq!(client.read(&mut byte).expect("read the readiness PING reply"), 1,
                "{server:?}: connection closed while waiting for PONG, saw {:?}", String::from_utf8_lossy(&line));
            line.push(byte[0]);
        }
        if line == b"+PONG\r\n" { return; }
        assert!(line.starts_with(b"-LOADING "), "{server:?}: unexpected PING reply {:?}", String::from_utf8_lossy(&line));
        assert!(Instant::now() < deadline, "{server:?}: still loading after 60 s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn manifest(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("appendonlydir").join(MANIFEST)).expect("read manifest")
}

// Test names are simple ASCII. Reject malformed lines, repeated names and
// missing active files rather than relying only on the server's loader.
fn parts(text: &str) -> Vec<(String, char)> {
    let mut names = BTreeSet::new();
    let mut bases = 0;
    let mut increments = 0;
    let mut previous_increment = 0_u64;
    let mut result = Vec::new();
    for line in text.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        assert_eq!(words.len(), 6, "malformed manifest: {line}");
        assert_eq!((words[0], words[2], words[4]), ("file", "seq", "type"));
        assert!(!words[1].contains('/') && !words[1].contains(".."), "unsafe filename");
        assert!(names.insert(words[1]), "duplicate manifest filename");
        let sequence: u64 = words[3].parse().expect("manifest sequence");
        match words[5] {
            "b" => bases += 1,
            "i" => {
                assert!(sequence > previous_increment, "unordered incremental files");
                previous_increment = sequence;
                increments += 1;
            }
            "h" => (),
            _ => panic!("unknown manifest kind: {line}"),
        }
        result.push((words[1].to_owned(), words[5].chars().next().unwrap()));
    }
    assert_eq!(bases, 1, "exactly one active base");
    assert!(increments > 0, "an active increment must remain");
    result
}

fn active_base(text: &str) -> String {
    parts(text).into_iter().find(|(_, kind)| *kind == 'b').unwrap().0
}

fn consistent(directory: &Path) {
    for (name, kind) in parts(&manifest(directory)) {
        if kind != 'h' {
            assert!(directory.join("appendonlydir").join(&name).is_file(), "manifest selects missing {name}");
        }
    }
}

fn initialise(directory: &Path) {
    let aof = directory.join("appendonlydir");
    std::fs::create_dir(&aof).unwrap();
    std::fs::write(aof.join(OLD_BASE), []).unwrap();
    std::fs::write(aof.join("appendonly.aof.1.incr.aof"), []).unwrap();
    std::fs::write(aof.join(MANIFEST), rewrite_manifest(1, 1)).unwrap();
}

fn acknowledged(client: &mut TcpStream, expected: &mut BTreeMap<String, String>, key: String, value: String) {
    client.write_all(&resp(&["SET", &key, &value])).unwrap();
    expect_replies(client, b"+OK\r\n", "record acknowledged SET");
    expected.insert(key, value);
}

fn seed(client: &mut TcpStream) -> BTreeMap<String, String> {
    let mut expected = BTreeMap::new();
    for batch in 0..SEEDS / 100 {
        let mut bytes = Vec::new();
        for index in batch * 100..(batch + 1) * 100 {
            let key = format!("seed:{index}");
            let value = format!("{index}:{}", "x".repeat(256));
            bytes.extend(resp(&["SET", &key, &value]));
            expected.insert(key, value);
        }
        client.write_all(&bytes).unwrap();
        expect_replies(client, &b"+OK\r\n".repeat(100), "acknowledge fixture batch");
    }
    expected
}

// everysec may acknowledge before appending. This is a process-crash test,
// not a power-cut or fsync test: wait for the exact last acknowledged record
// in an authoritative increment before killing, without claiming durability.
fn appended(directory: &Path, key: &str, value: &str) {
    let record = resp(&["SET", key, value]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if parts(&manifest(directory)).iter().filter(|(_, kind)| *kind == 'i').any(|(name, _)| {
            let bytes = std::fs::read(directory.join("appendonlydir").join(name)).unwrap();
            bytes.windows(record.len()).any(|window| window == record)
        }) { return; }
        assert!(Instant::now() < deadline, "last acknowledged write never reached an active increment");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scan_records(server: Server, directory: &Path) -> usize {
    let path = match server {
        Server::Firn(_) => directory.join("appendonlydir/temp-appendonly.aof.base"),
        Server::Redis => {
            let Some(path) = std::fs::read_dir(directory).unwrap().filter_map(Result::ok)
                .map(|entry| entry.path()).find(|path| path.file_name().unwrap().to_string_lossy().starts_with("temp-rewriteaof-"))
                else { return 0; };
            path
        }
    };
    let bytes = std::fs::read(path).unwrap_or_default();
    aof_records(&bytes).into_iter().filter(|record| record.len() >= 2 && record[1].starts_with("seed:"))
        .map(|record| record[1].clone()).collect::<BTreeSet<_>>().len()
}

// The group includes Redis's forked rewrite child and, for traced runs,
// strace. Keep process and thread IDs: SIGSTOP targets each subject process,
// while the final stopped-state witness must account for every thread.
#[derive(Debug)]
struct GroupTask {
    process: u32,
    thread: u32,
    state: char,
}

impl GroupTask {
    fn stopped(&self) -> bool { matches!(self.state, 'T' | 't' | 'Z') }
}

fn group_tasks(group: u32) -> Vec<GroupTask> {
    let mut result = Vec::new();
    for process in std::fs::read_dir("/proc").unwrap().filter_map(Result::ok) {
        let Ok(pid) = process.file_name().to_string_lossy().parse::<u32>() else { continue; };
        let Ok(tasks) = std::fs::read_dir(process.path().join("task")) else { continue; };
        for task in tasks.filter_map(Result::ok) {
            let Ok(stat) = std::fs::read_to_string(task.path().join("stat")) else { continue; };
            let Some((_, fields)) = stat.rsplit_once(") ") else { continue; };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if fields.get(2).and_then(|field| field.parse::<u32>().ok()) == Some(group) {
                result.push(GroupTask {
                    process: pid,
                    thread: task.file_name().to_string_lossy().parse().unwrap(),
                    state: fields[0].chars().next().unwrap(),
                });
            }
        }
    }
    result
}

fn signal_pid(pid: i32, signal: i32) -> bool {
    unsafe extern "C" { fn kill(pid: i32, signal: i32) -> i32; }
    if unsafe { kill(pid, signal) } == 0 { return true; }
    let error = std::io::Error::last_os_error();
    // A rewrite child may exit between reading /proc and signalling it.
    assert_eq!(error.raw_os_error(), Some(3), "signal {signal} to {pid}: {error}"); // ESRCH
    false
}

fn group_signal(child: &ProgramChild, signal: i32) {
    assert!(signal_pid(-(child.id() as i32), signal), "subject process group disappeared");
}

fn freeze(child: &ProgramChild, traced: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut signalled = BTreeSet::new();
    if !traced { group_signal(child, 19); } // SIGSTOP, including the forked writer.
    loop {
        let tasks = group_tasks(child.id());
        assert!(!tasks.is_empty(), "subject exited before crash point");
        if !traced {
            if tasks.iter().all(GroupTask::stopped) { return; }
        } else {
            // ptrace(2), Group-stop: a traced SIGSTOP first stops only its
            // recipient thread. The tracer must inject it before the other
            // threads stop. Stopping the entire group here also stops strace,
            // leaving sleeping siblings unable to finish that transition.
            // https://man7.org/linux/man-pages/man2/ptrace.2.html
            let subjects: Vec<_> = tasks.iter().filter(|task| task.process != child.id()).collect();
            assert!(subjects.iter().any(|task| task.state != 'Z'), "traced subject exited before crash point");
            for task in &subjects {
                if task.state != 'Z' && signalled.insert(task.process) {
                    signal_pid(task.process as i32, 19);
                }
            }
            if subjects.iter().all(|task| task.stopped()) {
                assert!(signal_pid(child.id() as i32, 19), "tracer exited before freeze");
                loop {
                    let held = group_tasks(child.id());
                    if held.iter().any(|task| task.thread == child.id() && task.state == 'T') {
                        // A 't' read while strace was alive may have been an
                        // ordinary syscall stop that strace subsequently
                        // resumed. Only this second scan, with strace itself
                        // stopped, establishes a stable freeze of all threads.
                        if held.iter().all(GroupTask::stopped) {
                            assert!(held.iter().any(|task| task.process != child.id() && task.state != 'Z'), "traced subject exited during freeze");
                            eprintln!("rewrite crash group frozen: {held:?}");
                            return;
                        }
                        // Resume only strace, never the stopped subject. It
                        // can now finish consuming the pending SIGSTOPs.
                        assert!(signal_pid(child.id() as i32, 18), "tracer exited during freeze"); // SIGCONT
                        break;
                    }
                    assert!(Instant::now() < deadline, "tracer did not stop: {held:?}");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        assert!(Instant::now() < deadline, "process group did not stop: {tasks:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn crash(child: ProgramChild) {
    let group = child.id();
    group_signal(&child, 9); // SIGKILL, never graceful shutdown.
    let output = child.wait_with_output().expect("reap killed server group");
    assert!(!output.status.success(), "SIGKILL subject exited successfully");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let live: Vec<_> = group_tasks(group).into_iter().filter(|task| task.state != 'Z').collect();
        if live.is_empty() { break; }
        assert!(Instant::now() < deadline, "rewrite descendants can still mutate AOF files: {live:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn copy_old_selection(directory: &Path, target: &Path, old: &str) {
    let destination = target.join("appendonlydir");
    std::fs::create_dir(&destination).unwrap();
    for (name, kind) in parts(old) {
        if kind != 'h' {
            std::fs::copy(directory.join("appendonlydir").join(&name), destination.join(&name))
                .unwrap_or_else(|error| panic!("old AOF lost {name} before cleanup: {error}"));
        }
    }
    std::fs::write(destination.join(MANIFEST), old).unwrap();
    consistent(target);
}

fn recovered(server: Server, oracle: &Path, directory: &Path, expected: &BTreeMap<String, String>, list: &[String]) {
    consistent(directory);
    let (child, mut client) = start(server, oracle, directory, false);
    for (key, value) in expected {
        client.write_all(&resp(&["GET", key])).unwrap();
        assert_eq!(bulk_reply(&mut client, "recovered acknowledged value"), *value, "{server:?}: {key}");
    }
    client.write_all(&resp(&["LRANGE", "list", "0", "-1"])).unwrap();
    let mut reply = format!("*{}\r\n", list.len()).into_bytes();
    for value in list { reply.extend(format!("${}\r\n{value}\r\n", value.len()).as_bytes()); }
    expect_replies(&mut client, &reply, "list order and multiplicity after replay");
    client.write_all(&resp(&["GET", "not-acknowledged"])).unwrap();
    expect_replies(&mut client, b"$-1\r\n", "incomplete command must never appear");
    client.write_all(&resp(&["DBSIZE"])).unwrap();
    assert_eq!(integer_reply(&mut client, "exact recovered keyspace"), (expected.len() + 1) as i64);
    consistent(directory);
    rewrite_stop(&mut client, child);
}

fn run(server: Server, oracle: &Path, point: Point) {
    let directory = fixture_directory();
    initialise(directory.path());
    let traced = !matches!(point, Point::EarlyScan);
    let (child, mut client) = start(server, oracle, directory.path(), traced);
    let mut expected = seed(&mut client);
    let mut list = vec!["before".to_owned()];
    client.write_all(&resp(&["INCR", "counter"])).unwrap();
    assert_eq!(integer_reply(&mut client, "initial INCR acknowledgement"), 1);
    expected.insert("counter".into(), "1".into());
    client.write_all(&resp(&["RPUSH", "list", "before"])).unwrap();
    assert_eq!(integer_reply(&mut client, "initial RPUSH acknowledgement"), 1);
    acknowledged(&mut client, &mut expected, "before".into(), "acknowledged-before-scan".into());
    appended(directory.path(), "before", "acknowledged-before-scan");
    rewrite_key_delay(&mut client, if matches!(point, Point::EarlyScan) { "2000" } else { "500" });
    rewrite_start(&mut client);
    assert_eq!(rewrite_info(&mut client)["aof_rewrite_in_progress"], "1");
    // Repeated overwrites detect resurrected old values as well as missing
    // keys. All responses are consumed: the only unacknowledged command is
    // deliberately incomplete and cannot have executed on either server.
    for index in 0..32 {
        client.write_all(&resp(&["INCR", "counter"])).unwrap();
        assert_eq!(integer_reply(&mut client, "during-scan INCR acknowledgement"), index + 2);
        expected.insert("counter".into(), (index + 2).to_string());
        let item = format!("during:{index}");
        client.write_all(&resp(&["RPUSH", "list", &item])).unwrap();
        assert_eq!(integer_reply(&mut client, "during-scan RPUSH acknowledgement"), index + 2);
        list.push(item);
        acknowledged(&mut client, &mut expected, format!("seed:{index}"), format!("during:{index}"));
        acknowledged(&mut client, &mut expected, format!("during:{index}"), format!("value:{index}"));
    }
    acknowledged(&mut client, &mut expected, "last".into(), "acknowledged-during-scan".into());
    appended(directory.path(), "last", "acknowledged-during-scan");
    let incomplete = resp(&["SET", "not-acknowledged", "must-not-appear"]);
    client.write_all(&incomplete[..incomplete.len() - 3]).unwrap();

    let mut old = manifest(directory.path());
    assert_eq!(active_base(&old), OLD_BASE);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let current = manifest(directory.path());
        let base = active_base(&current);
        if base == OLD_BASE { old = current; }
        let ready = match point {
            Point::EarlyScan => scan_records(server, directory.path()) >= 16,
            Point::BeforeManifest => directory.path().join("appendonlydir").join(NEW_BASE).is_file() && base == OLD_BASE,
            Point::AfterManifest => base == NEW_BASE,
        };
        if ready { break; }
        assert!(Instant::now() < deadline, "{server:?}: {point:?} never became observable");
        std::thread::sleep(Duration::from_millis(2));
    }
    freeze(&child, traced);
    let selected = active_base(&manifest(directory.path()));
    match point {
        Point::EarlyScan => {
            let scanned = scan_records(server, directory.path());
            assert!((16..SEEDS / 4).contains(&scanned), "kill must interrupt early scan, saw {scanned}/{SEEDS}");
            assert_eq!(selected, OLD_BASE, "early scan already published new base");
            eprintln!("{server:?}: early-scan kill witnessed {scanned}/{SEEDS} seed records");
        }
        Point::BeforeManifest | Point::AfterManifest => {
            assert_eq!(selected, if matches!(point, Point::BeforeManifest) { OLD_BASE } else { NEW_BASE }, "missed rename window");
            let bytes = std::fs::read(directory.path().join("appendonlydir").join(NEW_BASE)).unwrap();
            let seeds: BTreeSet<_> = aof_records(&bytes).into_iter().filter(|record| record.len() >= 2 && record[1].starts_with("seed:"))
                .map(|record| record[1].clone()).collect();
            assert_eq!(seeds.len(), SEEDS, "published base omitted scanned keys");
            let trace = std::fs::read_to_string(directory.path().join("rename.trace")).unwrap();
            assert!(trace.contains("rename"), "rename delay instrumentation never observed a rename");
            eprintln!("{server:?}: {point:?}, complete new base exists; selected base {selected}");
        }
    }
    consistent(directory.path());
    eprintln!("{server:?}: {point:?}; selected base={selected}; acknowledged SET replies={}; INCR replies=33; RPUSH replies=33; final keys={}; incomplete SET replies=0", SEEDS + 66, expected.len() + 1);
    let old_copy = fixture_directory();
    copy_old_selection(directory.path(), old_copy.path(), &old);
    crash(child);
    drop(client);
    // The saved authoritative selection proves the old AOF remained usable
    // even when a complete replacement or its new manifest already existed.
    recovered(server, oracle, old_copy.path(), &expected, &list);
    recovered(server, oracle, directory.path(), &expected, &list);
    eprintln!("{server:?}: {point:?}; old selection and crash directory both recovered all {} expected keys and exact counter/list multiplicities; unexpected keys=0", expected.len() + 1);
}

#[test]
fn rewrite_sigkill_early_scan_recovers_acknowledged_writes_against_redis() {
    let oracle = support::redis_oracle();
    for server in [Server::Redis, Server::Firn(false), Server::Firn(true)] {
        run(server, &oracle, Point::EarlyScan);
    }
}

#[test]
fn rewrite_sigkill_complete_base_before_manifest_recovers_against_redis() {
    let oracle = support::redis_oracle();
    for server in [Server::Redis, Server::Firn(false)] { run(server, &oracle, Point::BeforeManifest); }
}

#[test]
fn rewrite_sigkill_after_atomic_manifest_switch_recovers_against_redis() {
    let oracle = support::redis_oracle();
    for server in [Server::Redis, Server::Firn(false)] { run(server, &oracle, Point::AfterManifest); }
}

/// The server strace started: its only child.
fn traced_server(child: &ProgramChild) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let children = std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", child.id())).unwrap_or_default();
        if let Some(pid) = children.split_whitespace().next() { return pid.parse().unwrap(); }
        assert!(Instant::now() < deadline, "strace started no server");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Every line strace wrote with -ff under `prefix`, one file per thread, so
/// that no call is split by another thread's.
fn traced_lines(prefix: &Path) -> Vec<String> {
    let name = prefix.file_name().unwrap().to_str().unwrap().to_owned();
    let mut lines = Vec::new();
    for entry in std::fs::read_dir(prefix.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_str().unwrap().starts_with(&format!("{name}.")) {
            lines.extend(std::fs::read_to_string(path).unwrap().lines().map(str::to_owned));
        }
    }
    lines
}

/// When the bytes retained by a failed append drain after the cut and
/// before the base is complete, the rewrite rotates and installs as usual
/// (design/firn/aof-rewrite.md): the transaction sealed at the cut reaches
/// the old increment before the new one is published, and base(S1) is paired
/// only with the empty new increment. RLIMIT_FSIZE on the old increment makes
/// its appends fail; strace delays every sync of the temporary base by 2 s
/// (delay_enter, the result unchanged), so the base completes only after the
/// test lifts the limit at the observed cut. The trace must show the sealed
/// transaction written whole to the old increment, which a direct
/// installation never does, and the restart must apply its INCR and RPUSH
/// once. File adapter only: the native ring's I/O is not a syscall strace
/// can delay.
#[test]
fn rewrite_rotates_when_retained_bytes_drain_after_the_cut() {
    let directory = fixture_directory();
    initialise(directory.path());
    let root = directory.path().canonicalize().unwrap();
    let aof = root.join("appendonlydir");
    let base = aof.join("temp-appendonly.aof.base");
    let current = aof.join("appendonly.aof.1.incr.aof");
    let trace = root.join("drain.trace");
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let mut command = Command::new("strace");
    command.args(["-ff", "-qq", "-yy", "-s", "80", "-o"]).arg(&trace)
        .args(["-e", "trace=write,writev,pwrite64,pwritev,pwritev2,fsync,fdatasync",
            "-e", "inject=fsync,fdatasync:delay_enter=2s", "-P"]).arg(&base)
        .arg("-P").arg(&current).arg("--")
        .arg(std::env::var_os("FIRN").expect("FIRN"))
        .args([&port, "0", "appendonly.aof", "--auto-aof-rewrite-percentage", "0"])
        .env("WF_WORKERS", "2").env("WF_DRIVERS", "2").env_remove("WF_REQUIRE_WINDOWS_IOCP")
        .env("WF_IO_NO_NATIVE_RING", "1")
        .current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    {
        use std::os::unix::process::CommandExt;
        // SIG_IGN survives both execs, so RLIMIT_FSIZE makes write return
        // EFBIG instead of ending the server, as in aof_write_errors.
        unsafe extern "C" {
            fn signal(signal: i32, handler: usize) -> usize;
        }
        unsafe {
            command.pre_exec(|| {
                if signal(25, 1) == usize::MAX { Err(std::io::Error::last_os_error()) } else { Ok(()) }
            });
        }
    }
    let child = ProgramChild::spawn(&mut command).expect("spawn traced firn").with_port(&lease);
    let mut client = connect_when_ready(lease.port(), &child);
    loaded(&mut client, Server::Firn(false));
    let server = traced_server(&child);
    let status = std::fs::read_to_string(format!("/proc/{server}/status")).unwrap();
    let ignored = status.lines().find_map(|line| line.strip_prefix("SigIgn:")).unwrap().trim();
    assert_ne!(u64::from_str_radix(ignored, 16).unwrap() & (1 << 24), 0, "the server must ignore SIGXFSZ");

    client.write_all(&resp(&["INCR", "counter"])).unwrap();
    assert_eq!(integer_reply(&mut client, "INCR before the fault"), 1);
    client.write_all(&resp(&["RPUSH", "events", "before"])).unwrap();
    assert_eq!(integer_reply(&mut client, "RPUSH before the fault"), 1);
    // The limit is the old increment's length; padding makes it exceed the
    // base, so that only the old increment's appends fail.
    let padding = "p".repeat(1 << 16);
    acknowledged(&mut client, &mut BTreeMap::new(), "padding".into(), padding.clone());
    client.write_all(&resp(&["DEL", "padding"])).unwrap();
    assert_eq!(integer_reply(&mut client, "DEL padding"), 1);
    let mut expected = resp(&["INCR", "counter"]);
    expected.extend(resp(&["RPUSH", "events", "before"]));
    expected.extend(resp(&["SET", "padding", &padding]));
    expected.extend(resp(&["DEL", "padding"]));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let info = rewrite_info(&mut client);
        if std::fs::read(&current).unwrap() == expected && info["aof_buffer_length"] == "0"
            && info["aof_last_write_status"] == "ok" { break; }
        assert!(Instant::now() < deadline, "seed never reached the old increment: {info:?}");
        std::thread::sleep(Duration::from_millis(20));
    }

    let old = aof_write_errors::file_limit_pid(server, expected.len() as u64);
    client.write_all(&resp(&["MULTI"])).unwrap();
    expect_replies(&mut client, b"+OK\r\n", "MULTI");
    client.write_all(&resp(&["INCR", "counter"])).unwrap();
    expect_replies(&mut client, b"+QUEUED\r\n", "queued INCR");
    client.write_all(&resp(&["RPUSH", "events", "retained"])).unwrap();
    expect_replies(&mut client, b"+QUEUED\r\n", "queued RPUSH");
    client.write_all(&resp(&["EXEC"])).unwrap();
    expect_replies(&mut client, b"*2\r\n:2\r\n:2\r\n", "EXEC");
    let mut retained = resp(&["MULTI"]);
    retained.extend(resp(&["INCR", "counter"]));
    retained.extend(resp(&["RPUSH", "events", "retained"]));
    retained.extend(resp(&["EXEC"]));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = rewrite_info(&mut client);
        if info["aof_last_write_status"] == "err" { break; }
        assert!(Instant::now() < deadline, "the file limit never failed an append: {info:?}");
        std::thread::sleep(Duration::from_millis(10));
    }

    rewrite_start(&mut client);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let info = rewrite_info(&mut client);
        assert_eq!(info["aof_rewrite_in_progress"], "1", "the cut must be seen before completion: {info:?}");
        if info["firn_aof_rewrite_cut"] == "1" {
            assert_eq!(info["aof_last_write_status"], "err", "the cut sealed retained bytes: {info:?}");
            assert_eq!(info["aof_buffer_length"], retained.len().to_string(), "{info:?}");
            break;
        }
        assert!(Instant::now() < deadline, "the rewrite never cut: {info:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
    aof_write_errors::file_limit_pid(server, old.soft);
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "{info:?}");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "", "{info:?}");
    assert_eq!(info["aof_last_write_status"], "ok", "{info:?}");
    assert_eq!(manifest(&root), rewrite_manifest(2, 2));
    consistent(&root);
    let lines = traced_lines(&trace);
    let target = current.to_str().unwrap();
    let whole = format!("= {}", retained.len());
    assert!(lines.iter().any(|line| line.contains(target) && line.contains("MULTI") && line.ends_with(&whole)),
        "the sealed transaction was never written whole to the old increment: {lines:#?}");
    let temporary = base.to_str().unwrap();
    assert!(lines.iter().any(|line| line.contains(temporary) && line.contains("DELAYED")),
        "the temporary base's sync was never delayed: {lines:#?}");
    rewrite_stop(&mut client, child);

    let (child, mut client) = start(Server::Firn(false), Path::new("unused"), &root, false);
    client.write_all(&resp(&["GET", "counter"])).unwrap();
    expect_replies(&mut client, b"$1\r\n2\r\n", "each INCR applies once");
    client.write_all(&resp(&["LRANGE", "events", "0", "-1"])).unwrap();
    expect_replies(&mut client, b"*2\r\n$6\r\nbefore\r\n$8\r\nretained\r\n", "each RPUSH applies once");
    client.write_all(&resp(&["DBSIZE"])).unwrap();
    assert_eq!(integer_reply(&mut client, "exact keyspace"), 2);
    rewrite_stop(&mut client, child);
}
