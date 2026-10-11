//! The stage-1 sequence oracle is the owner-approved scan-log design, not
//! Redis INFO (this field is firn-specific). Command replies follow Redis
//! 7.0.15. Each positive delta requires the shared mutation publication path;
//! reads and refused/no-effect commands must leave that path untouched.

use super::*;

fn sequence(client: &mut TcpStream) -> u64 {
    client.write_all(&resp(&["INFO", "persistence"])).unwrap();
    let info = bulk_reply(client, "rewrite sequence");
    info_field(&info, "firn_aof_rewrite_commit_seq")
        .expect("firn reports its actual commit sequence even with AOF off")
        .parse().expect("u64 sequence")
}

fn start(aof: bool) -> (CompiledProgram, ProgramChild, TcpStream) {
    let program = CompiledProgram::from_environment();
    let port = free_port().to_string();
    let mut arguments = vec![port.as_bytes(), b"1".as_slice()];
    if aof { arguments.push(b"sequence.aof"); }
    let child = program.spawn_on_route(true, &arguments);
    let client = connect_when_ready(port.parse().unwrap());
    (program, child, client)
}

fn expect(client: &mut TcpStream, args: &[&str], reply: &str, delta: u64) {
    let before = sequence(client);
    assert_eq!(memory_request(client, args), reply, "{args:?}");
    assert_eq!(sequence(client), before + delta, "{args:?}: one outer statement");
}

/// Exercise new entries and in-place updates separately: a constructor-only
/// stamp misses the second write. Repeat with AOF off to reject an AOF hook.
#[test]
fn rewrite_sequence_all_value_types_and_no_effects() {
    for aof in [false, true] {
        let (_program, child, mut client) = start(aof);
        assert_eq!(sequence(&mut client), 0);
        let cases: &[(&[&str], &str, u64)] = &[
            (&["SET", "text", "1"], "+OK\r\n", 1),
            (&["INCR", "text"], ":2\r\n", 1),
            (&["APPEND", "text", "3"], ":2\r\n", 1),
            (&["SETRANGE", "text", "0", "4"], ":2\r\n", 1),
            (&["GET", "text"], "$2\r\n43\r\n", 0),
            (&["SET", "text", "0", "NX"], "$-1\r\n", 0),
            (&["SET", "absent", "0", "XX"], "$-1\r\n", 0),
            (&["SETRANGE", "text", "0", ""], ":2\r\n", 0),
            (&["INCRBY", "text", "bad"], "-ERR value is not an integer or out of range\r\n", 0),
            (&["MSET", "a", "1", "b", "2", "a", "3"], "+OK\r\n", 1),
            (&["MSETNX", "a", "0", "c", "0"], ":0\r\n", 0),
            (&["MSETNX", "c", "1", "d", "2"], ":1\r\n", 1),
            (&["LPUSH", "list", "a", "b", "c"], ":3\r\n", 1),
            (&["RPUSH", "list", "d"], ":4\r\n", 1),
            (&["LSET", "list", "0", "e"], "+OK\r\n", 1),
            (&["LREM", "list", "0", "missing"], ":0\r\n", 0),
            (&["LTRIM", "list", "0", "-1"], "+OK\r\n", 0),
            (&["LINSERT", "list", "BEFORE", "missing", "z"], ":-1\r\n", 0),
            (&["LINSERT", "list", "BEFORE", "e", "z"], ":5\r\n", 1),
            (&["LREM", "list", "0", "z"], ":1\r\n", 1),
            (&["LPOP", "list", "0"], "*0\r\n", 0),
            (&["LPOP", "list"], "$1\r\ne\r\n", 1),
            (&["LTRIM", "list", "0", "1"], "+OK\r\n", 1),
            (&["LPUSHX", "missing", "v"], ":0\r\n", 0),
            (&["SADD", "set", "a", "b"], ":2\r\n", 1),
            (&["SADD", "set", "c"], ":1\r\n", 1),
            (&["SADD", "set", "a"], ":0\r\n", 0),
            (&["SREM", "set", "missing"], ":0\r\n", 0),
            (&["SREM", "set", "c"], ":1\r\n", 1),
            (&["SPOP", "set", "0"], "*0\r\n", 0),
            (&["HSET", "hash", "a", "1", "b", "2"], ":2\r\n", 1),
            (&["HSET", "hash", "a", "3"], ":0\r\n", 1),
            (&["HSETNX", "hash", "a", "9"], ":0\r\n", 0),
            (&["HSETNX", "hash", "c", "9"], ":1\r\n", 1),
            (&["HINCRBY", "hash", "a", "1"], ":4\r\n", 1),
            (&["HINCRBYFLOAT", "hash", "a", "0.5"], "$3\r\n4.5\r\n", 1),
            (&["HDEL", "hash", "missing"], ":0\r\n", 0),
            (&["HDEL", "hash", "b"], ":1\r\n", 1),
            (&["ZADD", "sorted", "1", "a", "2", "b"], ":2\r\n", 1),
            (&["ZADD", "sorted", "3", "a"], ":0\r\n", 1),
            (&["ZADD", "sorted", "3", "a"], ":0\r\n", 0),
            (&["ZADD", "sorted", "NX", "4", "a"], ":0\r\n", 0),
            (&["ZADD", "sorted", "XX", "4", "missing"], ":0\r\n", 0),
            (&["ZINCRBY", "sorted", "1", "a"], "$1\r\n4\r\n", 1),
            (&["ZREM", "sorted", "missing"], ":0\r\n", 0),
            (&["ZREMRANGEBYSCORE", "sorted", "20", "30"], ":0\r\n", 0),
            (&["ZREMRANGEBYRANK", "sorted", "0", "0"], ":1\r\n", 1),
            (&["ZPOPMAX", "sorted", "0"], "*0\r\n", 0),
            (&["ZPOPMIN", "sorted"], "*2\r\n$1\r\na\r\n$1\r\n4\r\n", 1),
            (&["DEL", "a", "b", "a"], ":2\r\n", 1),
            (&["UNLINK", "c", "d"], ":2\r\n", 1),
            (&["DEL", "missing"], ":0\r\n", 0),
            (&["FLUSHDB"], "+OK\r\n", 1),
            (&["FLUSHALL"], "+OK\r\n", 1),
        ];
        for (args, reply, delta) in cases { expect(&mut client, args, reply, *delta); }
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

#[test]
fn rewrite_sequence_transfers_stores_and_expiry_metadata() {
    let (_program, child, mut client) = start(false);
    let cases: &[(&[&str], &str, u64)] = &[
        (&["MSET", "a", "v", "b", "w"], "+OK\r\n", 1),
        (&["RENAME", "a", "a"], "+OK\r\n", 0),
        (&["RENAMENX", "a", "b"], ":0\r\n", 0),
        (&["COPY", "a", "b"], ":0\r\n", 0),
        (&["COPY", "a", "b", "REPLACE"], ":1\r\n", 1),
        (&["RENAME", "a", "renamed"], "+OK\r\n", 1),
        (&["PEXPIREAT", "renamed", "9999999999999"], ":1\r\n", 1),
        (&["EXPIRE", "renamed", "60", "NX"], ":0\r\n", 0),
        (&["PERSIST", "renamed"], ":1\r\n", 1),
        (&["PERSIST", "renamed"], ":0\r\n", 0),
        (&["GETEX", "renamed", "PXAT", "9999999999999"], "$1\r\nv\r\n", 1),
        (&["GETEX", "renamed"], "$1\r\nv\r\n", 0),
        (&["GETEX", "renamed", "PERSIST"], "$1\r\nv\r\n", 1),
        (&["GETEX", "renamed", "PERSIST"], "$1\r\nv\r\n", 0),
        (&["PEXPIREAT", "renamed", "1"], ":1\r\n", 1),
        (&["GETDEL", "b"], "$1\r\nv\r\n", 1),
        (&["GETDEL", "b"], "$-1\r\n", 0),
        (&["RPUSH", "source", "a", "b", "c"], ":3\r\n", 1),
        (&["RPUSH", "target", "x"], ":1\r\n", 1),
        (&["LMOVE", "source", "target", "LEFT", "RIGHT"], "$1\r\na\r\n", 1),
        (&["RPOPLPUSH", "source", "target"], "$1\r\nc\r\n", 1),
        (&["LMOVE", "missing", "target", "LEFT", "RIGHT"], "$-1\r\n", 0),
        (&["SADD", "s1", "a", "b", "c"], ":3\r\n", 1),
        (&["SADD", "s2", "a"], ":1\r\n", 1),
        (&["SMOVE", "s1", "s2", "a"], ":1\r\n", 1),
        (&["SMOVE", "s1", "s2", "b"], ":1\r\n", 1),
        (&["SMOVE", "s2", "s2", "b"], ":1\r\n", 0),
        (&["SMOVE", "s1", "s2", "missing"], ":0\r\n", 0),
        (&["SUNIONSTORE", "out", "s1", "s2"], ":3\r\n", 1),
        (&["SDIFFSTORE", "out", "s2", "s1"], ":2\r\n", 1),
        (&["SINTERSTORE", "out", "s1", "s2"], ":0\r\n", 1),
        (&["SINTERSTORE", "out", "s1", "s2"], ":0\r\n", 0),
    ];
    for (args, reply, delta) in cases { expect(&mut client, args, reply, *delta); }
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// All types share the outer token, even repeated writes of the same key,
/// a failed command between writes, and FLUSH followed by re-creation.
#[test]
fn rewrite_sequence_exec_and_script_are_single_units() {
    for aof in [false, true] {
        let (_program, child, mut client) = start(aof);
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        let commands: &[&[&str]] = &[
            &["MSET", "n", "1", "other", "2"], &["INCR", "n"],
            &["HSET", "h", "f", "1"], &["HINCRBY", "h", "f", "1"],
            &["LPUSH", "l", "a", "b"], &["LSET", "l", "0", "c"],
            &["SADD", "s", "a"], &["SADD", "s", "b"],
            &["ZADD", "z", "1", "a"], &["ZINCRBY", "z", "1", "a"],
            &["GET", "l"], &["RENAME", "other", "renamed"],
        ];
        // INFO cannot execute inside MULTI; read the sequence only after EXEC.
        for args in commands { assert_eq!(memory_request(&mut client, args), "+QUEUED\r\n"); }
        let reply = memory_request(&mut client, &["EXEC"]);
        assert!(reply.starts_with("*12\r\n"), "{reply:?}");
        assert!(reply.contains("-WRONGTYPE "), "the error must be reached: {reply:?}");
        assert_eq!(sequence(&mut client), 1);
        expect(&mut client, &["GET", "n"], "$1\r\n2\r\n", 0);
        let script = "redis.call('FLUSHALL'); redis.call('MSET','n','1','other','2'); \
            redis.call('INCR','n'); redis.call('HSET','h','f','1'); redis.call('HINCRBY','h','f',1); \
            redis.call('LPUSH','l','a','b'); redis.call('LSET','l',0,'c'); \
            redis.call('SADD','s','a'); redis.call('SADD','s','b'); \
            redis.call('ZADD','z',1,'a'); redis.call('ZINCRBY','z',1,'a'); \
            redis.pcall('GET','l'); redis.call('RENAME','other','renamed'); return redis.call('GET','n')";
        expect(&mut client, &["EVAL", script, "0"], "$1\r\n2\r\n", 1);
        let before = sequence(&mut client);
        let error = memory_request(&mut client, &["EVAL", "redis.call('INCR','n'); error('after write')", "0"]);
        assert!(error.starts_with("-ERR ") && error.contains("after write"), "{error:?}");
        assert_eq!(sequence(&mut client), before + 1);
        expect(&mut client, &["GET", "n"], "$1\r\n3\r\n", 0);
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        for args in [&["GET", "n"][..], &["SADD", "s", "a"], &["DEL", "missing"], &["GET", "l"]] {
            assert_eq!(memory_request(&mut client, args), "+QUEUED\r\n");
        }
        let unchanged = sequence_after_exec(&mut client);
        assert_eq!(unchanged, before + 1, "read/no-effect/error EXEC takes no token");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

fn sequence_after_exec(client: &mut TcpStream) -> u64 {
    let reply = memory_request(client, &["EXEC"]);
    assert!(reply.starts_with("*4\r\n") && reply.contains("-WRONGTYPE "), "{reply:?}");
    sequence(client)
}

#[test]
fn rewrite_sequence_reads_access_refreshes_and_read_only_retries() {
    let (_program, child, mut client) = start(false);
    expect(&mut client, &["SET", "n", "1"], "+OK\r\n", 1);
    for policy in ["allkeys-lru", "allkeys-lfu"] {
        expect(&mut client, &["CONFIG", "SET", "maxmemory-policy", policy, "lfu-log-factor", "0", "lfu-decay-time", "0"], "+OK\r\n", 0);
        for _ in 0..3 { expect(&mut client, &["GET", "n"], "$1\r\n1\r\n", 0); }
        expect(&mut client, &["TOUCH", "n", "missing"], ":1\r\n", 0);
        expect(&mut client, &["MGET", "n", "n", "missing"], "*3\r\n$1\r\n1\r\n$1\r\n1\r\n$-1\r\n", 0);
    }
    // A policy switch retains the old LRU bits. Recreate under LFU so its
    // initial frequency is five, with room for the two observed refreshes.
    expect(&mut client, &["DEL", "n"], ":1\r\n", 1);
    expect(&mut client, &["SET", "n", "1"], "+OK\r\n", 1);
    let frequency = memory_object(&mut client, "FREQ", "n");
    assert_eq!(frequency, 5);
    let readonly = "redis.call('GET','n'); local n=0; for i=1,20000 do n=n+i end; return redis.call('GET','n')";
    expect(&mut client, &["EVAL", readonly, "0"], "$1\r\n1\r\n", 0);
    assert_eq!(memory_object(&mut client, "FREQ", "n"), frequency + 2, "abandoned attempts restore access stamps");
    expect(&mut client, &["EVAL", "redis.call('SET','n','2','NX'); redis.call('DEL','missing'); return 1", "0"], ":1\r\n", 0);
    let before = sequence(&mut client);
    assert!(memory_request(&mut client, &["EVAL", "error('before write')", "0"]).contains("before write"));
    assert_eq!(sequence(&mut client), before);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// DBSIZE counts physical entries without looking them up: only the active
/// expiry worker can remove this key during the polling loop.
#[test]
fn rewrite_sequence_active_expiry_and_stale_due_records() {
    let (_program, child, mut client) = start(false);
    expect(&mut client, &["SET", "expiring", "v", "PX", "1000"], "+OK\r\n", 1);
    expect(&mut client, &["PEXPIRE", "expiring", "1000"], ":1\r\n", 1);
    let before = sequence(&mut client);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let size = memory_request(&mut client, &["DBSIZE"]);
        if size == ":0\r\n" { break; }
        assert_eq!(size, ":1\r\n");
        assert!(Instant::now() < deadline, "active expiry did not remove the key");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sequence(&mut client), before + 1, "one actual removal, including a duplicate/stale due record");
    expect(&mut client, &["GET", "expiring"], "$-1\r\n", 0);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// DBSIZE within the same held unit witnesses physical presence before GET
/// expires it. Retry only when the active worker demonstrably won first,
/// following check_held_records; a missing stamp never qualifies for retry.
#[test]
fn rewrite_sequence_held_lazy_expiry() {
    for script in [false, true] {
        let mut witnessed = false;
        for _ in 0..5 {
            let program = CompiledProgram::from_environment();
            std::fs::write(program.working_directory().join("expired.aof"), resp(&["SET", "expired", "v", "PXAT", "1"])).unwrap();
            let port = free_port().to_string();
            let child = program.spawn_on_route(true, &[port.as_bytes(), b"1", b"expired.aof"]);
            let mut client = connect_when_ready(port.parse().unwrap());
            let reply = if script {
                memory_request(&mut client, &["EVAL", "local n=redis.call('DBSIZE'); redis.call('GET','expired'); return n", "0"])
            } else {
                for args in [&["MULTI"][..], &["DBSIZE"], &["GET", "expired"], &["EXISTS", "expired"]] {
                    let expected = if args[0] == "MULTI" { "+OK\r\n" } else { "+QUEUED\r\n" };
                    assert_eq!(memory_request(&mut client, args), expected);
                }
                memory_request(&mut client, &["EXEC"])
            };
            let present = if script { ":1\r\n" } else { "*3\r\n:1\r\n$-1\r\n:0\r\n" };
            let swept = if script { ":0\r\n" } else { "*3\r\n:0\r\n$-1\r\n:0\r\n" };
            assert!(reply == present || reply == swept, "{reply:?}");
            assert_eq!(sequence(&mut client), 1, "loading takes zero; either one lazy or one active removal");
            witnessed = reply == present;
            drop(client);
            assert_eq!(finished(child).0, 0);
            if witnessed { break; }
        }
        assert!(witnessed, "active expiry won before the held lazy-expiry witness in five starts");
    }
}

#[test]
fn rewrite_sequence_eviction_counts_actual_removals() {
    let (_program, child, mut client) = start(false);
    expect(&mut client, &["MSET", "a", "1", "b", "2", "c", "3"], "+OK\r\n", 1);
    let before = sequence(&mut client);
    assert_eq!(memory_request(&mut client, &["CONFIG", "SET", "maxmemory-policy", "allkeys-random", "maxmemory-eviction-tenacity", "100", "maxmemory", "1"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":0\r\n");
    assert_eq!(sequence(&mut client), before + 3, "each eviction holds and removes one key in its own statement");
    client.write_all(&resp(&["INFO", "stats"])).unwrap();
    assert_eq!(info_field(&bulk_reply(&mut client, "eviction count"), "evicted_keys").as_deref(), Some("3"));
    expect(&mut client, &["GET", "missing"], "$-1\r\n", 0);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

#[test]
fn rewrite_sequence_replay_starts_at_zero() {
    let program = CompiledProgram::from_environment();
    let commands: &[&[&str]] = &[
        &["MULTI"], &["SET", "n", "1"], &["INCR", "n"], &["HSET", "h", "f", "v"],
        &["LPUSH", "l", "v"], &["SADD", "s", "v"], &["ZADD", "z", "1", "v"],
        &["PEXPIREAT", "n", "9999999999999"], &["COPY", "n", "copied"], &["EXEC"],
    ];
    let file: Vec<u8> = commands.iter().flat_map(|args| resp(args)).collect();
    std::fs::write(program.working_directory().join("loaded.aof"), file).unwrap();
    let port = free_port().to_string();
    let child = program.spawn_on_route(true, &[port.as_bytes(), b"1", b"loaded.aof"]);
    let mut client = connect_when_ready(port.parse().unwrap());
    assert_eq!(sequence(&mut client), 0, "replay has no surviving capture");
    expect(&mut client, &["DBSIZE"], ":6\r\n", 0);
    expect(&mut client, &["GET", "copied"], "$1\r\n2\r\n", 0);
    expect(&mut client, &["INCR", "n"], ":3\r\n", 1);
    drop(client);
    assert_eq!(finished(child).0, 0);
}
