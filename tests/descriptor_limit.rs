//! The open-files limit at startup and on `CONFIG SET maxclients`, compared
//! with the pinned Redis 7.2.16 oracle. The hard limit is lowered in a shell
//! before the server starts, so each case holds whatever limits the host gives.

use super::*;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn servers() -> Vec<(&'static str, PathBuf)> {
    let mut servers = vec![("firn", PathBuf::from(std::env::var_os("FIRN").expect("FIRN")))];
    if let Some(binary) = std::env::var_os("REDIS_SERVER") {
        servers.push(("Redis 7.2.16", PathBuf::from(binary)));
    }
    servers
}

/// Starts the server after the shell has set the hard limit to `hard` and the soft limit to `soft`.
fn start_limited(binary: &PathBuf, directory: &std::path::Path, hard: u32, soft: u32, arguments: &[&str]) -> ProgramChild {
    let mut command = Command::new("/bin/sh");
    command.current_dir(directory)
        .args(["-c", "ulimit -Hn \"$1\" && ulimit -Sn \"$2\" && shift 2 && exec \"$@\"", "sh"])
        .arg(hard.to_string()).arg(soft.to_string()).arg(binary).args(arguments.iter().map(OsString::from))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .env_remove("WF_REQUIRE_WINDOWS_IOCP");
    ProgramChild::spawn(&mut command).expect("spawn limited subject")
}

/// The soft and hard limits on open files of a running process.
fn open_files(pid: u32) -> (u64, u64) {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).expect("read process limits");
    let line = limits.lines().find(|line| line.starts_with("Max open files")).expect("open-files row");
    let numbers: Vec<u64> = line.split_whitespace().filter_map(|word| word.parse().ok()).collect();
    (numbers[0], numbers[1])
}

fn log_bodies(log: &str) -> Vec<String> {
    log.lines().map(|line| line.splitn(6, ' ').nth(5).unwrap_or_default().get(2..).unwrap_or_default().to_owned()).collect()
}

fn stop(port: u16, child: ProgramChild) -> std::process::Output {
    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect to stop the server");
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).expect("stop the server");
    let mut byte = [0_u8; 1];
    assert_eq!(client.read(&mut byte).expect("shutdown closes the connection"), 0);
    child.wait_with_output().expect("bounded stop")
}

fn maxclients(client: &mut TcpStream) -> String {
    client.write_all(&resp(&["CONFIG", "GET", "maxclients"])).expect("query maxclients");
    let mut reply = Vec::new();
    let mut byte = [0_u8; 1];
    let mut lines = 0;
    while lines < 5 {
        client.read_exact(&mut byte).expect("read reply");
        reply.push(byte[0]);
        if reply.ends_with(b"\r\n") {
            lines += 1;
        }
    }
    String::from_utf8_lossy(&reply).lines().last().unwrap_or_default().to_owned()
}

#[test]
fn startup_raises_the_soft_limit_to_maxclients_plus_32_as_redis_does() {
    let fixture = fixture_directory();
    let mut logged = Vec::new();
    for (label, binary) in servers() {
        let lease = support::lease_port();
        let port = lease.port().to_string();
        let child = start_limited(&binary, fixture.path(), 8192, 1024,
            &["--bind", "127.0.0.1", "--port", &port, "--maxclients", "5000"]).with_port(&lease);
        let pid = child.id();
        let client = connect_when_ready(lease.port(), &child);
        assert_eq!(open_files(pid).0, 5032, "{label}: the soft limit after startup");
        drop(client);
        let output = stop(lease.port(), child);
        let bodies = log_bodies(&String::from_utf8_lossy(&output.stdout));
        let line = bodies.iter().find(|body| body.starts_with("Increased maximum number of open files"))
            .unwrap_or_else(|| panic!("{label}: no increase line in {bodies:?}")).clone();
        assert_eq!(line, "Increased maximum number of open files to 5032 (it was originally set to 1024).", "{label}");
        logged.push(line);
    }
    assert!(logged.windows(2).all(|pair| pair[0] == pair[1]), "{logged:?}");
}

#[test]
fn a_hard_limit_below_the_request_lowers_maxclients_as_redis_does() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let lease = support::lease_port();
        let port = lease.port().to_string();
        let child = start_limited(&binary, fixture.path(), 2048, 1024,
            &["--bind", "127.0.0.1", "--port", &port, "--maxclients", "5000"]).with_port(&lease);
        let pid = child.id();
        let mut client = connect_when_ready(lease.port(), &child);
        assert_eq!(open_files(pid).0, 2040, "{label}: the soft limit Redis's steps of 16 reach");
        assert_eq!(maxclients(&mut client), "2008", "{label}");
        drop(client);
        let output = stop(lease.port(), child);
        let bodies = log_bodies(&String::from_utf8_lossy(&output.stdout));
        let position = bodies.iter().position(|body| body.starts_with("You requested maxclients of"))
            .unwrap_or_else(|| panic!("{label}: no warning in {bodies:?}"));
        assert_eq!(bodies[position], "You requested maxclients of 5000 requiring at least 5032 max file descriptors.", "{label}");
        // The host's reason in the next line is the host's own text, which differs between the two.
        assert!(bodies[position + 1].starts_with("Server can't set maximum open files to 5032 because of OS error: "), "{label}");
        assert_eq!(bodies[position + 2],
            "Current maximum open files is 2040. maxclients has been reduced to 2008 to compensate for low ulimit. If you need higher maxclients increase 'ulimit -n'.",
            "{label}");
    }
}

#[test]
fn a_limit_of_32_stops_startup_with_status_1_and_redis_line() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let output = start_limited(&binary, fixture.path(), 32, 32, &["--bind", "127.0.0.1", "--maxclients", "100"])
            .wait_with_output().expect("bounded refusal");
        assert_eq!(output.status.code(), Some(1), "{label}: {output:?}");
        let bodies = log_bodies(&String::from_utf8_lossy(&output.stdout));
        assert!(bodies.iter().any(|body| body ==
            "Your current 'ulimit -n' of 32 is not enough for the server to start. Please increase your open file limit to at least 132. Exiting."),
            "{label}: {bodies:?}");
    }
}

#[test]
fn config_set_maxclients_raises_the_limit_or_answers_redis_error_and_keeps_the_old_value() {
    let fixture = fixture_directory();
    for (label, binary) in servers() {
        let lease = support::lease_port();
        let port = lease.port().to_string();
        let child = start_limited(&binary, fixture.path(), 3000, 1024,
            &["--bind", "127.0.0.1", "--port", &port, "--maxclients", "500"]).with_port(&lease);
        let pid = child.id();
        let mut client = connect_when_ready(lease.port(), &child);
        client.write_all(&resp(&["CONFIG", "SET", "maxclients", "5000"])).expect("set too many clients");
        expect_replies(&mut client,
            b"-ERR CONFIG SET failed (possibly related to argument 'maxclients') - The operating system is not able to handle the specified number of clients, try with 2968\r\n",
            &format!("{label}: refusal"));
        assert_eq!(maxclients(&mut client), "500", "{label}: the old value stays");
        client.write_all(&resp(&["CONFIG", "SET", "maxclients", "2000"])).expect("set a supported number");
        expect_replies(&mut client, b"+OK\r\n", &format!("{label}: success"));
        assert_eq!(maxclients(&mut client), "2000", "{label}");
        assert_eq!(open_files(pid).0, 3000, "{label}: the soft limit after the adjustments");
        drop(client);
        stop(lease.port(), child);
    }
}
