//! Concurrent histories and process-crash recovery (board firn-bl-01-02-rest).
//!
//! Redis 7.2.16 src/t_string.c:697-732 defines APPEND's byte-length reply.
//! src/server.c:1719-1739 calls the AOF flush before pending client replies;
//! src/aof.c:1591-1626 cuts an incomplete final record with aof-load-truncated
//! yes. These cases concern SIGKILL, which preserves the kernel page cache,
//! rather than power loss. Firn currently has only the everysec sync policy
//! (design/firn/append-only-file.md); there is no always variant to invoke.
//! The no-lost-ack assertion is the requested acceptance criterion for the
//! reply-after-append barrier; the former 10 ms writer exposed this race.
//! Even Redis everysec can postpone an append during a background fsync
//! (src/aof.c:1096-1123); page-cache retention protects appended bytes only.

#![cfg(target_os = "linux")]

use super::*;
use std::collections::HashSet;
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

const CLIENTS: usize = 8;
const SHARED_KEYS: usize = 4;
const TOKEN_BYTES: usize = 16;
const CONCURRENT_DURATION: Duration = Duration::from_secs(2);
const WRITE_PAUSE: Duration = Duration::from_millis(2);
const IO_LIMIT: Duration = Duration::from_secs(3);
const ACK_WAIT_LIMIT: Duration = Duration::from_secs(3);
const KILL_ROUNDS: usize = 5;
const KILL_MIN_MS: u64 = 150;
const KILL_JITTER_MS: u64 = 150;
const RELOAD_BATCH: usize = 64;
const AOF_NAME: &str = "durability.aof";

fn peer(port: u16, monitor: &support::ProgramMonitor) -> TcpStream {
    let stream = connect_when_ready(port, monitor);
    stream.set_nodelay(true).expect("disable peer Nagle delay");
    stream.set_read_timeout(Some(IO_LIMIT)).expect("bound durability reads");
    stream.set_write_timeout(Some(IO_LIMIT)).expect("bound durability writes");
    stream
}

fn start(
    program: &CompiledProgram,
    lease: &support::PortLease,
    aof: bool,
) -> (ProgramChild, TcpStream) {
    let port = lease.port().to_string();
    let name = if aof { AOF_NAME } else { "-" };
    let child = program.spawn_on_route_with(
        true,
        &[("WF_WORKERS", "4"), ("WF_DRIVERS", "2")],
        &[port.as_bytes(), b"0", name.as_bytes(), b"0",
          b"--auto-aof-rewrite-percentage", b"0"],
    ).with_port(lease);
    let control = peer(lease.port(), &child);
    (child, control)
}

fn stop(child: ProgramChild, control: &mut TcpStream) {
    control.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("request orderly stop");
    shutdown_finished(child);
    assert_eq!(control.read(&mut [0]).unwrap(), 0, "shutdown closes control peer");
}

fn start_aof_route(
    program: &CompiledProgram,
    lease: &support::PortLease,
    native: bool,
) -> (ProgramChild, TcpStream) {
    let port = lease.port().to_string();
    let child = program.spawn_on_route_with(
        native,
        &[("WF_WORKERS", "4"), ("WF_DRIVERS", "2")],
        &[port.as_bytes(), b"0", AOF_NAME.as_bytes(), b"0",
          b"--auto-aof-rewrite-percentage", b"0"],
    ).with_port(lease);
    let control = peer(lease.port(), &child);
    (child, control)
}

/// Inspect the file immediately after a reply, without polling INFO, sleeping,
/// requesting a flush or stopping the server. Later pipelined commands may
/// already be appended, but this acknowledged prefix must be complete.
fn assert_appended_prefix(path: &std::path::Path, expected: &[u8], context: &str) {
    let actual = std::fs::read(path).expect("read acknowledged AOF bytes");
    assert!(actual.starts_with(expected),
        "{context}: acknowledged AOF prefix is not written: expected={expected:?}, actual={actual:?}");
}

/// Redis 7.2.16 server.c:1723-1739 flushes before sending pending replies;
/// t_string.c's SETNX refusal and db.c's absent DEL produce no propagation.
/// The byte checks expose the old reply-before-10-ms-writer window. Reply
/// ordering itself is a regression check for reads and no-record commands
/// sharing a connection with writes held behind the new append barrier.
#[test]
fn aof_pipeline_replies_stay_in_order_after_their_covering_append() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let lease = support::lease_port();
        let (child, mut client) = start_aof_route(&program, &lease, native);
        let path = aof_incremental_path(program.working_directory(), AOF_NAME);
        let mut expected_aof = Vec::new();
        for round in 0..12 {
            let key = format!("pipeline:{round}");
            let other = format!("pipeline:other:{round}");
            let absent = format!("pipeline:absent:{round}");
            let commands: &[(&[&str], &str, bool)] = &[
                (&["GET", &absent], "$-1\r\n", false),
                (&["SET", &key, "first"], "+OK\r\n", true),
                (&["GET", &key], "$5\r\nfirst\r\n", false),
                (&["SETNX", &key, "ignored"], ":0\r\n", false),
                (&["GET", &key], "$5\r\nfirst\r\n", false),
                (&["MSET", &key, "second", &other, "third"], "+OK\r\n", true),
                (&["GET", &key], "$6\r\nsecond\r\n", false),
                (&["DEL", &absent], ":0\r\n", false),
                (&["GET", &other], "$5\r\nthird\r\n", false),
                (&["APPEND", &key, "!"], ":7\r\n", true),
                (&["GET", &key], "$7\r\nsecond!\r\n", false),
            ];
            let requests: Vec<_> = commands.iter().flat_map(|(args, _, _)| resp(args)).collect();
            client.write_all(&requests).expect("send interleaved AOF pipeline");
            for (args, expected_reply, recorded) in commands {
                let context = format!("native={native}, round={round}, {}", args.join(" "));
                assert_eq!(whole_reply(&mut client, &context), *expected_reply, "{context}");
                if *recorded {
                    expected_aof.extend(resp(args));
                    assert_appended_prefix(&path, &expected_aof, &context);
                }
            }
            assert_eq!(std::fs::read(&path).unwrap(), expected_aof,
                "reads, refused SETNX and absent DEL add no AOF record");
            // FUNCTION FLUSH records an effect even with no functions loaded:
            // its reply needs an append token although no dataset key changed.
            assert_eq!(memory_request(&mut client, &["FUNCTION", "FLUSH"]), "+OK\r\n");
            expected_aof.extend(resp(&["FUNCTION", "FLUSH"]));
            assert_eq!(std::fs::read(&path).unwrap(), expected_aof,
                "FUNCTION FLUSH must be appended before its reply");
        }
        stop(child, &mut client);
    }
}

/// Redis 7.2.16 server.c:3348-3380 brackets multiple propagated effects in
/// MULTI/EXEC, including scripts. Checking those complete bytes immediately
/// after EXEC/EVAL/EVALSHA detects an outer reply losing its effects' barrier
/// token; replay alone or a check after orderly shutdown would hide the gap.
#[test]
fn aof_exec_and_script_replies_wait_for_all_effect_records_on_both_routes() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let lease = support::lease_port();
        let (child, mut client) = start_aof_route(&program, &lease, native);
        let path = aof_incremental_path(program.working_directory(), AOF_NAME);
        let source = "redis.call('SET',KEYS[1],'alpha'); redis.call('SET',KEYS[2],'beta'); return redis.call('GET',KEYS[1])";
        let sha = memory_request(&mut client, &["SCRIPT", "LOAD", source]);
        let sha = sha.strip_prefix("$40\r\n").and_then(|s| s.strip_suffix("\r\n"))
            .expect("SCRIPT LOAD returns a SHA1").to_owned();
        let mut expected_aof = Vec::new();
        for round in 0..6 {
            let tx = format!("exec:{round}");
            let commands: &[&[&str]] = &[
                &["MULTI"], &["SET", &tx, "first"], &["APPEND", &tx, "!"],
                &["GET", &tx], &["EXEC"],
            ];
            let requests: Vec<_> = commands.iter().flat_map(|args| resp(args)).collect();
            client.write_all(&requests).expect("send AOF transaction");
            expect_replies(&mut client,
                b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n+OK\r\n:6\r\n$6\r\nfirst!\r\n",
                "transaction reply order");
            for args in [&["MULTI"][..], &["SET", &tx, "first"], &["APPEND", &tx, "!"], &["EXEC"]] {
                expected_aof.extend(resp(args));
            }
            assert_appended_prefix(&path, &expected_aof, "EXEC complete effect block");

            let first = format!("script:{round}:first");
            let second = format!("script:{round}:second");
            assert_eq!(memory_request(&mut client, &["EVALSHA", &sha, "2", &first, &second]),
                "$5\r\nalpha\r\n");
            for args in [&["MULTI"][..], &["SET", &first, "alpha"], &["SET", &second, "beta"], &["EXEC"]] {
                expected_aof.extend(resp(args));
            }
            assert_appended_prefix(&path, &expected_aof, "declared EVALSHA complete effect block");

            let fallback = "redis.call('SET','fallback:first','gamma'); redis.call('SET','fallback:second','delta'); return redis.call('GET','fallback:second')";
            assert_eq!(memory_request(&mut client, &["EVAL", fallback, "0"]), "$5\r\ndelta\r\n");
            for args in [
                &["MULTI"][..], &["SET", "fallback:first", "gamma"],
                &["SET", "fallback:second", "delta"], &["EXEC"],
            ] {
                expected_aof.extend(resp(args));
            }
            assert_appended_prefix(&path, &expected_aof, "fallback EVAL complete effect block");
            assert_eq!(memory_request(&mut client, &["EVAL", "return redis.call('GET',KEYS[1])", "1", &first]),
                "$5\r\nalpha\r\n");
            assert_eq!(std::fs::read(&path).unwrap(), expected_aof,
                "queued GET and read-only script add no AOF record");
        }
        stop(child, &mut client);
    }
}

#[derive(Clone)]
struct Append {
    key: usize,
    token: String,
    length: i64,
    invoked: Instant,
    completed: Instant,
}

/// A serial witness independent of firn: a key's reply ranks must be exactly
/// 1..N and its bytes must be the corresponding tokens in that order. The
/// rank order must also fit the invocation/completion intervals. In rank
/// order, the latest invocation in each prefix must be no later than that
/// operation's completion; otherwise a higher rank finished before a lower
/// rank was invoked. This gives a linearization within each key's intervals;
/// independent keys compose, without searching possible global orders.
fn check_appends(history: &[Append], final_values: &[String]) -> Result<(), String> {
    if final_values.len() != SHARED_KEYS {
        return Err("wrong number of final keys".to_owned());
    }
    let mut tokens = HashSet::new();
    for event in history {
        if event.key >= SHARED_KEYS || event.token.len() != TOKEN_BYTES
            || event.completed < event.invoked || !tokens.insert(&event.token) {
            return Err("invalid or duplicate history token/interval/key".to_owned());
        }
    }
    for (key, final_value) in final_values.iter().enumerate() {
        let mut events: Vec<_> = history.iter().filter(|event| event.key == key).collect();
        events.sort_unstable_by_key(|event| event.length);
        let mut expected = String::new();
        let mut latest_invocation = None;
        for (index, event) in events.iter().enumerate() {
            let expected_length = ((index + 1) * TOKEN_BYTES) as i64;
            if event.length != expected_length {
                return Err(format!("key {key}: duplicate/missing reply rank: wanted \
                    {expected_length}, got {}", event.length));
            }
            let latest = latest_invocation.map_or(event.invoked, |previous: Instant| {
                previous.max(event.invoked)
            });
            if latest > event.completed {
                return Err(format!("key {key}: reply order contradicts real-time order"));
            }
            latest_invocation = Some(latest);
            expected.push_str(&event.token);
        }
        if *final_value != expected {
            return Err(format!("key {key}: final value differs from acknowledged token order \
                (expected {} bytes, got {})", expected.len(), final_value.len()));
        }
    }
    Ok(())
}

fn read_shared_keys(control: &mut TcpStream) -> Vec<String> {
    (0..SHARED_KEYS).map(|key| {
        let name = format!("concurrent:{key}");
        control.write_all(&resp(&["GET", &name])).unwrap();
        bulk_reply(control, &name)
    }).collect()
}

#[test]
fn concurrent_appends_have_a_serial_witness_and_aof_reloads_the_same_state() {
    for aof in [false, true] {
        let program = CompiledProgram::from_environment();
        let lease = support::lease_port();
        let (child, mut control) = start(&program, &lease, aof);
        for key in 0..SHARED_KEYS {
            assert_eq!(memory_request(&mut control, &["SET", &format!("concurrent:{key}"), ""]),
                "+OK\r\n");
        }
        let peers: Vec<_> = (0..CLIENTS).map(|_| peer(lease.port(), &child)).collect();
        let barrier = Barrier::new(CLIENTS);
        let histories = std::thread::scope(|scope| {
            let handles: Vec<_> = peers.into_iter().enumerate().map(|(client, mut stream)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let deadline = Instant::now() + CONCURRENT_DURATION;
                    let mut history = Vec::new();
                    let mut sequence = 0_u64;
                    while Instant::now() < deadline {
                        let key = (client + sequence as usize) % SHARED_KEYS;
                        let token = format!("{client:02x}{sequence:014x}");
                        let request = resp(&["APPEND", &format!("concurrent:{key}"), &token]);
                        let invoked = Instant::now();
                        stream.write_all(&request).expect("send concurrent APPEND");
                        let length = integer_reply(&mut stream, "concurrent APPEND");
                        history.push(Append { key, token, length, invoked, completed: Instant::now() });
                        sequence += 1;
                        std::thread::sleep(WRITE_PAUSE);
                    }
                    assert!(history.len() >= SHARED_KEYS, "client {client} did not visit every key");
                    history
                })
            }).collect();
            handles.into_iter().flat_map(|handle| handle.join().expect("APPEND client panicked"))
                .collect::<Vec<_>>()
        });
        let before = read_shared_keys(&mut control);
        check_appends(&histories, &before).unwrap_or_else(|error| panic!("AOF={aof}: {error}"));
        stop(child, &mut control);
        if aof {
            let (child, mut control) = start(&program, &lease, true);
            let reloaded = read_shared_keys(&mut control);
            assert_eq!(reloaded, before, "replay preserves every key's final token order");
            check_appends(&histories, &reloaded).expect("reloaded serial witness");
            stop(child, &mut control);
        }
    }
}

#[test]
fn append_checker_rejects_lost_updates_duplicate_replies_and_impossible_orders() {
    let begin = Instant::now();
    let history = vec![
        Append { key: 0, token: "0000000000000001".into(), length: 16,
            invoked: begin, completed: begin + Duration::from_millis(1) },
        Append { key: 0, token: "0100000000000001".into(), length: 32,
            invoked: begin + Duration::from_millis(2), completed: begin + Duration::from_millis(3) },
    ];
    let mut values = vec![String::new(); SHARED_KEYS];
    values[0] = history.iter().map(|event| event.token.as_str()).collect();
    assert_eq!(check_appends(&history, &values), Ok(()), "valid serial history");

    let mut overlapping = history.clone();
    overlapping[0].completed = begin + Duration::from_millis(3);
    overlapping[1].invoked = begin;
    overlapping[1].completed = begin + Duration::from_millis(1);
    assert_eq!(check_appends(&overlapping, &values), Ok(()),
        "overlapping operations may receive replies out of rank order");

    let mut lost = values.clone();
    lost[0].truncate(TOKEN_BYTES);
    assert!(check_appends(&history, &lost).unwrap_err().contains("final value"));
    let mut duplicate = history.clone();
    duplicate[1].length = duplicate[0].length;
    assert!(check_appends(&duplicate, &values).unwrap_err().contains("reply rank"));
    let mut missing_rank = history.clone();
    missing_rank[1].length = 48;
    assert!(check_appends(&missing_rank, &values).unwrap_err().contains("reply rank"));
    let mut reversed = values.clone();
    reversed[0] = format!("{}{}", history[1].token, history[0].token);
    assert!(check_appends(&history, &reversed).unwrap_err().contains("final value"));
    let mut impossible = history.clone();
    impossible[0].invoked = history[1].completed + Duration::from_millis(1);
    impossible[0].completed = impossible[0].invoked;
    assert!(check_appends(&impossible, &values).unwrap_err().contains("real-time"));
}

struct Acknowledged {
    key: String,
    value: String,
}

/// The crash workload uses only simple SET replies and bulk GET replies.
/// Unlike network.rs's assertion reader, an incomplete frame returns its
/// I/O error: a partially received +OK is never an acknowledgment.
fn read_crash_reply(reader: &mut impl Read) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind};
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n") {
        if bytes.len() == 128 {
            return Err(Error::new(ErrorKind::InvalidData, "crash reply header too long"));
        }
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    if bytes.starts_with(b"$") {
        let length = std::str::from_utf8(&bytes[1..bytes.len() - 2])
            .ok().and_then(|digits| digits.parse::<i64>().ok())
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "invalid crash bulk length"))?;
        if !(-1..=1024).contains(&length) {
            return Err(Error::new(ErrorKind::InvalidData, "unexpected crash bulk length"));
        }
        if length >= 0 {
            // All crash values are short ASCII strings; bound a corrupt
            // length before allocating, and require the complete terminator.
            let mut body = vec![0_u8; length as usize + 2];
            reader.read_exact(&mut body)?;
            if !body.ends_with(b"\r\n") {
                return Err(Error::new(ErrorKind::InvalidData, "invalid crash bulk terminator"));
            }
            bytes.extend(body);
        }
    } else if !bytes.starts_with(b"+") && !bytes.starts_with(b"-") {
        return Err(Error::new(ErrorKind::InvalidData, "unexpected crash reply type"));
    }
    String::from_utf8(bytes)
        .map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

fn interrupted_by_kill(error: &std::io::Error, killed: bool) -> bool {
    use std::io::ErrorKind;
    killed && matches!(error.kind(),
        ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe)
}

#[test]
fn crash_reader_requires_complete_replies_and_transport_errors_require_a_kill() {
    use std::io::{Cursor, Error, ErrorKind};
    struct InterruptedReader<'a> {
        prefix: &'a [u8],
        kind: ErrorKind,
    }
    impl Read for InterruptedReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.prefix.is_empty() {
                Err(Error::from(self.kind))
            } else {
                self.prefix.read(buffer)
            }
        }
    }
    let ack = b"+OK\r\n";
    assert_eq!(read_crash_reply(&mut Cursor::new(ack)).unwrap(), "+OK\r\n");
    for length in 0..ack.len() {
        let error = read_crash_reply(&mut Cursor::new(&ack[..length])).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "prefix length {length}");
        assert!(!interrupted_by_kill(&error, false));
        assert!(interrupted_by_kill(&error, true));
    }
    for kind in [ErrorKind::ConnectionReset, ErrorKind::BrokenPipe] {
        let error = read_crash_reply(&mut InterruptedReader { prefix: &ack[..3], kind })
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(!interrupted_by_kill(&error, false));
        assert!(interrupted_by_kill(&error, true));
    }
    for kind in [ErrorKind::TimedOut, ErrorKind::WouldBlock, ErrorKind::InvalidData] {
        assert!(!interrupted_by_kill(&Error::from(kind), true));
    }
    assert_eq!(read_crash_reply(&mut Cursor::new(b"$-1\r\n")).unwrap(), "$-1\r\n");
    assert_eq!(read_crash_reply(&mut Cursor::new(b"$1\r\nv\r\n")).unwrap(), "$1\r\nv\r\n");
    assert_eq!(read_crash_reply(&mut Cursor::new(b"$1\r\nv\r")).unwrap_err().kind(),
        ErrorKind::UnexpectedEof);
}

fn kill_during_writes(child: ProgramChild, peers: Vec<TcpStream>, round: usize, delay: Duration)
    -> Vec<Acknowledged>
{
    let killed = AtomicBool::new(false);
    // Error handling and signal publication share a gate: a reader reset
    // caused by kill cannot race ahead of the successful kill's flag store.
    let kill_gate = Mutex::new(());
    let acknowledgment_count = AtomicUsize::new(0);
    let barrier = Barrier::new(CLIENTS + 1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = peers.into_iter().enumerate().map(|(client, mut stream)| {
            let killed = &killed;
            let kill_gate = &kill_gate;
            let acknowledgment_count = &acknowledgment_count;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                let mut acknowledged = Vec::new();
                let mut sequence = 0_u64;
                let deadline = Instant::now() + ACK_WAIT_LIMIT + delay + IO_LIMIT;
                while !killed.load(Ordering::Acquire) {
                    assert!(Instant::now() < deadline, "round {round} writer exceeded deadline");
                    // Never reuse a key, even across restarts. A completed
                    // unacknowledged SET cannot overwrite an acknowledged one.
                    let key = format!("kill:{round}:{client}:{sequence}");
                    let value = format!("value:{round}:{client}:{sequence}:acknowledged");
                    let outcome = stream.write_all(&resp(&["SET", &key, &value]))
                        .and_then(|()| read_crash_reply(&mut stream));
                    match outcome {
                        Ok(reply) => {
                            assert_eq!(reply, "+OK\r\n", "unexpected SET reply for {key}");
                            // Record success even if SIGKILL raced with reading
                            // the buffered reply; all threads join before checking.
                            acknowledged.push(Acknowledged { key, value });
                            acknowledgment_count.fetch_add(1, Ordering::Release);
                        }
                        Err(error) => {
                            let _gate = kill_gate.lock().expect("lock crash signal gate");
                            assert!(interrupted_by_kill(&error, killed.load(Ordering::Acquire)),
                                "client {client} unexpected transport error in round {round}: {error}");
                            break;
                        }
                    }
                    sequence += 1;
                    std::thread::sleep(WRITE_PAUSE);
                }
                acknowledged
            })
        }).collect();
        barrier.wait();
        let ack_deadline = Instant::now() + ACK_WAIT_LIMIT;
        while acknowledgment_count.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < ack_deadline,
                "round {round} received no acknowledgments before the kill deadline");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(delay);
        assert!(acknowledgment_count.load(Ordering::Acquire) > 0,
            "round {round} must acknowledge writes before SIGKILL");
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        {
            let _gate = kill_gate.lock().expect("lock crash signal gate");
            assert_eq!(unsafe { kill(child.id() as i32, 9) }, 0,
                "SIGKILL firn: {}", std::io::Error::last_os_error());
            killed.store(true, Ordering::Release);
        }
        let output = child.wait_with_output().expect("reap SIGKILLed firn");
        assert_eq!(output.status.signal(), Some(9), "firn must exit from SIGKILL: {output:?}");
        let acknowledged: Vec<_> = handles.into_iter()
            .flat_map(|handle| handle.join().expect("crash writer panicked")).collect();
        assert!(!acknowledged.is_empty(), "round {round} must check acknowledged writes");
        assert_eq!(acknowledged.len(), acknowledgment_count.load(Ordering::Acquire),
            "every complete acknowledgment must be retained for reload checks");
        acknowledged
    })
}

#[derive(Default)]
struct ReloadFailures {
    observations: usize,
    examples: Vec<String>,
}

/// Pipeline bounded GET batches, but compare each reply with that client's
/// recorded acknowledgment, never with an AOF-derived or pre-kill snapshot.
fn check_acknowledged(control: &mut TcpStream, acknowledged: &[Acknowledged],
    context: &str, failures: &mut ReloadFailures)
{
    for batch in acknowledged.chunks(RELOAD_BATCH) {
        let requests: Vec<_> = batch.iter().flat_map(|ack| resp(&["GET", &ack.key])).collect();
        control.write_all(&requests).expect("send acknowledgment reload checks");
        for ack in batch {
            let actual = read_crash_reply(control).unwrap_or_else(|error| {
                panic!("{context}: reload GET {} failed: {error}; acknowledgment must still be checked",
                    ack.key)
            });
            let expected = format!("${}\r\n{}\r\n", ack.value.len(), ack.value);
            if actual != expected {
                failures.observations += 1;
                if failures.examples.len() < 8 {
                    failures.examples.push(format!("{context}: {} expected {:?}, got {actual:?}",
                        ack.key, ack.value));
                }
            }
        }
    }
}

#[test]
fn sigkill_restarts_preserve_every_acknowledged_write_and_repair_a_truncated_tail() {
    let program = CompiledProgram::from_environment();
    let lease = support::lease_port();
    let (mut child, mut control) = start(&program, &lease, true);
    let mut random = std::fs::File::open("/dev/urandom").expect("open kill timing randomness");
    let mut acknowledged = Vec::new();
    let mut failures = ReloadFailures::default();
    for round in 0..KILL_ROUNDS {
        let peers = (0..CLIENTS).map(|_| peer(lease.port(), &child)).collect();
        let mut bytes = [0_u8; 8];
        random.read_exact(&mut bytes).expect("randomize SIGKILL moment");
        let delay = Duration::from_millis(KILL_MIN_MS + u64::from_ne_bytes(bytes) % KILL_JITTER_MS);
        drop(control);
        acknowledged.extend(kill_during_writes(child, peers, round, delay));
        (child, control) = start(&program, &lease, true);
        // Do not wait for INFO/aof_buffer_length or request a flush before
        // killing: either would hide acknowledgment-before-append loss.
        // Retain failures until the end so all five crash/restart rounds run.
        check_acknowledged(&mut control, &acknowledged, &format!("round {round}, kill after {delay:?}"),
            &mut failures);
    }
    stop(child, &mut control);

    // A small SET may reach the file in one write, so random kills need not
    // produce a partial record. Separately guarantee that recovery path on
    // the same accumulated AOF, after checking each unmodified crash file.
    let path = aof_incremental_path(program.working_directory(), AOF_NAME);
    let whole_length = std::fs::metadata(&path).unwrap().len();
    let tail = resp(&["SET", "kill:partial-tail", "unacknowledged"]);
    std::fs::OpenOptions::new().append(true).open(&path).unwrap()
        .write_all(&tail[..tail.len() - 3]).expect("write incomplete last AOF record");
    let (child, mut control) = start(&program, &lease, true);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), whole_length,
        "startup cuts only the incomplete tail, as Redis aof-load-truncated yes does");
    assert_eq!(memory_request(&mut control, &["GET", "kill:partial-tail"]), "$-1\r\n",
        "incomplete SET must not be applied");
    check_acknowledged(&mut control, &acknowledged, "truncated-tail reload", &mut failures);
    let after = Acknowledged { key: "kill:after-tail".into(), value: "new-record".into() };
    assert_eq!(memory_request(&mut control, &["SET", &after.key, &after.value]), "+OK\r\n");
    acknowledged.push(after);
    stop(child, &mut control);
    let (child, mut control) = start(&program, &lease, true);
    check_acknowledged(&mut control, &acknowledged, "reload after appending beyond repaired tail",
        &mut failures);
    stop(child, &mut control);
    assert_eq!(failures.observations, 0,
        "acknowledged writes missing or changed after restart (including repeated observations): {}",
        failures.examples.join("\n"));
}
