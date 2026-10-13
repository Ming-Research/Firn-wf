//! Writes without an append-only file hold their keys alone: a statement that
//! queues no expiry takes no Meta target (design/firn/command-parts.md). The
//! oracle is Redis 7.2.16's documented replies and expiry behavior, written
//! here by hand and not recorded from a run; these cases run with AOF off,
//! where that path is taken, and guard what Meta still carries there.

use super::*;

fn start(clients: &[u8]) -> (ProgramChild, TcpStream, u16) {
    let port = free_port();
    let text = port.to_string();
    let child = firn().spawn_on_route_with(true, &[("WF_DRIVERS", "4")], &[text.as_bytes(), clients]);
    (child, connect_when_ready(port), port)
}

fn pttl(client: &mut TcpStream, key: &str) -> i64 {
    client.write_all(&resp(&["PTTL", key])).unwrap();
    integer_reply(client, key)
}

/// Writes that queue an expiry still queue it, so the expiring context removes
/// keys nothing reads; writes that keep or remove an expiry through the
/// Meta-free statement keep or remove it. A timed SET, PSETEX or GETEX taking
/// the Meta-free statement would queue nothing and leave DBSIZE above three; a
/// KEEPTTL store that lost the key's expiry would leave it there too, and one
/// that kept a removed expiry would make PERSIST's or GETEX PERSIST's key
/// vanish. INFO leaves out the commit sequence, which only Meta-holding
/// statements would still advance.
#[test]
fn without_aof_expiry_writes_queue_and_plain_writes_keep_expiries() {
    let (child, mut client, _) = start(b"1");
    client.write_all(&resp(&["INFO", "persistence"])).unwrap();
    let info = bulk_reply(&mut client, "INFO persistence");
    assert_eq!(info_field(&info, "aof_enabled").as_deref(), Some("0"));
    assert_eq!(info_field(&info, "firn_aof_rewrite_commit_seq"), None, "a partial count is no fact: {info:?}");
    let cases: &[(&[&str], &str)] = &[
        (&["SET", "timed", "v", "PX", "100"], "+OK\r\n"),
        (&["PSETEX", "psetex", "100", "v"], "+OK\r\n"),
        (&["SET", "getex", "v"], "+OK\r\n"),
        (&["GETEX", "getex", "PX", "100"], "$1\r\nv\r\n"),
        (&["SET", "kept", "v", "PX", "1000"], "+OK\r\n"),
        (&["SET", "kept", "w", "KEEPTTL"], "+OK\r\n"),
        (&["SET", "persisted", "v", "PX", "1000"], "+OK\r\n"),
        (&["PERSIST", "persisted"], ":1\r\n"),
        (&["SET", "unexpired", "v", "PX", "1000"], "+OK\r\n"),
        (&["GETEX", "unexpired", "PERSIST"], "$1\r\nv\r\n"),
        (&["SET", "plain", "v"], "+OK\r\n"),
    ];
    for (args, reply) in cases {
        assert_eq!(memory_request(&mut client, args), *reply, "{args:?}");
    }
    assert!(pttl(&mut client, "kept") > 0, "KEEPTTL keeps the expiry");
    assert_eq!(memory_request(&mut client, &["GET", "kept"]), "$1\r\nw\r\n");
    for key in ["persisted", "unexpired", "plain"] {
        assert_eq!(pttl(&mut client, key), -1, "{key}");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let size = memory_request(&mut client, &["DBSIZE"]);
        if size == ":3\r\n" { break; }
        assert!(Instant::now() < deadline, "active expiry left {size:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(memory_request(&mut client, &["EXISTS", "persisted", "unexpired", "plain"]), ":3\r\n");
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// A write on a key whose expiry has passed replaces it as Redis does, the
/// new value without the old expiry and a key of another kind counting as
/// absent. Keys expire one millisecond after they are set and are written a
/// few milliseconds later, before the expiring context's next pass for most of
/// them, so most reach the write's own removal; the replies are the same
/// whichever removes a key. A write that kept the expired value, or gave the
/// new value the old expiry, answers WRONGTYPE or loses the key.
#[test]
fn without_aof_writes_replace_expired_values() {
    const KEYS: usize = 50;
    let (child, mut client, _) = start(b"1");
    let mut batch = Vec::new();
    for index in 0..KEYS {
        for kind in ["set", "incr", "push", "mset", "hset"] {
            batch.extend(resp(&["SET", &format!("{kind}:{index}"), "text", "PX", "1"]));
        }
    }
    client.write_all(&batch).unwrap();
    expect_replies(&mut client, &b"+OK\r\n".repeat(KEYS * 5), "the expiring keys");
    std::thread::sleep(Duration::from_millis(5));
    let mut batch = Vec::new();
    let mut expected = Vec::new();
    for index in 0..KEYS {
        batch.extend(resp(&["SET", &format!("set:{index}"), "new"]));
        batch.extend(resp(&["INCR", &format!("incr:{index}")]));
        batch.extend(resp(&["LPUSH", &format!("push:{index}"), "a"]));
        batch.extend(resp(&["MSET", &format!("mset:{index}"), "new", &format!("other:{index}"), "x"]));
        batch.extend(resp(&["HSET", &format!("hset:{index}"), "f", "v"]));
        expected.extend_from_slice(b"+OK\r\n:1\r\n:1\r\n+OK\r\n:1\r\n");
    }
    client.write_all(&batch).unwrap();
    expect_replies(&mut client, &expected, "writes over expired keys");
    for index in 0..KEYS {
        for key in [format!("set:{index}"), format!("incr:{index}"), format!("push:{index}"), format!("mset:{index}"), format!("hset:{index}")] {
            assert_eq!(pttl(&mut client, &key), -1, "{key} keeps no expiry");
        }
    }
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// MSET stores every pair in one statement on all its keys, and MGET reads
/// them in one: a reader on another driver never sees one key of a pair
/// written and the other not, which an MSET or MGET split into a statement
/// per key would show.
#[test]
fn without_aof_mset_is_seen_whole_by_a_concurrent_mget() {
    const ROUNDS: usize = 2000;
    let (child, mut control, port) = start(b"3");
    assert_eq!(memory_request(&mut control, &["MSET", "left", "0", "right", "0"]), "+OK\r\n");
    let writer = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        for round in 1..=ROUNDS {
            let value = round.to_string();
            assert_eq!(memory_request(&mut client, &["MSET", "left", &value, "right", &value]), "+OK\r\n");
        }
    });
    let reader = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        let mut last = 0_usize;
        while last < ROUNDS {
            let reply = memory_request(&mut client, &["MGET", "left", "right"]);
            let values: Vec<&str> = reply.split("\r\n").filter(|line| !line.starts_with(['*', '$']) && !line.is_empty()).collect();
            assert_eq!(values.len(), 2, "{reply:?}");
            assert_eq!(values[0], values[1], "a pair seen half written: {reply:?}");
            last = values[0].parse().unwrap();
        }
    });
    writer.join().expect("the writer");
    reader.join().expect("the reader");
    drop(control);
    assert_eq!(finished(child).0, 0);
}
