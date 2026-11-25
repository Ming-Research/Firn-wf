//! The server log compared with the pinned Redis 7.2.16 oracle: the line form,
//! the events, `loglevel` and `logfile`.

use super::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn start(binary: &Path, directory: &Path, arguments: &[&str]) -> ProgramChild {
    let mut command = Command::new(binary);
    command.current_dir(directory).args(arguments.iter().map(OsString::from))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .env("TZ", "Asia/Kolkata").env_remove("WF_REQUIRE_WINDOWS_IOCP");
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
    let output = Command::new("date").env("TZ", "Asia/Kolkata").env("LC_ALL", "C").arg("+%d %b %Y %H:%M").output().expect("date");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn run_and_stop(binary: &Path, directory: &Path, extra: &[&str]) -> (String, String) {
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
    let _ready = connect_when_ready(lease.port(), &child);
    drop(_ready);
    let output = stop(lease.port(), child);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
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
