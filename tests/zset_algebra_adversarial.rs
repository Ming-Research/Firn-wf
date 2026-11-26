//! Adversarial cases for the sorted-set algebra commands, ZRANGESTORE and ZMPOP
//! (board item 171), beyond the byte-for-byte cases of `zset_algebra.rs`.
//!
//! The reference is Redis 7.2.16: the redis-server `REDIS_SERVER` names gets the
//! same requests as firn on connections of the same number, and every reply is
//! compared byte for byte. What no reply of one connection shows is read from
//! what Redis 7.2.16's src/t_zset.c, src/multi.c and src/db.c do:
//! `signalModifiedKey` for a store that changes its destination (WATCH),
//! `setKey` dropping a destination's expiry, a partial pop keeping the
//! source's expiry, scores of negative zero printed by `d2string` as `-0`, and
//! one atomic command under concurrent connections (a command never sees half of
//! another's change).

use super::*;
use super::pubsub::request;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

/// One server and the connections open to it.
struct Side {
    child: ProgramChild,
    clients: Vec<TcpStream>,
}

impl Side {
    fn say(&mut self, who: usize, command: &str) -> Vec<u8> {
        let words: Vec<&str> = command.split(' ').collect();
        request(&mut self.clients[who], &words)
    }
}

fn open(port: u16, child: &ProgramChild, connections: usize) -> Vec<TcpStream> {
    (0..connections)
        .map(|_| {
            let stream = connect_when_ready(port, child);
            stream.set_nodelay(true).expect("send requests promptly");
            stream.set_read_timeout(Some(Duration::from_secs(60))).expect("bound the waits");
            stream
        })
        .collect()
}

fn spawn_redis(directory: &Path, connections: usize) -> Side {
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let mut command = support::command(support::redis_oracle());
    command.current_dir(directory)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .args(["--port", &text, "--bind", "127.0.0.1", "--save", "", "--appendonly", "no", "--dir"])
        .arg(directory);
    let child = ProgramChild::spawn(&mut command).expect("spawn redis-server").with_port(&lease);
    let clients = open(port, &child, connections);
    Side { child, clients }
}

fn spawn_firn(program: &CompiledProgram, connections: usize) -> Side {
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let child = program.spawn_on_route_with(true, &[("WF_DRIVERS", "4")], &[
        b"--port", text.as_bytes(), b"--bind", b"127.0.0.1", b"--firn-clients", b"0",
    ]).with_port(&lease);
    let clients = open(port, &child, connections);
    Side { child, clients }
}

fn stop(mut side: Side) {
    side.clients[0].write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("request the stop");
    let mut byte = [0_u8; 1];
    assert_eq!(side.clients[0].read(&mut byte).expect("shutdown closes the connection"), 0);
    let output = side.child.wait_with_output().expect("wait for the server");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Sends each step (connection, command) to both servers and requires equal
/// replies; returns the replies of Redis with their steps.
fn run(label: &str, firn: &mut Side, oracle: &mut Side, steps: &[(usize, String)]) -> Vec<(String, Vec<u8>)> {
    let mut replies = Vec::new();
    for (who, command) in steps {
        let want = oracle.say(*who, command);
        let got = firn.say(*who, command);
        if command.starts_with("HELLO") {
            continue;
        }
        assert_eq!(text(&got), text(&want), "{label}: connection {who} sent {command:?}; firn must answer as Redis 7.2.16 does");
        replies.push((command.clone(), want));
    }
    replies
}

fn steps(connection: usize, commands: &[&str]) -> Vec<(usize, String)> {
    commands.iter().map(|command| (connection, (*command).to_owned())).collect()
}

/// A write by another connection aborts a transaction that watches the key
/// exactly when Redis's command signals the key as modified: a store or pop that
/// changed its key, whether or not the value came out the same, and not a
/// read, a refusal, nor a store of an empty result over an absent key.
#[test]
fn zset_writes_signal_watched_keys_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = spawn_redis(directory.path(), 2);
    let mut firn = spawn_firn(&program, 2);
    let base = [
        "FLUSHALL", "ZADD a 1 x 2 y 3 z 4 w", "ZADD b 10 y 20 z 30 v", "ZADD out 9 old", "SET str hello",
        "ZADD ttl 1 p 2 q 3 r",
    ];
    let cases: &[(&str, &str)] = &[
        ("out", "ZUNIONSTORE out 2 a b"), ("out", "ZINTERSTORE out 2 a b"), ("out", "ZDIFFSTORE out 2 a b"),
        ("out", "ZRANGESTORE out a 0 -1"), ("out", "ZUNIONSTORE out 1 nosuch"), ("out", "ZINTERSTORE out 2 a nosuch"),
        ("out", "ZDIFFSTORE out 1 nosuch"), ("out", "ZRANGESTORE out a 50 60"), ("out", "ZRANGESTORE out nosuch 0 -1"),
        ("a", "ZUNIONSTORE a 1 a"), ("a", "ZRANGESTORE a a 0 -1"), ("a", "ZINTERSTORE a 2 a a"),
        ("a", "ZMPOP 1 a MIN"), ("a", "ZMPOP 2 nosuch a MAX COUNT 2"), ("ttl", "ZMPOP 1 ttl MIN"),
        ("str", "ZUNIONSTORE str 2 a b"), ("str", "ZRANGESTORE str a 0 -1"),
        ("fresh", "ZUNIONSTORE fresh 1 nosuch"), ("fresh", "ZINTERSTORE fresh 2 a nosuch"),
        ("fresh", "ZDIFFSTORE fresh 2 nosuch a"), ("fresh", "ZRANGESTORE fresh a 50 60"), ("fresh", "ZUNIONSTORE fresh 1 a"),
        ("a", "ZUNION 2 a b"), ("a", "ZINTER 2 a b"), ("a", "ZDIFF 2 a b"), ("a", "ZINTERCARD 2 a b"),
        ("a", "ZRANDMEMBER a"), ("a", "ZRANDMEMBER a 3 WITHSCORES"), ("a", "ZUNIONSTORE x 1 a"),
        ("a", "ZRANGESTORE x a 0 -1"), ("nosuch", "ZMPOP 2 nosuch a MIN COUNT 1"), ("a", "ZMPOP 2 nosuch gone MIN"),
        ("a", "ZMPOP 1 str MIN"), ("out", "ZUNIONSTORE out 2 str a"), ("out", "ZUNIONSTORE out 2 a b WEIGHTS x y"),
        ("out", "ZINTERSTORE out 0 a"), ("out", "ZRANGESTORE out a 0 -1 WITHSCORES"), ("out", "ZMPOP 1 a MIN COUNT 0"),
    ];
    for version in [2_u8, 3] {
        let mut aborted = 0;
        let mut ran = 0;
        for (index, (watched, write)) in cases.iter().enumerate() {
            let mut all = Vec::new();
            if index == 0 {
                let hello = format!("HELLO {version}");
                all.extend(steps(0, &[hello.as_str()]));
                all.extend(steps(1, &[hello.as_str()]));
            }
            all.extend(steps(1, &base));
            all.push((0, format!("WATCH {watched}")));
            all.push((1, (*write).to_owned()));
            all.extend(steps(0, &["MULTI", "PING", "EXEC"]));
            all.extend(steps(1, &["ZRANGE out 0 -1 WITHSCORES", "ZRANGE a 0 -1 WITHSCORES", "EXISTS fresh x", "DBSIZE"]));
            let replies = run(&format!("RESP{version} WATCH {watched} then {write}"), &mut firn, &mut oracle, &all);
            let exec = replies.iter().rev().find(|(command, _)| command == "EXEC").expect("EXEC was sent");
            if exec.1.starts_with(b"*-1") || exec.1.starts_with(b"_") {
                aborted += 1;
            } else {
                ran += 1;
            }
        }
        // Both outcomes occur, so the comparison separates a signal from none.
        assert!(aborted >= 10 && ran >= 10, "RESP{version}: {aborted} transactions aborted, {ran} ran");
    }
    stop(firn);
    stop(oracle);
}

/// A store gives its destination no expiry, a pop that leaves members keeps its
/// key's expiry, a pop of the last member removes the key, and reads leave
/// expiries alone.
#[test]
fn zset_stores_and_pops_treat_expiry_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = spawn_redis(directory.path(), 1);
    let mut firn = spawn_firn(&program, 1);
    let base = ["FLUSHALL", "ZADD a 1 x 2 y 3 z 4 w", "ZADD b 10 y 20 z 30 v", "ZADD ttl 1 p 2 q 3 r", "SET dst v"];
    let far = "4102444800000";
    let mut all = Vec::new();
    all.extend(steps(0, &base));
    for (setup, write) in [
        ("ttl", "ZMPOP 1 ttl MIN"), ("ttl", "ZMPOP 1 ttl MAX COUNT 2"), ("ttl", "ZMPOP 1 ttl MAX COUNT 5"),
        ("a", "ZUNIONSTORE a 1 a"), ("a", "ZRANGESTORE a a 0 -1"), ("a", "ZINTERSTORE a 2 a b"),
        ("a", "ZDIFFSTORE a 1 a"), ("a", "ZUNIONSTORE a 1 nosuch"), ("a", "ZRANGESTORE a a 7 9"),
        ("b", "ZUNIONSTORE b 2 a b"), ("dst", "ZUNIONSTORE dst 1 a"), ("dst", "ZINTERSTORE dst 2 a nosuch"),
        ("dst", "ZRANGESTORE dst a 0 1"), ("a", "ZUNION 2 a b"), ("a", "ZINTER 2 a b"), ("a", "ZDIFF 2 a b"),
        ("a", "ZINTERCARD 2 a b"), ("a", "ZRANDMEMBER a 2"), ("a", "ZUNIONSTORE copy 1 a"), ("a", "ZMPOP 2 nosuch a MIN"),
    ] {
        all.extend(steps(0, &base));
        all.push((0, format!("PEXPIREAT {setup} {far}")));
        all.push((0, (*write).to_owned()));
        all.extend(steps(0, &["PEXPIRETIME a", "PEXPIRETIME b", "PEXPIRETIME ttl", "PEXPIRETIME dst", "PEXPIRETIME copy", "DBSIZE"]));
    }
    // The expiry of a source also tells whether a store removed its source.
    let replies = run("expiry", &mut firn, &mut oracle, &all);
    assert!(replies.iter().any(|(_, reply)| reply == b":4102444800000\r\n"), "an expiry survives some case");
    assert!(replies.iter().any(|(command, reply)| command == "PEXPIRETIME dst" && reply == b":-1\r\n"));
    stop(firn);
    stop(oracle);
}

/// Scores of negative zero come out of the stores and algebra as Redis prints
/// them, in both protocols.
#[test]
fn zset_negative_zero_scores_answer_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = spawn_redis(directory.path(), 1);
    let mut firn = spawn_firn(&program, 1);
    for version in [2_u8, 3] {
        let hello = format!("HELLO {version}");
        let mut all = steps(0, &[hello.as_str()]);
        all.extend(steps(0, &[
            "FLUSHALL", "ZADD z0 0 m", "ZADD zn -0 m", "ZADD zn2 -0 n", "ZADD two 0 m -0 n 1 o",
            "ZSCORE zn m", "ZRANGE zn 0 -1 WITHSCORES", "ZMSCORE zn m nosuch",
            "ZUNIONSTORE o1 1 z0 WEIGHTS -1", "ZSCORE o1 m", "ZRANGE o1 0 -1 WITHSCORES",
            "ZUNION 1 z0 WEIGHTS -1 WITHSCORES", "ZINTER 2 z0 zn WITHSCORES", "ZINTER 2 zn z0 WITHSCORES",
            "ZUNION 2 z0 zn WITHSCORES", "ZUNION 2 zn z0 WITHSCORES", "ZUNION 2 z0 zn WITHSCORES AGGREGATE MIN",
            "ZUNION 2 zn z0 WITHSCORES AGGREGATE MIN", "ZUNION 2 z0 zn WITHSCORES AGGREGATE MAX",
            "ZUNION 2 zn z0 WITHSCORES AGGREGATE MAX", "ZUNION 2 zn zn WITHSCORES", "ZUNION 2 zn zn WITHSCORES AGGREGATE MAX",
            "ZUNIONSTORE o2 1 zn", "ZSCORE o2 m", "ZRANGE o2 0 -1 WITHSCORES",
            "ZUNIONSTORE o3 2 zn zn", "ZSCORE o3 m", "ZUNIONSTORE o3b 2 z0 zn", "ZSCORE o3b m",
            "ZRANGESTORE o4 zn 0 -1", "ZSCORE o4 m", "ZRANGE o4 0 -1 WITHSCORES",
            "ZRANGESTORE o4b two 0 -1", "ZRANGE o4b 0 -1 WITHSCORES", "ZRANGESTORE o4c two -inf +inf BYSCORE", "ZRANGE o4c 0 -1 WITHSCORES",
            "ZDIFFSTORE o5 1 zn", "ZSCORE o5 m", "ZDIFF 2 zn nosuch WITHSCORES", "ZDIFF 2 two zn WITHSCORES",
            "ZINTERSTORE o6 2 zn z0 WEIGHTS 1 -1", "ZSCORE o6 m", "ZINTERSTORE o7 2 zn z0 AGGREGATE MAX", "ZSCORE o7 m",
            "ZUNIONSTORE o8 1 z0 WEIGHTS -0", "ZSCORE o8 m", "ZUNIONSTORE o9 2 zn2 two WEIGHTS 1 -1", "ZRANGE o9 0 -1 WITHSCORES",
            "ZUNION 2 two zn2 WITHSCORES", "ZRANDMEMBER zn", "ZRANDMEMBER zn 1 WITHSCORES", "ZRANDMEMBER zn -2 WITHSCORES",
            "ZRANDMEMBER zn 5 WITHSCORES", "ZMPOP 1 zn MIN", "ZADD zn -0 m", "ZMPOP 1 zn MAX COUNT 3", "ZMPOP 1 o1 MIN",
            "ZMPOP 1 two MAX COUNT 2", "ZMPOP 1 two MIN COUNT 5", "ZRANGE o3 0 -1 WITHSCORES", "ZRANGE o6 0 -1 WITHSCORES",
        ]));
        // ZRANDMEMBER of one member and a count drawing it are the same everywhere.
        run(&format!("RESP{version}"), &mut firn, &mut oracle, &all);
    }
    stop(firn);
    stop(oracle);
}

/// Large sets with many equal scores: the result tables of the stores and the
/// orders of unions, intersections and differences are those of Redis at sizes
/// where the hash tables grow and where ties are settled by member.
#[test]
fn zset_algebra_over_thousands_of_members_answers_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = spawn_redis(directory.path(), 1);
    let mut firn = spawn_firn(&program, 1);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut all = steps(0, &["FLUSHALL"]);
    for (key, from, to) in [("a", 0_u64, 3000_u64), ("b", 1000, 4000), ("c", 2500, 3300), ("d", 0, 7)] {
        let mut start = from;
        while start < to {
            let end = (start + 200).min(to);
            let mut add = format!("ZADD {key}");
            for member in start..end {
                // Quarter steps over a small range: thousands of ties.
                add.push_str(&format!(" {} m{member}", next(400) as f64 / 4.0 - 20.0));
            }
            all.push((0, add));
            start = end;
        }
    }
    all.extend(steps(0, &["SADD p m5 m1500 m2999 m3500 m3999 q"]));
    let queries = [
        "ZUNIONSTORE o 2 a b", "ZUNIONSTORE o 3 a b c WEIGHTS 1 2 3", "ZUNIONSTORE o 3 c b a WEIGHTS 0.5 -1 3 AGGREGATE MIN",
        "ZUNIONSTORE o 2 a b AGGREGATE MAX", "ZUNIONSTORE o 4 a b a b", "ZUNIONSTORE o 3 a p c WEIGHTS 2 3 4",
        "ZINTERSTORE o 2 a b", "ZINTERSTORE o 3 a b c WEIGHTS 1 2 3", "ZINTERSTORE o 3 a b c AGGREGATE MAX",
        "ZINTERSTORE o 2 b p", "ZINTERSTORE o 3 c b a WEIGHTS 2 -1 0.5 AGGREGATE MIN", "ZINTERSTORE o 2 a d",
        "ZDIFFSTORE o 2 a b", "ZDIFFSTORE o 3 a b c", "ZDIFFSTORE o 3 b a c", "ZDIFFSTORE o 2 a p", "ZDIFFSTORE o 2 d a",
        "ZRANGESTORE o a 0 -1", "ZRANGESTORE o a 100 2000 REV", "ZRANGESTORE o b 0 5 BYSCORE", "ZRANGESTORE o b -3.5 7.25 BYSCORE LIMIT 10 500",
        "ZRANGESTORE o b 7.25 -3.5 BYSCORE REV LIMIT 3 40", "ZRANGESTORE o a - + BYLEX LIMIT 5 1000",
    ];
    for query in queries {
        all.push((0, query.to_owned()));
        all.extend(steps(0, &["ZRANGE o 0 -1 WITHSCORES", "ZCARD o"]));
    }
    for query in [
        "ZUNION 3 a b c WITHSCORES", "ZINTER 3 a b c WITHSCORES", "ZDIFF 2 a b WITHSCORES", "ZDIFF 3 a c b WITHSCORES",
        "ZUNION 2 a b WEIGHTS 3 -2 WITHSCORES", "ZINTER 2 a b WEIGHTS 0.1 0.7 AGGREGATE MAX WITHSCORES",
        "ZINTERCARD 3 a b c", "ZINTERCARD 3 a b c LIMIT 100", "ZINTERCARD 2 a b LIMIT 0", "ZINTERCARD 2 d a",
    ] {
        all.push((0, query.to_owned()));
    }
    // Pops of thousands leave the tables with removed cells; the algebra after
    // them must not see them.
    all.extend(steps(0, &[
        "ZMPOP 2 nosuch a MIN COUNT 1500", "ZCARD a", "ZUNIONSTORE o 2 a b", "ZRANGE o 0 -1 WITHSCORES",
        "ZMPOP 1 b MAX COUNT 2500", "ZMPOP 1 b MAX COUNT 2500", "ZUNION 3 a b c WITHSCORES", "ZINTERSTORE o 2 a c",
        "ZRANGE o 0 -1 WITHSCORES", "ZMPOP 3 a b c MIN COUNT 4000", "ZCARD a", "ZUNIONSTORE o 3 a b c", "EXISTS a b c o", "DBSIZE",
    ]));
    let replies = run("thousands of members", &mut firn, &mut oracle, &all);
    assert!(replies.iter().any(|(command, reply)| command == "ZCARD o" && reply.len() > 5), "stores of 4-digit sizes were compared");
    stop(firn);
    stop(oracle);
}

/// The members a ZMPOP reply holds and the key it names: `*2`, the key, then
/// an array of `[member, score]` pairs, or the null array.
fn popped(reply: &[u8]) -> Option<(String, Vec<String>)> {
    let body = text(reply);
    if body.starts_with("*-1") || body.starts_with('_') {
        return None;
    }
    assert!(body.starts_with("*2\r\n"), "a ZMPOP reply: {body:?}");
    let lines: Vec<&str> = body.split("\r\n").collect();
    let key = lines[2].to_owned();
    let members = lines.iter()
        .filter(|line| line.len() > 1 && matches!(line.as_bytes()[0], b'a' | b'b') && line[1..].bytes().all(|byte| byte.is_ascii_digit()))
        .map(|line| (*line).to_owned())
        .collect();
    Some((key, members))
}

fn integer(reply: &[u8]) -> i64 {
    let body = text(reply);
    assert!(body.starts_with(':'), "an integer reply: {body:?}");
    body[1..body.len() - 2].parse().expect("an integer")
}

/// Several connections pop two shared queues through ZMPOP in both key orders
/// while others build unions of the same queues: every member is popped by
/// exactly one connection, a reply names the key its members came from, and no
/// command waits for another for good. Redis is single-threaded, so each ZMPOP
/// there is one step: the same must hold of firn.
#[test]
fn zmpop_over_shared_queues_pops_every_member_once_across_connections() {
    const EACH: usize = 600;
    const POPPERS: usize = 4;
    const READERS: usize = 2;
    let program = CompiledProgram::from_environment();
    let mut server = spawn_firn(&program, 1 + POPPERS + READERS);
    for (key, prefix) in [("q1", "a"), ("q2", "b")] {
        let mut start = 0;
        while start < EACH {
            let end = (start + 100).min(EACH);
            let mut add = format!("ZADD {key}");
            for member in start..end {
                add.push_str(&format!(" {} {prefix}{member}", member % 37));
            }
            let reply = server.say(0, &add);
            assert_eq!(integer(&reply), (end - start) as i64);
            start = end;
        }
    }
    let finished = AtomicBool::new(false);
    let mut streams: Vec<TcpStream> = server.clients.drain(1..).collect();
    let readers: Vec<TcpStream> = streams.split_off(POPPERS);
    let all_popped = std::sync::Mutex::new(Vec::<String>::new());
    let outcome = std::thread::scope(|scope| {
        let readers: Vec<_> = readers.into_iter().enumerate().map(|(index, mut stream)| {
            let finished = &finished;
            scope.spawn(move || {
                let mut last = i64::MAX;
                let destination = format!("t{index}");
                while !finished.load(Ordering::Relaxed) {
                    let order = if index % 2 == 0 { "q1 q2" } else { "q2 q1" };
                    let united = integer(&request(&mut stream, &["ZUNIONSTORE", &destination, "2", &order[..2], &order[3..], "WEIGHTS", "1", "2"]));
                    assert!(united >= 0 && united <= last, "the union of queues that only lose members grew from {last} to {united}");
                    last = united;
                    assert_eq!(integer(&request(&mut stream, &["ZCARD", &destination])), united, "the stored union holds what the store counted");
                    assert_eq!(integer(&request(&mut stream, &["ZINTERCARD", "2", "q1", "q2"])), 0, "disjoint queues");
                    let drawn = text(&request(&mut stream, &["ZRANDMEMBER", "q1", "3"]));
                    assert!(drawn.starts_with('*') || drawn.starts_with('~'), "{drawn:?}");
                }
            })
        }).collect();
        let poppers: Vec<_> = streams.into_iter().enumerate().map(|(index, mut stream)| {
            let all_popped = &all_popped;
            scope.spawn(move || {
                let mut mine = Vec::new();
                loop {
                    let (command, count): (&[&str], usize) = match index % 3 {
                        0 => (&["ZMPOP", "2", "q1", "q2", "MIN", "COUNT", "7"], 7),
                        1 => (&["ZMPOP", "2", "q2", "q1", "MAX", "COUNT", "5"], 5),
                        _ => (&["ZMPOP", "2", "q1", "q2", "MAX"], 1),
                    };
                    let Some((key, members)) = popped(&request(&mut stream, command)) else { break };
                    assert!(!members.is_empty() && members.len() <= count, "{key}: {members:?}");
                    for member in &members {
                        let expected = if member.starts_with('a') { "q1" } else { "q2" };
                        assert_eq!(key, expected, "{member} was popped from {key}");
                    }
                    mine.extend(members);
                }
                all_popped.lock().unwrap().extend(mine);
            })
        }).collect();
        let mut failure = None;
        for popper in poppers {
            if let Err(payload) = popper.join() {
                failure.get_or_insert(payload);
            }
        }
        finished.store(true, Ordering::Relaxed);
        for reader in readers {
            if let Err(payload) = reader.join() {
                failure.get_or_insert(payload);
            }
        }
        failure
    });
    if let Some(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
    let mut popped_members = all_popped.into_inner().unwrap();
    assert_eq!(popped_members.len(), 2 * EACH, "every member is popped, none twice");
    popped_members.sort();
    popped_members.dedup();
    assert_eq!(popped_members.len(), 2 * EACH, "no member is popped by two connections");
    assert_eq!(text(&server.say(0, "EXISTS q1 q2")), ":0\r\n", "the emptied queues are gone");
    server.clients.truncate(1);
    stop(server);
}

/// Two queues kept equal by transactions that add or remove a member of both
/// at once: a difference, intersection or union of them taken by one command
/// never sees one changed and the other not.
#[test]
fn zset_algebra_sees_a_transaction_over_two_keys_whole() {
    const TOGGLES: usize = 3000;
    let program = CompiledProgram::from_environment();
    let mut server = spawn_firn(&program, 5);
    let mut streams: Vec<TcpStream> = server.clients.drain(1..).collect();
    let toggler = streams.remove(0);
    let finished = AtomicBool::new(false);
    let outcome = std::thread::scope(|scope| {
        let readers: Vec<_> = streams.into_iter().enumerate().map(|(index, mut stream)| {
            let finished = &finished;
            scope.spawn(move || {
                let mut seen = 0_u64;
                let mut present = 0_u64;
                while !finished.load(Ordering::Relaxed) {
                    let (first, second) = if index % 2 == 0 { ("q1", "q2") } else { ("q2", "q1") };
                    assert_eq!(text(&request(&mut stream, &["ZDIFF", "2", first, second])), "*0\r\n", "{first} lost or gained a member apart from {second}");
                    assert_eq!(integer(&request(&mut stream, &["ZDIFFSTORE", &format!("d{index}"), "2", first, second])), 0);
                    let inter = integer(&request(&mut stream, &["ZINTERSTORE", &format!("i{index}"), "2", first, second]));
                    let union = integer(&request(&mut stream, &["ZUNIONSTORE", &format!("u{index}"), "2", first, second]));
                    assert!(inter == 0 || inter == 1, "{inter}");
                    assert!(union == 0 || union == 1, "{union}");
                    seen += 1;
                    present += union as u64;
                }
                (seen, present)
            })
        }).collect();
        let toggling = scope.spawn(move || {
            let mut stream = toggler;
            // Each batch is sent whole so the two transactions run back to back.
            for round in 0..TOGGLES {
                let mut batch = Vec::new();
                for part in [["MULTI", "ZADD q1 1 m", "ZADD q2 1 m", "EXEC"], ["MULTI", "ZREM q1 m", "ZREM q2 m", "EXEC"]] {
                    for command in part {
                        batch.extend(resp(&command.split(' ').collect::<Vec<_>>()));
                    }
                }
                stream.write_all(&batch).expect("send the transactions");
                let mut replies = Vec::new();
                for _ in 0..8 {
                    replies.push(text(&super::pubsub::frame(&mut stream)));
                }
                assert_eq!(replies[3], "*2\r\n:1\r\n:1\r\n", "round {round}: {replies:?}");
                assert_eq!(replies[7], "*2\r\n:1\r\n:1\r\n", "round {round}: {replies:?}");
            }
        });
        let mut failure = toggling.join().err();
        finished.store(true, Ordering::Relaxed);
        let mut seen = 0;
        for reader in readers {
            match reader.join() {
                Ok((count, _)) => seen += count,
                Err(payload) => {
                    failure.get_or_insert(payload);
                }
            }
        }
        (failure, seen)
    });
    if let Some(payload) = outcome.0 {
        std::panic::resume_unwind(payload);
    }
    assert!(outcome.1 > 0, "the readers ran");
    server.clients.truncate(1);
    stop(server);
}

/// The peak resident memory of a process, in kibibytes.
fn peak_kibibytes(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("read the process status");
    let line = status.lines().find(|line| line.starts_with("VmHWM:")).expect("a peak resident size");
    line.split_whitespace().nth(1).expect("its number").parse().expect("a number")
}

/// A union that names one small set 400 000 times builds a result of that
/// small set's members: Redis's memory for it is the result's. The request is
/// about 3 MB, so what the server holds for it must not grow with the number
/// of names by more than a small multiple of that.
#[test]
fn zunion_naming_one_set_a_great_many_times_holds_memory_for_the_result_only() {
    const NAMES: usize = 400_000;
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = spawn_redis(directory.path(), 1);
    let mut firn = spawn_firn(&program, 1);
    let mut add = String::from("ZADD s");
    for member in 0..20 {
        add.push_str(&format!(" {member} member{member}"));
    }
    run("setup", &mut firn, &mut oracle, &steps(0, &["FLUSHALL", add.as_str()]));
    let before = peak_kibibytes(firn.child.id());
    let mut statement = vec!["ZUNION".to_owned(), NAMES.to_string()];
    statement.extend(std::iter::repeat("s".to_owned()).take(NAMES));
    statement.push("WITHSCORES".to_owned());
    let arguments: Vec<&str> = statement.iter().map(String::as_str).collect();
    let want = request(&mut oracle.clients[0], &arguments);
    let started = Instant::now();
    let got = request(&mut firn.clients[0], &arguments);
    let elapsed = started.elapsed();
    assert_eq!(text(&got), text(&want), "the union of one set named {NAMES} times");
    assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
    let grown = peak_kibibytes(firn.child.id()).saturating_sub(before);
    assert!(grown < 128 * 1024, "answering a union with 20 members grew the peak resident size by {grown} KiB");
    stop(firn);
    stop(oracle);
}
