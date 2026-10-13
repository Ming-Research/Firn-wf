//! Redis 7.2.16 src/aof.c loadSingleAppendOnlyFile is the source oracle.
//! REDIS_SERVER, when supplied, checks the same fixtures against that release.

use super::*;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const NAME: &str = "load-rules.aof";

fn redis() -> Option<std::path::PathBuf> {
    let binary = std::env::var_os("REDIS_SERVER").map(std::path::PathBuf::from)?;
    let version = support::run_command(Command::new(&binary).arg("--version"));
    assert!(version.status.success(), "Redis version command: {version:?}");
    assert!(String::from_utf8_lossy(&version.stdout).contains(" v=7.2.16 "), "{version:?}");
    Some(binary)
}

fn start_redis(binary: &Path, directory: &Path, lease: &support::PortLease) -> ProgramChild {
    start_redis_with_limit(binary, directory, lease, support::PROGRAM_DEADLINE)
}

fn start_redis_with_limit(
    binary: &Path, directory: &Path, lease: &support::PortLease, limit: Duration,
) -> ProgramChild {
    ProgramChild::spawn_with_limit(
        Command::new(binary)
            .current_dir(directory)
            .args(["--bind", "127.0.0.1", "--port", &lease.port().to_string(),
                "--appendonly", "yes", "--appendfilename", NAME,
                "--appenddirname", "appendonlydir", "--aof-use-rdb-preamble", "no",
                "--save", "", "--logfile", ""])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()), limit,
    ).expect("spawn Redis AOF oracle").with_port(lease)
}

fn bad_format_message() -> String {
    format!("Bad file format reading the append only file {NAME}.1.incr.aof: \
        make a backup of your AOF file, then use ./redis-check-aof --fix <filename.manifest>")
}

fn assert_bad_format(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "AOF format failure: {output:?}");
    let expected = bad_format_message();
    let logged = [&output.stdout, &output.stderr].iter().any(|bytes| {
        String::from_utf8_lossy(bytes).lines().any(|line| line.ends_with(&expected))
    });
    assert!(logged, "missing Redis's exact AOF format diagnostic {expected:?}: {output:?}");
}

fn assert_log_line(output: &Output, expected: &str) {
    let logged = [&output.stdout, &output.stderr].iter().any(|bytes| {
        String::from_utf8_lossy(bytes).lines().any(|line| line.ends_with(expected))
    });
    assert!(logged, "missing AOF diagnostic {expected:?}: {output:?}");
}

fn replay_child(
    program: &CompiledProgram, binary: Option<&Path>, directory: &Path,
    lease: &support::PortLease,
) -> ProgramChild {
    if let Some(binary) = binary {
        start_redis(binary, directory, lease)
    } else {
        let text = lease.port().to_string();
        program.spawn_in_directory(directory, &[text.as_bytes(), b"0", NAME.as_bytes()])
            .with_port(lease)
    }
}

#[test]
fn unknown_commands_stop_replay_before_transaction_tail_recovery() {
    // Redis 7.2.16 src/aof.c:1540-1547 looks up every complete command
    // before queuing it in MULTI and exits 1 if it is unknown. In particular,
    // an unclosed transaction cannot turn an unknown command into truncation.
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for command in ["BOGUSXX", "bogusxx"] {
        for transaction in [0, 1, 2] {
            let mut content = resp(&["SET", "before", "1"]);
            if transaction != 0 { content.extend(resp(&["MULTI"])); }
            content.extend(resp(&[command]));
            content.extend(resp(&["SET", "after", "2"]));
            if transaction == 1 { content.extend(resp(&["EXEC"])); }
            let expected = format!("Unknown command '{command}' reading the append only file {NAME}.1.incr.aof");
            for oracle in [false, true] {
                if oracle && binary.is_none() { continue; }
                let fixture = fixture_directory();
                let path = aof_incremental_fixture(fixture.path(), NAME);
                std::fs::write(&path, &content).unwrap();
                let lease = support::lease_port();
                let output = replay_child(&program, if oracle { binary.as_deref() } else { None },
                    fixture.path(), &lease).wait_with_output().expect("wait for unknown-command refusal");
                assert_eq!(output.status.code(), Some(1), "unknown-command startup: {output:?}");
                assert_log_line(&output, &expected);
                assert_eq!(std::fs::read(&path).unwrap(), content, "unknown commands never truncate their AOF");
            }
        }
    }
}

#[test]
fn unknown_container_subcommands_stop_replay_including_unfinished_transactions() {
    // Redis's lookupCommand resolves container subcommands before execution;
    // its AOF diagnostic prints the container name, not the subcommand.
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for container in ["FUNCTION", "CONFIG"] {
        for transaction in [false, true] {
            let mut content = Vec::new();
            if transaction { content.extend(resp(&["MULTI"])); }
            content.extend(resp(&[container, "BOGUS"]));
            content.extend(resp(&["SET", "after", "2"]));
            for oracle in [false, true] {
                if oracle && binary.is_none() { continue; }
                let fixture = fixture_directory();
                let path = aof_incremental_fixture(fixture.path(), NAME);
                std::fs::write(&path, &content).unwrap();
                let lease = support::lease_port();
                let output = replay_child(&program, if oracle { binary.as_deref() } else { None },
                    fixture.path(), &lease).wait_with_output().expect("wait for unknown-subcommand refusal");
                assert_eq!(output.status.code(), Some(1), "unknown-subcommand startup: {output:?}");
                assert_log_line(&output, &format!("Unknown command '{container}' reading the append only file {NAME}.1.incr.aof"));
                assert_eq!(std::fs::read(&path).unwrap(), content, "unknown subcommands never truncate");
            }
        }
    }
}

#[test]
fn nonzero_database_selection_refuses_firn_but_redis_loads_database_one() {
    // Redis's SELECT changes the AOF client's database, also when queued in
    // EXEC. Its AOF loader calls SELECT without an arity check, so an extra
    // argument still selects db 1. Firn must refuse both instead of applying
    // the SET in db 0 or treating the SELECT as a harmless arity error.
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for (selection, transaction) in [
        (vec!["SELECT", "1"], false),
        (vec!["SELECT", "1"], true),
        (vec!["SELECT", "1", "extra"], false),
        (vec!["SELECT", "1", "extra"], true),
    ] {
        let mut content = resp(&["SET", "before", "1"]);
        if transaction { content.extend(resp(&["MULTI"])); }
        content.extend(resp(&selection));
        content.extend(resp(&["SET", "after", "2"]));
        if transaction { content.extend(resp(&["EXEC"])); }
        for oracle in [false, true] {
            if oracle && binary.is_none() { continue; }
            let fixture = fixture_directory();
            let path = aof_incremental_fixture(fixture.path(), NAME);
            std::fs::write(&path, &content).unwrap();
            let lease = support::lease_port();
            let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
                fixture.path(), &lease);
            if oracle {
                let mut client = connect_when_ready(lease.port(), &child);
                assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
                assert_eq!(memory_request(&mut client, &["GET", "after"]), "$-1\r\n");
                assert_eq!(memory_request(&mut client, &["SELECT", "1"]), "+OK\r\n");
                assert_eq!(memory_request(&mut client, &["GET", "before"]), "$-1\r\n");
                assert_eq!(memory_request(&mut client, &["GET", "after"]), "$1\r\n2\r\n");
                assert_eq!(std::fs::read(&path).unwrap(), content, "Redis loads the db 1 record intact");
                client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
                let output = child.wait_with_output().unwrap();
                assert_eq!(output.status.code(), Some(0), "Redis supports database one: {output:?}");
            } else {
                let output = child.wait_with_output().expect("wait for unsupported database refusal");
                assert_eq!(output.status.code(), Some(1), "Firn refuses database one: {output:?}");
                assert_log_line(&output, &format!("Firn cannot load SELECT 1 reading the append only file {NAME}.1.incr.aof: only database 0 is supported"));
                assert_eq!(std::fs::read(&path).unwrap(), content, "database refusal preserves the AOF");
            }
        }
    }
}

#[test]
fn database_zero_selection_still_loads_directly_and_in_transactions() {
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for transaction in [false, true] {
        let mut content = resp(&["SET", "before", "1"]);
        if transaction { content.extend(resp(&["MULTI"])); }
        content.extend(resp(&["SELECT", "0"]));
        content.extend(resp(&["SET", "after", "2"]));
        if transaction { content.extend(resp(&["EXEC"])); }
        for oracle in [false, true] {
            if oracle && binary.is_none() { continue; }
            let fixture = fixture_directory();
            let path = aof_incremental_fixture(fixture.path(), NAME);
            std::fs::write(&path, &content).unwrap();
            let lease = support::lease_port();
            let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
                fixture.path(), &lease);
            loaded_checks(child, lease.port());
            assert_eq!(std::fs::read(&path).unwrap(), content, "supported SELECT 0 remains intact");
        }
    }
}

#[test]
fn cross_database_copy_refuses_firn_but_redis_copies_into_database_one() {
    // COPY can address another database without SELECT. Refusing only SELECT
    // would allow this record to copy into the wrong database or lose the copy.
    let content = [resp(&["SET", "source", "value"]),
        resp(&["COPY", "source", "target", "DB", "1"])].concat();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, &content).unwrap();
        let lease = support::lease_port();
        let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
            fixture.path(), &lease);
        if oracle {
            let mut client = connect_when_ready(lease.port(), &child);
            assert_eq!(memory_request(&mut client, &["GET", "source"]), "$5\r\nvalue\r\n");
            assert_eq!(memory_request(&mut client, &["GET", "target"]), "$-1\r\n");
            assert_eq!(memory_request(&mut client, &["SELECT", "1"]), "+OK\r\n");
            assert_eq!(memory_request(&mut client, &["GET", "target"]), "$5\r\nvalue\r\n");
            assert_eq!(std::fs::read(&path).unwrap(), content, "Redis loads cross-database COPY intact");
            client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(0), "Redis supports cross-database COPY: {output:?}");
        } else {
            let output = child.wait_with_output().expect("wait for unsupported replay state refusal");
            assert_eq!(output.status.code(), Some(1), "Firn refuses cross-database COPY: {output:?}");
            assert_log_line(&output, &format!("Firn cannot load COPY reading the append only file {NAME}.1.incr.aof: command requires unsupported replay state"));
            assert_eq!(std::fs::read(&path).unwrap(), content, "unsupported replay state preserves the AOF");
        }
    }
}

#[test]
fn unsupported_transaction_and_client_state_records_refuse_without_truncation() {
    // Firn's replay parser cannot represent these client-state transitions
    // or malformed transaction boundaries. They must not become ignored
    // replies followed by a successful load of the remaining SET record.
    let program = CompiledProgram::from_environment();
    for arguments in [
        vec!["DISCARD"], vec!["RESET"], vec!["WATCH", "key"],
        vec!["MULTI", "extra"], vec!["EXEC", "extra"],
    ] {
        for transaction in [false, true] {
            let mut content = Vec::new();
            if transaction {
                content.extend(resp(&["MULTI"]));
                content.extend(resp(&["SET", "before", "1"]));
            }
            content.extend(resp(&arguments));
            content.extend(resp(&["SET", "after", "2"]));
            // In particular, EXEC extra must be refused while scanning,
            // rather than mistaken for an unfinished block and truncated.
            let fixture = fixture_directory();
            let path = aof_incremental_fixture(fixture.path(), NAME);
            std::fs::write(&path, &content).unwrap();
            let lease = support::lease_port();
            let output = replay_child(&program, None, fixture.path(), &lease)
                .wait_with_output().expect("wait for unsupported client-state refusal");
            assert_eq!(output.status.code(), Some(1), "unsupported client-state startup: {output:?}");
            assert_log_line(&output, &format!("Firn cannot load {} reading the append only file {NAME}.1.incr.aof: command requires unsupported replay state", arguments[0]));
            assert_eq!(std::fs::read(&path).unwrap(), content, "unsupported client state preserves the AOF");
        }
    }
}

#[test]
fn persistent_function_load_refuses_firn_but_redis_restores_the_function() {
    // FUNCTION LOAD changes persistent state outside Firn's keyspace. Redis
    // restores the library and can call it; a successful Firn startup would
    // silently discard that state even if no following record invokes it.
    let source = "#!lua name=imported\nredis.register_function('loaded', function() return 1 end)";
    let content = [resp(&["FUNCTION", "LOAD", source]),
        resp(&["SET", "after", "2"])].concat();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, &content).unwrap();
        let lease = support::lease_port();
        let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
            fixture.path(), &lease);
        if oracle {
            let mut client = connect_when_ready(lease.port(), &child);
            assert_eq!(memory_request(&mut client, &["FCALL", "loaded", "0"]), ":1\r\n");
            assert_eq!(memory_request(&mut client, &["GET", "after"]), "$1\r\n2\r\n");
            assert_eq!(std::fs::read(&path).unwrap(), content, "Redis restores the persistent function intact");
            client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(0), "Redis supports persistent functions: {output:?}");
        } else {
            let output = child.wait_with_output().expect("wait for unsupported persistent function refusal");
            assert_eq!(output.status.code(), Some(1), "Firn refuses persistent functions: {output:?}");
            assert_log_line(&output, &format!("Firn cannot load FUNCTION reading the append only file {NAME}.1.incr.aof: command requires unsupported replay state"));
            assert_eq!(std::fs::read(&path).unwrap(), content, "persistent function refusal preserves the AOF");
        }
    }
}

#[test]
fn known_execution_errors_are_logged_and_replay_continues_including_exec() {
    // Redis 7.2.16 src/aof.c:1550-1563 directly calls known commands.
    // src/networking.c:561-586 logs errors and continues under the default
    // propagation-error-behavior. EXEC retains its lastcmd while executing
    // INCR, so Redis's critical log names 'exec' for that fixture.
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for transaction in [false, true] {
        let mut content = resp(&["SET", "k", "x"]);
        if transaction { content.extend(resp(&["MULTI"])); }
        content.extend(resp(&["INCR", "k"]));
        content.extend(resp(&["SET", "inside", "yes"]));
        if transaction { content.extend(resp(&["EXEC"])); }
        content.extend(resp(&["SET", "after", "yes"]));
        for oracle in [false, true] {
            if oracle && binary.is_none() { continue; }
            let fixture = fixture_directory();
            let path = aof_incremental_fixture(fixture.path(), NAME);
            std::fs::write(&path, &content).unwrap();
            let lease = support::lease_port();
            let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
                fixture.path(), &lease);
            let mut client = connect_when_ready(lease.port(), &child);
            assert_eq!(memory_request(&mut client, &["GET", "k"]), "$1\r\nx\r\n");
            assert_eq!(memory_request(&mut client, &["GET", "inside"]), "$3\r\nyes\r\n");
            assert_eq!(memory_request(&mut client, &["GET", "after"]), "$3\r\nyes\r\n");
            assert_eq!(std::fs::read(&path).unwrap(), content, "execution errors do not discard AOF records");
            client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
            let output = child.wait_with_output().unwrap();
            assert_eq!(output.status.code(), Some(0), "execution-error continuation: {output:?}");
            if oracle {
                let command = if transaction { "exec" } else { "incr" };
                assert_log_line(&output, &format!("== CRITICAL == This server is sending an error to its AOF-loading-client: 'value is not an integer or out of range' after processing the command '{command}'"));
            } else {
                assert_log_line(&output, &format!("Firn AOF replay error reading the append only file {NAME}.1.incr.aof: -ERR value is not an integer or out of range"));
            }
        }
    }
}

#[test]
fn exec_without_multi_logs_an_error_and_keeps_loading() {
    // Redis 7.2.16 src/multi.c:154-157 returns an execution error, rather
    // than a fatal AOF load error, when EXEC appears without MULTI.
    let content = [resp(&["SET", "before", "1"]), resp(&["EXEC"]),
        resp(&["SET", "after", "2"])].concat();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, &content).unwrap();
        let lease = support::lease_port();
        let child = replay_child(&program, if oracle { binary.as_deref() } else { None },
            fixture.path(), &lease);
        let mut client = connect_when_ready(lease.port(), &child);
        assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "after"]), "$1\r\n2\r\n");
        assert_eq!(std::fs::read(&path).unwrap(), content, "unmatched EXEC does not truncate");
        client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0), "unmatched EXEC continuation: {output:?}");
        assert_log_line(&output, &if oracle {
            "== CRITICAL == This server is sending an error to its AOF-loading-client: 'EXEC without MULTI' after processing the command 'exec'".to_string()
        } else {
            format!("Firn AOF replay error reading the append only file {NAME}.1.incr.aof: -ERR EXEC without MULTI")
        });
    }
}

fn refuse_fixture(content: &[u8]) {
    let program = CompiledProgram::from_environment();
    let fixture = fixture_directory();
    let path = aof_incremental_fixture(fixture.path(), NAME);
    std::fs::write(&path, content).unwrap();
    let lease = support::lease_port();
    let text = lease.port().to_string();
    let output = program.spawn_in_directory(
        fixture.path(), &[text.as_bytes(), b"1", NAME.as_bytes()],
    ).with_port(&lease).wait_with_output().expect("wait for rejected Firn AOF");
    assert_bad_format(&output);
    assert_eq!(std::fs::read(&path).unwrap(), content, "format errors never truncate");

    if let Some(binary) = redis() {
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, content).unwrap();
        let lease = support::lease_port();
        let output = start_redis(&binary, fixture.path(), &lease)
            .wait_with_output().expect("wait for rejected Redis AOF");
        assert_bad_format(&output);
        assert_eq!(std::fs::read(&path).unwrap(), content, "Redis preserves the bad file");
    }
}

#[test]
fn inline_commands_are_bad_aof_format_with_redis_status_and_diagnostic() {
    // Each is valid network inline syntax; startup must reject it instead of
    // executing it, including an inline command after a valid RESP command.
    for inline in [b"SET inline accepted\r\n".as_slice(), b"PING\n", b"\r\n"] {
        let content = [resp(&["SET", "before", "1"]), inline.to_vec(),
            resp(&["SET", "after", "2"])].concat();
        refuse_fixture(&content);
    }
}

#[test]
fn zero_argument_record_is_bad_format_without_tail_recovery() {
    // Redis rejects argc < 1 rather than skipping the record or recovering
    // it as a short read. The network parser previously skipped *0.
    let content = [resp(&["SET", "before", "1"]), b"*0\r\n".to_vec(),
        resp(&["SET", "after", "2"])].concat();
    refuse_fixture(&content);
}

#[test]
fn nul_array_header_is_a_read_error_but_bare_marker_at_eof_recovers() {
    // Redis's buf[1] == '\0' branch goes to readerr. An embedded NUL in a
    // line read before EOF is fatal; reading a bare '*' at EOF sets feof and
    // permits truncation. Both servers use status 1 for every fatal AOF
    // load failure, including this read error.
    let kept = resp(&["SET", "before", "1"]);
    let corrupt = [kept.clone(), b"*\0\n".to_vec(), resp(&["SET", "after", "2"])].concat();
    let partial = [kept.clone(), b"*".to_vec()].concat();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        let lease = support::lease_port();
        let text = lease.port().to_string();
        let start = || {
            if oracle {
                start_redis(binary.as_ref().unwrap(), fixture.path(), &lease)
            } else {
                program.spawn_in_directory(fixture.path(), &[text.as_bytes(), b"0", NAME.as_bytes()])
                    .with_port(&lease)
            }
        };
        std::fs::write(&path, &corrupt).unwrap();
        let output = start().wait_with_output().expect("wait for embedded-NUL read error");
        assert_eq!(output.status.code(), Some(1), "read-error startup: {output:?}");
        assert_eq!(std::fs::read(&path).unwrap(), corrupt, "a read error before EOF never truncates");

        std::fs::write(&path, &partial).unwrap();
        let child = start();
        let mut client = connect_when_ready(lease.port(), &child);
        assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "after"]), "$-1\r\n");
        assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":1\r\n");
        assert_eq!(std::fs::read(&path).unwrap(), kept, "bare marker at EOF cuts only the partial record");
        client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "bare-marker recovery: {output:?}");
    }
}

fn loaded_checks(child: ProgramChild, port: u16) {
    let mut client = connect_when_ready(port, &child);
    assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
    assert_eq!(memory_request(&mut client, &["GET", "after"]), "$1\r\n2\r\n");
    assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":2\r\n");
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    let output = child.wait_with_output().expect("stop loaded AOF server");
    assert!(output.status.success(), "loaded AOF server: {output:?}");
}

#[test]
fn replay_uses_decimal_prefix_headers_and_unchecked_bulk_trailers() {
    // loadSingleAppendOnlyFile uses atoi for argc and strtol for lengths:
    // whitespace, a plus sign, leading zeroes and trailing text are accepted.
    // fgets accepts LF alone, and fread discards two trailer bytes unchecked.
    // The network parser rejects these headers, so it cannot stand in for
    // the file loader. Both commands still mean SET before 1 / SET after 2.
    let content = concat!(
        "* \t+3ignored\n$ \t+3trailing\nSETxy$0006ignored\r\nbefore!?$+1trailing\n1--",
        "*03suffix\r\n$03suffix\r\nSETzz$ +5suffix\nafter??$01suffix\n2!!",
    ).as_bytes();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, content).unwrap();
        let lease = support::lease_port();
        let text = lease.port().to_string();
        let child = if oracle {
            start_redis(binary.as_ref().unwrap(), fixture.path(), &lease)
        } else {
            program.spawn_in_directory(fixture.path(), &[text.as_bytes(), b"0", NAME.as_bytes()])
                .with_port(&lease)
        };
        loaded_checks(child, lease.port());
        assert_eq!(std::fs::read(&path).unwrap(), content, "accepted noncanonical records stay intact");
    }
}

#[test]
fn imported_annotations_and_noncanonical_headers_survive_rewrite_and_restart() {
    // The base is canonical; the active incremental file imports Redis's
    // annotation and numeric-prefix forms. A network inline tokenizer rejects
    // the unbalanced quote even though Redis skips the entire annotation.
    // A boundary scan using the strict network RESP parser rejects the next
    // record's noncanonical argc/bulk headers. Both must work after startup.
    let program = CompiledProgram::from_environment();
    multipart_files(
        &program,
        &[("appendonly.aof.1.base.aof", &[&["SET", "before", "1"]]),
            ("appendonly.aof.1.incr.aof", &[])],
        &rewrite_manifest(1, 1),
    );
    let directory = program.working_directory().join("appendonlydir");
    std::fs::write(
        directory.join("appendonly.aof.1.incr.aof"),
        concat!("# \"unterminated\n", "* +3ignored\n$03suffix\nSETxy$ +5suffix\nafter??$01suffix\n2!!"),
    ).unwrap();
    let lease = support::lease_port();
    let text = lease.port().to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", b"appendonly.aof"])
        .with_port(&lease);
    let mut client = connect_when_ready(lease.port(), &child);
    multipart_values(&mut client, &[("before", Some("1")), ("after", Some("2"))]);
    rewrite_start(&mut client);
    let info = rewrite_wait(&mut client);
    assert_eq!(info["aof_last_bgrewrite_status"], "ok", "imported AOF rewrite must succeed");
    assert_eq!(info["aof_rewrites"], "1");
    assert_eq!(std::fs::read_to_string(directory.join("appendonly.aof.manifest")).unwrap(),
        rewrite_manifest(2, 2), "the completed rewrite publishes a replacement base and increment");
    rewrite_stop(&mut client, child);

    let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", b"appendonly.aof"])
        .with_port(&lease);
    let mut client = connect_when_ready(lease.port(), &child);
    multipart_values(&mut client, &[("before", Some("1")), ("after", Some("2"))]);
    assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":2\r\n");
    rewrite_stop(&mut client, child);
}

fn boundary_fixture(program: &CompiledProgram, listed: bool, contents: &[u8]) -> std::path::PathBuf {
    let directory = program.working_directory().join("appendonlydir");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join(format!("{NAME}.1.base.aof")), resp(&["SET", "before", "1"])).unwrap();
    let increment = directory.join(format!("{NAME}.1.incr.aof"));
    std::fs::write(&increment, contents).unwrap();
    let mut manifest = format!("file {NAME}.1.base.aof seq 1 type b\n");
    if listed { manifest.push_str(&format!("file {NAME}.1.incr.aof seq 1 type i\n")); }
    std::fs::write(directory.join(format!("{NAME}.manifest")), manifest).unwrap();
    increment
}

#[test]
fn retained_open_blocks_advance_on_appends_without_reopening_history() {
    use std::os::unix::fs::PermissionsExt;
    let prefix = [b"# \"ignored annotation\n".to_vec(), resp(&["SET", "before", "1"])].concat();
    let retained = [prefix.clone(), b"# annotation before first MULTI\n".to_vec(),
        b"* +1suffix\n$05suffix\nMULTI!!".to_vec(),
        resp(&["SET", "ghost", "never-applied"])].concat();
    let endings = [
        // EOF inside a command retains the previous complete command and its
        // open MULTI; clean nested EOF cuts only the latest MULTI.
        b"*3\r\n$3\r\nSET\r\n$7\r\npartial".to_vec(),
        [b"# annotation before nested MULTI\n".to_vec(),
            b"*01suffix\n$ +5suffix\nMULTI!!".to_vec(), resp(&["SET", "nested", "discarded"])].concat(),
    ];
    let payload = "*1\r\n$5\r\nMULTI\r\n# \"unterminated\n*0\r\n\0";
    for ending in endings {
        for close_block in [false, true] {
            let program = CompiledProgram::from_environment();
            let content = [retained.clone(), ending.clone()].concat();
            let path = boundary_fixture(&program, true, &content);
            let lease = support::lease_port();
            let text = lease.port().to_string();
            let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", NAME.as_bytes()]).with_port(&lease);
            let mut client = connect_when_ready(lease.port(), &child);
            multipart_values(&mut client, &[("before", Some("1")), ("ghost", None), ("nested", None)]);
            assert_eq!(std::fs::read(&path).unwrap(), retained, "startup retains the first unfinished block after either cut");
            let mut sealed = std::fs::File::open(&path).unwrap();
            // Changing permissions preserves authoritative bytes and the
            // writer's open descriptor. A fresh historical read must fail.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0)).unwrap();
            assert_eq!(std::fs::File::open(&path).unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
            let mut appended = Vec::new();
            if close_block { appended.extend(resp(&["MULTI"])); }
            appended.extend(resp(&["SET", "after", "2"]));
            appended.extend(resp(&["SET", "payload", payload]));
            if close_block { appended.extend(resp(&["EXEC"])); }
            client.write_all(&appended).unwrap();
            let replies: &[u8] = if close_block {
                b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n+OK\r\n+OK\r\n"
            } else { b"+OK\r\n+OK\r\n" };
            expect_replies(&mut client, replies, "ordinary appends update retained AOF state and skip syntax-looking bulk bytes");
            rewrite_start(&mut client);
            assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok", "known boundary needs no historical reopen");
            let mut sealed_bytes = Vec::new();
            sealed.read_to_end(&mut sealed_bytes).unwrap();
            let expected = if close_block { [retained.clone(), appended].concat() } else { prefix.clone() };
            assert_eq!(sealed_bytes, expected, "an unmatched block cuts through the preceding whole command, excluding its leading annotation; appended EXEC closes it");
            rewrite_stop(&mut client, child);

            let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", NAME.as_bytes()]).with_port(&lease);
            let mut client = connect_when_ready(lease.port(), &child);
            multipart_values(&mut client, &[("before", Some("1")), ("after", Some("2")),
                ("payload", Some(payload)), ("ghost", None), ("nested", None)]);
            assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":3\r\n");
            rewrite_stop(&mut client, child);
        }
    }
}

#[test]
fn an_unlisted_existing_increment_uses_aof_grammar_for_fallback() {
    // Startup loads only the canonical base. Its preexisting, unlisted
    // increment is opened for appends with unknown syntax state, so switching
    // must actually read it. This SET is idempotent with the base's value.
    let content = b"# \"unterminated\n* +3suffix\n$03suffix\nSET!!$ +6suffix\nbefore??$01suffix\n1xy";
    let program = CompiledProgram::from_environment();
    let path = boundary_fixture(&program, false, content);
    let lease = support::lease_port();
    let text = lease.port().to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", NAME.as_bytes()]).with_port(&lease);
    let mut client = connect_when_ready(lease.port(), &child);
    multipart_values(&mut client, &[("before", Some("1")), ("after", None)]);
    assert_eq!(std::fs::read(&path).unwrap(), content);
    assert_eq!(memory_request(&mut client, &["SET", "after", "2"]), "+OK\r\n");
    rewrite_start(&mut client);
    assert_eq!(rewrite_wait(&mut client)["aof_last_bgrewrite_status"], "ok", "fallback accepts Redis annotations and decimal-prefix headers");
    rewrite_stop(&mut client, child);
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", NAME.as_bytes()]).with_port(&lease);
    loaded_checks(child, lease.port());
}

#[test]
fn appending_after_an_eof_comment_invalidates_cached_and_unknown_boundaries() {
    let content = [resp(&["SET", "before", "1"]), b"# annotation without LF".to_vec()].concat();
    for listed in [true, false] {
        let program = CompiledProgram::from_environment();
        let path = boundary_fixture(&program, listed, &content);
        let lease = support::lease_port();
        let text = lease.port().to_string();
        let child = program.spawn_on_route(true, &[text.as_bytes(), b"0", NAME.as_bytes()]).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        multipart_values(&mut client, &[("before", Some("1")), ("after", None)]);
        assert_eq!(memory_request(&mut client, &["SET", "after", "2"]), "+OK\r\n");
        // At the next load, fgets consumes '*3\r\n' as the remainder of the
        // annotation, so the following '$3' starts a bad-format record.
        // Resetting the cached append parser to a fresh record misses this.
        rewrite_start(&mut client);
        let info = rewrite_wait(&mut client);
        assert_eq!(info["aof_last_bgrewrite_status"], "err");
        assert_eq!(info["firn_aof_rewrite_abort_reason"], "boundary");
        let expected = [content.clone(), resp(&["SET", "after", "2"])].concat();
        assert_eq!(std::fs::read(&path).unwrap(), expected, "failed validation neither truncates nor seals the bad increment");
        rewrite_stop(&mut client, child);
        let output = program.spawn_on_route(true, &[text.as_bytes(), b"1", NAME.as_bytes()])
            .with_port(&lease).wait_with_output().unwrap();
        assert_bad_format(&output);
        assert_eq!(std::fs::read(&path).unwrap(), expected, "restart refuses the same actual format fault");
    }
}

#[test]
fn comments_follow_redis_fgets_chunks_and_preserve_loaded_file() {
    let before = resp(&["SET", "before", "1"]);
    let after = resp(&["SET", "after", "2"]);
    let cases = [
        // Both LF and CRLF terminate fgets, and an EOF comment needs no LF.
        [b"#TS:1700000000\n# arbitrary comment\r\n".to_vec(), before.clone(),
            b"# between commands\n".to_vec(), after.clone(), b"# final comment".to_vec()].concat(),
        // The LF is byte 1023: the complete comment fits in one fgets call.
        [before.clone(), vec![b'#'; 1022], b"\n".to_vec(), after.clone()].concat(),
        // fgets skips precisely 1023 bytes, even without LF. The following
        // RESP command is therefore a separate record although it shares
        // the physical line with the long comment prefix.
        [before.clone(), vec![b'#'; 1023], after.clone()].concat(),
        // Another comment chunk is allowed when its first byte is '#'.
        [before.clone(), vec![b'#'; 2046], after.clone()].concat(),
        // Keep the fgets chunk offset while crossing Firn's read window.
        [before, vec![b'#'; 65 * 1023], after].concat(),
    ];
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for content in cases {
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, &content).unwrap();
        let lease = support::lease_port();
        let text = lease.port().to_string();
        let child = program.spawn_in_directory(
            fixture.path(), &[text.as_bytes(), b"0", NAME.as_bytes()],
        ).with_port(&lease);
        loaded_checks(child, lease.port());
        assert_eq!(std::fs::read(&path).unwrap(), content, "comments are not truncated");

        if let Some(binary) = &binary {
            let fixture = fixture_directory();
            let path = aof_incremental_fixture(fixture.path(), NAME);
            std::fs::write(&path, &content).unwrap();
            let lease = support::lease_port();
            loaded_checks(start_redis(binary, fixture.path(), &lease), lease.port());
            assert_eq!(std::fs::read(&path).unwrap(), content, "Redis preserves comments");
        }
    }
}

#[test]
fn long_comment_remainders_are_records_not_unconditionally_skipped() {
    // A 1023-byte prefix fills fgets without consuming the LF. The next
    // chunk starts with LF or ordinary text, hence Redis reports bad format.
    for remainder in [b"\n".as_slice(), b"ordinary text\n"] {
        let content = [resp(&["SET", "before", "1"]), vec![b'#'; 1023],
            remainder.to_vec(), resp(&["SET", "after", "2"])].concat();
        refuse_fixture(&content);
    }
}

#[test]
fn comments_do_not_advance_the_truncated_tail_recovery_offset() {
    let kept = [b"#TS:1700000000\n".to_vec(), resp(&["SET", "before", "1"])].concat();
    let content = [kept.clone(), b"# comment after last command\n".to_vec(),
        b"*3\r\n$3\r\nSET\r\n$5\r\nafter\r\n$8\r\npartial".to_vec()].concat();
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [false, true] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        std::fs::write(&path, &content).unwrap();
        let lease = support::lease_port();
        let text = lease.port().to_string();
        let child = if oracle {
            start_redis(binary.as_ref().unwrap(), fixture.path(), &lease)
        } else {
            program.spawn_in_directory(fixture.path(), &[text.as_bytes(), b"0", NAME.as_bytes()])
                .with_port(&lease)
        };
        let mut client = connect_when_ready(lease.port(), &child);
        assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "after"]), "$-1\r\n");
        assert_eq!(std::fs::read(&path).unwrap(), kept, "only whole commands advance the cut offset");
        client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "truncated-tail startup: {output:?}");
    }
}

const LARGE_VALUE_BYTES: u64 = 500 * 1024 * 1024;

/// A real >2 GiB record without a matching allocation in the test process.
/// Sparse holes are zero-valued payload bytes, with distinct endpoints that
/// prove argument positions and order. Each bulk stays below 512 MiB.
fn large_record(path: &Path) -> u64 {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(&resp(&["SET", "before", "1"])).unwrap();
    file.write_all(b"*7\r\n$5\r\nRPUSH\r\n$5\r\nlarge\r\n").unwrap();
    for index in 0..5_u8 {
        write!(file, "${LARGE_VALUE_BYTES}\r\n").unwrap();
        file.write_all(&[b'A' + index]).unwrap();
        file.seek(SeekFrom::Current((LARGE_VALUE_BYTES - 2) as i64)).unwrap();
        file.write_all(&[b'a' + index]).unwrap();
        file.write_all(b"\r\n").unwrap();
    }
    file.write_all(&resp(&["SET", "after", "2"])).unwrap();
    let size = file.stream_position().unwrap();
    assert!(5 * LARGE_VALUE_BYTES > 2 * 1024 * 1024 * 1024);
    assert_eq!(file.metadata().unwrap().len(), size);
    size
}

fn large_loaded_checks(child: ProgramChild, port: u16) {
    // This case is deliberately run alone on a CI runner with at least 12 GiB free.
    // Its longer process/readiness bounds are harness limits, not timings.
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut client = loop {
        child.assert_running(port);
        match TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(100)) {
            Ok(stream) => break bounded_stream(stream),
            Err(error) => {
                assert!(Instant::now() < deadline, "large AOF startup never listened: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(memory_request(&mut client, &["GET", "before"]), "$1\r\n1\r\n");
    assert_eq!(memory_request(&mut client, &["GET", "after"]), "$1\r\n2\r\n");
    assert_eq!(memory_request(&mut client, &["LLEN", "large"]), ":5\r\n");
    let mut buffer = [0_u8; 64 * 1024];
    for index in 0..5_u8 {
        client.write_all(&resp(&["LPOP", "large"])).unwrap();
        assert_eq!(reply_line(&mut client, "large element header"), format!("${LARGE_VALUE_BYTES}\r\n"));
        let mut endpoint = [0_u8; 1];
        client.read_exact(&mut endpoint).unwrap();
        assert_eq!(endpoint, [b'A' + index], "argument start/order");
        let mut remaining = LARGE_VALUE_BYTES - 2;
        while remaining != 0 {
            let length = remaining.min(buffer.len() as u64) as usize;
            client.read_exact(&mut buffer[..length]).unwrap();
            assert!(buffer[..length].iter().all(|&byte| byte == 0), "argument body differs");
            remaining -= length as u64;
        }
        client.read_exact(&mut endpoint).unwrap();
        assert_eq!(endpoint, [b'a' + index], "argument end/order");
        let mut tail = [0_u8; 2];
        client.read_exact(&mut tail).unwrap();
        assert_eq!(&tail, b"\r\n");
    }
    assert_eq!(memory_request(&mut client, &["LLEN", "large"]), ":0\r\n");
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "large AOF replay: {output:?}");
}

#[test]
#[ignore = "requires at least 12 GiB free on a CI runner; a 2500 MiB record and payload/value/reply copies exceed hosted-runner memory beside the parallel suites"]
fn a_single_record_over_two_gib_loads_all_five_large_arguments() {
    // Run outside the gate, alone on a CI runner with at least 12 GiB free:
    // FIRN=<abs path to build/firn-test> cargo test --manifest-path tests/Cargo.toml --locked --test network aof_load::a_single_record_over_two_gib_loads_all_five_large_arguments -- --ignored --exact
    // A standard ubuntu-24.04 hosted runner with 15 GiB passed this branch's
    // case alone in 16.7 s: https://github.com/Ming-Research/Firn-wf/actions/runs/38437869851
    // Against main's firn it failed with exit status 4 before listening:
    // https://github.com/Ming-Research/Firn-wf/actions/runs/38438864780
    // The old replay grows a single input window only up to 2 GiB, then
    // refuses this valid RPUSH before applying it. The stored list permits
    // these five 500 MiB values; no argument crosses the 512 MiB bulk limit.
    let program = CompiledProgram::from_environment();
    let binary = redis();
    for oracle in [true, false] {
        if oracle && binary.is_none() { continue; }
        let fixture = fixture_directory();
        let path = aof_incremental_fixture(fixture.path(), NAME);
        let size = large_record(&path);
        let lease = support::lease_port();
        let limit = Duration::from_secs(600);
        let child = if oracle {
            start_redis_with_limit(binary.as_ref().unwrap(), fixture.path(), &lease, limit)
        } else {
            ProgramChild::spawn_with_limit(
                Command::new(std::env::var_os("FIRN").unwrap())
                    .current_dir(fixture.path())
                    .args([lease.port().to_string(), "0".to_string(), NAME.to_string()])
                    .env("WF_WORKERS", "1").env("WF_DRIVERS", "1")
                    .env_remove("WF_IO_NO_NATIVE_RING").env_remove("WF_REQUIRE_WINDOWS_IOCP")
                    .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()), limit,
            ).expect("spawn large AOF Firn").with_port(&lease)
        };
        large_loaded_checks(child, lease.port());
        assert!(std::fs::metadata(&path).unwrap().len() >= size, "large record must not be cut");
    }
    drop(program);
}
