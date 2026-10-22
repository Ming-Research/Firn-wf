//! Concurrent histories checked against a serial order (board item
//! firn-concurrent-check-gate).
//!
//! Redis 7.2.16 src/t_string.c:697-727 defines APPEND's byte-length reply.
//! Eight clients interleave APPENDs on shared keys, and the checker below
//! finds a serial order that explains every reply and the final values, which
//! the append-only file must then reproduce on reload.

#![cfg(target_os = "linux")]

use super::*;
use std::collections::HashSet;
use std::sync::Barrier;

const CLIENTS: usize = 8;
const SHARED_KEYS: usize = 4;
const TOKEN_BYTES: usize = 16;
const CONCURRENT_DURATION: Duration = Duration::from_secs(2);
const WRITE_PAUSE: Duration = Duration::from_millis(2);
const IO_LIMIT: Duration = Duration::from_secs(3);
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
