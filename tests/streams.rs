//! Streams against Redis 7.2.16 (board item firn-streams-core).
//!
//! Each case sends the same commands to firn and to the redis-server
//! `REDIS_SERVER` names and compares every reply, and, with the append-only
//! file on, the bytes each server appended. The commands are chosen to cross
//! the places where Redis 7.2.16's src/t_stream.c starts a new node: the
//! entry and byte limits, entries stored as integers or as strings, deleted
//! entries that still occupy a node, and an append after a node lost its last
//! live entry. Trimming with `~` removes only whole nodes, so its replies and
//! the lengths written into the file show whether firn starts nodes where
//! Redis does.

use super::*;
use std::path::Path;
use std::process::{Command, Stdio};

const STREAM_AOF: &str = "appendonly.aof";

struct Server {
    child: ProgramChild,
    client: TcpStream,
}

fn redis(directory: &Path, aof: bool) -> Server {
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let mut command = Command::new(support::redis_oracle());
    command.current_dir(directory)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .args(["--port", &port, "--bind", "127.0.0.1", "--save", "",
            "--appendonly", if aof { "yes" } else { "no" },
            "--appendfilename", STREAM_AOF, "--aof-use-rdb-preamble", "no",
            "--auto-aof-rewrite-percentage", "0", "--dir"])
        .arg(directory);
    let child = ProgramChild::spawn(&mut command).expect("spawn redis-server").with_port(&lease);
    let client = connect_when_ready(lease.port(), &child);
    Server { child, client }
}

fn firn_in(program: &CompiledProgram, aof: bool) -> Server {
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let appendonly: &[u8] = if aof { b"yes" } else { b"no" };
    let child = program.spawn_on_route(true, &[
        b"--port", port.as_bytes(), b"--bind", b"127.0.0.1", b"--firn-clients", b"0",
        b"--appendonly", appendonly, b"--appendfilename", STREAM_AOF.as_bytes(),
        b"--auto-aof-rewrite-percentage", b"0",
    ]).with_port(&lease);
    let client = connect_when_ready(lease.port(), &child);
    Server { child, client }
}

fn stop(mut server: Server) {
    server.client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("request the stop");
    let mut byte = [0_u8; 1];
    assert_eq!(server.client.read(&mut byte).expect("shutdown closes the connection"), 0);
    let output = server.child.wait_with_output().expect("wait for the server");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

fn ask(server: &mut Server, command: &[&str]) -> Vec<u8> {
    pubsub::request(&mut server.client, command)
}

/// Sends every command to both servers and compares the replies one by one.
fn compare(firn: &mut Server, redis: &mut Server, commands: &[Vec<String>]) {
    for command in commands {
        let arguments: Vec<&str> = command.iter().map(String::as_str).collect();
        let expected = ask(redis, &arguments);
        let actual = ask(firn, &arguments);
        // HELLO's reply names the server and the connection; only the
        // protocol it switches to matters here.
        if arguments[0] == "HELLO" {
            continue;
        }
        assert_eq!(String::from_utf8_lossy(&actual), String::from_utf8_lossy(&expected),
            "firn must answer {arguments:?} as Redis 7.2.16 does");
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(' ').map(str::to_owned).collect()
}

/// Node boundaries: each block sets the node limits, fills a stream with
/// entries whose fields and values are integers or strings of the sizes where
/// listpack encodings change, deletes some, and trims with `~`, whose result
/// depends on where every node starts.
fn boundary_commands() -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    for (entries, bytes) in [("3", "4096"), ("0", "64"), ("100", "200"), ("5", "0"), ("2", "1")] {
        commands.push(words(&format!("CONFIG SET stream-node-max-entries {entries}")));
        commands.push(words(&format!("CONFIG SET stream-node-max-bytes {bytes}")));
        commands.push(words("DEL s"));
        for index in 1..=40_u64 {
            let value = match index % 5 {
                0 => "127".to_owned(),
                1 => "128".to_owned(),
                2 => format!("-{}", 4096 + index),
                3 => "x".repeat(60 + index as usize),
                _ => "01".to_owned(),
            };
            let field = if index % 7 == 0 { "other" } else { "f" };
            commands.push(vec!["XADD".into(), "s".into(), format!("{index}-{index}"),
                field.into(), value, "g".into(), "2147483648".into()]);
        }
        commands.push(words("XDEL s 1-1 2-2 3-3 10-10 11-11 12-12"));
        commands.push(words("XLEN s"));
        commands.push(words("XADD s 41-0 f v"));
        commands.push(words("XTRIM s MAXLEN ~ 30"));
        commands.push(words("XLEN s"));
        commands.push(words("XTRIM s MINID ~ 25"));
        commands.push(words("XRANGE s - + COUNT 3"));
        commands.push(words("XTRIM s MAXLEN ~ 5 LIMIT 3"));
        commands.push(words("XLEN s"));
        commands.push(words("XTRIM s MAXLEN = 4"));
        commands.push(words("XRANGE s - +"));
        commands.push(words("XADD s MAXLEN ~ 2 1000-0 f v"));
        commands.push(words("XLEN s"));
    }
    // An emptied node is freed and the one before it is the tail again.
    commands.push(words("CONFIG SET stream-node-max-entries 2"));
    commands.push(words("CONFIG SET stream-node-max-bytes 4096"));
    commands.push(words("DEL t"));
    for id in ["1-1", "2-1", "3-1", "4-1", "5-1"] {
        commands.push(words(&format!("XADD t {id} a 1")));
    }
    commands.push(words("XDEL t 5-1"));
    commands.push(words("XADD t 6-1 a 1"));
    commands.push(words("XDEL t 3-1 4-1"));
    commands.push(words("XADD t 7-1 a 1"));
    commands.push(words("XTRIM t MAXLEN ~ 1"));
    commands.push(words("XRANGE t - +"));
    commands.push(words("XLEN t"));
    commands
}

fn reply_commands() -> Vec<Vec<String>> {
    [
        "DEL s r", "XADD s 1-1 a 1 b 2", "XADD s 1-* c 3", "XADD s 2-* d 4", "XADD s 5 e 5",
        "XADD s 1-1 a 1", "XADD s 0-0 a 1", "XADD s 0 a 1", "XADD s NOMKSTREAM 9-0 a", "XADD r NOMKSTREAM 9-0 a 1",
        "EXISTS r", "XADD s MAXLEN 1 MINID 1 9-0 a 1", "XADD s MAXLEN -1 9-0 a 1", "XADD s MAXLEN = x 9-0 a 1",
        "XADD s LIMIT 5 9-0 a 1", "XADD s MAXLEN 5 LIMIT 5 9-0 a 1", "XADD s MAXLEN ~ 5 LIMIT -1 9-0 a 1",
        "XADD s MINID ~ bad 9-0 a 1", "XADD s 18446744073709551615-18446744073709551615 a 1",
        "XADD s 9-x a 1", "XADD s +5-1 a 1", "TYPE s", "SCAN 0 TYPE stream", "XLEN s", "XLEN r",
        "XRANGE s - +", "XREVRANGE s + -", "XRANGE s (1-1 +", "XRANGE s 1 2", "XREVRANGE s 2 (1-1",
        "XRANGE s - + COUNT 2", "XRANGE s - + COUNT 0", "XRANGE s - + COUNT -1", "XRANGE s - + COUNT 1 COUNT 2",
        "XRANGE s - + LIMIT 1", "XRANGE s (18446744073709551615-18446744073709551615 +", "XRANGE s - (0-0",
        "XRANGE r - +", "XRANGE s x +", "XREVRANGE s - +", "XDEL r 1-1", "XDEL s bad", "XDEL s 2-0 2-0 77-1",
        "XRANGE s - +", "XREAD STREAMS s 0", "XREAD COUNT 1 STREAMS s r 0 0", "XREAD STREAMS s $",
        "XREAD STREAMS s", "XREAD STREAMS s r 0", "XREAD STREAMS s >", "XREAD GROUP g c STREAMS s 0",
        "XREAD NOACK STREAMS s 0", "XREAD COUNT x STREAMS s 0", "XREAD BLOCK x STREAMS s 0",
        "XREAD BLOCK -1 STREAMS s 0", "XREAD BLOCK 0 STREAMS s 0", "XREAD COUNT 1", "XREAD STREAMS s +",
        "XSETID r 5-0", "XSETID s 1-0", "XSETID s 100-0 ENTRIESADDED 1", "XSETID s 100-0 ENTRIESADDED -1",
        "XSETID s 100-0 MAXDELETEDID 200-0", "XSETID s 100-0 ENTRIESADDED 9 MAXDELETEDID 90-0",
        "XSETID s 80-0", "XADD s 100-0 a 1", "XADD s 100-1 a 1", "XTRIM s", "XTRIM s FOO 1",
        "XTRIM r MAXLEN 0", "XTRIM s MINID 100-1", "XRANGE s - +", "XTRIM s MAXLEN 0", "XLEN s", "EXISTS s",
        "SET k v", "XADD k 1-1 a 1", "XLEN k", "XRANGE k - +", "XTRIM k MAXLEN 1",
        "XDEL k 1-1", "XSETID k 1-1", "XREAD STREAMS k 0", "XADD s 1-1 a", "XADD s 1-1",
        "HELLO 3", "XRANGE s - +", "XREAD STREAMS s 0", "XREAD STREAMS r 0", "HELLO 2",
    ].iter().map(|text| words(text)).collect()
}

fn transaction_commands() -> Vec<Vec<String>> {
    [
        "DEL m", "MULTI", "XADD m 1-1 a 1", "XADD m 2-* b 2", "XADD m MAXLEN = 1 3-0 c 3", "XLEN m",
        "XRANGE m - +", "XREAD STREAMS m 0", "XREAD BLOCK 0 STREAMS m $", "XDEL m 3-0", "XSETID m 9-0",
        "XTRIM m MAXLEN 0", "EXEC", "XLEN m",
        "EVAL return{redis.call('XADD',KEYS[1],'5-1','f','v'),redis.call('XLEN',KEYS[1])} 1 m",
        "EVAL return(redis.call('XRANGE',KEYS[1],'-','+')) 1 m",
        "EVAL return(redis.call('XREAD','STREAMS',KEYS[1],'0')) 1 m",
        "EVAL return(redis.pcall('XREAD','BLOCK','0','STREAMS',KEYS[1],'0')) 1 m",
        "EVAL return(redis.pcall('XADD',KEYS[1],'1-1','f','v')) 1 m",
        "EVAL return(redis.call('XTRIM',KEYS[1],'MAXLEN','~','0')) 1 m",
        "EVAL return(redis.call('XDEL',KEYS[1],'5-1')) 1 m",
        "EVAL return(redis.call('XSETID',KEYS[1],'10-0')) 1 m",
        "XADD m 10-0 a 1",
    ].iter().map(|text| words(text)).collect()
}

#[test]
fn streams_answer_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), false);
    let mut server = firn_in(&program, false);
    compare(&mut server, &mut oracle, &reply_commands());
    compare(&mut server, &mut oracle, &transaction_commands());
    compare(&mut server, &mut oracle, &boundary_commands());
    stop(server);
    stop(oracle);
}

fn increment(directory: &Path) -> Vec<u8> {
    let manifest = std::fs::read_to_string(directory.join("appendonlydir").join(format!("{STREAM_AOF}.manifest")))
        .expect("read the manifest");
    let name = manifest.lines().find(|line| line.ends_with("type i"))
        .and_then(|line| line.split(' ').nth(1)).expect("an increment in the manifest").to_owned();
    std::fs::read(directory.join("appendonlydir").join(name)).expect("read the increment")
}

/// The file records XADD with the ID it made and a trim with `~` with the
/// threshold Redis writes, the length after trimming or the first entry's
/// ID, so the bytes appended show both the rewriting and the node layout.
#[test]
fn streams_record_their_writes_as_redis_does() {
    let directory = fixture_directory();
    let program = CompiledProgram::from_environment();
    let mut oracle = redis(directory.path(), true);
    let mut server = firn_in(&program, true);
    let mut commands = boundary_commands();
    commands.extend(transaction_commands());
    commands.extend([
        "DEL w", "XADD w 5-* a 1", "XADD w 5-* a 1", "XADD w MAXLEN ~ 1 6-* a 1", "XADD w MINID ~ 6 7-* a 1",
        "XTRIM w MINID ~ 9", "XTRIM w MAXLEN ~ 0", "XTRIM w MINID = 7", "XDEL w 6-0 404-0", "XDEL w 404-0",
        "XSETID w 99-0 ENTRIESADDED 50 MAXDELETEDID 9-0",
    ].iter().map(|text| words(text)));
    compare(&mut server, &mut oracle, &commands);
    stop(server);
    stop(oracle);
    let expected = increment(directory.path());
    let actual = increment(program.working_directory());
    assert_eq!(String::from_utf8_lossy(&actual), String::from_utf8_lossy(&expected),
        "firn must append the stream commands Redis 7.2.16 appends");
}

/// A restart replays the file and a rewrite writes one XADD per live entry
/// and XSETID; both must leave the streams answering as before, the last ID,
/// entries added and deleted IDs included, which XADD's refusals and XSETID's
/// checks show.
#[test]
fn streams_survive_a_restart_and_a_rewrite() {
    let program = CompiledProgram::from_environment();
    let mut server = firn_in(&program, true);
    for command in [
        "XADD a 1-1 f v g 1", "XADD a 2-1 f w", "XADD a 3-1 h 128", "XDEL a 2-1", "XADD b 1-1 x y",
        "XDEL b 1-1", "XSETID a 50-0 ENTRIESADDED 9 MAXDELETEDID 2-1",
    ] {
        let arguments: Vec<&str> = command.split(' ').collect();
        let reply = ask(&mut server, &arguments);
        assert!(!reply.starts_with(b"-"), "{command}: {reply:?}");
    }
    let reads: Vec<Vec<&str>> = vec![
        vec!["XRANGE", "a", "-", "+"], vec!["XLEN", "a"], vec!["XLEN", "b"], vec!["TYPE", "b"],
        vec!["XADD", "a", "50-0", "f", "v"], vec!["XSETID", "a", "1-0"], vec!["XSETID", "a", "3-1", "ENTRIESADDED", "1"],
        vec!["XADD", "b", "1-1", "f", "v"], vec!["XREAD", "STREAMS", "a", "b", "0", "0"],
    ];
    let before: Vec<Vec<u8>> = reads.iter().map(|read| ask(&mut server, read)).collect();
    stop(server);
    let mut restarted = firn_in(&program, true);
    for (read, want) in reads.iter().zip(&before) {
        assert_eq!(&ask(&mut restarted, read), want, "after a restart {read:?}");
    }
    assert_eq!(ask(&mut restarted, &["BGREWRITEAOF"]), b"+Background append only file rewriting started\r\n");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        restarted.client.write_all(&resp(&["INFO", "persistence"])).expect("query the rewrite");
        let info = bulk_reply(&mut restarted.client, "persistence");
        if info_field(&info, "aof_rewrite_in_progress").as_deref() == Some("0") {
            assert_eq!(info_field(&info, "aof_last_bgrewrite_status").as_deref(), Some("ok"), "{info:?}");
            break;
        }
        assert!(Instant::now() < deadline, "the rewrite did not finish: {info:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    stop(restarted);
    let mut rewritten = firn_in(&program, true);
    for (read, want) in reads.iter().zip(&before) {
        assert_eq!(&ask(&mut rewritten, read), want, "after a rewrite {read:?}");
    }
    stop(rewritten);
}
