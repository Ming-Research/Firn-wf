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
    fixture_keys(program, 6000);
}

fn fixture_keys(program: &CompiledProgram, count: usize) {
    multipart_files(program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let mut bytes = Vec::new();
    for index in 0..count {
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

fn long_increment_history(program: &CompiledProgram) -> std::path::PathBuf {
    fixture(program);
    let increment = program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof");
    let mut history = std::fs::OpenOptions::new().append(true).open(&increment).unwrap();
    let obsolete = vec![b'o'; 32 * 1024];
    let record = wire(&[b"SET", b"hot", &obsolete]);
    for _ in 0..4096 { history.write_all(&record).unwrap(); }
    history.write_all(&wire(&[b"SET", b"hot", b"seed"])).unwrap();
    // Fixture history is already durable before the server starts, so its
    // first switch sync measures ongoing appends rather than dirty setup.
    history.sync_all().unwrap();
    drop(history);
    assert!(std::fs::metadata(&increment).unwrap().len() > 128 * 1024 * 1024);
    increment
}

#[track_caller]
fn start(program: &CompiledProgram) -> (ProgramChild, TcpStream, u16) {
    start_with_minimum(program, b"67108864")
}

#[track_caller]
fn start_with_minimum(program: &CompiledProgram, minimum: &[u8]) -> (ProgramChild, TcpStream, u16) {
    let port_lease = support::lease_port();
    let port = port_lease.port();
    let text = port.to_string();
    let child = program.spawn_on_route_with(true, &[("WF_WORKERS", "2"), ("WF_DRIVERS", "2")], &[text.as_bytes(), b"0", b"appendonly.aof", b"--auto-aof-rewrite-min-size", minimum]).with_port(&port_lease);
    let client = connect_when_ready(port, &child);
    (child, client, port)
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

/// Redis 7.0.15's hidden rdb-key-save-delay sleeps that many microseconds
/// after each key its rewrite writes (aof.c rewriteAppendOnlyFileRio), and
/// Redis's own suite sets it to keep a rewrite running. 100 microseconds per
/// key restores the pace these cases were written against, one key and one
/// such sleep per step, leaving a scan window to write behind and ahead of
/// the cursor.
fn paced_rewrite_start(client: &mut TcpStream) {
    assert_eq!(request(client, &[b"CONFIG", b"SET", b"rdb-key-save-delay", b"100"]), Reply::Line(b"+OK".to_vec()));
    rewrite_start(client);
}

fn assert_old_manifest(program: &CompiledProgram) {
    assert_eq!(std::fs::read_to_string(program.working_directory().join("appendonlydir/appendonly.aof.manifest")).unwrap(), rewrite_manifest(1, 1));
}

fn journal_path(program: &CompiledProgram) -> std::path::PathBuf {
    program.working_directory().join("appendonlydir/temp-appendonly.aof.journal")
}

#[track_caller]
fn assert_journal_removed(program: &CompiledProgram) {
    assert!(!journal_path(program).exists(), "a finished rewrite must remove its temporary journal");
}

// Counters describe completed file records, so a client reply alone no longer
// proves they have advanced. Keep the phase assertion: these cases need the
// records on disk before S1, not merely after a successful final drain.
#[track_caller]
fn journal_drained(client: &mut TcpStream, images: u64, effects: u64) -> BTreeMap<String, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let info = rewrite_info(client);
        assert_eq!(info["firn_aof_rewrite_abort_reason"], "", "journal drain must not hide an abort: {info:?}");
        assert_eq!(info["aof_rewrite_in_progress"], "1", "journal must drain during capture: {info:?}");
        assert_eq!(info["firn_aof_rewrite_scan_done"], "0", "journal must drain before S1: {info:?}");
        let actual_images = info["firn_aof_rewrite_journal_images"].parse::<u64>().unwrap();
        let actual_effects = info["firn_aof_rewrite_journal_effects"].parse::<u64>().unwrap();
        if actual_images >= images && actual_effects >= effects {
            assert_eq!((actual_images, actual_effects), (images, effects), "count complete records exactly once");
            return info;
        }
        assert!(Instant::now() < deadline, "journal did not drain: {info:?}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn reconciled_scan_disk_journal_exceeds_reserve_and_restores() {
    let program = CompiledProgram::from_environment();
    // Leave a scan window for paced disk appends without a server test hook.
    fixture_keys(&program, 12000);
    let (child, mut client, _) = start(&program);
    client.write_all(&resp(&["INFO", "memory"])).unwrap();
    let memory = bulk_reply(&mut client, "admission heap");
    let used: u64 = info_field(&memory, "used_memory").unwrap().parse().unwrap();
    let limit = (used * 5 / 4 + 8 * 1024 * 1024).max(40 * 1024 * 1024);
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", limit.to_string().as_bytes()]), Reply::Line(b"+OK".to_vec()));
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let reserve = rewrite_info(&mut client)["firn_aof_rewrite_reserve"].parse::<u64>().unwrap();
    assert_eq!(reserve, limit / 20);
    // Preserve admitted R while eliminating eviction as a competing failure.
    request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]);
    let mut payload = vec![b'p'; (reserve / 8).min(128 * 1024) as usize];
    let mut writes = 0_u64;
    loop {
        payload[0] = (writes % 251) as u8;
        assert_eq!(request(&mut client, &[b"SET", b"disk:hot", &payload]), Reply::Line(b"+OK".to_vec()));
        writes += 1;
        assert_eq!(request(&mut client, &[b"INCR", b"disk:count"]), Reply::Line(format!(":{writes}").into_bytes()));
        let info = journal_drained(&mut client, 2, 2 * (writes - 1));
        assert_eq!(info["firn_aof_rewrite_reserve"].parse::<u64>().unwrap(), reserve);
        // Waiting for this record to reach disk separates total journal size
        // from a producer burst that legitimately fills the staging allowance.
        if std::fs::metadata(journal_path(&program)).unwrap().len() > reserve { break; }
    }
    assert!(writes > 1, "exercise both first-image and effect records");
    assert_old_manifest(&program);
    let complete = rewrite_wait(&mut client);
    assert_eq!(complete["aof_last_bgrewrite_status"], "ok", "disk bytes must not retain reserve charges");
    assert_eq!(complete["firn_aof_rewrite_abort_reason"], "");
    assert_eq!(complete["firn_aof_rewrite_journal_images"], "2");
    assert_eq!(complete["firn_aof_rewrite_journal_effects"].parse::<u64>().unwrap(), 2 * (writes - 1));
    assert_eq!(complete["firn_aof_rewrite_used"], "0");
    assert_journal_removed(&program);
    let expected = snapshot(&mut client);
    assert_eq!(bulk(request(&mut client, &[b"GET", b"disk:hot"])), payload);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), expected, "the complete dataset must survive journal readback and installation");
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"disk:count"])), writes.to_string().as_bytes(), "every file record's non-idempotent effect must run exactly once");
    assert_journal_removed(&program);
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_atomic_effect_backlog_aborts_without_losing_write() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    // Keep the burst small and deterministic; remove service pressure after
    // admission, while the rewrite keeps this fixed allowance.
    client.write_all(&resp(&["INFO", "memory"])).unwrap();
    let memory = bulk_reply(&mut client, "admission heap");
    let used: u64 = info_field(&memory, "used_memory").unwrap().parse().unwrap();
    let limit = (used * 5 / 4 + 8 * 1024 * 1024).max(40 * 1024 * 1024);
    request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", limit.to_string().as_bytes()]);
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let reserve = rewrite_info(&mut client)["firn_aof_rewrite_reserve"].parse::<u64>().unwrap();
    request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]);
    request(&mut client, &[b"SET", b"backlog:hot", b"anchor"]);
    journal_drained(&mut client, 1, 0);
    assert!(std::fs::metadata(journal_path(&program)).unwrap().len() > 0);
    // The first write anchors the key, so this is an effect, not oversized
    // first-image scratch. No drainer can run inside its publishing statement.
    let payload = vec![b'b'; reserve as usize + 1];
    assert_eq!(request(&mut client, &[b"SET", b"backlog:hot", &payload]), Reply::Line(b"+OK".to_vec()));
    let failed = rewrite_wait(&mut client);
    assert_eq!(failed["aof_last_bgrewrite_status"], "err");
    assert_eq!(failed["firn_aof_rewrite_abort_reason"], "journal_backlog");
    assert_eq!(failed["firn_aof_rewrite_journal_images"], "1");
    assert_eq!(failed["firn_aof_rewrite_journal_effects"], "0", "an unstageable record never reaches disk");
    assert_old_manifest(&program);
    assert_journal_removed(&program);
    assert_eq!(request(&mut client, &[b"INCR", b"after-backlog"]), Reply::Line(b":1".to_vec()));
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"backlog:hot"])), payload);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-backlog"])), b"1");
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":6002".to_vec()));
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_restores_live_dump_and_non_idempotent_units() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut control, port) = start(&program);
    let binary = b"binary\0\r\n\xff";
    request(&mut control, &[b"SET", binary, b"before"]);
    paced_rewrite_start(&mut control);
    let keys = scanned_keys(&program, &mut control);
    // Counters the scan has not observed yet: a write journaled as a bare
    // effect before the scan observes its key would be counted twice.
    let observed: Vec<Vec<Vec<u8>>> = prefix_records(&std::fs::read(program.working_directory().join("appendonlydir/temp-appendonly.aof.base")).expect("the scan's temporary base"));
    let unobserved: Vec<Vec<u8>> = (0..6000).step_by(5).rev()
        .map(|index| format!("kind:0:{index}").into_bytes())
        .filter(|key| !observed.iter().any(|words| words.get(1) == Some(key)))
        .take(20).collect();
    assert!(!unobserved.is_empty(), "the scan must still have string keys ahead of it");
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = running.clone();
    let worker_keys = keys.clone();
    let worker_unobserved = unobserved.clone();
    let monitor = child.monitor();
    let worker = std::thread::spawn(move || {
        let mut client = connect_when_ready(port, &monitor);
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
            // Transfer and store commands change their keys through helpers
            // that do not capture by themselves; their new destinations reach
            // base(S1) only through the journal.
            request(&mut client, &[b"LMOVE", &worker_keys[1], b"moved:list", b"LEFT", b"RIGHT"]);
            request(&mut client, &[b"LTRIM", b"moved:list", b"-2", b"-1"]);
            request(&mut client, &[b"SMOVE", &worker_keys[2], b"moved:set", b"replacement"]);
            request(&mut client, &[b"SUNIONSTORE", b"stored:set", &worker_keys[2], b"moved:set"]);
            // Plain single-key writes journal their own bytes once the key
            // has a journal record in this rewrite.
            request(&mut client, &[b"INCR", b"single:count"]);
            request(&mut client, &[b"APPEND", b"single:text", b"x"]);
            // Their AOF forms differ from the commands sent: PXAT,
            // PEXPIREAT, SET KEEPTTL, SREM and HSET.
            request(&mut client, &[b"SET", b"single:ex", value.as_bytes(), b"EX", b"100000"]);
            request(&mut client, &[b"EXPIRE", b"single:count", b"100000"]);
            request(&mut client, &[b"GETEX", b"single:text", b"PX", b"100000000"]);
            request(&mut client, &[b"INCRBYFLOAT", b"single:float", b"1.5"]);
            request(&mut client, &[b"HINCRBYFLOAT", b"single:hash", b"field", b"0.5"]);
            request(&mut client, &[b"SADD", b"single:set", b"a", b"b", b"c"]);
            request(&mut client, &[b"SPOP", b"single:set"]);
            for key in &worker_unobserved {
                request(&mut client, &[b"INCR", key]);
            }
            // Pace the writer so its journal stays well inside the reserve.
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
fn reconciled_scan_repeated_sets_journal_one_image_then_effects() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    paced_rewrite_start(&mut client);
    let keys = scanned_keys(&program, &mut client);
    let before = rewrite_info(&mut client);
    assert_eq!(before["firn_aof_rewrite_abort_reason"], "");
    assert_eq!(before["firn_aof_rewrite_journal_images"], "0");
    assert_eq!(before["firn_aof_rewrite_journal_effects"], "0");
    // Cover replacement of an existing list as well as a newly created key.
    // With the old SET path each write loses last_write and emits an image.
    for key in [keys[1].as_slice(), b"set:journal:new".as_slice()] {
        for index in 0..8 {
            let value = index.to_string();
            assert_eq!(request(&mut client, &[b"SET", key, value.as_bytes()]), Reply::Line(b"+OK".to_vec()));
        }
    }
    let captured = journal_drained(&mut client, 2, 14);
    assert_eq!(captured["firn_aof_rewrite_scan_done"], "0", "all writes must precede the cut");
    assert_eq!(captured["firn_aof_rewrite_journal_images"], "2", "only each key's first write needs an image");
    assert_eq!(captured["firn_aof_rewrite_journal_effects"], "14", "later SETs must use statement bytes");
    let complete = rewrite_wait(&mut client);
    assert_eq!(complete["aof_last_bgrewrite_status"], "ok");
    assert_eq!(complete["firn_aof_rewrite_abort_reason"], "");
    assert_eq!(complete["firn_aof_rewrite_journal_images"], "2");
    assert_eq!(complete["firn_aof_rewrite_journal_effects"], "14");
    assert_eq!(complete["firn_aof_rewrite_scan_done"], "1");
    assert!(complete["firn_aof_rewrite_scan_steps"].parse::<u64>().unwrap() > 0);
    assert_eq!(complete["firn_aof_rewrite_used"], "0");
    assert!(complete["firn_aof_rewrite_peak"].parse::<u64>().unwrap() > 0);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    for key in [keys[1].as_slice(), b"set:journal:new".as_slice()] {
        assert_eq!(bulk(request(&mut restored, &[b"GET", key])), b"7");
    }
    rewrite_stop(&mut restored, child);
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
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    assert!(journal_path(&program).exists(), "admitted scan owns a temporary journal");
    let admitted = rewrite_info(&mut client);
    let reserve = admitted["firn_aof_rewrite_reserve"].parse::<u64>().unwrap();
    assert_eq!(reserve, limit / 20);
    // Keep the admitted R but remove service pressure as a competing cause.
    // Repeated small effects can exhaust scratch, chunks or journal slots;
    // a fresh image larger than R must fail sequence_room first, as Capture.
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]), Reply::Line(b"+OK".to_vec()));
    let capturing = rewrite_info(&mut client);
    assert_eq!(capturing["aof_rewrite_in_progress"], "1");
    assert_eq!(capturing["firn_aof_rewrite_scan_done"], "0", "the oversized replacement must still reach image capture");
    assert_eq!(capturing["firn_aof_rewrite_reserve"].parse::<u64>().unwrap(), reserve, "CONFIG must not change the admitted allowance");
    // Also exceed the automatic-retry threshold with ordinary accepted bytes.
    let payload = vec![b'x'; (reserve + 1).max(1048577) as usize];
    assert_eq!(request(&mut client, &[b"SET", b"hot", &payload]), Reply::Line(b"+OK".to_vec()));
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "err");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "capture", "oversized first-image scratch must be the initiating failure");
    assert_journal_removed(&program);
    assert_eq!(info["firn_aof_rewrite_reserve"].parse::<u64>().unwrap(), reserve);
    // Repeated ordinary writes remain accepted after the capture failure.
    for _ in 0..2 {
        assert_eq!(request(&mut client, &[b"SET", b"hot", &payload]), Reply::Line(b"+OK".to_vec()));
    }
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
        paced_rewrite_start(&mut client);
        scanned_keys(&program, &mut client);
        assert!(journal_path(&program).exists(), "scan owns a temporary journal before flush");
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
        let failed = rewrite_wait(&mut client);
        assert_eq!(failed["aof_last_bgrewrite_status"], "err");
        assert_eq!(failed["firn_aof_rewrite_abort_reason"], "flush");
        assert_journal_removed(&program);
        assert_eq!(failed["firn_aof_rewrite_scan_done"], "0");
        let used = failed["firn_aof_rewrite_used"].parse::<u64>().unwrap();
        let peak = failed["firn_aof_rewrite_peak"].parse::<u64>().unwrap();
        assert!(used > 0 && peak >= used, "failure must retain its reserve snapshot after cleanup");
        assert_old_manifest(&program);
        let expected = snapshot(&mut client);
        assert_eq!(expected.len(), 1);
        let retained = rewrite_info(&mut client);
        for field in ["firn_aof_rewrite_abort_reason", "firn_aof_rewrite_used", "firn_aof_rewrite_peak"] {
            assert_eq!(retained[field], failed[field], "service and cleanup must preserve the first failure");
        }
        rewrite_stop(&mut client, child);
        let (child, mut restored, _) = start(&program);
        assert_eq!(snapshot(&mut restored), expected, "flush must never install scanned stale keys");
        rewrite_stop(&mut restored, child);
    }
}

#[test]
fn reconciled_scan_cleanup_failure_preserves_flush_and_serves_after_restart() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    let initial = rewrite_info(&mut client);
    assert_eq!(initial["firn_aof_rewrite_cleanup_failed"], "0");
    assert_eq!(initial["firn_aof_rewrite_journal_leftover"], "0");
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    request(&mut client, &[b"SET", b"cleanup:anchor", b"before"]);
    journal_drained(&mut client, 1, 0);
    let journal = journal_path(&program);
    assert!(std::fs::metadata(&journal).unwrap().len() > 0);
    // Renaming leaves both open journal handles valid. A nonempty directory
    // at the old name makes unlink fail without permissions or root assumptions.
    let detached = journal.with_extension("detached-for-test");
    std::fs::rename(&journal, &detached).unwrap();
    std::fs::create_dir(&journal).unwrap();
    let sentinel = journal.join("keep");
    std::fs::write(&sentinel, b"cleanup obstruction").unwrap();
    let scanning = rewrite_info(&mut client);
    assert_eq!(scanning["aof_rewrite_in_progress"], "1");
    assert_eq!(scanning["firn_aof_rewrite_scan_done"], "0");
    assert_eq!(request(&mut client, &[b"FLUSHALL"]), Reply::Line(b"+OK".to_vec()));
    assert_eq!(request(&mut client, &[b"SET", b"after-flush", b"retained"]), Reply::Line(b"+OK".to_vec()));
    let failed = rewrite_wait(&mut client);
    assert_eq!(failed["aof_last_bgrewrite_status"], "err");
    assert_eq!(failed["firn_aof_rewrite_abort_reason"], "flush", "a later unlink failure must not replace the initiating cause");
    assert_eq!(failed["firn_aof_rewrite_scan_done"], "0");
    assert!(failed["firn_aof_rewrite_cleanup_failed"].parse::<u64>().unwrap() > 0, "failed cleanup must remain separately observable");
    assert_eq!(failed["firn_aof_rewrite_journal_leftover"], "1");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"cleanup obstruction");
    assert_old_manifest(&program);
    assert_eq!(request(&mut client, &[b"INCR", b"after-cleanup"]), Reply::Line(b":1".to_vec()));
    let retained = rewrite_info(&mut client);
    for field in ["firn_aof_rewrite_abort_reason", "firn_aof_rewrite_used", "firn_aof_rewrite_peak", "firn_aof_rewrite_cleanup_failed", "firn_aof_rewrite_journal_leftover"] {
        assert_eq!(retained[field], failed[field], "ordinary service must retain {field}");
    }
    // A refused admission starts new attempt diagnostics, but it cannot claim
    // that an obstruction it never removed has disappeared.
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"1"]), Reply::Line(b"+OK".to_vec()));
    assert_eq!(memory_request(&mut client, &["BGREWRITEAOF"]), "-ERR Can't execute an AOF background rewriting. Please check the server logs for more information.\r\n");
    let refused = rewrite_info(&mut client);
    assert_eq!(refused["firn_aof_rewrite_abort_reason"], "admission");
    assert_eq!(refused["firn_aof_rewrite_cleanup_failed"], "0");
    assert_eq!(refused["firn_aof_rewrite_journal_leftover"], "1");
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]), Reply::Line(b"+OK".to_vec()));
    let expected = snapshot(&mut client);
    assert_eq!(expected.len(), 2);
    rewrite_stop(&mut client, child);
    std::fs::remove_file(&detached).unwrap();
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), expected, "an unremovable unlisted journal must not prevent authoritative recovery");
    let startup = rewrite_info(&mut restored);
    assert_eq!(startup["firn_aof_rewrite_cleanup_failed"], "0", "startup is not a rewrite attempt");
    assert_eq!(startup["firn_aof_rewrite_journal_leftover"], "1");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"cleanup obstruction");
    assert_eq!(request(&mut restored, &[b"INCR", b"after-cleanup"]), Reply::Line(b":2".to_vec()));
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_startup_cleanup_failure_serves_and_retry_removes_leftover() {
    let program = CompiledProgram::from_environment();
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[&["SET", "authoritative", "retained"]]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let journal = journal_path(&program);
    std::fs::create_dir(&journal).unwrap();
    let sentinel = journal.join("keep");
    std::fs::write(&sentinel, b"startup obstruction").unwrap();
    let (child, mut client, _) = start(&program);
    assert_eq!(bulk(request(&mut client, &[b"GET", b"authoritative"])), b"retained");
    let startup = rewrite_info(&mut client);
    assert_eq!(startup["firn_aof_rewrite_cleanup_failed"], "0");
    assert_eq!(startup["firn_aof_rewrite_journal_leftover"], "1", "startup must report its failed unlink while continuing service");
    assert_eq!(startup["aof_rewrites"], "0");
    rewrite_start(&mut client);
    let failed = rewrite_wait(&mut client);
    assert_eq!(failed["aof_last_bgrewrite_status"], "err");
    assert_eq!(failed["firn_aof_rewrite_abort_reason"], "journal_io");
    assert!(failed["firn_aof_rewrite_cleanup_failed"].parse::<u64>().unwrap() > 0);
    assert_eq!(failed["firn_aof_rewrite_journal_leftover"], "1");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"startup obstruction", "failed pre-open cleanup must not reuse or truncate the path");
    assert_old_manifest(&program);
    assert_eq!(request(&mut client, &[b"INCR", b"after-obstruction"]), Reply::Line(b":1".to_vec()));
    std::fs::remove_file(&sentinel).unwrap();
    std::fs::remove_dir(&journal).unwrap();
    rewrite_start(&mut client);
    let recovered = rewrite_wait(&mut client);
    assert_eq!(recovered["aof_last_bgrewrite_status"], "ok");
    assert_eq!(recovered["firn_aof_rewrite_abort_reason"], "");
    assert_eq!(recovered["firn_aof_rewrite_cleanup_failed"], "0", "accepted admission must clear the preceding cleanup count");
    assert_eq!(recovered["firn_aof_rewrite_journal_leftover"], "0", "successful absence/unlink must clear the persistent flag");
    assert_journal_removed(&program);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"authoritative"])), b"retained");
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-obstruction"])), b"1");
    let clean_startup = rewrite_info(&mut restored);
    assert_eq!(clean_startup["firn_aof_rewrite_cleanup_failed"], "0");
    assert_eq!(clean_startup["firn_aof_rewrite_journal_leftover"], "0");
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_base_open_failure_removes_journal() {
    let program = CompiledProgram::from_environment();
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[&["SET", "authoritative", "retained"]]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let base = program.working_directory().join("appendonlydir/temp-appendonly.aof.base");
    std::fs::create_dir(&base).unwrap();
    let sentinel = base.join("keep");
    std::fs::write(&sentinel, b"base obstruction").unwrap();
    let (child, mut client, _) = start(&program);
    rewrite_start(&mut client);
    let failed = rewrite_wait(&mut client);
    assert_eq!(failed["aof_last_bgrewrite_status"], "err");
    assert_eq!(failed["firn_aof_rewrite_abort_reason"], "base_io");
    assert_eq!(failed["firn_aof_rewrite_scan_steps"], "0", "the base must fail before a scan worker starts");
    assert_eq!(failed["firn_aof_rewrite_cleanup_failed"], "0", "closing/removing the journal succeeds on this early-exit branch");
    assert_eq!(failed["firn_aof_rewrite_journal_leftover"], "0");
    assert_journal_removed(&program);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"base obstruction");
    assert_old_manifest(&program);
    assert_eq!(request(&mut client, &[b"INCR", b"after-base-refusal"]), Reply::Line(b":1".to_vec()));
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"authoritative"])), b"retained");
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-base-refusal"])), b"1");
    assert_journal_removed(&program);
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_retry_clears_abort_diagnostics() {
    for refused in [true, false] {
        let program = CompiledProgram::from_environment();
        fixture(&program);
        let obstacle = program.working_directory().join("appendonlydir/appendonly.aof.2.base.aof");
        // Ordinary host refusal at installation seeds scan_done=1 and file
        // record counters, without racing pressure against publication.
        std::fs::create_dir(&obstacle).unwrap();
        let (child, mut client, _) = start(&program);
        paced_rewrite_start(&mut client);
        scanned_keys(&program, &mut client);
        // Seed a high peak with one complete image, then leave only a small
        // live value for the next scan. Disk draining cannot erase that peak.
        let large_image = vec![b'i'; 512 * 1024];
        assert_eq!(request(&mut client, &[b"SET", b"hot", &large_image]), Reply::Line(b"+OK".to_vec()));
        let payload = vec![b'p'; 32 * 1024];
        for _ in 0..200 {
            assert_eq!(request(&mut client, &[b"SET", b"hot", &payload]), Reply::Line(b"+OK".to_vec()));
        }
        let captured = journal_drained(&mut client, 1, 200);
        assert_eq!(captured["firn_aof_rewrite_scan_done"], "0", "seed every image/effect before S1");
        assert_eq!(captured["firn_aof_rewrite_journal_images"], "1");
        assert_eq!(captured["firn_aof_rewrite_journal_effects"], "200");
        let failed = rewrite_wait(&mut client);
        assert_eq!(failed["aof_last_bgrewrite_status"], "err");
        assert_eq!(failed["firn_aof_rewrite_abort_reason"], "install_io");
        assert_eq!(failed["firn_aof_rewrite_journal_images"], "1");
        assert_eq!(failed["firn_aof_rewrite_journal_effects"], "200");
        assert_eq!(failed["firn_aof_rewrite_scan_done"], "1");
        let previous_steps = failed["firn_aof_rewrite_scan_steps"].parse::<u64>().unwrap();
        let previous_used = failed["firn_aof_rewrite_used"].parse::<u64>().unwrap();
        let previous_peak = failed["firn_aof_rewrite_peak"].parse::<u64>().unwrap();
        assert!(previous_steps > 0);
        assert!(previous_used > 0 && previous_peak >= previous_used, "seed failure must retain its reserve snapshot");
        assert!(previous_peak >= 1024 * 1024, "first-image scratch and its staged copy must seed a distinguishable peak");
        std::fs::remove_dir(&obstacle).unwrap();
        if refused {
            assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"1"]), Reply::Line(b"+OK".to_vec()));
            // This reaches aof_rw_admit (unlike the command's busy refusal),
            // and has no worker: all numeric diagnostics must start at zero.
            assert_eq!(memory_request(&mut client, &["BGREWRITEAOF"]), "-ERR Can't execute an AOF background rewriting. Please check the server logs for more information.\r\n");
            let reset = rewrite_info(&mut client);
            assert_eq!(reset["aof_rewrite_in_progress"], "0");
            assert_eq!(reset["aof_last_bgrewrite_status"], "err");
            assert_eq!(reset["firn_aof_rewrite_abort_reason"], "admission", "a refused S0 must replace the previous reason");
            for field in ["firn_aof_rewrite_journal_images", "firn_aof_rewrite_journal_effects", "firn_aof_rewrite_scan_steps", "firn_aof_rewrite_scan_done", "firn_aof_rewrite_used", "firn_aof_rewrite_peak"] {
                assert_eq!(reset[field], "0", "refused admission must clear {field} from its nonzero predecessor");
            }
            assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]), Reply::Line(b"+OK".to_vec()));
        } else {
            paced_rewrite_start(&mut client);
            let reset = rewrite_info(&mut client);
            assert_eq!(reset["aof_rewrite_in_progress"], "1", "reset evidence must precede completion");
            assert_eq!(reset["firn_aof_rewrite_abort_reason"], "", "accepted S0 clears the prior failure immediately");
            assert_eq!(reset["firn_aof_rewrite_journal_images"], "0");
            assert_eq!(reset["firn_aof_rewrite_journal_effects"], "0");
            assert_eq!(reset["firn_aof_rewrite_scan_done"], "0");
            // S0 charges fixed storage and the worker may already have scanned
            // keys. The old current use no longer retains journal bytes, so it
            // cannot order these samples. Its deliberately large image peak
            // must still be cleared before scanning the now-small live value.
            assert!(reset["firn_aof_rewrite_scan_steps"].parse::<u64>().unwrap() < previous_steps, "accepted S0 must restart the scan counter");
            let current_used = reset["firn_aof_rewrite_used"].parse::<u64>().unwrap();
            let current_peak = reset["firn_aof_rewrite_peak"].parse::<u64>().unwrap();
            let reserve = reset["firn_aof_rewrite_reserve"].parse::<u64>().unwrap();
            assert!(current_used > 0 && current_used <= current_peak && current_peak <= reserve);
            assert!(current_peak < previous_peak, "accepted S0 must discard the previous attempt's peak");
            let succeeded = rewrite_wait(&mut client);
            assert_eq!(succeeded["aof_last_bgrewrite_status"], "ok");
            assert_eq!(succeeded["firn_aof_rewrite_abort_reason"], "");
            assert_eq!(succeeded["firn_aof_rewrite_journal_images"], "0");
            assert_eq!(succeeded["firn_aof_rewrite_journal_effects"], "0");
            assert_eq!(succeeded["firn_aof_rewrite_used"], "0");
            assert_eq!(succeeded["firn_aof_rewrite_scan_done"], "1");
            assert_eq!(bulk(request(&mut client, &[b"GET", b"hot"])), payload);
        }
        rewrite_stop(&mut client, child);
    }
}

#[test]
fn reconciled_scan_crash_uses_old_authoritative_files() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    let committed = resp(&["INCR", "crash-counter"]);
    client.write_all(&committed).unwrap();
    assert_eq!(integer_reply(&mut client, "pre-crash mutation"), 1);
    journal_drained(&mut client, 1, 0);
    assert!(std::fs::metadata(journal_path(&program)).unwrap().len() > 0);
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
    assert!(journal_path(&program).exists(), "SIGKILL leaves the journal for startup cleanup");
    let (child, mut restored, _) = start(&program);
    assert_journal_removed(&program);
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":6001".to_vec()));
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"crash-counter"])), b"1");
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_preserves_manifest_listed_journal_name() {
    let program = CompiledProgram::from_environment();
    let manifest = "file temp-appendonly.aof.journal seq 1 type b\nfile appendonly.aof.1.incr.aof seq 1 type i\n";
    multipart_files(
        &program,
        &[("temp-appendonly.aof.journal", &[&["SET", "authoritative", "retained"]]), ("appendonly.aof.1.incr.aof", &[])],
        manifest,
    );
    let authoritative = std::fs::read(journal_path(&program)).unwrap();
    let (child, mut client, _) = start(&program);
    assert_eq!(bulk(request(&mut client, &[b"GET", b"authoritative"])), b"retained");
    assert_eq!(std::fs::read(journal_path(&program)).unwrap(), authoritative, "startup must not unlink a manifest-selected base");
    rewrite_start(&mut client);
    let failed = rewrite_wait(&mut client);
    assert_eq!(failed["aof_last_bgrewrite_status"], "err");
    assert_eq!(failed["firn_aof_rewrite_abort_reason"], "journal_io", "temporary-name collision must refuse opening the authoritative file");
    assert_eq!(std::fs::read(journal_path(&program)).unwrap(), authoritative, "neither open nor failure cleanup may truncate or unlink the base");
    assert_eq!(std::fs::read_to_string(program.working_directory().join("appendonlydir/appendonly.aof.manifest")).unwrap(), manifest);
    assert_eq!(request(&mut client, &[b"INCR", b"after-collision"]), Reply::Line(b":1".to_vec()));
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"authoritative"])), b"retained");
    assert_eq!(bulk(request(&mut restored, &[b"GET", b"after-collision"])), b"1");
    assert_eq!(std::fs::read(journal_path(&program)).unwrap(), authoritative);
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
fn reconciled_scan_pressure_after_s1_keeps_rewrite_and_refuses_denyoom() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    client.write_all(&resp(&["INFO", "memory"])).unwrap();
    let before = bulk_reply(&mut client, "AOF exclusion before rewrite");
    let aof_capacity: u64 = info_field(&before, "mem_not_counted_for_evict").unwrap().parse().unwrap();
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    // No client mutation has appended bytes since startup and S1 is still
    // ahead. Ordinary AOF capacity is unchanged, so this additional exclusion
    // must be the live rewrite ledger rather than a collecting-buffer charge.
    client.write_all(&resp(&["INFO", "all"])).unwrap();
    let capturing = bulk_reply(&mut client, "rewrite memory exclusion");
    assert_eq!(info_field(&capturing, "aof_rewrite_in_progress").as_deref(), Some("1"), "{capturing}");
    assert_eq!(info_field(&capturing, "firn_aof_rewrite_cut").as_deref(), Some("0"), "{capturing}");
    let rewrite_used: u64 = info_field(&capturing, "firn_aof_rewrite_used").unwrap().parse().unwrap();
    let excluded: u64 = info_field(&capturing, "mem_not_counted_for_evict").unwrap().parse().unwrap();
    assert!(rewrite_used > 0, "{capturing}");
    assert_eq!(excluded, aof_capacity + rewrite_used, "rewrite allocations must be excluded exactly once: {capturing}");
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
    // Service pressure still refuses DENYOOM under noeviction, but it must
    // not cancel rewrite work whose bounded memory is outside maxmemory.
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"1"]), Reply::Line(b"+OK".to_vec()));
    assert_eq!(request(&mut client, &[b"PING"]), Reply::Line(b"+PONG".to_vec()));
    assert_eq!(memory_request(&mut client, &["SET", "refused", "value"]),
        "-OOM command not allowed when used memory > 'maxmemory'.\r\n");
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "post-S1 service pressure must leave the bounded rewrite intact: {info:?}");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "", "{info:?}");
    assert_eq!(info["firn_aof_rewrite_used"], "0", "successful cleanup must release the rewrite charge");
    assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"maxmemory", b"0"]), Reply::Line(b"+OK".to_vec()));
    let text = std::fs::read_to_string(&manifest).unwrap();
    assert_eq!(text, rewrite_manifest(2, 2), "successful rewrite must publish the new base");
    assert_eq!(request(&mut client, &[b"GET", b"refused"]), Reply::Bulk(None));
    let expected = snapshot(&mut client);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), expected);
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_long_history_completes_under_writes_without_duplicate_effects() {
    let program = CompiledProgram::from_environment();
    long_increment_history(&program);
    // Keep the 128-MiB history from triggering an automatic rewrite. Default
    // unlimited maxmemory retains R = 16 MiB, allowing ordinary switch I/O
    // while the old history reread still produces enough backlog to exceed R.
    let (child, mut control, port) = start_with_minimum(&program, b"4294967296");
    assert_eq!(request(&mut control, &[b"CONFIG", b"SET", b"rdb-key-save-delay", b"100"]), Reply::Line(b"+OK".to_vec()));
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = running.clone();
    let monitor = child.monitor();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut client = connect_when_ready(port, &monitor);
        let mut count = 0_u64;
        let mut appended = Vec::new();
        let mut members = Vec::new();
        let mut cut_count = None;
        let mut last_value = vec![b'n'; 16 * 1024];
        loop {
            count += 1;
            let marker = count.to_le_bytes();
            last_value[..marker.len()].copy_from_slice(&marker);
            // A small pipeline keeps the old reread's collecting buffer
            // growing, while bounded values and staging avoid scan pressure.
            let mut batch = Vec::new();
            batch.extend(wire(&[b"SET", b"hot", &last_value]));
            batch.extend(wire(&[b"INCR", b"history:counter"]));
            batch.extend(wire(&[b"APPEND", b"history:text", &marker]));
            batch.extend(wire(&[b"RPUSH", b"history:list", &marker]));
            client.write_all(&batch).unwrap();
            assert_eq!(read_reply(&mut client), Reply::Line(b"+OK".to_vec()));
            assert_eq!(read_reply(&mut client), Reply::Line(format!(":{count}").into_bytes()));
            assert_eq!(read_reply(&mut client), Reply::Line(format!(":{}", count * 8).into_bytes()));
            assert_eq!(read_reply(&mut client), Reply::Line(format!(":{count}").into_bytes()));
            appended.extend_from_slice(&marker);
            members.push(Reply::Bulk(Some(marker.to_vec())));
            let progress = rewrite_info(&mut client);
            if progress["firn_aof_rewrite_cut"] == "1" && cut_count.is_none() { cut_count = Some(count); }
            if count == 1 { ready_tx.send(()).unwrap(); }
            // At least one whole acknowledged pipeline must follow an
            // observed S1, even if the fast switch finished before sampling.
            if !worker_running.load(Ordering::Acquire) && cut_count.is_none_or(|cut| count > cut) { break; }
            std::thread::sleep(Duration::from_millis(1));
        }
        (count, cut_count, last_value, appended, members)
    });
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    rewrite_start(&mut control);
    scanned_keys(&program, &mut control);
    assert_eq!(request(&mut control, &[b"CONFIG", b"SET", b"rdb-key-save-delay", b"0"]), Reply::Line(b"+OK".to_vec()));
    let info = rewrite_wait(&mut control);
    running.store(false, Ordering::Release);
    let (count, cut_count, hot, appended, members) = worker.join().unwrap();
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "long history must not suspend ordinary draining at S1; the old full reread exhausts R with rotation_reserve: {info:?}");
    let cut_count = cut_count.expect("the continuous writer must observe S1");
    assert!(cut_count > 1 && count > cut_count, "writes must be acknowledged before rewrite admission and after an observed S1");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "");
    assert_eq!(info["firn_aof_rewrite_cut"], "1");
    assert_eq!(info["firn_aof_rewrite_reserve"], "16777216");
    let peak: u64 = info["firn_aof_rewrite_peak"].parse().unwrap();
    assert!(peak <= 16_777_216, "removing the reread must retain the reserve bound");
    let check = |client: &mut TcpStream| {
        assert_eq!(bulk(request(client, &[b"GET", b"hot"])), hot);
        assert_eq!(bulk(request(client, &[b"GET", b"history:counter"])), count.to_string().into_bytes());
        assert_eq!(bulk(request(client, &[b"GET", b"history:text"])), appended);
        assert_eq!(array(request(client, &[b"LRANGE", b"history:list", b"0", b"-1"])), members);
        assert_eq!(request(client, &[b"DBSIZE"]), Reply::Line(b":6004".to_vec()));
    };
    check(&mut control);
    rewrite_stop(&mut control, child);
    let (child, mut restored, _) = start(&program);
    check(&mut restored);
    rewrite_stop(&mut restored, child);
}

#[test]
fn reconciled_scan_post_cut_backlog_charges_rotation_reserve() {
    let program = CompiledProgram::from_environment();
    let increment = long_increment_history(&program);
    // The 128 MiB history exceeds the default minimum, and Redis starts an
    // automatic rewrite from it as from any grown AOF; raise the minimum so
    // this case's BGREWRITEAOF is the only rewrite.
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let child = program.spawn_on_route_ignoring_xfsz(true, &[text.as_bytes(), b"0", b"appendonly.aof", b"--auto-aof-rewrite-min-size", b"4294967296"]).with_port(&lease);
    let mut client = connect_when_ready(port, &child);
    // A real failed append makes its boundary outcome unknown under the
    // Whitefoot I/O contract. Successful recovery must retain that unknown
    // state until the exceptional switch-time reread refreshes it. This
    // host failure, rather than ordinary history, now keeps rotation busy.
    let previous = aof_write_errors::file_limit(&child, std::fs::metadata(&increment).unwrap().len());
    assert_eq!(request(&mut client, &[b"SET", b"hot", b"failed-append"]), Reply::Line(b"+OK".to_vec()));
    let recovery_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = rewrite_info(&mut client);
        if info["aof_last_write_status"] == "err" { break; }
        assert!(Instant::now() < recovery_deadline, "file limit did not fail the ordinary append: {info:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    aof_write_errors::file_limit(&child, previous.soft);
    loop {
        let info = rewrite_info(&mut client);
        if info["aof_last_write_status"] == "ok" && info["aof_buffer_length"] == "0" { break; }
        assert!(Instant::now() < recovery_deadline, "ordinary append did not recover after restoring its file limit: {info:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
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
            assert_eq!(info["firn_aof_rewrite_abort_reason"], "rotation_reserve", "post-cut collecting-buffer growth must identify its own resource failure");
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

/// Keys above 512 elements or 64 KiB serialized, scanned and written during
/// the rewrite, rewrite and restore as main's replay and Redis rewrite them.
#[test]
fn reconciled_scan_rewrites_large_keys_scanned_and_written() {
    let program = CompiledProgram::from_environment();
    fixture(&program);
    let (child, mut client, _) = start(&program);
    let members: Vec<Vec<u8>> = (0..2000).map(|index| format!("member:{index}").into_bytes()).collect();
    for chunk in members.chunks(500) {
        let mut args: Vec<&[u8]> = vec![b"SADD", b"large:set"];
        args.extend(chunk.iter().map(|member| member.as_slice()));
        assert_eq!(request(&mut client, &args), Reply::Line(b":500".to_vec()));
    }
    let text = vec![b't'; 200 * 1024];
    request(&mut client, &[b"SET", b"large:text", &text]);
    paced_rewrite_start(&mut client);
    scanned_keys(&program, &mut client);
    request(&mut client, &[b"SADD", b"large:set", b"added"]);
    request(&mut client, &[b"APPEND", b"large:text", b"!"]);
    assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok", "large keys must not abort the rewrite");
    let live = snapshot(&mut client);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(snapshot(&mut restored), live, "large images restore whole");
    rewrite_stop(&mut restored, child);
}

/// A key longer than R can never enter a scan step within R: the scan must
/// abort with `reserve` rather than wait, and neither the independently
/// sampled heap growth nor the reserve ledger may pass R on the way.
#[test]
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
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "reserve", "a key that can never fit R must abort as reserve exhaustion: {info:?}");
    let peak: u64 = info["firn_aof_rewrite_scan_peak"].parse().unwrap();
    let reserve: u64 = info["firn_aof_rewrite_reserve"].parse().unwrap();
    assert_eq!(reserve, 16 * 1024 * 1024, "unlimited maxmemory must retain its explicit 16-MiB rewrite allowance");
    assert!(peak <= reserve, "independently observed allocator growth for one scan step exceeds the entire reserve: {peak} > {reserve}");
    let ledger_peak: u64 = info["firn_aof_rewrite_peak"].parse().unwrap();
    let ledger_used: u64 = info["firn_aof_rewrite_used"].parse().unwrap();
    assert!(ledger_used <= ledger_peak && ledger_peak <= reserve, "the reserve ledger must never record a charge beyond R: used {ledger_used}, peak {ledger_peak}, reserve {reserve}");
    assert_old_manifest(&program);
    assert_eq!(bulk(request(&mut client, &[b"GET", &key])), b"v", "reserve refusal must not reject or remove the live key");
    rewrite_stop(&mut client, child);
}

/// A scan step crosses many keys and yields only once per time slice, so a
/// keyspace far larger than one key per paced step rewrites while a client
/// keeps writing. Count hint 1 with a sleep per step needs one step per key
/// and, at the driver's millisecond timer, longer than rewrite_wait allows.
#[test]
fn reconciled_scan_time_slices_many_keys_per_step_under_writes() {
    let program = CompiledProgram::from_environment();
    let keys = 100_000_usize;
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let mut bytes = Vec::new();
    for index in 0..keys {
        let key = format!("slice:{index}");
        let value = index.to_string();
        bytes.extend(wire(&[b"SET", key.as_bytes(), value.as_bytes()]));
    }
    std::fs::write(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof"), bytes).unwrap();
    let (child, mut control, port) = start(&program);
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = running.clone();
    let monitor = child.monitor();
    let worker = std::thread::spawn(move || {
        let mut client = connect_when_ready(port, &monitor);
        let mut count = 0_u64;
        while worker_running.load(Ordering::Acquire) || count < 100 {
            count += 1;
            assert_eq!(request(&mut client, &[b"INCR", b"slice:counter"]), Reply::Line(format!(":{count}").into_bytes()));
            let key = format!("slice:{}", (count as usize * 7919) % keys);
            let value = format!("written:{count}");
            assert_eq!(request(&mut client, &[b"SET", key.as_bytes(), value.as_bytes()]), Reply::Line(b"+OK".to_vec()));
        }
        count
    });
    let started = Instant::now();
    rewrite_start(&mut control);
    let info = rewrite_wait(&mut control);
    let elapsed = started.elapsed();
    running.store(false, Ordering::Release);
    let count = worker.join().unwrap();
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "the rewrite must complete under writes: {info:?}");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "");
    let steps: u64 = info["firn_aof_rewrite_scan_steps"].parse().unwrap();
    assert!(steps * 8 <= keys as u64, "each step must cross many keys, not one: {steps} steps for {keys} keys");
    assert!(elapsed < Duration::from_secs(30), "a time-sliced scan of {keys} keys took {elapsed:?}");
    let reserve: u64 = info["firn_aof_rewrite_reserve"].parse().unwrap();
    let ledger_peak: u64 = info["firn_aof_rewrite_peak"].parse().unwrap();
    assert!(ledger_peak <= reserve, "the reserve ledger must stay within R: {ledger_peak} > {reserve}");
    let names: Vec<Vec<u8>> = (0..keys).map(|index| format!("slice:{index}").into_bytes()).chain([b"slice:counter".to_vec()]).collect();
    let values = |client: &mut TcpStream| -> Vec<Reply> {
        names.chunks(1000).flat_map(|chunk| {
            let mut args: Vec<&[u8]> = vec![b"MGET"];
            args.extend(chunk.iter().map(|name| name.as_slice()));
            array(request(client, &args))
        }).collect()
    };
    let live = values(&mut control);
    assert_eq!(live.last(), Some(&Reply::Bulk(Some(count.to_string().into_bytes()))));
    assert_eq!(request(&mut control, &[b"DBSIZE"]), Reply::Line(format!(":{}", keys + 1).into_bytes()));
    rewrite_stop(&mut control, child);
    let (child, mut restored, _) = start(&program);
    assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(format!(":{}", keys + 1).into_bytes()));
    assert!(values(&mut restored) == live, "the installed base and increment must restore every live value");
    rewrite_stop(&mut restored, child);
}

/// A first step of up to 64 positions holding 1-MiB values cannot fit the
/// default 16-MiB R, so it must be discarded unwritten, its cursor kept, and
/// retried one position at a time: each key's image reaches the base once.
/// Successful steps cover distinct positions, at most one per key, and the
/// pinned runtime returns the end cursor with the last key, so more steps
/// than keys shows a discarded step. A runtime that ends with a separate
/// empty step would need one step more; the specification allows either.
#[test]
fn reconciled_scan_discards_a_step_that_does_not_fit_and_retries_narrower() {
    let program = CompiledProgram::from_environment();
    let keys = 100_usize;
    let names: Vec<Vec<u8>> = (0..keys).map(|index| format!("large:{index}").into_bytes()).collect();
    let value = |index: usize| -> Vec<u8> {
        let mut bytes = vec![b'v'; 1024 * 1024];
        bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
        bytes
    };
    multipart_files(&program, &[("appendonly.aof.1.base.aof", &[]), ("appendonly.aof.1.incr.aof", &[])], &rewrite_manifest(1, 1));
    let mut bytes = Vec::new();
    for (index, name) in names.iter().enumerate() {
        bytes.extend(wire(&[b"SET", name, &value(index)]));
    }
    std::fs::write(program.working_directory().join("appendonlydir/appendonly.aof.1.incr.aof"), bytes).unwrap();
    // The 100 MiB increment over an empty base exceeds the default minimum,
    // and Redis starts an automatic rewrite from it as from any grown AOF
    // (its startup baseline is the base file's size); when the writer's first
    // check ran before this case's BGREWRITEAOF arrived, the request found
    // that rewrite in progress. Raise the minimum so BGREWRITEAOF is the only
    // rewrite.
    let (child, mut client, _) = start_with_minimum(&program, b"4294967296");
    rewrite_start(&mut client);
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "a step too large for R must be narrowed, not abort: {info:?}");
    assert_eq!(info["firn_aof_rewrite_abort_reason"], "");
    let reserve: u64 = info["firn_aof_rewrite_reserve"].parse().unwrap();
    assert_eq!(reserve, 16 * 1024 * 1024);
    let ledger_peak: u64 = info["firn_aof_rewrite_peak"].parse().unwrap();
    assert!(ledger_peak <= reserve, "the reserve ledger must stay within R: {ledger_peak} > {reserve}");
    let steps: u64 = info["firn_aof_rewrite_scan_steps"].parse().unwrap();
    assert!(steps > keys as u64, "a discarded step must precede the narrowed ones: {steps} steps for {keys} keys");
    let base = std::fs::read(program.working_directory().join("appendonlydir/appendonly.aof.2.base.aof")).unwrap();
    let records = prefix_records(&base);
    for (index, name) in names.iter().enumerate() {
        let images: Vec<_> = records.iter().filter(|words| words.get(1) == Some(name)).collect();
        assert_eq!(images.len(), 1, "{} must reach the base exactly once", String::from_utf8_lossy(name));
        assert!(images[0].len() == 3 && images[0][0] == b"SET" && images[0][2] == value(index), "{} must keep its whole value", String::from_utf8_lossy(name));
    }
    let expected = snapshot(&mut client);
    assert_eq!(expected.len(), keys);
    rewrite_stop(&mut client, child);
    let (child, mut restored, _) = start(&program);
    assert!(snapshot(&mut restored) == expected, "the narrowed rewrite must restore every value");
    rewrite_stop(&mut restored, child);
}

/// Redis 7.0.15's unit/type/set sets rdb-key-save-delay to 2147483647, about
/// 36 minutes per key, and Redis kills its rewrite child on shutdown. firn's
/// scan must leave such a sleep when SHUTDOWN cancels the rewrite or CONFIG
/// SET changes the delay.
#[test]
fn reconciled_scan_key_delay_yields_to_shutdown_and_config() {
    for stopping in [true, false] {
        let program = CompiledProgram::from_environment();
        fixture_keys(&program, 10);
        let (child, mut client, _) = start(&program);
        assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"rdb-key-save-delay", b"2147483647"]), Reply::Line(b"+OK".to_vec()));
        rewrite_start(&mut client);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let info = rewrite_info(&mut client);
            assert_eq!(info["aof_rewrite_in_progress"], "1", "the delayed scan must still run: {info:?}");
            if info["firn_aof_rewrite_scan_steps"] != "0" { break; }
            assert!(Instant::now() < deadline, "the scan never took its first step");
            std::thread::sleep(Duration::from_millis(1));
        }
        if stopping {
            // rewrite_stop allows the orderly stop five seconds.
            rewrite_stop(&mut client, child);
            assert_old_manifest(&program);
            assert_journal_removed(&program);
            let (child, mut restored, _) = start(&program);
            assert_eq!(request(&mut restored, &[b"DBSIZE"]), Reply::Line(b":10".to_vec()));
            rewrite_stop(&mut restored, child);
        } else {
            let released = Instant::now();
            assert_eq!(request(&mut client, &[b"CONFIG", b"SET", b"rdb-key-save-delay", b"0"]), Reply::Line(b"+OK".to_vec()));
            let info = rewrite_wait(&mut client);
            assert_eq!(info["aof_last_bgrewrite_status"], "ok");
            assert!(released.elapsed() < Duration::from_secs(5), "CONFIG SET 0 must end the delay: {:?}", released.elapsed());
            rewrite_stop(&mut client, child);
        }
    }
}
