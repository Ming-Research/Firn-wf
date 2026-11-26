//! The server log compared with the pinned Redis 7.2.16 oracle: the line form,
//! the events, `loglevel` and `logfile`.

use super::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Whether a line has Redis's log form: `pid:role dd Mon yyyy hh:mm:ss.mmm mark message`.
pub fn is_log_line(line: &str) -> bool {
    let mut parts = line.splitn(6, ' ');
    let (Some(origin), Some(day), Some(month), Some(year), Some(clock), Some(rest)) =
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
        else { return false };
    let Some((pid, role)) = origin.split_once(':') else { return false };
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let digits = |text: &str, count: usize| text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit());
    let time = clock.as_bytes();
    let well_timed = time.len() == 12 && time[2] == b':' && time[5] == b':' && time[8] == b'.'
        && [0, 1, 3, 4, 6, 7, 9, 10, 11].into_iter().all(|at| time[at].is_ascii_digit());
    !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()) && matches!(role, "C" | "M")
        && digits(day, 2) && months.contains(&month) && digits(year, 4) && well_timed
        && rest.len() >= 3 && matches!(rest.as_bytes()[0], b'.' | b'-' | b'*' | b'#')
        && rest.as_bytes()[1] == b' '
}

/// The mark and message of every line of a log.
fn messages(log: &str) -> Vec<(char, String)> {
    log.lines().map(|line| {
        let rest = line.splitn(6, ' ').nth(5).unwrap_or_default();
        (rest.chars().next().unwrap_or('?'), rest.get(2..).unwrap_or_default().to_owned())
    }).collect()
}

fn servers() -> Vec<(&'static str, PathBuf)> {
    let mut servers = vec![("firn", PathBuf::from(std::env::var_os("FIRN").expect("FIRN")))];
    if std::env::var_os("REDIS_SERVER").is_some() {
        servers.push(("Redis 7.2.16", support::redis_oracle()));
    }
    servers
}

/// Starts the subject with two drivers: none of these cases depends on how many the host has, and a
/// server that starts one ring per core fails to start when the host has no rings left for it.
fn start(binary: &Path, directory: &Path, arguments: &[&str]) -> ProgramChild {
    let mut command = support::command(binary);
    command.current_dir(directory).args(arguments.iter().map(OsString::from))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .env("TZ", "Asia/Kolkata").env("WF_DRIVERS", "2").env_remove("WF_REQUIRE_WINDOWS_IOCP");
    ProgramChild::spawn(&mut command).expect("spawn log subject")
}

fn stop(port: u16, child: ProgramChild) -> std::process::Output {
    let mut client = bounded_stream(TcpStream::connect(("127.0.0.1", port)).expect("connect to stop the server"));
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("stop the server");
    let mut byte = [0_u8; 1];
    assert_eq!(client.read(&mut byte).expect("shutdown closes the connection"), 0);
    child.wait_with_output().expect("bounded stop")
}

/// The local minute as `dd Mon yyyy hh:mm` in the zone the subjects run in.
fn minute() -> String {
    let output = support::run_command(support::command("date").env("TZ", "Asia/Kolkata").env("LC_ALL", "C").arg("+%d %b %Y %H:%M"));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Runs one start of the subject, lets `during` use a connected client, stops it with SHUTDOWN NOSAVE and
/// returns the subject's process id with what it wrote to standard output and standard error.
fn run_logged(binary: &Path, directory: &Path, extra: &[&str], during: impl FnOnce(&mut TcpStream)) -> (u32, String, String) {
    let lease = support::lease_port();
    let port = lease.port().to_string();
    // A configuration file, when there is one, is the first argument.
    let (config, rest) = match extra.split_first() {
        Some((first, rest)) if !first.starts_with("--") => (Some(*first), rest),
        _ => (None, extra),
    };
    let mut arguments: Vec<&str> = config.into_iter().collect();
    arguments.extend(["--bind", "127.0.0.1", "--port", port.as_str()]);
    arguments.extend_from_slice(rest);
    let child = start(binary, directory, &arguments).with_port(&lease);
    let pid = child.id();
    let mut client = connect_when_ready(lease.port(), &child);
    during(&mut client);
    drop(client);
    let output = stop(lease.port(), child);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    (pid, String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

fn run_and_stop(binary: &Path, directory: &Path, extra: &[&str]) -> (String, String) {
    let (_, stdout, stderr) = run_logged(binary, directory, extra, |_| {});
    (stdout, stderr)
}

/// Every line of a log as (role, mark, body) with what differs between two runs or between the two
/// products made equal: the product name, the port, the load time and the rewrite child's process id.
/// Redis's signal-handler line has another form and is left out.
fn normalized(log: &str) -> Vec<(char, char, String)> {
    log.lines().filter(|line| !line.contains(":signal-handler (")).map(|line| {
        let role = line.split(' ').next().and_then(|origin| origin.split_once(':')).and_then(|(_, role)| role.chars().next())
            .unwrap_or('?');
        let rest = line.splitn(6, ' ').nth(5).unwrap_or_default();
        let mark = rest.chars().next().unwrap_or('?');
        let mut body = rest.get(2..).unwrap_or_default().replacen("firn is ", "Redis is ", 1);
        for prefix in ["Running mode=standalone, port=", "DB loaded from append only file: ", "Background append only file rewriting started"] {
            if body.starts_with(prefix) {
                body = format!("{prefix}...");
            }
        }
        (role, mark, body)
    }).collect()
}

/// Fails unless every line of `ours` is found in `reference`, in the same order.
fn assert_ordered_subsequence(ours: &[(char, char, String)], reference: &[(char, char, String)], log: &str, oracle_log: &str) {
    let mut at = 0;
    for line in ours {
        match reference[at..].iter().position(|candidate| candidate == line) {
            Some(found) => at += found + 1,
            None => panic!("{line:?} of firn's log is not in Redis's after position {at}\nfirn:\n{log}\nRedis:\n{oracle_log}"),
        }
    }
}

#[test]
fn log_lines_have_redis_form_local_time_and_the_events_redis_logs() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let directory = fixture.path().join(label.replace(' ', "-"));
        std::fs::create_dir(&directory).expect("create directory");
        std::fs::write(directory.join("seed.conf"), "appendonly yes\n").expect("write config");
        let before = minute();
        // A second start loads what the first wrote, so the load line appears.
        run_and_stop(&binary, &directory, &["seed.conf"]);
        let (stdout, stderr) = run_and_stop(&binary, &directory, &["seed.conf"]);
        let after = minute();
        assert!(stderr.is_empty(), "{label}: {stderr:?}");
        let mut pids = std::collections::HashSet::new();
        for line in stdout.lines() {
            assert!(is_log_line(line), "{label}: not a log line: {line:?}");
            pids.insert(line.split(':').next().expect("pid").to_owned());
            let stamp: Vec<&str> = line.splitn(6, ' ').collect();
            let minute_of_line = format!("{} {} {} {}", stamp[1], stamp[2], stamp[3], &stamp[4][..5]);
            assert!(minute_of_line == before || minute_of_line == after,
                "{label}: time {minute_of_line:?} is outside {before:?}..{after:?} in Asia/Kolkata: {line:?}");
        }
        assert_eq!(pids.len(), 1, "{label}: one process id carries every line: {pids:?}");
        let events = messages(&stdout);
        let has = |mark: char, body: &str| events.iter().any(|(m, text)| *m == mark && text.starts_with(body));
        let product = if label == "firn" { "firn" } else { "Redis" };
        assert!(has('*', &format!("oO0OoO0OoO0Oo {product} is starting")), "{label}: {stdout}");
        assert!(has('*', "Running mode=standalone, port="), "{label}: {stdout}");
        assert!(has('*', "Server initialized"), "{label}: {stdout}");
        assert!(has('*', "DB loaded from append only file: "), "{label}: {stdout}");
        assert!(has('*', "Ready to accept connections tcp"), "{label}: {stdout}");
        assert!(has('*', "User requested shutdown..."), "{label}: {stdout}");
        assert!(has('*', "Calling fsync() on the AOF file."), "{label}: {stdout}");
        assert!(has('#', &format!("{product} is now ready to exit, bye bye...")), "{label}: {stdout}");
        let (_, loaded) = events.iter().find(|(_, text)| text.starts_with("DB loaded from append only file: "))
            .expect("load line");
        let seconds = loaded.trim_start_matches("DB loaded from append only file: ").trim_end_matches(" seconds");
        assert!(loaded.ends_with(" seconds") && seconds.split_once('.').map(|(whole, fraction)|
            !whole.is_empty() && fraction.len() == 3).unwrap_or(false), "{label}: {loaded:?}");
    }
}

#[test]
fn loglevel_warning_keeps_only_warning_lines() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let (stdout, _) = run_and_stop(&binary, fixture.path(), &["--loglevel", "warning"]);
        let events = messages(&stdout);
        assert!(events.iter().all(|(mark, _)| *mark == '#'), "{label}: {stdout}");
        assert!(events.iter().any(|(_, body)| body.ends_with("is now ready to exit, bye bye...")), "{label}: {stdout}");
        let (silent, _) = run_and_stop(&binary, fixture.path(), &["--loglevel", "nothing"]);
        assert!(silent.is_empty(), "{label}: loglevel nothing writes no line: {silent:?}");
    }
}

#[test]
fn logfile_takes_the_lines_and_a_renamed_file_is_created_anew() {
    let fixture = fixture_directory();
    let root = fixture.path().canonicalize().expect("resolve fixture");
    for (label, binary) in servers() {
        let path = root.join(format!("{}.log", label.replace(' ', "-")));
        let lease = support::lease_port();
        let port = lease.port().to_string();
        let child = start(&binary, &root, &["--bind", "127.0.0.1", "--port", &port, "--logfile", path.to_str().expect("path")])
            .with_port(&lease);
        let ready = connect_when_ready(lease.port(), &child);
        drop(ready);
        let rotated = root.join(format!("{}.log.1", label.replace(' ', "-")));
        // The line is written by another thread, so wait for it before rotating.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !std::fs::read_to_string(&path).map(|text| text.contains("Ready to accept connections tcp")).unwrap_or(false) {
            assert!(std::time::Instant::now() < deadline, "{label}: the ready line never reached the log file");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::rename(&path, &rotated).expect("rotate the log by rename");
        let output = stop(lease.port(), child);
        assert!(output.stdout.is_empty(), "{label}: a logfile replaces standard output: {output:?}");
        let before = std::fs::read_to_string(&rotated).expect("rotated log");
        let after = std::fs::read_to_string(&path).expect("log created anew after the rename");
        assert!(before.lines().all(is_log_line) && before.contains("Ready to accept connections tcp"), "{label}: {before}");
        assert!(after.lines().all(is_log_line) && after.contains("bye bye..."), "{label}: {after}");
        assert!(!after.contains("Ready to accept connections tcp"), "{label}: lines went to the renamed file only until the rename");
    }
}

#[test]
fn logfile_written_before_dir_lands_in_the_final_directory() {
    let fixture = fixture_directory();
    let root = fixture.path().canonicalize().expect("resolve fixture");
    std::fs::create_dir(root.join("logs")).expect("create logs");
    for (label, binary) in servers() {
        let name = format!("{}.log", label.replace(' ', "-"));
        std::fs::write(root.join("startup.conf"), format!("logfile {name}\ndir {}\n", root.join("logs").display()))
            .expect("write config");
        run_and_stop(&binary, &root, &["startup.conf"]);
        assert!(root.join("logs").join(&name).is_file(), "{label}: the file belongs in the final dir");
        assert!(!root.join(&name).exists(), "{label}: nothing is written in the start directory");
    }
}

#[test]
fn an_unopenable_logfile_stops_startup_with_the_title_frame_alone() {
    let fixture = fixture_directory();
    let missing = fixture.path().join("absent-directory").join("x.log");
    for (label, binary) in servers() {
        let output = start(&binary, fixture.path(), &["--logfile", missing.to_str().expect("path")])
            .wait_with_output().expect("bounded refusal");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        assert_eq!(output.stderr,
            b"\n*** FATAL CONFIG FILE ERROR (Redis 7.2.16) ***\nCan't open the log file: No such file or directory\n",
            "{label}: {output:?}");
    }
}

#[test]
fn config_set_loglevel_changes_the_level_while_running_and_logfile_is_immutable() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let lease = support::lease_port();
        let port = lease.port().to_string();
        let child = start(&binary, fixture.path(), &["--bind", "127.0.0.1", "--port", &port]).with_port(&lease);
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["CONFIG", "SET", "loglevel", "loud"])).expect("set a bad level");
        expect_replies(&mut client,
            b"-ERR CONFIG SET failed (possibly related to argument 'loglevel') - argument(s) must be one of the following: debug, verbose, notice, warning, nothing\r\n",
            &format!("{label}: bad level"));
        client.write_all(&resp(&["CONFIG", "SET", "logfile", "x.log"])).expect("set logfile");
        expect_replies(&mut client,
            b"-ERR CONFIG SET failed (possibly related to argument 'logfile') - can't set immutable config\r\n",
            &format!("{label}: logfile"));
        client.write_all(&resp(&["CONFIG", "SET", "loglevel", "WARNING"])).expect("set the level");
        expect_replies(&mut client, b"+OK\r\n", &format!("{label}: set"));
        client.write_all(&resp(&["CONFIG", "GET", "loglevel"])).expect("read the level");
        expect_replies(&mut client, b"*2\r\n$8\r\nloglevel\r\n$7\r\nwarning\r\n", &format!("{label}: read back"));
        drop(client);
        let output = stop(lease.port(), child);
        let events = messages(&String::from_utf8_lossy(&output.stdout));
        assert!(events.iter().any(|(_, body)| body.contains("bye bye...")), "{label}: {output:?}");
        assert!(!events.iter().any(|(_, body)| body.starts_with("User requested shutdown")),
            "{label}: notice lines stop once the level is raised: {output:?}");
    }
}

/// A stop signal is logged and then followed by the lines Redis logs for a shutdown. Redis writes the
/// signal's own line from its signal handler, as `pid:signal-handler (unix time) message`, so only that
/// line's message is compared and the others must have the usual form.
#[cfg(target_os = "linux")]
#[test]
fn a_stop_signal_is_logged_and_followed_by_the_shutdown_lines() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        for (signal, name) in [("TERM", "SIGTERM"), ("INT", "SIGINT")] {
            let lease = support::lease_port();
            let port = lease.port().to_string();
            let child = start(&binary, fixture.path(), &["--bind", "127.0.0.1", "--port", &port]).with_port(&lease);
            let ready = connect_when_ready(lease.port(), &child);
            drop(ready);
            send_signal(&child, signal);
            let output = child.wait_with_output().expect("bounded stop");
            assert_eq!(output.status.code(), Some(0), "{label} {signal}: {output:?}");
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let received = format!("Received {name} scheduling shutdown...");
            assert!(stdout.lines().any(|line| line.ends_with(&received)), "{label} {signal}: {stdout}");
            let ordinary: Vec<&str> = stdout.lines().filter(|line| !line.contains(":signal-handler (")).collect();
            assert!(ordinary.iter().all(|line| is_log_line(line)), "{label} {signal}: {stdout}");
            let events = messages(&ordinary.join("\n"));
            let has = |mark: char, body: &str| events.iter().any(|(m, text)| *m == mark && text.starts_with(body));
            assert!(has('*', "User requested shutdown..."), "{label} {signal}: {stdout}");
            assert!(has('#', "firn is now ready to exit, bye bye...") || has('#', "Redis is now ready to exit, bye bye..."),
                "{label} {signal}: {stdout}");
        }
    }
}

/// Redis follows a symbolic link named as its log file; firn refuses the link, because the host opens a log
/// file's last name for appending without following links. The refusal is firn's documented difference, so
/// only firn is started here.
#[cfg(unix)]
#[test]
fn a_logfile_that_is_a_symbolic_link_is_refused_at_startup() {
    let fixture = fixture_directory();
    let target = fixture.path().join("target.log");
    std::fs::write(&target, "").expect("write target");
    let link = fixture.path().join("link.log");
    std::os::unix::fs::symlink(&target, &link).expect("create link");
    let binary = PathBuf::from(std::env::var_os("FIRN").expect("FIRN"));
    let output = start(&binary, fixture.path(), &["--logfile", link.to_str().expect("path")])
        .wait_with_output().expect("bounded refusal");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("\n*** FATAL CONFIG FILE ERROR (Redis 7.2.16) ***\nCan't open the log file: "), "{output:?}");
    assert_eq!(std::fs::read_to_string(&target).expect("read target"), "", "nothing is written through the link");
}

/// An append-only file whose last command is cut short is cut after its last whole command, and Redis logs
/// the short read, the truncation with its offset and that the file was loaded anyway, in that order, as
/// warnings. The file name is the one the manifest gives.
#[test]
fn a_cut_append_only_tail_is_logged_as_redis_logs_it() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let directory = fixture.path().join(label.replace(' ', "-"));
        let aof = directory.join("appendonlydir");
        std::fs::create_dir_all(&aof).expect("create the append-only directory");
        let whole = resp(&["SET", "a", "1"]);
        let partial = resp(&["SET", "b", "2"]);
        std::fs::write(aof.join("appendonly.aof.1.base.aof"), resp(&["SET", "base", "1"])).expect("write the base");
        std::fs::write(aof.join("appendonly.aof.1.incr.aof"), [whole.clone(), partial[..12].to_vec()].concat())
            .expect("write the increment");
        std::fs::write(aof.join("appendonly.aof.manifest"),
            "file appendonly.aof.1.base.aof seq 1 type b\nfile appendonly.aof.1.incr.aof seq 1 type i\n")
            .expect("write the manifest");
        let (stdout, _) = run_and_stop(&binary, &directory, &["--appendonly", "yes"]);
        let events = messages(&stdout);
        let name = "appendonly.aof.1.incr.aof";
        let expected = [
            format!("!!! Warning: short read while loading the AOF file {name}!!!"),
            format!("!!! Truncating the AOF {name} at offset {} !!!", whole.len()),
            format!("AOF {name} loaded anyway because aof-load-truncated is enabled"),
        ];
        let positions: Vec<usize> = expected.iter().map(|line| {
            events.iter().position(|(mark, text)| *mark == '#' && text == line)
                .unwrap_or_else(|| panic!("{label}: {line:?} is not a warning of {stdout}"))
        }).collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{label}: the lines are out of order in {stdout}");
        let kept = std::fs::read(aof.join(name)).expect("read the increment");
        assert_eq!(kept, whole, "{label}: the file is cut after its last whole command");
    }
}

/// Redis 7.2.16 logs a line's role as `C` until the server is initialized and `M` afterwards, and the
/// process id is the server's own. Each of firn's lines must be a line of Redis's log with the same role,
/// mark and message, in the same order, and carry the subject's process id.
#[test]
fn every_firn_log_line_is_a_redis_line_with_the_same_role_mark_order_and_the_childs_pid() {
    let fixture = fixture_directory();
    let mut logs: Vec<(String, Vec<(char, char, String)>, String)> = Vec::new();
    for (label, binary) in servers() {
        let directory = fixture.path().join(label.replace(' ', "-"));
        std::fs::create_dir(&directory).expect("create directory");
        std::fs::write(directory.join("seed.conf"), "appendonly yes\n").expect("write config");
        run_and_stop(&binary, &directory, &["seed.conf"]);
        let (pid, stdout, _) = run_logged(&binary, &directory, &["seed.conf"], |_| {});
        for line in stdout.lines() {
            assert_eq!(line.split(':').next(), Some(pid.to_string().as_str()), "{label}: the line carries the child's pid: {line:?}");
        }
        let lines = normalized(&stdout);
        let role_of = |body: &str| lines.iter().find(|(_, _, text)| text.starts_with(body)).map(|(role, _, _)| *role);
        assert_eq!(role_of("oO0OoO0OoO0Oo Redis is starting"), Some('C'), "{label}: {stdout}");
        assert_eq!(role_of("Configuration loaded"), Some('C'), "{label}: {stdout}");
        assert_eq!(role_of("Ready to accept connections tcp"), Some('M'), "{label}: {stdout}");
        logs.push((label.to_owned(), lines, stdout));
    }
    if let [(_, ours, log), (_, reference, oracle_log)] = &logs[..] {
        assert_ordered_subsequence(ours, reference, log, oracle_log);
    }
}

/// A rewrite logs its start (Redis adds `by pid N`, which firn has no process to name), the child's
/// success and the finished line, as Redis 7.2.16's rewrite handler writes them, in that order.
#[test]
fn a_rewrite_is_logged_as_redis_logs_it() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let directory = fixture.path().join(label.replace(' ', "-"));
        std::fs::create_dir(&directory).expect("create directory");
        let (_, stdout, _) = run_logged(&binary, &directory, &["--appendonly", "yes"], |client| {
            client.write_all(&resp(&["SET", "k", "v"])).expect("write");
            expect_replies(client, b"+OK\r\n", &format!("{label}: set"));
            client.write_all(&resp(&["BGREWRITEAOF"])).expect("rewrite");
            expect_replies(client, b"+Background append only file rewriting started\r\n", &format!("{label}: rewrite"));
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                client.write_all(&resp(&["INFO", "persistence"])).expect("query the rewrite");
                let info = bulk_reply(client, "persistence");
                if info_field(&info, "aof_rewrite_in_progress").as_deref() == Some("0") {
                    assert_eq!(info_field(&info, "aof_last_bgrewrite_status").as_deref(), Some("ok"), "{label}: {info:?}");
                    break;
                }
                assert!(Instant::now() < deadline, "{label}: the rewrite did not finish: {info:?}");
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let lines = normalized(&stdout);
        let wanted = [
            ('M', '*', "Background append only file rewriting started...".to_owned()),
            ('M', '*', "Background AOF rewrite terminated with success".to_owned()),
            ('M', '*', "Background AOF rewrite finished successfully".to_owned()),
        ];
        assert_ordered_subsequence(&wanted, &lines, "the three rewrite lines", &stdout);
    }
}

/// Redis 7.2.16 logs a configuration file it cannot open to the log file and level the directives before
/// it set; a level above warning writes nothing. Both servers write the line to the file, or nothing.
#[test]
fn the_failure_to_open_an_include_follows_the_logfile_and_loglevel_read_so_far() {
    let fixture = fixture_directory();
    let root = fixture.path().canonicalize().expect("resolve fixture");
    let missing = root.join("absent.conf");
    for (label, binary) in servers() {
        let log = root.join(format!("{}.log", label.replace(' ', "-")));
        std::fs::write(root.join("a.conf"), format!("logfile {}\ninclude {}\n", log.display(), missing.display())).expect("write");
        let output = start(&binary, &root, &["a.conf"]).wait_with_output().expect("bounded refusal");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{label}: the line goes to the log file: {output:?}");
        let text = std::fs::read_to_string(&log).unwrap_or_else(|error| panic!("{label}: no log file: {error}"));
        let expected = format!("Fatal error, can't open config file '{}': No such file or directory", missing.display());
        assert!(text.lines().any(|line| line.ends_with(&expected) && is_log_line(line)), "{label}: {text:?}");
        std::fs::write(root.join("b.conf"), format!("loglevel nothing\ninclude {}\n", missing.display())).expect("write");
        let output = start(&binary, &root, &["b.conf"]).wait_with_output().expect("bounded refusal");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty(), "{label}: loglevel nothing writes no line: {output:?}");
    }
}
