//! Startup configuration compared with the pinned Redis 7.2.16 oracle.
//! Interim path restrictions are firn-only and identify the Whitefoot request.

use super::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn servers() -> Vec<(&'static str, PathBuf)> {
    let mut servers = vec![("firn", PathBuf::from(std::env::var_os("FIRN").expect("FIRN")))];
    if let Some(binary) = std::env::var_os("REDIS_SERVER") {
        let binary = PathBuf::from(binary);
        let version = support::run_command(Command::new(&binary).arg("--version"));
        assert!(version.status.success(), "Redis version: {version:?}");
        assert!(String::from_utf8_lossy(&version.stdout).contains(" v=7.2.16 "),
            "REDIS_SERVER must name Redis 7.2.16: {version:?}");
        servers.push(("Redis 7.2.16", binary));
    }
    servers
}

fn start(binary: &Path, directory: &Path, arguments: &[OsString], stdin: Option<&Path>) -> ProgramChild {
    let mut command = Command::new(binary);
    command.current_dir(directory).args(arguments)
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .env_remove("WF_REQUIRE_WINDOWS_IOCP");
    if let Some(input) = stdin {
        command.stdin(std::fs::File::open(input).expect("open config supplied on stdin"));
    } else {
        command.stdin(Stdio::null());
    }
    ProgramChild::spawn(&mut command).expect("spawn startup subject")
}

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(|value| OsString::from(*value)).collect()
}

fn config_value(client: &mut TcpStream, name: &str, value: &str, label: &str) {
    client.write_all(&resp(&["CONFIG", "GET", name])).expect("query startup setting");
    let expected = format!("*2\r\n${}\r\n{name}\r\n${}\r\n{value}\r\n", name.len(), value.len());
    expect_replies(client, expected.as_bytes(), &format!("{label}: CONFIG GET {name}"));
}

fn shutdown(client: &mut TcpStream, child: ProgramChild, label: &str) {
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("stop startup subject");
    let mut byte = [0_u8; 1];
    assert_eq!(client.read(&mut byte).expect("shutdown closes connection"), 0, "{label}");
    let output = child.wait_with_output().expect("bounded startup shutdown");
    assert_eq!(output.status.code(), Some(0), "{label}: {output:?}");
    assert!(output.stderr.is_empty(), "{label}: {output:?}");
}

#[test]
fn startup_reads_quoted_config_and_applies_later_command_line_overrides() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    // Both servers consume these exact bytes from the same config file.
    // No include is accepted until firn-req-host-paths is resolved.
    let config = concat!(
        "  # startup comments have leading whitespace\n\n",
        " PoRt 1 \r\nBiNd 127.0.0.1\nTimeout 2\ntimeout 3\n",
        "AppendOnly YeS\nAppendFileName 'startup\\'aof.aof'\n",
        "RequirePass \"line\\n\\r\\t\\b\\a\\\\\\\" # \\x41\"\n",
        "dir ./\nmaxmemory 1mb\nmaxmemory-policy allkeys-lfu\n",
        "maxmemory-samples 7\nmaxmemory-eviction-tenacity 42\n",
        "lfu-log-factor 3\nlfu-decay-time 4\n",
        "auto-aof-rewrite-percentage 0\nauto-aof-rewrite-min-size 2mb\n",
    );
    std::fs::write(fixture.path().join("startup.conf"), config).expect("write shared config");
    let password = "line\n\r\t\u{8}\u{7}\\\" # A";
    for (label, binary) in servers() {
        let args = arguments(&["startup.conf", "--port", &port, "--timeout", "9",
            "--TIMEOUT", "11", "--maxmemory", "2mb"]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["AUTH", password])).expect("authenticate decoded quoted password");
        expect_replies(&mut client, b"+OK\r\n", &format!("{label}: quoted password"));
        for (name, value) in [
            ("port", port.as_str()), ("bind", "127.0.0.1"), ("timeout", "11"),
            ("appendonly", "yes"), ("appendfilename", "startup'aof.aof"),
            ("requirepass", password), ("maxmemory", "2097152"),
            ("maxmemory-policy", "allkeys-lfu"), ("maxmemory-samples", "7"),
            ("maxmemory-eviction-tenacity", "42"), ("lfu-log-factor", "3"),
            ("lfu-decay-time", "4"), ("auto-aof-rewrite-percentage", "0"),
            ("auto-aof-rewrite-min-size", "2097152"),
        ] {
            config_value(&mut client, name, value, label);
        }
        assert!(fixture.path().join("appendonlydir").is_dir(), "{label}: appendonly was ignored");
        shutdown(&mut client, child, label);
        std::fs::remove_dir_all(fixture.path().join("appendonlydir")).expect("isolate next oracle AOF state");
    }
}

#[test]
fn startup_command_line_preserves_argument_bytes_and_empty_password() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let password = "quote\" slash\\ apostrophe' # \n\r\t\u{8}\u{7}";
    for (label, binary) in servers() {
        let args = arguments(&["--port", &port, "--bind", "127.0.0.1", "--requirepass", password]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["AUTH", password])).expect("authenticate raw argv password");
        expect_replies(&mut client, b"+OK\r\n", label);
        config_value(&mut client, "requirepass", password, label);
        shutdown(&mut client, child, label);

        let args = arguments(&["--port", &port, "--bind", "127.0.0.1", "--requirepass", ""]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["PING"])).expect("no authentication for empty password");
        expect_replies(&mut client, b"+PONG\r\n", label);
        config_value(&mut client, "requirepass", "", label);
        shutdown(&mut client, child, label);
    }
}

fn fatal(line_number: usize, line: &str, reason: &str) -> Vec<u8> {
    format!("\n*** FATAL CONFIG FILE ERROR (Redis 7.2.16) ***\nReading the configuration file, at line {line_number}\n>>> '{line}'\n{reason}\n").into_bytes()
}

fn assert_fatal(output: Output, expected: &[u8], label: &str) {
    assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
    assert_eq!(output.stderr, expected, "{label}: fatal config diagnostic; stdout={:?}", output.stdout);
}

#[test]
fn startup_errors_match_redis_exactly_on_line_three() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let cases = [
        ("  no-such-startup-directive value \t", "no-such-startup-directive value", "Bad directive or wrong number of arguments"),
        ("  timeout 1 2 \t", "timeout 1 2", "wrong number of arguments"),
        ("  timeout nope \t", "timeout nope", "argument couldn't be parsed into an integer"),
        ("  requirepass \"unfinished \t", "requirepass \"unfinished", "Unbalanced quotes in configuration line"),
        ("requirepass \"closed\"suffix", "requirepass \"closed\"suffix", "Unbalanced quotes in configuration line"),
        ("include", "include", "Bad directive or wrong number of arguments"),
        ("appendfilename \"\"", "appendfilename \"\"", "appendfilename can't be empty"),
        ("appendfilename sub/name.aof", "appendfilename sub/name.aof", "appendfilename can't be a path, just a filename"),
        ("port -1", "port -1", "argument must be between 0 and 65535 inclusive"),
        ("port 65536", "port 65536", "argument must be between 0 and 65535 inclusive"),
        ("timeout 01", "timeout 01", "argument couldn't be parsed into an integer"),
        ("timeout -1", "timeout -1", "argument must be between 0 and 2147483647 inclusive"),
        ("maxmemory-samples 0", "maxmemory-samples 0", "argument must be between 1 and 2147483647 inclusive"),
        ("maxmemory-eviction-tenacity 101", "maxmemory-eviction-tenacity 101", "argument must be between 0 and 100 inclusive"),
        ("lfu-log-factor -1", "lfu-log-factor -1", "argument must be between 0 and 2147483647 inclusive"),
        ("maxmemory nonsense", "maxmemory nonsense", "argument must be a memory value"),
    ];
    for (raw, printed, reason) in cases {
        std::fs::write(fixture.path().join("startup.conf"), format!("# line one\n\n{raw}\n"))
            .expect("write invalid config");
        for (label, binary) in servers() {
            let args = arguments(&["startup.conf", "--port", &port]);
            let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
                .wait_with_output().expect("bounded fatal startup");
            assert_fatal(output, &fatal(3, printed, reason), label);
        }
    }
}

#[test]
fn startup_command_line_errors_continue_configuration_line_numbers() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    for (label, binary) in servers() {
        // Redis adds a newline even when there is no config file.
        let args = arguments(&["--no-such-startup-directive", "value", "--port", &port]);
        let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
            .wait_with_output().expect("bounded CLI-only failure");
        assert_fatal(output, &fatal(2, "no-such-startup-directive \"value\"", "Bad directive or wrong number of arguments"), label);

        for (file, line_number) in [("# one\n\n# three", 4), ("# one\n\n# three\n", 5)] {
            std::fs::write(fixture.path().join("startup.conf"), file).expect("write line-count config");
            let args = arguments(&["startup.conf", "--timeout", "bad integer", "--port", &port]);
            let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
                .wait_with_output().expect("bounded CLI-after-file failure");
            assert_fatal(output, &fatal(line_number, "timeout \"bad integer\"", "argument couldn't be parsed into an integer"), label);
        }
    }
}

#[test]
fn startup_reads_standard_input_before_command_line_overrides() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let input = fixture.path().join("stdin.conf");
    std::fs::write(&input, "# stdin\nport 1\nbind 127.0.0.1\ntimeout 3\n").expect("write stdin config");
    for (label, binary) in servers() {
        let args = arguments(&["-", "--port", &port, "--timeout", "7"]);
        let child = start(&binary, fixture.path(), &args, Some(&input)).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        config_value(&mut client, "port", &port, label);
        config_value(&mut client, "timeout", "7", label);
        shutdown(&mut client, child, label);
    }
}


#[test]
fn startup_standard_input_errors_keep_exact_file_diagnostics() {
    let fixture = fixture_directory();
    let input = fixture.path().join("stdin.conf");
    std::fs::write(&input, "# stdin\n\ntimeout nope\n").expect("write invalid stdin config");
    for (label, binary) in servers() {
        let output = start(&binary, fixture.path(), &arguments(&["-"]), Some(&input))
            .wait_with_output().expect("bounded stdin config refusal");
        assert_fatal(output, &fatal(3, "timeout nope", "argument couldn't be parsed into an integer"), label);
    }
}

#[test]
fn startup_refuses_interim_path_gaps_without_creating_directories() {
    let fixture = fixture_directory();
    let binary = PathBuf::from(std::env::var_os("FIRN").expect("FIRN"));
    for (line, reason) in [
        ("include missing.conf", "firn cannot yet read include paths (firn-req-host-paths)"),
        ("dir missing-directory", "firn cannot yet change its working directory (firn-req-host-paths)"),
        ("dir /tmp", "firn cannot yet change its working directory (firn-req-host-paths)"),
        ("bind 127.0.0.1 127.0.0.2", "firn supports only one IPv4 bind address or *"),
        ("databases 1", "Bad directive or wrong number of arguments"),
        ("appendfsync everysec", "Bad directive or wrong number of arguments"),
    ] {
        std::fs::write(fixture.path().join("startup.conf"), format!("{line}\n")).expect("write gap config");
        let output = start(&binary, fixture.path(), &arguments(&["startup.conf"]), None)
            .wait_with_output().expect("bounded gap refusal");
        assert_fatal(output, &fatal(1, line, reason), "firn interim path restrictions");
        assert!(!fixture.path().join("missing-directory").exists(), "dir must not create the refused directory");
    }
    let absolute = fixture.path().join("startup.conf");
    let output = start(&binary, fixture.path(), &[absolute.into_os_string()], None)
        .wait_with_output().expect("bounded config-path refusal");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(output.stderr, b"firn cannot yet open paths outside its working directory (firn-req-host-paths)\n");
}

#[test]
fn startup_defaults_keep_loopback_and_accept_explicit_wildcard_bind() {
    let fixture = fixture_directory();
    let binary = PathBuf::from(std::env::var_os("FIRN").expect("FIRN"));
    let lease = support::lease_port();
    let port = lease.port().to_string();
    for bind in [None, Some("*")] {
        let mut args = arguments(&["--port", &port]);
        if let Some(address) = bind {
            args.extend(arguments(&["--bind", address]));
        }
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        for (name, value) in [
            ("bind", bind.unwrap_or("127.0.0.1")), ("timeout", "0"),
            ("appendonly", "no"), ("appendfilename", "appendonly.aof"),
            ("requirepass", ""), ("maxmemory", "0"), ("maxmemory-policy", "noeviction"),
        ] {
            config_value(&mut client, name, value, "firn startup defaults");
        }
        assert!(!fixture.path().join("appendonlydir").exists(), "default startup must not create AOF state");
        shutdown(&mut client, child, "firn startup defaults");
    }
}

#[test]
fn startup_opens_nested_relative_config_paths() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    std::fs::create_dir(fixture.path().join("sub")).expect("create config subdirectory");
    std::fs::write(fixture.path().join("sub/startup.conf"), "bind 127.0.0.1\ntimeout 5\n")
        .expect("write nested config");
    for (label, binary) in servers() {
        let args = arguments(&["sub/startup.conf", "--port", &port]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        config_value(&mut client, "timeout", "5", label);
        shutdown(&mut client, child, label);
    }
}

#[test]
fn startup_reports_missing_config_files_without_starting() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let output = start(&binary, fixture.path(), &arguments(&["missing.conf"]), None)
            .wait_with_output().expect("bounded missing config refusal");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        if label == "firn" {
            assert_eq!(output.stderr, b"Fatal error, can't open config file 'missing.conf': No such file or directory\n");
        } else {
            // Redis's ordinary log adds a PID/time prefix and resolves the
            // supplied filename to an absolute path; firn has no such logger.
            let expected = format!("Fatal error, can't open config file '{}': No such file or directory",
                fixture.path().join("missing.conf").display());
            assert!(String::from_utf8_lossy(&output.stdout).contains(&expected),
                "{label}: {output:?}");
        }
    }
}

#[test]
fn startup_single_quotes_only_unescape_apostrophes() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    std::fs::write(fixture.path().join("startup.conf"),
        "bind 127.0.0.1\nrequirepass 'single\\n\\x41\\'password'\n")
        .expect("write singly quoted password");
    let password = "single\\n\\x41'password";
    for (label, binary) in servers() {
        let args = arguments(&["startup.conf", "--port", &port]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["AUTH", password])).expect("authenticate singly quoted password");
        expect_replies(&mut client, b"+OK\r\n", label);
        config_value(&mut client, "requirepass", password, label);
        shutdown(&mut client, child, label);
    }
}

#[test]
fn startup_preserves_nul_in_password_and_truncates_c_string_filename() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    std::fs::write(fixture.path().join("startup.conf"), concat!(
        "bind 127.0.0.1\n",
        "requirepass \"binary\\x00password\"\n",
        "appendfilename \"binary\\x00/ignored\"\n",
    )).expect("write NUL-escaped settings");
    let password = "binary\0password";
    for (label, binary) in servers() {
        let args = arguments(&["startup.conf", "--port", &port]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["AUTH", password])).expect("authenticate password containing NUL");
        expect_replies(&mut client, b"+OK\r\n", label);
        config_value(&mut client, "requirepass", password, label);
        config_value(&mut client, "appendfilename", "binary", label);
        config_value(&mut client, "appendonly", "no", label);
        assert!(!fixture.path().join("appendonlydir").exists(), "filename alone must not enable AOF");
        shutdown(&mut client, child, label);
    }
}

#[test]
fn startup_zero_final_port_exits_but_a_later_port_override_listens() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let reason = "Configured to not listen anywhere, exiting.";
    for (label, binary) in servers() {
        let output = start(&binary, fixture.path(), &arguments(&["--port", "0"]), None)
            .wait_with_output().expect("bounded no-listener startup");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        if label == "firn" {
            assert_eq!(output.stderr, format!("{reason}\n").as_bytes());
        } else {
            assert!(String::from_utf8_lossy(&output.stdout).contains(reason),
                "{label}: {output:?}");
        }
        let args = arguments(&["--port", "0", "--port", &port, "--bind", "127.0.0.1"]);
        let child = start(&binary, fixture.path(), &args, None).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        config_value(&mut client, "port", &port, label);
        shutdown(&mut client, child, label);
    }
}

#[test]
fn startup_rejects_naked_cli_arguments_after_the_file_in_source_order() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    std::fs::write(fixture.path().join("empty.conf"), "").expect("write empty config");
    for (label, binary) in servers() {
        let args = arguments(&["empty.conf", "orphan", "--port", &port]);
        let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
            .wait_with_output().expect("bounded naked unknown directive refusal");
        assert_fatal(output, &fatal(2, "\"orphan\"", "Bad directive or wrong number of arguments"), label);
    }
    let binary = PathBuf::from(std::env::var_os("FIRN").expect("FIRN"));
    // Redis can interpret these quoted naked arguments as a valid directive;
    // firn's invocation requires the -- prefix even when the name is known.
    let args = arguments(&["empty.conf", "timeout", "3", "--port", &port]);
    let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
        .wait_with_output().expect("bounded missing directive prefix refusal");
    assert_fatal(output, &fatal(2, "\"timeout\" \"3\"", "Bad directive or wrong number of arguments"), "firn");

    std::fs::write(fixture.path().join("invalid.conf"), "# first\n\ntimeout nope\n")
        .expect("write earlier invalid directive");
    let args = arguments(&["invalid.conf", "timeout", "3", "--port", &port]);
    let output = start(&binary, fixture.path(), &args, None).with_port(&lease)
        .wait_with_output().expect("bounded earlier file error refusal");
    assert_fatal(output, &fatal(3, "timeout nope", "argument couldn't be parsed into an integer"), "firn");
}

#[test]
fn startup_raw_nul_truncates_one_input_line_without_discarding_later_lines() {
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port().to_string();
    let input = fixture.path().join("raw.conf");
    // Redis fgets consumes through the first LF, while sdscat copies only
    // through the NUL. The next blank line supplies the missing LF, making
    // timeout the second line of the aggregate configuration string.
    let mut chunk_boundary = vec![b'#'];
    chunk_boundary.extend(std::iter::repeat_n(b'a', 1022));
    chunk_boundary.extend_from_slice(b"\0\ntimeout nope\n");
    // A NUL in byte 1024 ends a full fgets chunk. Its following LF starts
    // the next chunk and must be retained even though the prior chunk had NUL.
    for (variant, bytes) in [
        ("line", b"# comment\0tail\n\ntimeout nope\n".as_slice()),
        ("1024-byte boundary", chunk_boundary.as_slice()),
    ] {
        std::fs::write(&input, bytes).expect("write raw NUL configuration");
        for (label, binary) in servers() {
            for stdin in [false, true] {
                let source = if stdin { "-" } else { "raw.conf" };
                let args = arguments(&[source, "--port", &port]);
                let output = start(&binary, fixture.path(), &args, stdin.then_some(input.as_path()))
                    .with_port(&lease).wait_with_output().expect("bounded raw NUL config refusal");
                assert_fatal(output, &fatal(2, "timeout nope", "argument couldn't be parsed into an integer"),
                    &format!("{label}: {variant}, stdin={stdin}"));
            }
        }
    }
}
