//! Runs the operator guide's marked examples literally through the gate.
//!
//! Every fenced block in the guide must be either tagged `firn-check`, which
//! this test runs and checks, or tagged `untested-example` (for instance
//! ```` ```ini untested-example ````), which states that nothing checks it.
//! Any other fence, including an untagged one, fails the test; shell and
//! config fences cannot be untested.

use super::*;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output, Stdio};

#[derive(Debug)]
struct Block {
    line: usize,
    kind: String,
    action: String,
    attributes: HashMap<String, String>,
    body: String,
}

fn blocks(document: &str) -> Vec<Block> {
    let mut result = Vec::new();
    let mut lines = document.lines().enumerate();
    while let Some((line, text)) = lines.next() {
        let Some(header) = text.strip_prefix("```") else { continue };
        // A final stream expectation consumes the remainder of the header,
        // including spaces. The two stream forms are mutually exclusive.
        let stream = ["stdout", "stderr"].into_iter()
            .filter_map(|name| header.find(&format!(" {name}=")).map(|position| (position, name)))
            .min_by_key(|(position, _)| *position);
        let (header, stream) = match stream {
            Some((position, name)) => {
                let value = &header[position + name.len() + 2..];
                assert!(!value.is_empty(), "line {}: empty {name} expectation", line + 1);
                assert!(!value.contains(" stdout=") && !value.contains(" stderr="),
                    "line {}: only one final stream expectation is supported", line + 1);
                (&header[..position], Some((name, value)))
            }
            None => (header, None),
        };
        let mut words = header.split_whitespace();
        let kind = words.next().unwrap_or_default();
        let marker = words.next();
        let checked = marker == Some("firn-check");
        assert!(!matches!(kind, "sh" | "bash" | "shell" | "conf") || checked,
            "line {}: every shell/config example needs firn-check", line + 1);
        assert!(checked || marker == Some("untested-example"),
            "line {}: a fence must be tagged firn-check or untested-example", line + 1);
        if !checked {
            // Skip other fenced material so its closing fence is not treated
            // as the start of another block.
            assert!(lines.by_ref().any(|(_, text)| text == "```"),
                "line {}: unclosed fence", line + 1);
            continue;
        }
        assert!(matches!(kind, "sh" | "conf"), "line {}: unsupported checked fence", line + 1);
        let mut action = String::new();
        let mut attributes = HashMap::new();
        if let Some((name, value)) = stream {
            assert!(kind == "sh", "line {}: {name} requires a shell block", line + 1);
            attributes.insert(name.to_owned(), value.to_owned());
        }
        for word in words {
            if matches!(word, "start" | "stop") {
                assert!(kind == "sh" && action.is_empty()
                    && attributes.keys().all(|name| matches!(name.as_str(), "stdout" | "stderr")),
                    "line {}: misplaced action", line + 1);
                action = word.to_owned();
            } else {
                let (name, value) = word.split_once('=').expect("fence attribute needs name=value");
                assert!(!value.is_empty(), "line {}: empty attribute", line + 1);
                assert!(if kind == "conf" { name == "file" }
                    else { matches!(name, "exit" | "stdout" | "stderr") },
                    "line {}: unknown attribute {name}", line + 1);
                assert!(attributes.insert(name.to_owned(), value.to_owned()).is_none(),
                    "line {}: duplicate attribute {name}", line + 1);
            }
        }
        if kind == "conf" {
            let file = attributes.get("file").expect("config fence needs file");
            assert!(file != "." && file != ".." && !file.contains(['/','\\']),
                "line {}: config filename must be a single component", line + 1);
        } else {
            attributes.get("exit").expect("shell fence needs exit")
                .parse::<i32>().expect("exit must be an integer");
        }
        let mut body = String::new();
        let mut closed = false;
        for (_, text) in lines.by_ref() {
            if text == "```" { closed = true; break; }
            body.push_str(text);
            body.push('\n');
        }
        assert!(closed && !body.trim().is_empty(), "line {}: empty or unclosed example", line + 1);
        result.push(Block { line: line + 1, kind: kind.to_owned(), action, attributes, body });
    }
    assert!(!result.is_empty(), "the guide has no checked examples");
    result
}

fn matches_output(block: &Block, output: &Output) -> bool {
    output.status.code() == Some(block.attributes["exit"].parse().unwrap())
        && String::from_utf8_lossy(&output.stdout).trim()
            == block.attributes.get("stdout").map(String::as_str).unwrap_or("")
        && match block.attributes.get("stderr") {
            Some(expected) => String::from_utf8_lossy(&output.stderr).contains(expected),
            None => output.stderr.is_empty(),
        }
}

fn check_output(block: &Block, output: &Output) {
    assert!(matches_output(block, output),
        "operations.md:{}: {:?}\ncommand:\n{}\nactual: {output:?}",
        block.line, block.attributes, block.body);
}

fn shell(block: &Block, directory: &Path, path: &std::ffi::OsStr) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-eu", "-c", &block.body]).current_dir(directory)
        .env("PATH", path).env_remove("REDISCLI_AUTH")
        .env_remove("WF_REQUIRE_WINDOWS_IOCP")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

/// The server must have consumed the literal service config, rather than
/// merely listening with a coincidentally correct default port.
fn check_service_config(child: &ProgramChild, directory: &Path) {
    let mut client = connect_when_ready(6379, child);
    client.write_all(&resp(&["AUTH", "ops-example-secret"])).expect("authenticate service example");
    expect_replies(&mut client, b"+OK\r\n", "operator example authentication");
    let config = std::fs::read_to_string(directory.join("redis.conf")).expect("read service config");
    for line in config.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let (name, value) = line.split_once(char::is_whitespace).expect("service directive and value");
        client.write_all(&resp(&["CONFIG", "GET", name])).expect("query documented setting");
        let value = value.trim();
        let expected = format!("*2\r\n${}\r\n{name}\r\n${}\r\n{value}\r\n", name.len(), value.len());
        expect_replies(&mut client, expected.as_bytes(), &format!("documented service setting {name}"));
    }
}

#[test]
fn operator_guide_examples_run_literally() {
    let examples = blocks(include_str!("../docs/operations.md"));
    let fixture = fixture_directory();
    let bin = fixture.path().join("bin");
    std::fs::create_dir(&bin).expect("create example PATH directory");
    let firn = std::env::var_os("FIRN").expect("FIRN is required by the guide test");
    let redis = std::env::var_os("REDIS_SERVER").expect("pinned REDIS_SERVER is required by the guide test");
    let redis_cli = Path::new(&redis).with_file_name("redis-cli");
    for (source, name) in [(Path::new(&firn), "firn"), (redis_cli.as_path(), "redis-cli")] {
        assert!(source.is_absolute() && source.is_file(), "missing example executable: {}", source.display());
        std::os::unix::fs::symlink(source, bin.join(name)).expect("link example executable");
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").expect("PATH")));
    let path = std::env::join_paths(paths).expect("example PATH");
    let lease = support::lease_named_port(6379);
    let mut server: Option<(ProgramChild, &Block)> = None;
    let mut starts = 0;
    let mut configurations = std::collections::HashSet::new();
    let mut loaded = std::collections::HashSet::new();
    for block in &examples {
        if block.kind == "conf" {
            assert!(configurations.insert(block.attributes["file"].clone()),
                "line {}: each config must be declared exactly once", block.line);
            std::fs::write(fixture.path().join(&block.attributes["file"]), &block.body)
                .expect("write literal guide config");
            continue;
        }
        let mut command = shell(block, fixture.path(), &path);
        if block.action == "start" {
            assert!(server.is_none(), "line {}: server already started", block.line);
            let child = ProgramChild::spawn(&mut command).expect("start literal guide command").with_port(&lease);
            check_service_config(&child, fixture.path());
            server = Some((child, block));
            starts += 1;
        } else {
            check_output(block, &support::run_command(&mut command));
            if block.action == "stop" {
                let (child, start) = server.take().expect("stop needs a running server");
                check_output(start, &child.wait_with_output().expect("wait for documented stop"));
            }
        }
        // The guide's format uses a standalone literal `firn <filename>`
        // line to load each config. An unused added config fails the gate;
        // no shell interpreter or rewritten invocation lives in the checker.
        for file in &configurations {
            if block.body.lines().any(|line| line.trim() == format!("firn {file}")) {
                loaded.insert(file.clone());
            }
        }
    }
    assert!(starts > 0, "guide must check a service start");
    assert!(server.is_none(), "guide must stop every started server");
    assert_eq!(loaded, configurations, "every guide config must be loaded by a checked command");
}

#[test]
fn guide_checker_rejects_wrong_exit_reply_and_missing_marker() {
    let mut examples = blocks("```sh firn-check exit=0 stdout=PONG\nprintf PONG\n```\n");
    let block = examples.pop().unwrap();
    let actual = support::run_command(Command::new("/bin/sh").args(["-c", "printf PONG"]));
    assert!(matches_output(&block, &actual), "matching witness must pass");
    let wrong = support::run_command(Command::new("/bin/sh").args(["-c", "printf WRONG"]));
    assert!(!matches_output(&block, &wrong), "wrong reply must fail even with exit 0");
    let wrong = support::run_command(Command::new("/bin/sh").args(["-c", "printf PONG; exit 7"]));
    assert!(!matches_output(&block, &wrong), "wrong exit must fail even with the correct reply");
    let wrong = support::run_command(Command::new("/bin/sh")
        .args(["-c", "printf PONG; printf UNEXPECTED >&2"]));
    assert!(!matches_output(&block, &wrong), "unexpected diagnostics must fail");
    let expected_error = blocks("```sh firn-check exit=0 stderr=FATAL CONFIG FILE ERROR\ntrue\n```\n").pop().unwrap();
    let no_error = support::run_command(Command::new("/bin/sh").args(["-c", "true"]));
    assert!(!matches_output(&expected_error, &no_error), "missing expected refusal diagnostic must fail");
    let shortened_error = support::run_command(Command::new("/bin/sh")
        .args(["-c", "printf FATAL >&2"]));
    assert!(!matches_output(&expected_error, &shortened_error), "shortened refusal frame must fail");
    let both = blocks("```text untested-example\nnot run\n```\n```sh firn-check exit=0\ntrue\n```\n");
    assert_eq!(both.len(), 1, "an explicitly untested fence is skipped, not run");
    for invalid in [
        "```console\n$ printf PONG\n```\n```sh firn-check exit=0\ntrue\n```\n",
        "```\nplain\n```\n```sh firn-check exit=0\ntrue\n```\n",
        "```ini\n[Service]\n```\n```sh firn-check exit=0\ntrue\n```\n",
        "```sh untested-example\nprintf PONG\n```\n```sh firn-check exit=0\ntrue\n```\n",
        "```sh\nprintf PONG\n```\n",
        "```sh firn-check stdout=PONG\nprintf PONG\n```\n",
        "```sh firn-check exit=0 ignored=value\nprintf PONG\n```\n",
        "```sh firn-check exit=0 stdout=PONG stderr=FATAL\nprintf PONG\n```\n",
        "```conf firn-check file=redis.conf\nport 6379\n",
    ] {
        assert!(std::panic::catch_unwind(|| blocks(invalid)).is_err(),
            "malformed or unmarked example must fail: {invalid}");
    }
}

fn write_succeeds(client: &mut TcpStream, arguments: &[&str]) {
    let reply = pubsub::request(client, arguments);
    assert!(!reply.starts_with(b"-"), "{arguments:?}: {reply:?}");
}

fn copy_directory(from: &Path, to: &Path) {
    std::fs::create_dir(to).expect("create the copied append-only directory");
    for entry in std::fs::read_dir(from).expect("list the source append-only directory") {
        let entry = entry.expect("read a directory entry");
        assert!(entry.file_type().expect("entry type").is_file(), "{:?}", entry.path());
        std::fs::copy(entry.path(), to.join(entry.file_name())).expect("copy an append-only file");
    }
}

/// The guide's migration steps with Redis 7.2.16 as the source: write a
/// dataset with `aof-use-rdb-preamble no`, rewrite once so that both a base
/// and an increment hold records, stop, copy the directory and start firn
/// with the same `appendfilename`. firn must answer each read as Redis did
/// before the stop. A firn that skipped the base or the increment would miss
/// the keys or the later writes.
#[test]
fn redis_aof_directory_replays_in_firn() {
    let source = fixture_directory();
    let source_lease = support::lease_port();
    let source_port = source_lease.port().to_string();
    let mut command = Command::new(support::redis_oracle());
    command.current_dir(source.path())
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .args(["--port", &source_port, "--bind", "127.0.0.1", "--save", "", "--appendonly", "yes",
            "--appendfilename", "appendonly.aof", "--aof-use-rdb-preamble", "no", "--dir"])
        .arg(source.path());
    let redis = ProgramChild::spawn(&mut command).expect("spawn the source redis-server")
        .with_port(&source_lease);
    let mut client = connect_when_ready(source_lease.port(), &redis);
    for arguments in [
        &["SET", "string", "v1"][..], &["SET", "counter", "1"], &["RPUSH", "list", "a", "b", "c"],
        &["HSET", "hash", "a", "1", "b", "2"], &["SADD", "set", "x", "y"],
        &["ZADD", "zset", "1", "one", "2", "two"], &["SET", "gone", "temporary"],
    ] {
        write_succeeds(&mut client, arguments);
    }
    assert_eq!(pubsub::request(&mut client, &["BGREWRITEAOF"]),
        b"+Background append only file rewriting started\r\n");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        client.write_all(&resp(&["INFO", "persistence"])).expect("query the source rewrite");
        let info = bulk_reply(&mut client, "source persistence");
        if info_field(&info, "aof_rewrite_in_progress").as_deref() == Some("0") {
            assert_eq!(info_field(&info, "aof_last_bgrewrite_status").as_deref(), Some("ok"), "{info:?}");
            break;
        }
        assert!(Instant::now() < deadline, "the source rewrite did not finish: {info:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    for arguments in [
        &["APPEND", "string", "-more"][..], &["INCRBY", "counter", "41"], &["RPUSH", "list", "d"],
        &["HSET", "hash", "c", "3"], &["SADD", "set", "z"], &["ZADD", "zset", "3", "three"],
        &["DEL", "gone"],
    ] {
        write_succeeds(&mut client, arguments);
    }
    let reads: Vec<Vec<&str>> = vec![
        vec!["GET", "string"], vec!["GET", "counter"], vec!["LRANGE", "list", "0", "-1"],
        vec!["HGET", "hash", "a"], vec!["HGET", "hash", "c"], vec!["HLEN", "hash"],
        vec!["SCARD", "set"], vec!["SISMEMBER", "set", "z"],
        vec!["ZRANGE", "zset", "0", "-1", "WITHSCORES"], vec!["EXISTS", "gone"], vec!["DBSIZE"],
    ];
    let expected: Vec<Vec<u8>> = reads.iter().map(|read| pubsub::request(&mut client, read)).collect();
    assert_eq!(expected[0], b"$7\r\nv1-more\r\n", "Redis holds the string written before and after the rewrite");
    assert_eq!(expected[10], b":6\r\n", "Redis holds six keys, the deleted one gone");
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("stop the source");
    let mut byte = [0_u8; 1];
    assert_eq!(client.read(&mut byte).expect("shutdown closes the source connection"), 0);
    let stopped = redis.wait_with_output().expect("wait for the source");
    assert_eq!(stopped.status.code(), Some(0), "{stopped:?}");

    let manifest = std::fs::read_to_string(source.path().join("appendonlydir/appendonly.aof.manifest"))
        .expect("read the source manifest");
    assert!(manifest.contains(".base.aof ") && manifest.contains("type b")
        && manifest.contains("type i") && !manifest.contains(".rdb"),
        "the source directory must hold a command-format base and an increment: {manifest}");
    let program = CompiledProgram::from_environment();
    copy_directory(&source.path().join("appendonlydir"),
        &program.working_directory().join("appendonlydir"));
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let firn = program.spawn_on_route(true, &[
        b"--port", port.as_bytes(), b"--bind", b"127.0.0.1", b"--appendonly", b"yes",
        b"--appendfilename", b"appendonly.aof",
    ]).with_port(&lease);
    let mut migrated = connect_when_ready(lease.port(), &firn);
    for (read, want) in reads.iter().zip(&expected) {
        assert_eq!(&pubsub::request(&mut migrated, read), want,
            "firn must answer {read:?} as Redis did before the directory was copied");
    }
    migrated.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("stop firn");
    assert_eq!(migrated.read(&mut byte).expect("shutdown closes the connection"), 0);
    let output = firn.wait_with_output().expect("wait for firn");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}
