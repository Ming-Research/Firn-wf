//! Reconciled-scan oracles use ordinary Redis reads and independently counted
//! client operations. Observing temporary-file progress forces writes behind
//! the scan cursor without a server test hook. No case accepts a failed rewrite
//! as evidence of successful reconstruction.

use super::*;
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reply {
    Line(Vec<u8>),
    Bulk(Option<Vec<u8>>),
    Array(Vec<Reply>),
}

fn wire(args: &[&[u8]]) -> Vec<u8> {
    let mut bytes = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        bytes.extend(format!("${}\r\n", arg.len()).as_bytes());
        bytes.extend_from_slice(arg);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

fn read_reply(stream: &mut TcpStream) -> Reply {
    let mut tag = [0];
    stream.read_exact(&mut tag).unwrap();
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if line.ends_with(b"\r\n") { line.truncate(line.len() - 2); break; }
    }
    let number = || std::str::from_utf8(&line).unwrap().parse::<i64>().unwrap();
    match tag[0] {
        b'$' if number() == -1 => Reply::Bulk(None),
        b'$' => {
            let mut bytes = vec![0; number() as usize + 2];
            stream.read_exact(&mut bytes).unwrap();
            assert!(bytes.ends_with(b"\r\n"));
            bytes.truncate(bytes.len() - 2);
            Reply::Bulk(Some(bytes))
        }
        b'*' => Reply::Array((0..number()).map(|_| read_reply(stream)).collect()),
        b'+' | b':' => { line.insert(0, tag[0]); Reply::Line(line) }
        b'-' => panic!("unexpected Redis error: {}", String::from_utf8_lossy(&line)),
        _ => panic!("invalid RESP reply tag: {:?}", tag),
    }
}

fn request(stream: &mut TcpStream, args: &[&[u8]]) -> Reply {
    stream.write_all(&wire(args)).unwrap();
    read_reply(stream)
}

fn bulk(reply: Reply) -> Vec<u8> {
    match reply { Reply::Bulk(Some(bytes)) => bytes, other => panic!("bulk expected: {other:?}") }
}

fn array(reply: Reply) -> Vec<Reply> {
    match reply { Reply::Array(items) => items, other => panic!("array expected: {other:?}") }
}

fn snapshot(stream: &mut TcpStream) -> BTreeMap<Vec<u8>, (Vec<u8>, Reply, Reply)> {
    let keys = array(request(stream, &[b"KEYS", b"*"]));
    let mut result = BTreeMap::new();
    for reply in keys {
        let key = bulk(reply);
        let kind = match request(stream, &[b"TYPE", &key]) {
            Reply::Line(line) => line[1..].to_vec(),
            other => panic!("TYPE: {other:?}"),
        };
        let mut value = match kind.as_slice() {
            b"string" => request(stream, &[b"GET", &key]),
            b"list" => request(stream, &[b"LRANGE", &key, b"0", b"-1"]),
            b"set" => {
                let mut values = array(request(stream, &[b"SMEMBERS", &key]));
                values.sort();
                Reply::Array(values)
            }
            b"hash" => {
                let pairs = array(request(stream, &[b"HGETALL", &key]));
                let mut pairs: Vec<_> = pairs.chunks_exact(2).map(|pair| Reply::Array(pair.to_vec())).collect();
                pairs.sort();
                Reply::Array(pairs)
            }
            b"zset" => request(stream, &[b"ZRANGE", &key, b"0", b"-1", b"WITHSCORES"]),
            other => panic!("unsupported dump type {other:?}"),
        };
        // Keep the complete RESP value, including null and binary data.
        let expiry = request(stream, &[b"PEXPIRETIME", &key]);
        result.insert(key, (kind, std::mem::replace(&mut value, Reply::Bulk(None)), expiry));
    }
    result
}

fn fixture(program: &CompiledProgram) {
    multipart_files(program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let mut bytes = Vec::new();
    for index in 0..6000 {
        let kind = index % 5;
        let key = format!("kind:{kind}:{index}");
        let args: Vec<&str> = match kind {
            0 => vec!["SET", &key, "0"],
            1 => vec!["RPUSH", &key, "original"],
            2 => vec!["SADD", &key, "original"],
            3 => vec!["HSET", &key, "field", "original"],
            _ => vec!["ZADD", &key, "0.25", "member"],
        };
        bytes.extend(resp(&args));
        if kind == 3 { bytes.extend(resp(&["PEXPIREAT", &key, "99999999999000"])); }
    }
    std::fs::write(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof"), bytes).unwrap();
}

#[track_caller]
fn start(program: &CompiledProgram) -> (ProgramChild, TcpStream, u16) {
    start_with_minimum(program, b"67108864")
}

#[track_caller]
fn start_with_minimum(program: &CompiledProgram, minimum: &[u8]) -> (ProgramChild, TcpStream, u16) {
    let port = free_port();
    let text = port.to_string();
    let mut child = program.spawn_on_route_with(true, &[("WF_WORKERS", "2"), ("WF_DRIVERS", "2")], &[text.as_bytes(), b"0", b"appendonly.aof", b"--auto-aof-rewrite-min-size", minimum]);
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            return (child, bounded_stream(stream), port);
        }
        // A program that exits while starting, such as one refusing its AOF,
        // reports its own output instead of a refused connection.
        if child.try_wait().expect("poll starting firn").is_some() {
            let output = child.wait_with_output().expect("collect firn output");
            panic!(
                "firn exited before listening on {port} ({:?})\nstdout:\n{}\nstderr:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(Instant::now() < deadline, "firn never listened on {port} and is still running");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Decode only complete temporary-file records, tolerating an append in flight.
// This parser is independent of Firn's image encoder and reconciliation.
fn prefix_records(bytes: &[u8]) -> Vec<Vec<Vec<u8>>> {
    fn line(bytes: &[u8], at: &mut usize) -> Option<usize> {
        let end = bytes.get(*at..)?.windows(2).position(|pair| pair == b"\r\n")? + *at;
        let value = std::str::from_utf8(bytes.get(*at..end)?).ok()?.parse().ok()?;
        *at = end + 2;
        Some(value)
    }
    fn record(bytes: &[u8], at: &mut usize) -> Option<Vec<Vec<u8>>> {
        if *bytes.get(*at)? != b'*' { return None; }
        *at += 1;
        let count = line(bytes, at)?;
        let mut words = Vec::new();
        for _ in 0..count {
            if *bytes.get(*at)? != b'$' { return None; }
            *at += 1;
            let len = line(bytes, at)?;
            let end = at.checked_add(len)?;
            words.push(bytes.get(*at..end)?.to_vec());
            if bytes.get(end..end + 2)? != b"\r\n" { return None; }
            *at = end + 2;
        }
        Some(words)
    }
    let mut at = 0;
    let mut records = Vec::new();
    while let Some(words) = record(bytes, &mut at) { records.push(words); }
    records
}

fn scanned_keys(program: &CompiledProgram, client: &mut TcpStream) -> Vec<Vec<u8>> {
    let directory = program.working_directory().join("appendonlydir");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut found = BTreeMap::new();
        let mut observed = Vec::new();
        let bytes = std::fs::read(directory.join("temp-appendonly.aof.base")).unwrap_or_default();
        for words in prefix_records(&bytes) {
            if let Some(key) = words.get(1) {
                if key.starts_with(b"kind:") {
                    found.entry(key[5]).or_insert_with(|| key.clone());
                    if !observed.contains(key) { observed.push(key.clone()); }
                }
            }
        }
        if found.len() == 5 && observed.len() >= 7 {
            assert_eq!(rewrite_info(client)["aof_rewrite_in_progress"], "1");
            assert_eq!(std::fs::read_to_string(directory.join("appendonly.aof.manifest")).unwrap(), rewrite_manifest(1, 1), "observations must precede S1 rotation");
            let mut keys: Vec<_> = found.into_values().collect();
            for key in observed { if !keys.contains(&key) { keys.push(key); } if keys.len() == 7 { break; } }
            return keys;
        }
        assert!(Instant::now() < deadline, "scan never exposed every value type");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_old_manifest(program: &CompiledProgram) {
    assert_eq!(std::fs::read_to_string(program.working_directory().join("appendonlydir/appendonly.aof.manifest")).unwrap(), rewrite_manifest(1, 1));
}

#[test]
fn reconciled_scan_restores_live_dump_and_non_idempotent_units() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut control, port) = start(&program);
    let binary = b"binary\0\r\n\xff";
    request(&mut control, &[b"SET", binary, b"before"]);
    rewrite_start(&mut control);
    let keys = scanned_keys(&program, &mut control);
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = running.clone();
    let worker_keys = keys.clone();
    let worker = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        request(&mut client, &[b"DEL", &worker_keys[5]]);
        request(&mut client, &[b"RENAME", &worker_keys[6], b"renamed\0\xff"]);
        request(&mut client, &[b"PERSIST", &worker_keys[3]]);
        let mut count = 0_u64;
        while worker_running.load(Ordering::Acquire) {
            count += 1;
            let value = count.to_string();
            let score = format!("{count}.25");
            request(&mut client, &[b"INCR", &worker_keys[0]]);
            request(&mut client, &[b"MULTI"]);
            for args in [
                vec![b"RPUSH".as_slice(), &worker_keys[1], value.as_bytes()],
                vec![b"LTRIM", &worker_keys[1], b"-2", b"-1"],
                vec![b"SREM", &worker_keys[2], b"original"],
                vec![b"SADD", &worker_keys[2], b"replacement"],
                vec![b"HSET", &worker_keys[3], b"field", value.as_bytes()],
                vec![b"ZADD", &worker_keys[4], score.as_bytes(), b"member"],
                vec![b"SET", binary, value.as_bytes()],
                vec![b"PEXPIREAT", &worker_keys[1], b"99999999999000"],
                vec![b"INCR", b"exec:a"],
                vec![b"INCR", b"exec:b"],
            ] { assert_eq!(request(&mut client, &args), Reply::Line(b"+QUEUED".to_vec())); }
            let replies = array(request(&mut client, &[b"EXEC"]));
            assert_eq!(&replies[8..], &[Reply::Line(format!(":{count}").into_bytes()), Reply::Line(format!(":{count}").into_bytes())]);
            assert_eq!(request(&mut client, &[b"EVAL", b"redis.call('INCR','script:a'); return redis.call('INCR','script:b')", b"0"]), Reply::Line(format!(":{count}").into_bytes()));
            // Keep the capture within its declared byte/work limits while
            // continuously writing; large-value abort has its own strict case.
            std::thread::sleep(Duration::from_millis(5));
        }
        count
    });
    assert_eq!(rewrite_wait(&mut control)["aof_last_bgrewrite_status"], "ok");
    running.store(false, Ordering::Release);
    let count = worker.join().unwrap();
    assert!(count > 1, "must commit repeated mutations after scan observation");
    assert_eq!(bulk(request(&mut control, &[b"GET", &keys[0]])), count.to_string().as_bytes());
    let live = snapshot(&mut control);
    rewrite_stop(&mut control, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), live, "base(S1) plus only >S1 commands must equal the independent quiescent dump");
    for key in [&keys[0][..], b"exec:a", b"exec:b", b"script:a", b"script:b"] {
        assert_eq!(bulk(request(&mut restored, &[b"GET", key])), count.to_string().as_bytes(), "every non-idempotent operation occurs exactly once");
    }
    rewrite_stop(&mut restored, child);

    // Inspect the installed base independently of its suffix: S1 cannot
    // bisect either complete multi-key execution unit.
    let base_only = CompiledProgram::from_environment();
    let source = program.working_directory().join("appendonlydir/appendonly.aof.2.base.aof");
    multipart_files(&base_only, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    std::fs::copy(source, base_only.working_directory().join("appendonlydir/appendonly.aof.1.base.aof")).unwrap();
    let (child, mut cut, _) = start(&base_only);
    for (left, right) in [(b"exec:a".as_slice(), b"exec:b".as_slice()), (b"script:a".as_slice(), b"script:b".as_slice())] {
        let a = request(&mut cut, &[b"GET", left]);
        let b = request(&mut cut, &[b"GET", right]);
        assert_eq!(a, b, "an execution unit must be wholly before or after S1");
        assert_ne!(a, Reply::Bulk(None), "window must include each execution-unit kind");
    }
    rewrite_stop(&mut cut, child);
}

#[test]
fn reconciled_scan_reserve_abort_keeps_writes_and_old_files() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start_with_minimum(&program, b"1048576");
    client.write_all(&resp(&["INFO", "memory"])).unwrap();
    let memory = bulk_reply(&mut client, "admission heap");
    let used: u64 = info_field(&memory, "used_memory").unwrap().parse().unwrap();
    let limit = used * 5 / 4 + 4 * 1024 * 1024;
    assert_eq!(memory_request(&mut client, &["CONFIG", "SET", "maxmemory", &limit.to_string()]), "+OK\r\n");
    rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let payload = vec![b'x'; 32 * 1024];
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut writes = 0;
    loop {
        assert_eq!(request(&mut client, &[b"SET", b"hot", &payload]), Reply::Line(b"+OK".to_vec()));
        writes += 1;
        let info = rewrite_info(&mut client);
        if info["aof_rewrite_in_progress"] == "0" {
            assert_eq!(info["aof_last_bgrewrite_status"], "err");
            assert_eq!(info["firn_aof_rewrite_reserve"].parse::<u64>().unwrap(), limit / 20);
            break;
        }
        assert!(Instant::now() < deadline, "retained images never exhausted R");
    }
    assert!(writes > 1);
    assert_old_manifest(&program);
    assert_eq!(request(&mut client, &[b"INCR", b"after-abort"]), Reply::Line(b":1".to_vec()));
    assert!(std::fs::metadata(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof")).unwrap().len() > 1048576, "automatic retry threshold must actually be exceeded");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(rewrite_info(&mut client)["aof_rewrites"], "1", "abort must not start an automatic retry loop");
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"hot"])), payload);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-abort"])), b"1");
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":6002".to_vec()));
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_flush_aborts_even_in_exec_and_script() {
    for command in ["FLUSHALL", "FLUSHDB", "EXEC", "EVAL"] {
        let program = CompiledProgram::from_environment();
        fixture(&program);
        let (child, mut client, _) = start(&program);
        rewrite_start(&mut client);
        scanned_keys(&program, &mut client);
        match command {
            "EXEC" => {
                request(&mut client, &[b"MULTI"]);
                request(&mut client, &[b"FLUSHALL"]);
                request(&mut client, &[b"SET", b"after", b"value"]);
                request(&mut client, &[b"EXEC"]);
            }
            "EVAL" => { request(&mut client, &[b"EVAL", b"redis.call('FLUSHDB'); return redis.call('SET','after','value')", b"0"]); }
            other => { request(&mut client, &[other.as_bytes()]); request(&mut client, &[b"SET", b"after", b"value"]); }
        }
        assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "err");
        assert_old_manifest(&program);
        let expected = snapshot(&mut client);
        assert_eq!(expected.len(), 1);
        rewrite_stop(&mut client, child);
        let (child, mut restored, _) = start(&program);
        assert_eq!(snapshot(&mut restored), expected, "flush must never install scanned stale keys");
        rewrite_stop(&mut restored, child);
    }
}

#[test]
fn reconciled_scan_crash_uses_old_authoritative_files() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let committed = resp(&["INCR", "crash-counter"]);
    client.write_all(&committed).unwrap();
    assert_eq!(integer_reply(&mut client, "pre-crash mutation"), 1);
    let path = program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = std::fs::read(&path).unwrap();
        if bytes.windows(committed.len()).any(|window| window == committed) { break; }
        assert!(Instant::now() < deadline, "ordinary writer did not append during scan");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_old_manifest(&program);
    assert_eq!(rewrite_info(&mut client)["aof_rewrite_in_progress"], "1");
    let status = std::process::Command::new("/bin/kill").args(["-KILL", &child.id().to_string()]).status().unwrap();
    assert!(status.success());
    assert!(!child.wait_with_output().unwrap().status.success());
    let (child, mut restored, _) = start(&program);
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":6001".to_vec()));
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"crash-counter"])), b"1");
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_streams_large_history_for_a_small_live_value() {
    let program = CompiledProgram::from_environment();
    let payload: Vec<u8> = wire(&[b"MULTI"]).into_iter().cycle().take(128 * 1024).collect();
    let mut history = wire(&[b"SET", b"history", &payload]);
    history.extend(wire(&[b"SET", b"history", b"small"]));
    // Redis's loader accepts only RESP arrays in an AOF; mixed case still
    // exercises the case-insensitive MULTI/EXEC classification.
    history.extend(wire(&[b"mUlTi"]));
    history.extend(wire(&[b"INCR", b"once"]));
    history.extend(wire(&[b"eXeC"]));
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    std::fs::write(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof"), history).unwrap();
    let (child, mut client, _) = start(&program);
    let expected = snapshot(&mut client);
    rewrite_start(&mut client);
    assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok", "historical bulk payloads must stream without becoming current-image work");
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), expected);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"once"])), b"1");
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_pressure_after_s1_aborts_before_installation() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let payload = vec![b'p'; 32 * 1024];
    for _ in 0..200 {
        request(&mut client, &[b"SET", b"hot", &payload]);
    }
    let manifest = program.working_directory().join("appendonlydir/appendonly.aof.manifest");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = std::fs::read_to_string(&manifest).unwrap();
        if text.contains("appendonly.aof.2.incr.aof") {
            assert!(text.contains("appendonly.aof.1.base.aof"), "must catch the rotation before installation");
            break;
        }
        assert!(Instant::now() < deadline, "S1 never rotated");
        std::thread::sleep(Duration::from_micros(100));
    }
    // Lower the ordinary Redis limit while frozen images still await release.
    // PING is admitted even under noeviction; it drives the normal pressure check.
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"1"]), Reply::Line(b"+OK".to_vec()));
    request(&mut client, &[b"PING"]);
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "err", "post-S1 pressure must cancel retained work");
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]), Reply::Line(b"+OK".to_vec()));
    let text = std::fs::read_to_string(&manifest).unwrap();
    assert!(text.contains("appendonly.aof.1.base.aof"));
    assert!(text.contains("appendonly.aof.1.incr.aof"));
    assert!(text.contains("appendonly.aof.2.incr.aof"));
    let expected = snapshot(&mut client);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), expected);
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_post_cut_backlog_charges_rotation_reserve() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let increment = program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof");
    let mut history = std::fs::OpenOptions::new().append(true).open(&increment).unwrap();
    let obsolete = vec![b'o'; 32 * 1024];
    let record = wire(&[b"SET", b"hot", &obsolete]);
    for _ in 0..4096 { history.write_all(&record).unwrap(); }
    history.write_all(&wire(&[b"SET", b"hot", b"seed"])).unwrap();
    drop(history);
    // The 128 MiB history exceeds the default minimum, and Redis starts an
    // automatic rewrite from it as from any grown AOF; raise the minimum so
    // this case's BGREWRITEAOF is the only rewrite.
    let (child, mut client, _) = start_with_minimum(&program, b"4294967296");
    request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"67108864"]);
    rewrite_start(&mut client);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let info = rewrite_info(&mut client);
        assert_eq!(info["aof_rewrite_in_progress"], "1", "must observe S1 before completion");
        if info["firn_aof_rewrite_cut"] == "1" { break; }
        assert!(Instant::now() < deadline, "S1 never sealed the old increment");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_old_manifest(&program);
    let value = vec![b'n'; 16 * 1024];
    let mut writes = 0;
    loop {
        request(&mut client, &[b"SET", b"hot", &value]);
        writes += 1;
        let info = rewrite_info(&mut client);
        if info["aof_rewrite_in_progress"] == "0" {
            assert_eq!(info["aof_last_bgrewrite_status"], "err", "bounded live values must still exhaust the rotation backlog allowance");
            break;
        }
        assert!(Instant::now() < deadline, "post-cut backlog never aborted");
    }
    assert!(writes > 1);
    assert_old_manifest(&program);
    request(&mut client, &[b"INCR", b"after-backlog"]);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"hot"])), value);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-backlog"])), b"1");
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":6002".to_vec()));
    rewrite_stop(&mut restored, child);
}

#[test]
#[ignore = "pending firn-gap-scan-bound (status board): pinned map_scan copies a step's keys before the caller can charge them"]
fn reconciled_scan_exact_reserve_includes_one_long_key_step() {
    let program = CompiledProgram::from_environment();
    let key = vec![b'k'; 17 * 1024 * 1024];
    let data = wire(&[b"SET", &key, b"v"]);
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    std::fs::write(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof"), data).unwrap();
    let (child, mut client, _) = start(&program);
    rewrite_start(&mut client);
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "err");
    let peak: u64 = info["firn_aof_rewrite_scan_peak"].parse().unwrap();
    let reserve: u64 = info["firn_aof_rewrite_reserve"].parse().unwrap();
    assert!(peak <= reserve, "independently observed allocator growth for one scan step exceeds the entire reserve: {peak} > {reserve}");
    assert_eq!(bulk(request(&mut client, &[b"GET", &key])), b"v", "reserve refusal must not reject or remove the live key");
    rewrite_stop(&mut client, child);
}
