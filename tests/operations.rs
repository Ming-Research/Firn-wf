//! Runs the operator guide's marked examples literally through the gate.

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
        let checked = words.next() == Some("firn-check");
        assert!(!matches!(kind, "sh" | "bash" | "shell" | "conf") || checked,
            "line {}: every shell/config example needs firn-check", line + 1);
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
    for invalid in [
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
