//! Database compatibility derives from Redis 7.2.16's src/db.c,
//! src/object.c, src/aof.c, src/script.c and tests/unit/{keyspace,scripting,
//! multi}.tcl. These cases belong to board item firn-scope-select-db.
//! Recorded Redis 7.2.16 replies and the rewrite base: https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856.
//! Each case below identifies its recorded batch and missing fixture context.
//! SCAN cursors and dictionary order are deliberately checked on their own
//! server; deterministic replies and controlled incremental AOFs are exact.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

const DEADLINE: &str = "99999999999000";
// Raw command-format base from the recorded aof-mixed-databases-rewrite batch.
const REDIS_7216_REWRITE_BASE: &[u8] = b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$3\r\nSET\r\n$4\r\nsame\r\n$7\r\nfifteen\r\n*2\r\n$6\r\nSELECT\r\n$1\r\n1\r\n*3\r\n$3\r\nSET\r\n$4\r\ncopy\r\n$5\r\nseven\r\n*3\r\n$3\r\nSET\r\n$4\r\nsame\r\n$6\r\nscript\r\n*2\r\n$6\r\nSELECT\r\n$1\r\n7\r\n*3\r\n$3\r\nSET\r\n$4\r\nsame\r\n$5\r\nseven\r\n*3\r\n$9\r\nPEXPIREAT\r\n$4\r\nsame\r\n$14\r\n99999999999000\r\n*3\r\n$3\r\nSET\r\n$11\r\nscript-only\r\n$1\r\ns\r\n*2\r\n$6\r\nSELECT\r\n$2\r\n15\r\n*3\r\n$3\r\nSET\r\n$4\r\nsame\r\n$4\r\nzero\r\n";

const DB_RANGE: &str = "-ERR DB index is out of range\r\n";
const INTEGER: &str = "-ERR value is not an integer or out of range\r\n";
const INT_RANGE: &str = "-ERR value is out of range, value must between -2147483648 and 2147483647\r\n";
const SAME: &str = "-ERR source and destination objects are the same\r\n";

#[derive(Debug)]
enum Reply {
    Line(String),
    Bulk(Option<String>),
    Verbatim(String),
    Array(Vec<Reply>),
    Map(Vec<Reply>),
    Null,
}

impl Reply {
    fn wire(&self) -> String {
        match self {
            Self::Line(line) => line.clone(),
            Self::Bulk(None) => "$-1\r\n".to_owned(),
            Self::Bulk(Some(value)) => format!("${}\r\n{value}\r\n", value.len()),
            Self::Verbatim(value) => format!("={}\r\n{value}\r\n", value.len()),
            Self::Array(values) => format!("*{}\r\n{}", values.len(), values.iter().map(Self::wire).collect::<String>()),
            Self::Map(values) => format!("%{}\r\n{}", values.len() / 2, values.iter().map(Self::wire).collect::<String>()),
            Self::Null => "_\r\n".to_owned(),
        }
    }

    fn bulk(self) -> String {
        match self { Self::Bulk(Some(value)) => value, other => panic!("expected bulk: {other:?}") }
    }

    fn text(self) -> String {
        match self {
            Self::Bulk(Some(value)) => value,
            Self::Verbatim(value) => value.strip_prefix("txt:").expect("verbatim text format").to_owned(),
            other => panic!("expected Redis text: {other:?}"),
        }
    }

    fn array(self) -> Vec<Reply> {
        match self { Self::Array(values) => values, other => panic!("expected array: {other:?}") }
    }
}

fn read(client: &mut TcpStream) -> Reply {
    let line = reply_line(client, "database reply");
    let number = || line[1..line.len() - 2].parse::<usize>().unwrap();
    match line.as_bytes()[0] {
        b'$' if line == "$-1\r\n" => Reply::Bulk(None),
        b'$' | b'=' => {
            let mut bytes = vec![0; number() + 2];
            client.read_exact(&mut bytes).unwrap();
            assert!(bytes.ends_with(b"\r\n"));
            bytes.truncate(bytes.len() - 2);
            let value = String::from_utf8(bytes).unwrap();
            if line.starts_with('=') { Reply::Verbatim(value) } else { Reply::Bulk(Some(value)) }
        }
        b'*' => Reply::Array((0..number()).map(|_| read(client)).collect()),
        b'%' => Reply::Map((0..number() * 2).map(|_| read(client)).collect()),
        b'_' => { assert_eq!(line, "_\r\n"); Reply::Null }
        b'+' | b'-' | b':' => Reply::Line(line),
        _ => panic!("unexpected database reply: {line:?}"),
    }
}

fn request(client: &mut TcpStream, args: &[&str]) -> Reply {
    client.write_all(&resp(args)).unwrap();
    read(client)
}

fn check(client: &mut TcpStream, args: &[&str], expected: &str) {
    assert_eq!(request(client, args).wire(), expected, "{}", args.join(" "));
}

fn select(client: &mut TcpStream, db: usize) {
    check(client, &["SELECT", &db.to_string()], "+OK\r\n");
}

fn hello(client: &mut TcpStream, protocol: u8) {
    let reply = request(client, &["HELLO", &protocol.to_string()]);
    assert!(matches!((protocol, reply), (2, Reply::Array(_)) | (3, Reply::Map(_))), "HELLO must use the selected protocol");
}

fn null(protocol: u8) -> &'static str {
    if protocol == 3 { "_\r\n" } else { "$-1\r\n" }
}

fn start(program: &CompiledProgram, clients: &[u8], aof: Option<&[u8]>) -> (ProgramChild, TcpStream, u16) {
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let mut args: Vec<&[u8]> = vec![text.as_bytes(), clients];
    if let Some(name) = aof { args.push(name); }
    let child = program.spawn_on_route(true, &args).with_port(&lease);
    let client = connect_when_ready(port, &child);
    (child, client, port)
}

fn stop(mut client: TcpStream, child: ProgramChild) {
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    shutdown_finished(child);
}

/// A cached handle must follow every pipelined SELECT, including database 15;
/// a new peer and RESET start in database zero independently of other peers.
/// Oracle recording: sixteen-database-isolation-resp2/resp3 records the reply values, CLIENT INFO
/// framing and RESET. The pipelined history, later reread and fresh second connection are not in
/// the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn all_sixteen_databases_are_isolated_and_selection_is_connection_local() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, port) = start(&program, b"0", None);
        hello(&mut client, protocol);
        let mut pipeline = Vec::new();
        for db in 0..16 {
            pipeline.extend(resp(&["SELECT", &db.to_string()]));
            pipeline.extend(resp(&["SET", "same", &format!("db{db}")]));
            pipeline.extend(resp(&["GET", "same"]));
        }
        client.write_all(&pipeline).unwrap();
        for db in 0..16 {
            assert_eq!(read(&mut client).wire(), "+OK\r\n");
            assert_eq!(read(&mut client).wire(), "+OK\r\n");
            let value = format!("db{db}");
            assert_eq!(read(&mut client).wire(), format!("${}\r\n{value}\r\n", value.len()));
        }
        let mut fresh = connect_when_ready(port, &child);
        check(&mut fresh, &["GET", "same"], "$3\r\ndb0\r\n");
        for db in 0..16 {
            select(&mut client, db);
            check(&mut client, &["DBSIZE"], ":1\r\n");
            let value = format!("db{db}");
            check(&mut client, &["GET", "same"], &format!("${}\r\n{value}\r\n", value.len()));
            // networking.c clientInfoCommand reports the selected DB under
            // RESP2 bulk framing and RESP3 txt verbatim framing.
            let info = match request(&mut client, &["CLIENT", "INFO"]) {
                Reply::Bulk(Some(info)) if protocol == 2 => info,
                Reply::Verbatim(info) if protocol == 3 => info.strip_prefix("txt:").expect("CLIENT INFO text format").to_owned(),
                other => panic!("CLIENT INFO framing: {other:?}"),
            };
            let field = format!("db={db}");
            assert!(info.split_whitespace().any(|value| value == field), "selected db missing: {info}");
        }
        check(&mut client, &["RESET"], "+RESET\r\n");
        check(&mut client, &["GET", "same"], "$3\r\ndb0\r\n");
        drop(fresh);
        stop(client, child);
    }
}

/// db.c parses database ids as signed int before range checking or looking up
/// a key. SWAPDB supplies its own parse errors, in first/second-id order.
/// Oracle recording: database-errors-resp2/resp3 records every error below. The final
/// SET/SELECT/GET selection-retention check is not in the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn database_command_errors_match_redis_integer_rules_and_precedence() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", None);
        hello(&mut client, protocol);
        select(&mut client, 7);
        for (args, name) in [
            (vec!["SELECT"], "select"), (vec!["SELECT", "0", "1"], "select"),
            (vec!["MOVE", "missing"], "move"), (vec!["SWAPDB", "0"], "swapdb"),
            (vec!["COPY", "missing"], "copy"), (vec!["DBSIZE", "extra"], "dbsize"),
            (vec!["KEYS"], "keys"), (vec!["SCAN"], "scan"),
            (vec!["RANDOMKEY", "extra"], "randomkey"),
        ] {
            check(&mut client, &args, &format!("-ERR wrong number of arguments for '{name}' command\r\n"));
        }
        for (id, expected) in [
            ("-1", DB_RANGE), ("16", DB_RANGE), ("2147483647", DB_RANGE), ("-2147483648", DB_RANGE),
            ("2147483648", INT_RANGE), ("-2147483649", INT_RANGE),
            ("9223372036854775808", INTEGER), ("x", INTEGER), ("1.0", INTEGER),
            ("01", INTEGER), ("+1", INTEGER), (" 1", INTEGER), ("1 ", INTEGER), ("", INTEGER),
        ] {
            for args in [vec!["SELECT", id], vec!["MOVE", "missing", id], vec!["COPY", "missing", "missing", "DB", id]] {
                check(&mut client, &args, expected);
            }
        }
        check(&mut client, &["MOVE", "missing", "7"], SAME);
        check(&mut client, &["COPY", "missing", "missing"], SAME);
        check(&mut client, &["COPY", "missing", "missing", "DB", "1", "BAD"], "-ERR syntax error\r\n");
        check(&mut client, &["COPY", "missing", "other", "DB"], "-ERR syntax error\r\n");
        check(&mut client, &["COPY", "missing", "other", "DB", "16", "BAD"], DB_RANGE);
        for id in ["x", "1.0", "2147483648", "9223372036854775808"] {
            check(&mut client, &["SWAPDB", id, "x"], "-ERR invalid first DB index\r\n");
            check(&mut client, &["SWAPDB", "0", id], "-ERR invalid second DB index\r\n");
        }
        check(&mut client, &["SWAPDB", "16", "x"], "-ERR invalid second DB index\r\n");
        for args in [vec!["SWAPDB", "16", "0"], vec!["SWAPDB", "0", "-1"]] { check(&mut client, &args, DB_RANGE); }
        for command in ["FLUSHDB", "FLUSHALL"] {
            check(&mut client, &[command, "BAD"], "-ERR syntax error\r\n");
            check(&mut client, &[command, "SYNC", "ASYNC"], "-ERR syntax error\r\n");
        }
        // No failed database operation can change the caller's selection.
        check(&mut client, &["SET", "selected", "seven"], "+OK\r\n");
        select(&mut client, 0);
        check(&mut client, &["GET", "selected"], null(protocol));
        select(&mut client, 7);
        check(&mut client, &["GET", "selected"], "$5\r\nseven\r\n");
        stop(client, child);
    }
}

/// MOVE retains the absolute deadline and removes only its source. COPY
/// allows the same name in another database, preserves TTL, deep-copies a
/// collection, and REPLACE removes an old destination's expiry when needed.
/// Oracle recording: move-copy-values-and-ttl-resp2/resp3 records this command history. Expired-
/// destination cleanup may be lazy or active; no cleanup scheduling is claimed.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn move_and_copy_cross_database_values_and_absolute_expiries() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", None);
        hello(&mut client, protocol);
        select(&mut client, 7);
        check(&mut client, &["RPUSH", "same", "a", "b"], ":2\r\n");
        check(&mut client, &["PEXPIREAT", "same", DEADLINE], ":1\r\n");
        check(&mut client, &["COPY", "same", "same", "DB", "15"], ":1\r\n");
        check(&mut client, &["COPY", "same", "same", "DB", "15"], ":0\r\n");
        select(&mut client, 15);
        check(&mut client, &["PEXPIRETIME", "same"], &format!(":{DEADLINE}\r\n"));
        check(&mut client, &["RPUSH", "same", "c"], ":3\r\n");
        select(&mut client, 7);
        check(&mut client, &["LRANGE", "same", "0", "-1"], "*2\r\n$1\r\na\r\n$1\r\nb\r\n");
        check(&mut client, &["MOVE", "same", "15"], ":0\r\n");
        check(&mut client, &["MOVE", "same", "1"], ":1\r\n");
        check(&mut client, &["EXISTS", "same"], ":0\r\n");
        check(&mut client, &["MOVE", "missing", "1"], ":0\r\n");
        select(&mut client, 1);
        check(&mut client, &["PEXPIRETIME", "same"], &format!(":{DEADLINE}\r\n"));
        check(&mut client, &["LRANGE", "same", "0", "-1"], "*2\r\n$1\r\na\r\n$1\r\nb\r\n");
        check(&mut client, &["SET", "permanent", "p"], "+OK\r\n");
        check(&mut client, &["COPY", "permanent", "same", "db", "15", "replace"], ":1\r\n");
        check(&mut client, &["COPY", "missing", "same", "DB", "15", "REPLACE"], ":0\r\n");
        check(&mut client, &["MOVE", "permanent", "0"], ":1\r\n");
        select(&mut client, 15);
        check(&mut client, &["GET", "same"], "$1\r\np\r\n");
        check(&mut client, &["PEXPIRETIME", "same"], ":-1\r\n");
        select(&mut client, 0);
        check(&mut client, &["PEXPIRETIME", "permanent"], ":-1\r\n");
        // A destination whose deadline has passed is available to both commands.
        check(&mut client, &["SET", "gone", "old", "PXAT", "1"], "+OK\r\n");
        select(&mut client, 1);
        check(&mut client, &["SET", "gone", "live"], "+OK\r\n");
        check(&mut client, &["MOVE", "gone", "0"], ":1\r\n");
        select(&mut client, 0);
        check(&mut client, &["GET", "gone"], "$4\r\nlive\r\n");
        select(&mut client, 15);
        check(&mut client, &["SET", "copygone", "old", "PXAT", "1"], "+OK\r\n");
        select(&mut client, 0);
        check(&mut client, &["COPY", "gone", "copygone", "DB", "15"], ":1\r\n");
        check(&mut client, &["SET", "expired-source", "old", "PXAT", "1"], "+OK\r\n");
        check(&mut client, &["MOVE", "expired-source", "15"], ":0\r\n");
        check(&mut client, &["COPY", "expired-source", "copy", "DB", "15"], ":0\r\n");
        stop(client, child);
    }
}

/// A swap changes the contents visible through handles held by existing
/// peers. The exchanged deadline must not later delete the old db's value.
/// Oracle recording: Related SWAPDB replies appear in the mixed-database AOF batches; this two-
/// connection swap fixture is absent from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn swapdb_exchanges_contents_seen_by_existing_connections() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut zero, port) = start(&program, b"0", None);
        let mut fifteen = connect_when_ready(port, &child);
        hello(&mut zero, protocol);
        hello(&mut fifteen, protocol);
        select(&mut fifteen, 15);
        check(&mut zero, &["SET", "same", "zero"], "+OK\r\n");
        check(&mut fifteen, &["SET", "same", "fifteen"], "+OK\r\n");
        check(&mut fifteen, &["PEXPIREAT", "same", DEADLINE], ":1\r\n");
        check(&mut zero, &["SWAPDB", "0", "15"], "+OK\r\n");
        check(&mut zero, &["GET", "same"], "$7\r\nfifteen\r\n");
        check(&mut zero, &["PEXPIRETIME", "same"], &format!(":{DEADLINE}\r\n"));
        check(&mut fifteen, &["GET", "same"], "$4\r\nzero\r\n");
        check(&mut fifteen, &["PEXPIRETIME", "same"], ":-1\r\n");
        check(&mut fifteen, &["SET", "after", "new"], "+OK\r\n");
        check(&mut zero, &["GET", "after"], null(protocol));
        check(&mut zero, &["SWAPDB", "15", "15"], "+OK\r\n");
        check(&mut fifteen, &["GET", "after"], "$3\r\nnew\r\n");
        check(&mut zero, &["SWAPDB", "0", "15"], "+OK\r\n");
        check(&mut zero, &["GET", "after"], "$3\r\nnew\r\n");
        check(&mut fifteen, &["GET", "same"], "$7\r\nfifteen\r\n");
        drop(fifteen);
        stop(zero, child);
    }
}

fn keys(client: &mut TcpStream, pattern: &str) -> BTreeSet<String> {
    let values: Vec<_> = request(client, &["KEYS", pattern]).array().into_iter().map(Reply::bulk).collect();
    let unique: BTreeSet<_> = values.iter().cloned().collect();
    assert_eq!(unique.len(), values.len(), "KEYS never repeats a key");
    unique
}

fn scan(client: &mut TcpStream, pattern: &str, kind: Option<&str>) -> BTreeSet<String> {
    let mut cursor = "0".to_owned();
    let mut found = BTreeSet::new();
    for _ in 0..1000 {
        let mut args = vec!["SCAN", &cursor, "MATCH", pattern, "COUNT", "1"];
        if let Some(kind) = kind { args.extend(["TYPE", kind]); }
        let mut items = request(client, &args).array().into_iter();
        cursor = items.next().unwrap().bulk();
        let _: u64 = cursor.parse().expect("SCAN cursor is numeric");
        found.extend(items.next().unwrap().array().into_iter().map(Reply::bulk));
        assert!(items.next().is_none(), "SCAN returns exactly cursor and keys");
        if cursor == "0" { return found; }
    }
    panic!("SCAN never reached cursor zero");
}

/// Enumeration and INFO use the selected database; FLUSHDB affects exactly
/// that database and FLUSHALL empties every database without changing SELECT.
/// Oracle recording: Related commands appear in other batches; this enumeration, INFO and
/// successful FLUSH fixture is absent from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn database_enumeration_info_and_flush_have_database_scope() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", None);
        hello(&mut client, protocol);
        for db in [0, 7, 15] {
            select(&mut client, db);
            let key = format!("only:{db}");
            check(&mut client, &["SET", &key, "v"], "+OK\r\n");
            check(&mut client, &["RANDOMKEY"], &format!("${}\r\n{key}\r\n", key.len()));
            check(&mut client, &["RPUSH", "list", "x"], ":1\r\n");
            check(&mut client, &["PEXPIREAT", "list", DEADLINE], ":1\r\n");
            check(&mut client, &["DBSIZE"], ":2\r\n");
            assert_eq!(keys(&mut client, "*"), BTreeSet::from([key.clone(), "list".to_owned()]));
            assert_eq!(scan(&mut client, "only:*", None), BTreeSet::from([key]));
            assert_eq!(scan(&mut client, "*", Some("list")), BTreeSet::from(["list".to_owned()]));
        }
        let info = request(&mut client, &["INFO", "keyspace"]).text();
        for db in 0..16 {
            let field = format!("db{db}");
            if [0, 7, 15].contains(&db) {
                let value = info_field(&info, &field).unwrap_or_else(|| panic!("missing {field}: {info}"));
                assert_eq!(value.split(',').next(), Some("keys=2"), "{field}: {value}");
            } else { assert!(info_field(&info, &field).is_none(), "empty database {db} has no INFO line"); }
        }
        for option in [None, Some("SYNC"), Some("ASYNC")] {
            select(&mut client, 7);
            check(&mut client, &["SET", "cleared", "v"], "+OK\r\n");
            let args = if let Some(option) = option { vec!["FLUSHDB", option] } else { vec!["FLUSHDB"] };
            check(&mut client, &args, "+OK\r\n");
            check(&mut client, &["DBSIZE"], ":0\r\n");
            check(&mut client, &["RANDOMKEY"], null(protocol));
            assert!(keys(&mut client, "*").is_empty());
            assert!(scan(&mut client, "*", None).is_empty());
            select(&mut client, 0);
            check(&mut client, &["DBSIZE"], ":2\r\n");
            select(&mut client, 15);
            check(&mut client, &["DBSIZE"], ":2\r\n");
        }
        for option in [None, Some("SYNC"), Some("ASYNC")] {
            select(&mut client, 0);
            check(&mut client, &["SET", "cleared", "v"], "+OK\r\n");
            select(&mut client, 15);
            let args = if let Some(option) = option { vec!["FLUSHALL", option] } else { vec!["FLUSHALL"] };
            check(&mut client, &args, "+OK\r\n");
            check(&mut client, &["SET", "retained-selection", "v"], "+OK\r\n");
            for db in 0..16 {
                select(&mut client, db);
                check(&mut client, &["DBSIZE"], if db == 15 { ":1\r\n" } else { ":0\r\n" });
            }
        }
        stop(client, child);
    }
}

/// db.c expiry belongs to the database containing the value after MOVE or
/// SWAPDB. DBSIZE does no lookup, so this also observes active expiry in an
/// otherwise idle database rather than passing by lazy GET cleanup.
/// Oracle recording: This TIME-derived deadline and active-expiry observation fixture is absent
/// from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn active_expiry_follows_database_moves_and_swaps() {
    let program = CompiledProgram::from_environment();
    let (child, mut client, _) = start(&program, b"0", None);
    client.write_all(&resp(&["TIME"])).unwrap();
    let deadline = (time_reply(&mut client, "expiry clock") + 1000).to_string();
    check(&mut client, &["SET", "due", "expires", "PXAT", &deadline], "+OK\r\n");
    check(&mut client, &["MOVE", "due", "7"], ":1\r\n");
    check(&mut client, &["SWAPDB", "7", "15"], "+OK\r\n");
    check(&mut client, &["SET", "due", "permanent"], "+OK\r\n");
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        select(&mut client, 15);
        let size = request(&mut client, &["DBSIZE"]).wire();
        select(&mut client, 0);
        if size == ":0\r\n" { break; }
        assert_eq!(size, ":1\r\n");
        assert!(Instant::now() < until, "active expiry never visited database 15");
        std::thread::sleep(Duration::from_millis(20));
    }
    check(&mut client, &["GET", "due"], "$9\r\npermanent\r\n");
    check(&mut client, &["PEXPIRETIME", "due"], ":-1\r\n");
    select(&mut client, 7);
    check(&mut client, &["DBSIZE"], ":0\r\n");
    stop(client, child);
}

/// EXEC must run SELECT in queue order and leave that selection on the
/// network client. A runtime SELECT error does not abort later queued work;
/// DISCARD and a queue-time arity error must never perform queued selections.
/// Oracle recording: multi-database-context-resp2/resp3 records this command history and both
/// EXEC replies.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn transactions_execute_database_changes_in_order_and_keep_final_selection() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", None);
        hello(&mut client, protocol);
        check(&mut client, &["SET", "same", "zero"], "+OK\r\n");
        check(&mut client, &["MULTI"], "+OK\r\n");
        for args in [vec!["SELECT", "7"], vec!["SET", "same", "seven"], vec!["COPY", "same", "same", "DB", "15"], vec!["SELECT", "15"], vec!["GET", "same"], vec!["SELECT", "16"], vec!["SET", "after-error", "v"]] {
            check(&mut client, &args, "+QUEUED\r\n");
        }
        check(&mut client, &["EXEC"], &format!("*7\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n$5\r\nseven\r\n{DB_RANGE}+OK\r\n"));
        check(&mut client, &["GET", "after-error"], "$1\r\nv\r\n");
        check(&mut client, &["MULTI"], "+OK\r\n");
        check(&mut client, &["SELECT", "0"], "+QUEUED\r\n");
        check(&mut client, &["DISCARD"], "+OK\r\n");
        check(&mut client, &["GET", "same"], "$5\r\nseven\r\n");
        check(&mut client, &["MULTI"], "+OK\r\n");
        check(&mut client, &["SELECT", "0"], "+QUEUED\r\n");
        check(&mut client, &["SELECT"], "-ERR wrong number of arguments for 'select' command\r\n");
        check(&mut client, &["EXEC"], "-EXECABORT Transaction discarded because of previous errors.\r\n");
        check(&mut client, &["GET", "same"], "$5\r\nseven\r\n");
        select(&mut client, 0);
        check(&mut client, &["GET", "same"], "$4\r\nzero\r\n");
        stop(client, child);
    }
}

/// script.c initializes the script client from its caller. Script SELECT is
/// local, including on a final error; a later
/// attempt begins afresh in its own caller's database.
/// Oracle recording: script-database-context-resp2/resp3 records this command history, including
/// script errors, local selection and the declared-key APPEND result.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn scripts_start_in_the_callers_database_and_select_only_the_script_client() {
    for protocol in [2, 3] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", None);
        hello(&mut client, protocol);
        select(&mut client, 7);
        check(&mut client, &["SET", "same", "seven"], "+OK\r\n");
        check(&mut client, &["EVAL", "return redis.call('GET',KEYS[1])", "1", "same"], "$5\r\nseven\r\n");
        let script = "redis.call('SELECT',15); redis.call('SET','same','fifteen'); redis.call('COPY','same','copy','DB',0); return redis.call('GET','same')";
        check(&mut client, &["EVAL", script, "0"], "$7\r\nfifteen\r\n");
        check(&mut client, &["GET", "same"], "$5\r\nseven\r\n");
        check(&mut client, &["EVAL", "return redis.call('GET','same')", "0"], "$5\r\nseven\r\n");
        // The declared name alone does not grant access to that name in a
        // different database; a narrow attempt must expand and run once.
        let selected_key = "local before=redis.call('GET',KEYS[1]); redis.call('SELECT',15); redis.call('APPEND',KEYS[1],'!'); return {before,redis.call('GET',KEYS[1])}";
        check(&mut client, &["EVAL", selected_key, "1", "same"], "*2\r\n$5\r\nseven\r\n$8\r\nfifteen!\r\n");
        select(&mut client, 15);
        check(&mut client, &["SET", "same", "fifteen"], "+OK\r\n");
        select(&mut client, 7);
        check(&mut client, &["EVAL", "return redis.pcall('SELECT',16)", "0"], DB_RANGE);
        check(&mut client, &["EVAL", "redis.call('SELECT',15); redis.call('SET','before-error','v'); return redis.error_reply('deliberate')", "0"], "-ERR deliberate\r\n");
        check(&mut client, &["GET", "before-error"], null(protocol));
        check(&mut client, &["MULTI"], "+OK\r\n");
        for args in [vec!["SELECT", "0"], vec!["GET", "copy"]] {
            check(&mut client, &args, "+QUEUED\r\n");
        }
        check(&mut client, &["EXEC"], "*2\r\n+OK\r\n$7\r\nfifteen\r\n");
        check(&mut client, &["GET", "same"], null(protocol));
        select(&mut client, 15);
        check(&mut client, &["GET", "before-error"], "$1\r\nv\r\n");
        stop(client, child);
    }
}

/// An impossible limit exhausts every eligible key regardless of the
/// requesting client's database; a volatile policy preserves permanent keys.
/// Each database's eviction DEL must persist in that database's AOF context.
/// Oracle recording: This allkeys-random/volatile-ttl admission, eviction statistics and restart
/// fixture is absent from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn eviction_searches_all_databases_with_global_policy_and_statistics() {
    for policy in ["allkeys-random", "volatile-ttl"] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", Some(b"eviction.aof"));
        let value = "v".repeat(64 * 1024);
        for db in [0, 7, 15] {
            select(&mut client, db);
            check(&mut client, &["SET", "victim", &value, "PXAT", DEADLINE], "+OK\r\n");
        }
        select(&mut client, 4);
        check(&mut client, &["SET", "permanent", &value], "+OK\r\n");
        select(&mut client, 1);
        check(&mut client, &["CONFIG", "SET", "maxmemory-policy", policy, "maxmemory-eviction-tenacity", "100", "maxmemory", "1"], "+OK\r\n");
        check(&mut client, &["SET", "refused", "v"], "-OOM command not allowed when used memory > 'maxmemory'.\r\n");
        for db in [0, 7, 15] {
            select(&mut client, db);
            check(&mut client, &["EXISTS", "victim"], ":0\r\n");
        }
        select(&mut client, 4);
        check(&mut client, &["EXISTS", "permanent"], if policy == "volatile-ttl" { ":1\r\n" } else { ":0\r\n" });
        let stats = request(&mut client, &["INFO", "stats"]).text();
        assert_eq!(info_field(&stats, "evicted_keys").as_deref(), Some(if policy == "volatile-ttl" { "3" } else { "4" }));
        stop(client, child);
        let (child, mut client, _) = start(&program, b"0", Some(b"eviction.aof"));
        for db in [0, 7, 15] {
            select(&mut client, db);
            check(&mut client, &["EXISTS", "victim"], ":0\r\n");
        }
        select(&mut client, 4);
        check(&mut client, &["EXISTS", "permanent"], if policy == "volatile-ttl" { ":1\r\n" } else { ":0\r\n" });
        stop(client, child);
    }
}

/// feedAppendOnlyFile emits the first SELECT even for db zero, and emits
/// later SELECTs only when a propagated effect targets a different db.
/// Oracle recording: aof-first-select-and-database-changes records command replies, restart reads
/// and FUNCTION FLUSH. Incremental file bytes are source-derived and absent from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn incremental_aof_records_first_select_and_only_effect_database_changes() {
    let program = CompiledProgram::from_environment();
    let (child, mut client, _) = start(&program, b"0", Some(b"selected.aof"));
    for args in [
        vec!["SET", "same", "zero"], vec!["SELECT", "7"], vec!["GET", "missing"],
        vec!["SET", "same", "seven"], vec!["SET", "extra", "more"],
        vec!["SELECT", "0"], vec!["SET", "same", "ZERO"],
        vec!["SELECT", "15"], vec!["SET", "same", "fifteen"],
        vec!["SELECT", "7"], vec!["GET", "same"], vec!["SELECT", "7"], vec!["SET", "extra", "MORE"],
    ] {
        let result = request(&mut client, &args).wire();
        assert!(!result.starts_with('-'), "{args:?}: {result}");
    }
    stop(client, child);
    let expected: Vec<Vec<&str>> = vec![
        vec!["SELECT", "0"], vec!["SET", "same", "zero"],
        vec!["SELECT", "7"], vec!["SET", "same", "seven"], vec!["SET", "extra", "more"],
        vec!["SELECT", "0"], vec!["SET", "same", "ZERO"],
        vec!["SELECT", "15"], vec!["SET", "same", "fifteen"],
        vec!["SELECT", "7"], vec!["SET", "extra", "MORE"],
    ];
    let bytes = std::fs::read(aof_incremental_path(program.working_directory(), "selected.aof")).unwrap();
    let wire: Vec<u8> = expected.iter().flat_map(|args| resp(args)).collect();
    assert_eq!(bytes, wire, "Redis 7.2.16 feedAppendOnlyFile database context");
    let (child, mut client, _) = start(&program, b"0", Some(b"selected.aof"));
    for (db, value, size) in [(0, "ZERO", 1), (7, "seven", 2), (15, "fifteen", 1)] {
        select(&mut client, db);
        check(&mut client, &["GET", "same"], &format!("${}\r\n{value}\r\n", value.len()));
        check(&mut client, &["DBSIZE"], &format!(":{size}\r\n"));
    }
    select(&mut client, 7);
    check(&mut client, &["GET", "extra"], "$4\r\nMORE\r\n");
    // FUNCTION FLUSH has global scope but Redis propagates it with its
    // caller's database. It must also establish the fresh writer context.
    select(&mut client, 15);
    check(&mut client, &["FUNCTION", "FLUSH"], "+OK\r\n");
    select(&mut client, 0);
    check(&mut client, &["SET", "marker", "v"], "+OK\r\n");
    stop(client, child);
    let mut expected = wire;
    expected.extend(resp(&["SELECT", "15"]));
    expected.extend(resp(&["FUNCTION", "FLUSH"]));
    expected.extend(resp(&["SELECT", "0"]));
    expected.extend(resp(&["SET", "marker", "v"]));
    assert_eq!(std::fs::read(aof_incremental_path(program.working_directory(), "selected.aof")).unwrap(), expected, "global FUNCTION FLUSH preserves the caller's AOF database context");
}

fn seed_persisted_databases(client: &mut TcpStream) {
    check(client, &["SET", "same", "zero"], "+OK\r\n");
    check(client, &["MULTI"], "+OK\r\n");
    for args in [vec!["SELECT", "7"], vec!["SET", "same", "seven"], vec!["COPY", "same", "copy", "DB", "15"], vec!["SELECT", "15"], vec!["SET", "same", "fifteen"]] {
        check(client, &args, "+QUEUED\r\n");
    }
    check(client, &["EXEC"], "*5\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n+OK\r\n");
    let script = "redis.call('SELECT',1); redis.call('SET','same','script'); redis.call('SELECT',7); redis.call('SET','script-only','s'); return 1";
    check(client, &["EVAL", script, "0"], ":1\r\n");
    check(client, &["MOVE", "copy", "1"], ":1\r\n");
    check(client, &["SWAPDB", "0", "15"], "+OK\r\n");
    select(client, 7);
    check(client, &["PEXPIREAT", "same", DEADLINE], ":1\r\n");
    select(client, 3);
    check(client, &["SET", "flushed", "x"], "+OK\r\n");
    check(client, &["FLUSHDB"], "+OK\r\n");
}

fn check_persisted_databases(client: &mut TcpStream) {
    for db in 0..16 {
        select(client, db);
        let (value, size) = match db { 0 => (Some("fifteen"), 1), 1 => (Some("script"), 2), 7 => (Some("seven"), 2), 15 => (Some("zero"), 1), _ => (None, 0) };
        let expected = match value { Some(value) => format!("${}\r\n{value}\r\n", value.len()), None => "$-1\r\n".to_owned() };
        check(client, &["GET", "same"], &expected);
        check(client, &["DBSIZE"], &format!(":{size}\r\n"));
        let expiry = if db == 7 { format!(":{DEADLINE}\r\n") } else if value.is_some() { ":-1\r\n".to_owned() } else { ":-2\r\n".to_owned() };
        check(client, &["PEXPIRETIME", "same"], &expiry);
    }
    select(client, 1);
    check(client, &["GET", "copy"], "$5\r\nseven\r\n");
    select(client, 7);
    check(client, &["GET", "script-only"], "$1\r\ns\r\n");
}

/// Keeps exact records and their order for each (database,key), permitting
/// only independent keys' dictionary traversal order to differ from Redis.
fn rewrite_key_records(bytes: &[u8]) -> BTreeMap<(Vec<u8>, Vec<u8>), Vec<Vec<u8>>> {
    let mut database = None;
    let mut grouped: BTreeMap<_, Vec<_>> = BTreeMap::new();
    let mut complete = Vec::new();
    for record in file_records(bytes) {
        let mut wire = format!("*{}\r\n", record.len()).into_bytes();
        for argument in &record {
            wire.extend_from_slice(format!("${}\r\n", argument.len()).as_bytes());
            wire.extend_from_slice(argument);
            wire.extend_from_slice(b"\r\n");
        }
        complete.extend_from_slice(&wire);
        if record[0] == b"SELECT" {
            assert_eq!(record.len(), 2, "SELECT database arity");
            database = Some(record[1].clone());
        } else {
            assert!(record.len() >= 2, "rewrite record names a key: {record:?}");
            let key = (database.clone().expect("SELECT precedes rewritten data"), record[1].clone());
            grouped.entry(key).or_default().push(wire);
        }
    }
    assert_eq!(complete, bytes, "rewrite records use exact RESP array/bulk framing");
    grouped
}

/// CI checks the comparison machinery against representative wrong results:
/// independent-key order may differ, but data, expiry and RESP framing may not.
/// The input is the raw Redis base recorded in
/// [this oracle run](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn rewrite_oracle_comparison_preserves_per_key_records_and_rejects_wrong_results() {
    let expected = rewrite_key_records(REDIS_7216_REWRITE_BASE);
    let records = file_records(REDIS_7216_REWRITE_BASE);
    let wire = |rows: &[Vec<Vec<u8>>]| -> Vec<u8> {
        rows.iter().flat_map(|row| {
            let args: Vec<_> = row.iter().map(|arg| std::str::from_utf8(arg).unwrap()).collect();
            resp(&args)
        }).collect()
    };
    let copy = records.iter().position(|row| row[0] == b"SET" && row[1] == b"copy").unwrap();
    let script = records.iter().position(|row| row[0] == b"SET" && row[2] == b"script").unwrap();
    let mut reordered = records.clone();
    reordered.swap(copy, script);
    assert_eq!(rewrite_key_records(&wire(&reordered)), expected, "independent db1 keys may traverse in either order");
    let expiry = records.iter().position(|row| row[0] == b"PEXPIREAT").unwrap();
    let mut missing_expiry = records.clone();
    missing_expiry.remove(expiry);
    assert_ne!(rewrite_key_records(&wire(&missing_expiry)), expected, "missing expiry must be detected");
    let mut changed_value = records.clone();
    changed_value[copy][2] = b"wrong".to_vec();
    assert_ne!(rewrite_key_records(&wire(&changed_value)), expected, "changed payload must be detected");
    let mut reversed_expiry = records;
    reversed_expiry.swap(expiry - 1, expiry);
    assert_ne!(rewrite_key_records(&wire(&reversed_expiry)), expected, "SET then PEXPIREAT order within one key must be retained");
    let mut malformed = REDIS_7216_REWRITE_BASE.to_vec();
    malformed[0] = b'+';
    assert!(std::panic::catch_unwind(|| rewrite_key_records(&malformed)).is_err(), "non-array RESP framing must be rejected");
}

/// Effects nested in transactions/scripts, COPY DB, MOVE, SWAPDB and
/// FLUSHDB must replay once in their original databases. Rewriting must
/// retain every nonempty db and absolute TTL, and its post-rewrite suffix
/// must begin with a new SELECT context.
/// Oracle recording: aof-mixed-databases-restart/rewrite records seed/restart replies and the raw
/// rewritten base. Pre-restart verification, DEL suffix and its subsequent verification,
/// incremental bytes, rewrite status and manifest are absent. The equal-context suffix-reset
/// mutation below also needs recording; it preserves the recorded rewritten dataset.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn several_databases_survive_aof_restart_rewrite_and_post_rewrite_suffix() {
    let program = CompiledProgram::from_environment();
    let (child, mut client, _) = start(&program, b"0", Some(b"appendonly.aof"));
    seed_persisted_databases(&mut client);
    check_persisted_databases(&mut client);
    stop(client, child);
    // server.c propagatePendingCommands writes MULTI/EXEC with database -1;
    // SELECT is inserted immediately before the first effect in a new db.
    let effects: &[&[&str]] = &[
        &["SELECT", "0"], &["SET", "same", "zero"],
        &["MULTI"], &["SELECT", "7"], &["SET", "same", "seven"],
        &["COPY", "same", "copy", "DB", "15"], &["SELECT", "15"], &["SET", "same", "fifteen"], &["EXEC"],
        &["MULTI"], &["SELECT", "1"], &["SET", "same", "script"],
        &["SELECT", "7"], &["SET", "script-only", "s"], &["EXEC"],
        &["SELECT", "15"], &["MOVE", "copy", "1"], &["SWAPDB", "0", "15"],
        &["SELECT", "7"], &["PEXPIREAT", "same", DEADLINE],
        &["SELECT", "3"], &["SET", "flushed", "x"], &["FLUSHDB"],
    ];
    let expected: Vec<_> = effects.iter().flat_map(|args| resp(args)).collect();
    assert_eq!(std::fs::read(aof_incremental_path(program.working_directory(), "appendonly.aof")).unwrap(), expected, "transaction and script AOF contexts follow Redis 7.2.16 effects");
    let (child, mut client, _) = start(&program, b"0", Some(b"appendonly.aof"));
    check_persisted_databases(&mut client);
    // Establish db seven as the last emitted context on this writer without
    // changing the dataset. Otherwise the seed's final FLUSHDB in db three
    // would require SELECT seven even if rewrite forgot no context.
    select(&mut client, 7);
    check(&mut client, &["SET", "script-only", "s"], "+OK\r\n");
    rewrite_start(&mut client);
    assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok");
    let directory = program.working_directory().join("appendonlydir");
    let manifest = std::fs::read_to_string(directory.join("appendonly.aof.manifest")).unwrap();
    let base_name = manifest.lines().find(|line| line.ends_with("type b")).unwrap().split_whitespace().nth(1).unwrap();
    let base = std::fs::read(directory.join(base_name)).unwrap();
    // The recorded base preserves Redis's observed traversal order. Firn
    // may traverse independent keys differently; compare each (db,key)'s
    // ordered raw records, including the presence and position of expiry.
    assert_eq!(rewrite_key_records(&base), rewrite_key_records(REDIS_7216_REWRITE_BASE));
    let records = aof_records(&base);
    let selected: Vec<_> = records.iter().filter(|record| record[0] == "SELECT").map(|record| record[1].clone()).collect();
    assert_eq!(selected, ["0", "1", "7", "15"], "rewrite selects every nonempty database in db order");
    // Rotation forgets the old incremental context, including when the last
    // propagated pre-rewrite write and the first suffix write use db seven.
    select(&mut client, 7);
    check(&mut client, &["SET", "suffix", "after"], "+OK\r\n");
    stop(client, child);
    let suffix_name = manifest.lines().find(|line| line.ends_with("type i")).unwrap().split_whitespace().nth(1).unwrap();
    let suffix = std::fs::read(directory.join(suffix_name)).unwrap();
    let mut expected = resp(&["SELECT", "7"]);
    expected.extend(resp(&["SET", "suffix", "after"]));
    assert_eq!(suffix, expected);
    let (child, mut client, _) = start(&program, b"0", Some(b"appendonly.aof"));
    // The suffix adds one key to database seven; check the base before removing it.
    select(&mut client, 7);
    check(&mut client, &["GET", "suffix"], "$5\r\nafter\r\n");
    check(&mut client, &["DEL", "suffix"], ":1\r\n");
    check_persisted_databases(&mut client);
    stop(client, child);
}

/// AOF loading uses a private connection; SELECT in a completed MULTI
/// changes subsequent replay context, while an unfinished MULTI is discarded.
/// Opening an ordinary client after either replay always starts at db zero.
/// Oracle recording: This hand-written committed/unfinished MULTI AOF startup fixture is absent
/// from the recording.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn aof_replay_applies_committed_database_selection_and_discards_unfinished_selection() {
    for committed in [false, true] {
        let program = CompiledProgram::from_environment();
        let path = aof_incremental_fixture(program.working_directory(), "context.aof");
        let mut bytes = resp(&["SELECT", "7"]);
        bytes.extend(resp(&["SET", "same", "seven"]));
        bytes.extend(resp(&["MULTI"]));
        bytes.extend(resp(&["SELECT", "15"]));
        bytes.extend(resp(&["SET", "same", "fifteen"]));
        if committed {
            bytes.extend(resp(&["EXEC"]));
            bytes.extend(resp(&["SET", "after", "fifteen"]));
        }
        std::fs::write(&path, bytes).unwrap();
        let (child, mut client, _) = start(&program, b"0", Some(b"context.aof"));
        check(&mut client, &["GET", "same"], "$-1\r\n");
        select(&mut client, 7);
        check(&mut client, &["GET", "same"], "$5\r\nseven\r\n");
        select(&mut client, 15);
        check(&mut client, &["GET", "same"], if committed { "$7\r\nfifteen\r\n" } else { "$-1\r\n" });
        check(&mut client, &["GET", "after"], if committed { "$7\r\nfifteen\r\n" } else { "$-1\r\n" });
        check(&mut client, &["SET", "new", "v"], "+OK\r\n");
        stop(client, child);
        let (child, mut client, _) = start(&program, b"0", Some(b"context.aof"));
        select(&mut client, 15);
        check(&mut client, &["GET", "new"], "$1\r\nv\r\n");
        stop(client, child);
    }
}

/// Replacement images and subsequent journal effects distinguish (db,key).
/// Repeated writes to the same name in two databases must not coalesce into
/// one image or attach one database's commands to the other's rewrite state.
/// Oracle recording: This paced concurrent rewrite, journal-image count and restart fixture is
/// absent from the recording; the image counter is Firn-specific.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn aof_rewrite_journal_keeps_same_named_keys_in_distinct_databases() {
    let program = CompiledProgram::from_environment();
    let (child, mut client, _) = start(&program, b"0", Some(b"appendonly.aof"));
    let padding = "p".repeat(4096);
    for index in 0..128 {
        check(&mut client, &["SET", &format!("padding:{index}"), &padding], "+OK\r\n");
    }
    check(&mut client, &["SET", "same", "before-zero"], "+OK\r\n");
    select(&mut client, 15);
    check(&mut client, &["SET", "same", "before-fifteen"], "+OK\r\n");
    rewrite_key_delay(&mut client, "10000");
    rewrite_start(&mut client);
    assert_eq!(rewrite_info(&mut client)["aof_rewrite_in_progress"], "1", "the writes must overlap capture");
    for generation in ["first", "second"] {
        for db in [0, 15] {
            select(&mut client, db);
            check(&mut client, &["SET", "same", &format!("{generation}:{db}")], "+OK\r\n");
        }
    }
    let capturing = rewrite_info(&mut client);
    assert_eq!(capturing["aof_rewrite_in_progress"], "1", "fixture must hold capture through both rounds of writes");
    assert_eq!(capturing["firn_aof_rewrite_journal_images"], "2", "same key name requires one image in each database");
    assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok");
    stop(client, child);
    let (child, mut client, _) = start(&program, b"0", Some(b"appendonly.aof"));
    for db in [0, 15] {
        select(&mut client, db);
        let value = format!("second:{db}");
        check(&mut client, &["GET", "same"], &format!("${}\r\n{value}\r\n", value.len()));
    }
    stop(client, child);
}

/// A script/EXEC stores an already-expired key and looks it up with OBJECT
/// in the same frozen execution unit. Active expiry cannot interpose, so the
/// DEL is necessarily OBJECT's lazy-expiry effect in the selected database.
/// Oracle recording: exec-object-lazy-expiry-selected-database and script-object-lazy-expiry-
/// selected-database record this command history and restart replies. Incremental file bytes are
/// source-derived and absent.
/// [https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856](https://github.com/Ming-Research/Firn-wf/actions/runs/38400985856).
#[test]
fn object_lazy_expiry_after_select_logs_the_selected_database() {
    for scripted in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, b"0", Some(b"object.aof"));
        check(&mut client, &["SET", "doomed", "keep-zero"], "+OK\r\n");
        if scripted {
            let script = "redis.call('SELECT',15); redis.call('SET','doomed','gone','PXAT',1); redis.call('SELECT',0); redis.call('SET','marker','v'); redis.call('SELECT',15); return redis.call('OBJECT','IDLETIME','doomed')";
            check(&mut client, &["EVAL", script, "0"], "$-1\r\n");
            check(&mut client, &["GET", "doomed"], "$9\r\nkeep-zero\r\n");
        } else {
            check(&mut client, &["MULTI"], "+OK\r\n");
            for args in [vec!["SELECT", "15"], vec!["SET", "doomed", "gone", "PXAT", "1"], vec!["SELECT", "0"], vec!["SET", "marker", "v"], vec!["SELECT", "15"], vec!["OBJECT", "IDLETIME", "doomed"]] {
                check(&mut client, &args, "+QUEUED\r\n");
            }
            check(&mut client, &["EXEC"], "*6\r\n+OK\r\n+OK\r\n+OK\r\n+OK\r\n+OK\r\n$-1\r\n");
        }
        stop(client, child);
        let records: &[&[&str]] = &[
            &["SELECT", "0"], &["SET", "doomed", "keep-zero"],
            &["MULTI"], &["SELECT", "15"], &["SET", "doomed", "gone", "PXAT", "1"],
            &["SELECT", "0"], &["SET", "marker", "v"], &["SELECT", "15"], &["DEL", "doomed"], &["EXEC"],
        ];
        let expected: Vec<_> = records.iter().flat_map(|args| resp(args)).collect();
        assert_eq!(std::fs::read(aof_incremental_path(program.working_directory(), "object.aof")).unwrap(), expected, "OBJECT's removal keeps its execution database");
        let (child, mut client, _) = start(&program, b"0", Some(b"object.aof"));
        check(&mut client, &["GET", "doomed"], "$9\r\nkeep-zero\r\n");
        check(&mut client, &["GET", "marker"], "$1\r\nv\r\n");
        select(&mut client, 15);
        check(&mut client, &["GET", "doomed"], "$-1\r\n");
        stop(client, child);
    }
}
