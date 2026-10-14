//! Concurrent histories and process-crash recovery (board firn-bl-01-02-rest).
//!
//! Redis 7.2.16 src/t_string.c:697-732 defines APPEND's byte-length reply.
//! src/server.c:1719-1739 calls the AOF flush before pending client replies;
//! src/aof.c:1591-1626 cuts an incomplete final record with aof-load-truncated
//! yes. These cases concern SIGKILL, which preserves the kernel page cache,
//! rather than power loss. Firn currently has only the everysec sync policy
//! (design/firn/append-only-file.md); there is no always variant to invoke.
//! The no-lost-ack assertion is the requested acceptance criterion: firn's
//! reply path does not wait for its 10 ms AOF writer, so it may fail today.
//! Even Redis everysec can postpone an append during a background fsync
//! (src/aof.c:1096-1123); page-cache retention protects appended bytes only.

#![cfg(target_os = "linux")]

use super::*;
use super::append_history::{Append, check_appends, SHARED_KEYS, TOKEN_BYTES};
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

const CLIENTS: usize = 8;
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
