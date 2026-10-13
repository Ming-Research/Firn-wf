//! Redis 7.2.16 data migration through its command-only multipart AOF and
//! through a client-side, per-type copy. Both read the live reference back;
//! neither derives its oracle from the bytes the copy helper wrote.

use super::*;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DATABASES: &[u8] = &[0]; // Firn implements SELECT 0 only, and no streams.
const TTL_MILLIS: i64 = 300_000;
/// Beyond observed elapsed time, permit 250 ms for request sampling and
/// millisecond clock rounding. A copied TTL must never exceed the source
/// sampled immediately before it; missing or out-of-bounds TTLs fail.
const TTL_TOLERANCE_MILLIS: i64 = 250;
/// Round elapsed time upward and subtract one extra millisecond when setting
/// a relative expiry. PEXPIRE installation latency is not known in advance;
/// compare() checks the resulting sampled PTTL rather than promising an
/// identical absolute deadline.
const COPY_EXPIRY_MARGIN_MILLIS: i64 = 1;

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Simple(Vec<u8>),
    Error(Vec<u8>),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Vec<Reply>),
}

struct Client(BufReader<TcpStream>);

impl Client {
    fn connect(port: u16, child: &ProgramChild) -> Self {
        Self(BufReader::new(connect_when_ready(port, &child.monitor())))
    }

    fn request(&mut self, arguments: &[&[u8]]) -> Reply {
        let mut bytes = format!("*{}\r\n", arguments.len()).into_bytes();
        for argument in arguments {
            bytes.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
            bytes.extend_from_slice(argument);
            bytes.extend_from_slice(b"\r\n");
        }
        self.0.get_mut().write_all(&bytes).expect("write migration request");
        let reply = self.read_reply();
        assert!(!matches!(reply, Reply::Error(_)), "migration command {:?}: {reply:?}", arguments[0]);
        reply
    }

    fn read_reply(&mut self) -> Reply {
        let mut line = Vec::new();
        self.0.read_until(b'\n', &mut line).expect("read migration reply header");
        assert!(line.ends_with(b"\r\n"), "malformed RESP header: {line:?}");
        line.truncate(line.len() - 2);
        let number = |digits: &[u8]| -> i64 {
            std::str::from_utf8(digits).expect("ASCII RESP length").parse().expect("RESP length")
        };
        match line[0] {
            b'+' => Reply::Simple(line[1..].to_vec()),
            b'-' => Reply::Error(line[1..].to_vec()),
            b':' => Reply::Integer(number(&line[1..])),
            b'$' => {
                let size = number(&line[1..]);
                if size == -1 {
                    return Reply::Bulk(None);
                }
                assert!(size >= 0, "invalid bulk length {size}");
                let mut body = vec![0; usize::try_from(size).unwrap()];
                self.0.read_exact(&mut body).expect("read migration bulk body");
                let mut end = [0; 2];
                self.0.read_exact(&mut end).expect("read migration bulk terminator");
                assert_eq!(&end, b"\r\n");
                Reply::Bulk(Some(body))
            }
            b'*' => {
                let size = number(&line[1..]);
                assert!(size >= 0, "unexpected null array");
                Reply::Array((0..size).map(|_| self.read_reply()).collect())
            }
            other => panic!("unexpected RESP kind {other}: {line:?}"),
        }
    }

    fn ok(&mut self, arguments: &[&[u8]]) {
        assert_eq!(self.request(arguments), Reply::Simple(b"OK".to_vec()), "{:?}", arguments[0]);
    }

    fn integer(&mut self, arguments: &[&[u8]]) -> i64 {
        match self.request(arguments) {
            Reply::Integer(value) => value,
            other => panic!("expected integer for {:?}: {other:?}", arguments[0]),
        }
    }

    fn bulk(&mut self, arguments: &[&[u8]]) -> Vec<u8> {
        match self.request(arguments) {
            Reply::Bulk(Some(bytes)) => bytes,
            other => panic!("expected bulk for {:?}: {other:?}", arguments[0]),
        }
    }

    fn array(&mut self, arguments: &[&[u8]]) -> Vec<Vec<u8>> {
        bulk_elements(self.request(arguments))
    }

    fn select(&mut self, database: u8) {
        self.ok(&[b"SELECT", database.to_string().as_bytes()]);
    }
}

fn bulk_elements(reply: Reply) -> Vec<Vec<u8>> {
    match reply {
        Reply::Array(items) => items.into_iter().map(|item| match item {
            Reply::Bulk(Some(bytes)) => bytes,
            other => panic!("expected bulk array element: {other:?}"),
        }).collect(),
        other => panic!("expected array: {other:?}"),
    }
}

fn oracle() -> PathBuf {
    let binary = std::env::var_os("REDIS_SERVER").map(PathBuf::from)
        .expect("REDIS_SERVER must name the reference.sh-built Redis 7.2.16 oracle");
    assert!(binary.is_absolute() && binary.is_file(), "REDIS_SERVER must name an existing absolute file");
    let version = support::run_command(Command::new(&binary).arg("--version"));
    assert!(version.status.success() && String::from_utf8_lossy(&version.stdout).contains(" v=7.2.16 "),
        "migration requires Redis 7.2.16: {version:?}");
    binary
}

fn start_redis(binary: &Path, directory: &Path, aof: bool) -> (ProgramChild, Client) {
    let lease = support::lease_port();
    let mut command = Command::new(binary);
    command.current_dir(directory)
        .args(["--bind", "127.0.0.1", "--port", &lease.port().to_string(),
            "--save", "", "--appendonly", if aof { "yes" } else { "no" },
            "--appendfilename", "migration.aof", "--appenddirname", "appendonlydir",
            "--aof-use-rdb-preamble", "no", "--auto-aof-rewrite-percentage", "0", "--dir"])
        .arg(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = ProgramChild::spawn(&mut command).expect("start Redis migration oracle").with_port(&lease);
    let client = Client::connect(lease.port(), &child);
    (child, client)
}

fn start_firn(program: &CompiledProgram, native: bool, aof: bool) -> (ProgramChild, Client) {
    let lease = support::lease_port();
    let port = lease.port().to_string();
    // The third positional argument enables AOF; omitting it keeps the online
    // destination in memory. Both routes use the same supported commands.
    let arguments: &[&[u8]] = if aof {
        &[port.as_bytes(), b"0", b"migration.aof"]
    } else {
        &[port.as_bytes(), b"0"]
    };
    let child = program.spawn_on_route(native, arguments).with_port(&lease);
    let client = Client::connect(lease.port(), &child);
    (child, client)
}

fn stop(mut client: Client, child: ProgramChild) {
    // SHUTDOWN closes the socket rather than replying; bypass request().
    client.0.get_mut().write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    shutdown_finished(child);
    assert_eq!(client.0.read(&mut [0]).unwrap(), 0);
}

fn populate(client: &mut Client) {
    let large: Vec<u8> = (0..1_048_833).map(|index| (index % 256) as u8).collect();
    for &database in DATABASES {
        client.select(database);
        // Every supported type has one persistent and one expiring key.
        for suffix in [b"persistent".as_slice(), b"expires".as_slice()] {
            let name = |kind: &[u8]| [kind, b":", suffix].concat();
            client.ok(&[b"SET", &name(b"string"), b"\0\xff\r\nnon-UTF8"]);
            client.ok(&[b"SET", &name(b"empty"), b""]);
            client.ok(&[b"SET", &name(b"integer"), b"-9223372036854775808"]);
            client.ok(&[b"SET", &name(b"large"), &large]);
            assert_eq!(client.integer(&[b"RPUSH", &name(b"list"), b"", b"\0\xff", b"duplicate", b"duplicate"]), 4);
            assert_eq!(client.integer(&[b"HSET", &name(b"hash"), b"", b"empty field", b"\0\xff", b"", b"count", b"-42"]), 3);
            assert_eq!(client.integer(&[b"SADD", &name(b"set"), b"", b"\0\xff", b"-42", b"member"]), 4);
            assert_eq!(client.integer(&[b"ZADD", &name(b"zset"), b"-inf", b"low", b"-3.5", b"negative",
                b"0", b"", b"2", b"tie-a", b"2", b"tie-b", b"2", b"\0\xff", b"inf", b"high"]), 7);
            if suffix == b"expires" {
                for kind in [b"string".as_slice(), b"empty", b"integer", b"large", b"list", b"hash", b"set", b"zset"] {
                    assert_eq!(client.integer(&[b"PEXPIRE", &name(kind), TTL_MILLIS.to_string().as_bytes()]), 1);
                }
            }
        }
        client.ok(&[b"SET", b"binary-key:\0\xff\r\n", b"binary key survives"]);
        // COUNT 1 cannot copy these by assuming SCAN returns a single batch.
        for index in 0..32 {
            client.ok(&[b"SET", format!("scan:{index}").as_bytes(), index.to_string().as_bytes()]);
        }
    }
}

/// The key's complete value, with only the unspecified hash/set iteration
/// order normalized. List and sorted-set order and score spelling stay exact.
fn full_value(client: &mut Client, key: &[u8], kind: &[u8]) -> Vec<Vec<u8>> {
    match kind {
        b"string" => vec![client.bulk(&[b"GET", key])],
        b"list" => client.array(&[b"LRANGE", key, b"0", b"-1"]),
        b"hash" => {
            let values = client.array(&[b"HGETALL", key]);
            assert_eq!(values.len() % 2, 0, "HGETALL field/value pairs");
            let mut pairs: Vec<_> = values.chunks_exact(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect();
            pairs.sort();
            pairs.into_iter().flat_map(|(field, value)| [field, value]).collect()
        }
        b"set" => {
            let mut members = client.array(&[b"SMEMBERS", key]);
            members.sort();
            members
        }
        b"zset" => client.array(&[b"ZRANGE", key, b"0", b"-1", b"WITHSCORES"]),
        other => panic!("unsupported source type {:?}; migration must not omit it", other),
    }
}

fn elapsed_millis(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_nanos().div_ceil(1_000_000)).unwrap()
}

fn compare(source: &mut Client, destination: &mut Client) {
    for &database in DATABASES {
        source.select(database);
        destination.select(database);
        // Independent enumeration detects SCAN omissions and extra keys; do
        // not reuse the online helper's enumerated keys as the oracle.
        let source_keys: BTreeSet<_> = source.array(&[b"KEYS", b"*"]).into_iter().collect();
        let destination_keys: BTreeSet<_> = destination.array(&[b"KEYS", b"*"]).into_iter().collect();
        assert_eq!(destination_keys, source_keys, "database {database}: complete key sets");
        for key in source_keys {
            let kind = source.request(&[b"TYPE", &key]);
            assert_eq!(destination.request(&[b"TYPE", &key]), kind, "database {database}, key {key:?}: type");
            let Reply::Simple(kind) = kind else { panic!("TYPE is not a simple reply"); };
            let expected = full_value(source, &key, &kind);
            let actual = full_value(destination, &key, &kind);
            assert!(actual == expected,
                "database {database}, key {key:?}: complete exact bytes differ ({} source parts, {} destination parts)", expected.len(), actual.len());
            let sampled = Instant::now();
            let before = source.integer(&[b"PTTL", &key]);
            let actual = destination.integer(&[b"PTTL", &key]);
            let after = source.integer(&[b"PTTL", &key]);
            if before == -1 {
                assert_eq!(actual, -1, "database {database}, key {key:?}: persistent key gained TTL");
                assert_eq!(after, -1);
            } else {
                assert!(before > 0 && actual > 0 && after > 0, "database {database}, key {key:?}: lost/expired TTL ({before}, {actual}, {after})");
                let sampling = elapsed_millis(sampled);
                assert!(actual <= before, "database {database}, key {key:?}: firn renewed TTL ({actual} > source {before})");
                assert!(before - actual <= sampling + TTL_TOLERANCE_MILLIS,
                    "database {database}, key {key:?}: firn shortened TTL by {} ms, exceeding sampling {sampling} ms + tolerance {TTL_TOLERANCE_MILLIS} ms", before - actual);
                assert!(after <= before && before - after <= sampling + TTL_TOLERANCE_MILLIS,
                    "source TTL sampling is not stable: {before} -> {after}");
            }
        }
    }
}

/// Firn has no MIGRATE, DUMP or RESTORE. Copy only with its existing per-type
/// reads/writes, preserving raw bytes and applying PTTL after value transfer.
/// This is for a quiescent live source and an empty destination, not a
/// concurrent-write snapshot or production migration command.
fn online_copy(source: &mut Client, destination: &mut Client) {
    for &database in DATABASES {
        source.select(database);
        destination.select(database);
        assert_eq!(destination.integer(&[b"DBSIZE"]), 0, "online copy requires an empty destination");
        let mut cursor = b"0".to_vec();
        let mut seen = BTreeSet::new();
        let started = Instant::now();
        loop {
            assert!(started.elapsed() < Duration::from_secs(20), "SCAN failed to finish");
            let Reply::Array(mut batch) = source.request(&[b"SCAN", &cursor, b"COUNT", b"1"]) else { panic!("SCAN array"); };
            assert_eq!(batch.len(), 2, "SCAN cursor and keys");
            let keys = bulk_elements(batch.pop().unwrap());
            let Reply::Bulk(Some(next)) = batch.pop().unwrap() else { panic!("SCAN bulk cursor"); };
            cursor = next;
            for key in keys {
                // SCAN may repeat a key even without writes; copying it twice
                // would duplicate lists. The key set comparison is separate.
                if !seen.insert(key.clone()) { continue; }
                let sampled = Instant::now();
                let ttl = source.integer(&[b"PTTL", &key]);
                assert!(ttl == -1 || ttl > 0, "source key disappeared during online copy: {key:?}");
                let Reply::Simple(kind) = source.request(&[b"TYPE", &key]) else { panic!("TYPE simple reply"); };
                let value = full_value(source, &key, &kind);
                match kind.as_slice() {
                    b"string" => destination.ok(&[b"SET", &key, &value[0]]),
                    b"list" | b"hash" | b"set" => {
                        let command: &[u8] = match kind.as_slice() { b"list" => b"RPUSH", b"hash" => b"HSET", _ => b"SADD" };
                        let mut arguments = vec![command, key.as_slice()];
                        arguments.extend(value.iter().map(Vec::as_slice));
                        destination.integer(&arguments);
                    }
                    b"zset" => {
                        assert_eq!(value.len() % 2, 0);
                        let mut arguments: Vec<&[u8]> = vec![b"ZADD", &key];
                        for pair in value.chunks_exact(2) { arguments.extend([pair[1].as_slice(), pair[0].as_slice()]); }
                        destination.integer(&arguments);
                    }
                    _ => panic!("unsupported type {kind:?}"),
                }
                if ttl > 0 {
                    let remaining = ttl - elapsed_millis(sampled) - COPY_EXPIRY_MARGIN_MILLIS;
                    assert!(remaining > 0, "key expires during transfer: {key:?}");
                    assert_eq!(destination.integer(&[b"PEXPIRE", &key, remaining.to_string().as_bytes()]), 1);
                }
            }
            if cursor == b"0" { break; }
        }
    }
}

fn rewrite(source: &mut Client) {
    let before = source.bulk(&[b"INFO", b"persistence"]);
    let before = std::str::from_utf8(&before).unwrap();
    let rewrites: u64 = info_field(before, "aof_rewrites").unwrap().parse().unwrap();
    assert_eq!(source.request(&[b"BGREWRITEAOF"]), Reply::Simple(b"Background append only file rewriting started".to_vec()));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let info = source.bulk(&[b"INFO", b"persistence"]);
        let info = std::str::from_utf8(&info).unwrap();
        let completed: u64 = info_field(info, "aof_rewrites").unwrap().parse().unwrap();
        if completed > rewrites && info_field(info, "aof_rewrite_in_progress").as_deref() == Some("0") {
            assert_eq!(info_field(info, "aof_last_bgrewrite_status").as_deref(), Some("ok"), "{info}");
            return;
        }
        assert!(Instant::now() < deadline, "Redis rewrite did not finish: {info}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Make every incremental type contribute state the rewritten base lacks.
fn incremental_writes(source: &mut Client) {
    source.select(0);
    source.ok(&[b"SET", b"incremental-only", b"must load increment"]);
    source.ok(&[b"SET", b"string:persistent", b"incremental string:\0\xff"]);
    source.integer(&[b"RPUSH", b"list:persistent", b"incremental list"]);
    source.integer(&[b"HSET", b"hash:persistent", b"incremental field", b"incremental value"]);
    source.integer(&[b"SADD", b"set:persistent", b"incremental member"]);
    source.integer(&[b"ZADD", b"zset:persistent", b"-4", b"incremental member"]);
}

fn copy_appenddir(source: &Path, destination: &Path) {
    let manifest = std::fs::read_to_string(source.join("migration.aof.manifest")).expect("Redis AOF manifest");
    let mut bases = 0;
    let mut increments = 0;
    for line in manifest.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        assert_eq!(words.len(), 6, "Redis-generated manifest record: {line}");
        assert_eq!(words[0], "file");
        assert_eq!(words[4], "type");
        if words[5] == "b" {
            bases += 1;
            assert!(words[1].ends_with(".base.aof"), "expected command-only AOF base: {line}");
            let base = std::fs::read(source.join(words[1])).unwrap();
            assert_eq!(base.first(), Some(&b'*'), "base must contain RESP, not an RDB preamble");
        } else if words[5] == "i" {
            increments += 1;
            assert!(std::fs::metadata(source.join(words[1])).unwrap().len() > 0,
                "incremental writes must reach a manifest-listed file");
        }
    }
    assert_eq!(bases, 1);
    assert!(increments > 0);
    std::fs::create_dir(destination).unwrap();
    // Copy the actual directory, without reconstructing or filtering the
    // manifest; Firn must consume Redis's persisted layout unchanged.
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file(), "unexpected appenddir entry");
        let copied = destination.join(entry.file_name());
        std::fs::copy(entry.path(), &copied).unwrap();
        assert_eq!(std::fs::read(copied).unwrap(), std::fs::read(entry.path()).unwrap());
    }
}

#[test]
fn redis_7216_command_only_multipart_aof_migrates_every_supported_type() {
    let binary = oracle();
    for native in [false, true] {
        let directory = fixture_directory();
        let program = CompiledProgram::from_environment();
        let (source_child, mut source) = start_redis(&binary, directory.path(), true);
        populate(&mut source);
        rewrite(&mut source);
        incremental_writes(&mut source);
        // Redis's shutdown flushes its increment and stops changing files.
        // Reopen the same reference dataset as the independent live oracle.
        stop(source, source_child);
        copy_appenddir(&directory.path().join("appendonlydir"), &program.working_directory().join("appendonlydir"));
        let (source_child, mut source) = start_redis(&binary, directory.path(), true);
        let (destination_child, mut destination) = start_firn(&program, native, true);
        compare(&mut source, &mut destination);
        stop(destination, destination_child);
        stop(source, source_child);
    }
}

#[test]
fn redis_7216_online_per_type_copy_migrates_every_supported_type() {
    let binary = oracle();
    for native in [false, true] {
        let directory = fixture_directory();
        let program = CompiledProgram::from_environment();
        let (source_child, mut source) = start_redis(&binary, directory.path(), false);
        let (destination_child, mut destination) = start_firn(&program, native, false);
        populate(&mut source);
        online_copy(&mut source, &mut destination);
        compare(&mut source, &mut destination);
        stop(destination, destination_child);
        stop(source, source_child);
    }
}
