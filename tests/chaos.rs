//! Seeded, opt-in fault experiment; see research/investigations/chaos/README.md.
#![cfg(target_os = "linux")]
#![allow(dead_code)]
mod append_history;
mod chaos_check;
mod support;

use chaos_check::{Event, Findings, KEYS, Kind, LostWrite, Outcome, State};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

const IO_LIMIT: Duration = Duration::from_secs(3);
const MAX_FRAME: usize = 16 * 1024 * 1024;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn append_file(path: &Path) -> File {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open healthy artifact")
}
fn note(file: &Mutex<File>, origin: Instant, message: &str) {
    writeln!(locked(file), "{}\t{}", origin.elapsed().as_nanos(), message)
        .expect("write healthy artifact");
}
fn resp(args: &[&str]) -> Vec<u8> {
    let mut bytes = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        bytes.extend_from_slice(format!("${}\r\n{}\r\n", arg.len(), arg).as_bytes());
    }
    bytes
}
#[derive(Debug, PartialEq)]
enum Frame {
    Simple(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Frame>>),
}
// Record raw bytes even when a frame is truncated; cap sizes/depth before allocating.
fn frame(reader: &mut impl Read, raw: &mut Vec<u8>, depth: usize) -> io::Result<Frame> {
    if depth > 8 || raw.len() > MAX_FRAME {
        return Err(invalid("RESP bound exceeded"));
    }
    fn byte(reader: &mut impl Read, raw: &mut Vec<u8>) -> io::Result<u8> {
        let mut b = [0];
        reader.read_exact(&mut b)?;
        raw.push(b[0]);
        Ok(b[0])
    }
    let prefix = byte(reader, raw)?;
    let mut line = Vec::new();
    loop {
        let b = byte(reader, raw)?;
        if b == b'\r' {
            if byte(reader, raw)? != b'\n' {
                return Err(invalid("RESP missing LF"));
            }
            break;
        }
        if b == b'\n' || line.len() >= 4096 {
            return Err(invalid("RESP malformed line"));
        }
        line.push(b);
    }
    let text = String::from_utf8(line).map_err(|_| invalid("RESP non-UTF8 header"))?;
    let number = || {
        text.parse::<i64>()
            .map_err(|_| invalid("RESP invalid integer"))
    };
    match prefix {
        b'+' => Ok(Frame::Simple(text)),
        b'-' => Ok(Frame::Error(text)),
        b':' => Ok(Frame::Integer(number()?)),
        b'$' => {
            let n = number()?;
            if n == -1 {
                return Ok(Frame::Bulk(None));
            }
            if n < 0 || n as usize > MAX_FRAME - raw.len().min(MAX_FRAME) {
                return Err(invalid("RESP bulk bound"));
            }
            let mut bytes = vec![0; n as usize];
            // Reading in chunks retains partial bytes in the history on EOF.
            let mut used = 0;
            while used < bytes.len() {
                let count = reader.read(&mut bytes[used..])?;
                if count == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                raw.extend_from_slice(&bytes[used..used + count]);
                used += count;
            }
            if byte(reader, raw)? != b'\r' || byte(reader, raw)? != b'\n' {
                return Err(invalid("RESP bulk terminator"));
            }
            Ok(Frame::Bulk(Some(bytes)))
        }
        b'*' => {
            let n = number()?;
            if n == -1 {
                return Ok(Frame::Array(None));
            }
            if !(0..=32).contains(&n) {
                return Err(invalid("RESP array bound"));
            }
            let values = (0..n)
                .map(|_| frame(reader, raw, depth + 1))
                .collect::<io::Result<Vec<_>>>()?;
            Ok(Frame::Array(Some(values)))
        }
        _ => Err(invalid("RESP unknown prefix")),
    }
}
fn has_set(kind: Kind) -> bool {
    matches!(kind, Kind::Set | Kind::Both)
}
fn has_incr(kind: Kind) -> bool {
    matches!(kind, Kind::Incr | Kind::Both)
}
fn kind_code(kind: Kind) -> u8 {
    match kind {
        Kind::Append => 0,
        Kind::Set => 1,
        Kind::Incr => 2,
        Kind::Both => 3,
    }
}
fn kind_from(n: u8) -> Kind {
    match n {
        0 => Kind::Append,
        1 => Kind::Set,
        2 => Kind::Incr,
        3 => Kind::Both,
        _ => panic!("bad archived kind"),
    }
}
fn keys(epoch: usize, key: usize) -> [String; 3] {
    [
        format!("chaos:{epoch}:{key}:log"),
        format!("chaos:{epoch}:{key}:set"),
        format!("chaos:{epoch}:{key}:count"),
    ]
}
fn request(epoch: usize, key: usize, token: &str, kind: Kind) -> (Vec<u8>, usize) {
    let k = keys(epoch, key);
    let mut commands = vec![vec!["APPEND", k[0].as_str(), token]];
    if has_set(kind) {
        commands.push(vec!["SET", &k[1], token]);
    }
    if has_incr(kind) {
        commands.push(vec!["INCR", &k[2]]);
    }
    if matches!(kind, Kind::Append) {
        return (resp(&commands[0]), 1);
    }
    let mut bytes = resp(&["MULTI"]);
    for command in &commands {
        bytes.extend(resp(command));
    }
    bytes.extend(resp(&["EXEC"]));
    (bytes, commands.len() + 2)
}
fn refusal(f: &Frame) -> bool {
    matches!(f, Frame::Error(s) if s.starts_with("MISCONF ") || s.starts_with("EXECABORT "))
}
fn classify(kind: Kind, replies: &[Frame], disk_window: bool) -> Result<Outcome, String> {
    if matches!(kind, Kind::Append) {
        return match replies {
            [Frame::Integer(length)] if *length > 0 => Ok(Outcome::Applied {
                length: *length,
                counter: None,
            }),
            [f] if disk_window && refusal(f) => Ok(Outcome::Rejected),
            _ => Err(format!("invalid APPEND replies {replies:?}")),
        };
    }
    let count = 1 + usize::from(has_set(kind)) + usize::from(has_incr(kind));
    if replies.len() != count + 2 || replies[0] != Frame::Simple("OK".into()) {
        return Err(format!("invalid MULTI replies {replies:?}"));
    }
    if disk_window
        && refusal(replies.last().unwrap())
        && replies[1..=count]
            .iter()
            .all(|f| *f == Frame::Simple("QUEUED".into()) || refusal(f))
    {
        return Ok(Outcome::Rejected);
    }
    if replies[1..=count]
        .iter()
        .any(|f| *f != Frame::Simple("QUEUED".into()))
    {
        return Err(format!("invalid queued replies {replies:?}"));
    }
    let Frame::Array(Some(values)) = &replies[count + 1] else {
        return Err(format!("invalid EXEC reply {replies:?}"));
    };
    if values.len() != count {
        return Err(format!("wrong EXEC arity {values:?}"));
    }
    let Frame::Integer(length) = values[0] else {
        return Err(format!("invalid APPEND inside EXEC {values:?}"));
    };
    if length <= 0 {
        return Err("nonpositive APPEND length".into());
    }
    if has_set(kind) && values[1] != Frame::Simple("OK".into()) {
        return Err(format!("SET inside EXEC not OK {values:?}"));
    }
    let counter = if has_incr(kind) {
        match values.last().unwrap() {
            Frame::Integer(n) if *n > 0 => Some(*n),
            _ => return Err(format!("invalid INCR inside EXEC {values:?}")),
        }
    } else {
        None
    };
    Ok(Outcome::Applied { length, counter })
}
fn connect(port: u16) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), IO_LIMIT)?;
    stream.set_read_timeout(Some(IO_LIMIT))?;
    stream.set_write_timeout(Some(IO_LIMIT))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}
fn epoch_seconds(seed: &mut u64) -> u64 {
    15 + random(seed) % 11
}
fn random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut n = *seed;
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}

struct Config {
    server: String,
    seconds: u64,
    restart_seconds: u64,
    seed: u64,
    clients: usize,
    out: PathBuf,
    mount: Option<PathBuf>,
    fix: String,
    fault_skip_reason: String,
}
impl Config {
    fn read() -> Self {
        let number = |key: &str, default: u64| {
            std::env::var(key)
                .map(|v| v.parse::<u64>().expect(key))
                .unwrap_or(default)
        };
        let c = Self {
            server: std::env::var("CHAOS_SERVER").unwrap_or("firn".into()),
            seconds: number("CHAOS_SECONDS", 120),
            restart_seconds: number("CHAOS_RESTART_SECONDS", 300),
            seed: number("CHAOS_SEED", 1),
            clients: number("CHAOS_CLIENTS", 32) as usize,
            out: PathBuf::from(std::env::var_os("CHAOS_OUT").expect("CHAOS_OUT required")),
            mount: std::env::var_os("CHAOS_FAULT_MOUNT").map(PathBuf::from),
            fix: std::env::var("CHAOS_FIRN_APPEND_BEFORE_REPLY").unwrap_or("unknown".into()),
            fault_skip_reason: std::env::var("CHAOS_FAULT_SKIP_REASON")
                .unwrap_or("CHAOS_FAULT_MOUNT not provided".into()),
        };
        assert!(
            c.seconds > 0 && c.seconds <= 604800 && c.clients > 0 && c.clients <= 256,
            "invalid duration/client bounds"
        );
        assert!(
            (1..=3600).contains(&c.restart_seconds),
            "invalid restart budget"
        );
        assert!(
            matches!(c.fix.as_str(), "unknown" | "absent" | "present"),
            "invalid fix status"
        );
        c
    }
    fn binary(&self) -> Option<PathBuf> {
        let (env, default) = match self.server.as_str() {
            "firn" => ("FIRN", ""),
            "redis" => ("REDIS_SERVER", ""),
            "valkey" => ("VALKEY", "valkey-server"),
            "dragonfly" => ("DRAGONFLY", "dragonfly"),
            "garnet" => ("GARNET", "garnet-server"),
            _ => panic!("unknown CHAOS_SERVER"),
        };
        let name = PathBuf::from(std::env::var_os(env).unwrap_or_else(|| default.into()));
        if name.as_os_str().is_empty() {
            return None;
        }
        if self.server == "firn" || self.server == "redis" {
            assert!(name.is_absolute(), "{env} must be absolute");
        }
        if name.components().count() > 1 {
            return name.is_file().then(|| fs::canonicalize(name).unwrap());
        }
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(&name))
            .find(|p| p.is_file())
            .map(|p| fs::canonicalize(p).unwrap())
    }
}
struct Device<'a> {
    enabled: bool,
    mount: Option<PathBuf>,
    script: PathBuf,
    shared: &'a Shared,
}
impl Device<'_> {
    fn checked_action(&self, action: &str) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let output = support::output_within(
            Command::new("sudo").args(["-n", "cat", "/var/tmp/firn-sync-fault.state"]),
            Duration::from_secs(10),
        )?;
        if !output.status.success() {
            return Err(invalid(format!("read device ownership: {output:?}")));
        }
        let state =
            String::from_utf8(output.stdout).map_err(|_| invalid("device state not UTF8"))?;
        let owner = state
            .lines()
            .nth(5)
            .ok_or_else(|| invalid("device state missing owned mount"))?;
        let actual = fs::canonicalize(owner)?;
        let expected = fs::canonicalize(
            self.mount
                .as_ref()
                .ok_or_else(|| invalid("missing fault mount"))?,
        )?;
        if actual != expected {
            return Err(invalid("refuse to change a device owned by another mount"));
        }
        let output = support::output_within(
            Command::new("sudo").arg("-n").arg(&self.script).arg(action),
            Duration::from_secs(30),
        )?;
        if !output.status.success() {
            return Err(invalid(format!("device {action}: {output:?}")));
        }
        Ok(())
    }
    fn run(&self, action: &str) {
        self.checked_action(action)
            .unwrap_or_else(|error| panic!("device {action}: {error}"));
    }
}
impl Drop for Device<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.checked_action("heal") {
            self.shared.errors.fetch_add(1, Ordering::Relaxed);
            let message = format!("CLEANUP_FAIL device heal: {error}");
            let _ = writeln!(
                locked(&self.shared.findings),
                "{}\t{message}",
                self.shared.origin.elapsed().as_nanos()
            );
            let _ = writeln!(io::stderr(), "{message}");
        }
    }
}

#[derive(Clone, Debug)]
struct FaultRecord {
    kind: &'static str,
    epoch: usize,
    at_ns: u128,
}

#[derive(Debug)]
struct Coverage {
    epochs_started: usize,
    epochs_completed: usize,
    sigkill_sent: usize,
    sigkill_reaped: usize,
    restart_attempts: usize,
    sigkill_restarts_completed: usize,
    write_eio_injected_windows: usize,
    flush_eio_injected_windows: usize,
    enospc_injected_windows: usize,
    connection_drops: usize,
    ack_adjacent_sigkills_within_10ms: usize,
    skipped_faults: Vec<String>,
    faults: Vec<FaultRecord>,
}
impl Default for Coverage {
    fn default() -> Self {
        Self { epochs_started:0, epochs_completed:0, sigkill_sent:0, sigkill_reaped:0,
            restart_attempts:0, sigkill_restarts_completed:0, write_eio_injected_windows:0,
            flush_eio_injected_windows:0, enospc_injected_windows:0, connection_drops:0,
            ack_adjacent_sigkills_within_10ms:0,
            skipped_faults:vec!["ENOSPC not selected: this experiment uses the failing-disk alternative".into(),"flush-only failure not isolated: write EIO and flush EIO share one combined injection window".into()],
            faults:Vec::new() }
    }
}
impl Coverage {
    fn report(&self) -> String {
        self.report_with_skips(&[])
    }
    fn report_with_skips(&self, additional: &[String]) -> String {
        let mut skips = self.skipped_faults.clone();
        skips.extend_from_slice(additional);
        format!(
            "coverage.epochs_started={}\ncoverage.epochs_completed={}\ncoverage.sigkill_sent={}\ncoverage.sigkill_reaped={}\ncoverage.restart_attempts={}\ncoverage.sigkill_restarts_completed={}\ncoverage.write_eio_injected_windows={}\ncoverage.flush_eio_injected_windows={}\ncoverage.enospc_injected_windows={}\ncoverage.connection_drops={}\ncoverage.ack_adjacent_sigkills_within_10ms={}\ncoverage.skipped_faults={:?}\ncoverage.faults={:?}\ncoverage.eio_definition=injected windows armed for failing writes/flushes; not observed syscall failures\n",
            self.epochs_started,
            self.epochs_completed,
            self.sigkill_sent,
            self.sigkill_reaped,
            self.restart_attempts,
            self.sigkill_restarts_completed,
            self.write_eio_injected_windows,
            self.flush_eio_injected_windows,
            self.enospc_injected_windows,
            self.connection_drops,
            self.ack_adjacent_sigkills_within_10ms,
            skips,
            self.faults
        )
    }
    fn smoke_gaps(&self, seconds: u64, device: bool) -> Vec<&'static str> {
        let mut gaps = Vec::new();
        if self.connection_drops == 0 {
            gaps.push("no completed injected connection drop");
        }
        if seconds >= 120 {
            if self.sigkill_restarts_completed == 0 {
                gaps.push("no completed SIGKILL restart");
            }
            if device && self.write_eio_injected_windows == 0 {
                gaps.push("no write-EIO injection window");
            }
            if device && self.flush_eio_injected_windows == 0 {
                gaps.push("no flush-EIO injection window");
            }
        }
        gaps
    }
}

#[derive(Clone, Debug)]
struct StartupFailure {
    server: String,
    kind: &'static str,
    reason: String,
    lifetime: usize,
    preceding_faults: Vec<FaultRecord>,
    stdout_tail: String,
    stderr_tail: String,
}
fn file_tail(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(4096)))?;
    let mut bytes = Vec::new();
    file.take(4096).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
impl StartupFailure {
    fn capture(c: &Config, shared: &Shared, lifetime: usize, reason: String) -> Self {
        Self {
            server: c.server.clone(),
            kind: if lifetime == 0 {
                "failed startup"
            } else {
                "refused restart"
            },
            reason,
            lifetime,
            preceding_faults: locked(&shared.fault_coverage).faults.clone(),
            stdout_tail: file_tail(&c.out.join(format!("server-{lifetime}.stdout")))
                .expect("read startup stdout artifact"),
            stderr_tail: file_tail(&c.out.join(format!("server-{lifetime}.stderr")))
                .expect("read startup stderr artifact"),
        }
    }
}
fn record_startup_failure(shared: &Shared, failure: StartupFailure) -> String {
    problem(shared, &format!("SERVER_FINDING {failure:?}"));
    locked(&shared.fault_coverage).skipped_faults.push(format!(
        "remaining planned epochs/faults/checkpoints not executed after {} for {} lifetime {}",
        failure.kind, failure.server, failure.lifetime
    ));
    locked(&shared.server_failures).push(failure);
    "failed".into()
}

fn start(
    c: &Config,
    binary: &Path,
    data: &Path,
    lease: &support::PortLease,
    number: usize,
    shared: &Shared,
) -> Result<support::ProgramChild, StartupFailure> {
    let log = &shared.wire;
    let origin = shared.origin;
    let port = lease.port().to_string();
    let mut args: Vec<String> = match c.server.as_str() {
        "firn" => vec![
            port,
            "0".into(),
            "chaos.aof".into(),
            "0".into(),
            "--auto-aof-rewrite-percentage".into(),
            "0".into(),
        ],
        "redis" | "valkey" => [
            "--port",
            &port,
            "--bind",
            "127.0.0.1",
            "--save",
            "",
            "--appendonly",
            "yes",
            "--appendfsync",
            "everysec",
            "--auto-aof-rewrite-percentage",
            "0",
            "--aof-use-rdb-preamble",
            "no",
            "--dir",
            data.to_str().unwrap(),
            "--daemonize",
            "no",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        "dragonfly" => vec![
            format!("--port={port}"),
            "--bind=127.0.0.1".into(),
            "--proactor_threads=4".into(),
            "--dbfilename=".into(),
            "--logtostderr".into(),
        ],
        "garnet" => [
            "--port",
            &port,
            "--bind",
            "127.0.0.1",
            "--aof",
            "--recover",
            "--aof-commit-freq",
            "1000",
            "--logdir",
            data.to_str().unwrap(),
            "--checkpointdir",
            data.to_str().unwrap(),
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        _ => unreachable!(),
    };
    note(
        log,
        origin,
        &format!("START binary={binary:?} args={args:?} lifetime={number}"),
    );
    let mut cmd = if std::env::var_os("FIRN_REDIS_BENCH_RECORD").is_some() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c","python3 \"$1\" register \"$FIRN_REDIS_BENCH_RECORD\" parent || exit 1; shift; exec \"$@\"","chaos-server"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../research/experiments/redis-bench/session.py")).arg(binary);
        cmd
    } else {
        Command::new(binary)
    };
    cmd.args(args.drain(..))
        .current_dir(data)
        .env("WF_WORKERS", "4")
        .env("WF_DRIVERS", "2")
        .env_remove("WF_REQUIRE_WINDOWS_IOCP")
        .env_remove("WF_IO_NO_NATIVE_RING")
        .stdin(Stdio::null())
        .stdout(File::create(c.out.join(format!("server-{number}.stdout"))).unwrap())
        .stderr(File::create(c.out.join(format!("server-{number}.stderr"))).unwrap());
    let child = match support::ProgramChild::spawn_with_limit(
        &mut cmd,
        Duration::from_secs(c.seconds + 3600),
    ) {
        Ok(child) => child.with_port(lease).with_name(&c.server),
        Err(error) => {
            return Err(StartupFailure::capture(
                c,
                shared,
                number,
                format!("spawn {}: {error}", c.server),
            ));
        }
    };
    let ready = (|| -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(c.restart_seconds);
        let mut stream = support::try_connect_to_when_ready_with_limit(
            SocketAddr::from(([127, 0, 0, 1], lease.port())),
            &child,
            Duration::from_secs(c.restart_seconds),
        )
        .map_err(|error| error.to_string())?;
        loop {
            child
                .ensure_running(lease.port())
                .map_err(|error| error.to_string())?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "{} did not answer PONG within {} seconds",
                    c.server, c.restart_seconds
                ));
            }
            stream
                .set_read_timeout(Some(remaining))
                .and_then(|_| stream.set_write_timeout(Some(remaining)))
                .map_err(|error| format!("{} startup socket: {error}", c.server))?;
            let bytes = resp(&["PING"]);
            let invoked = Instant::now();
            let mut raw = Vec::new();
            let result = stream
                .write_all(&bytes)
                .and_then(|_| frame(&mut stream, &mut raw, 0));
            note(
                log,
                origin,
                &format!(
                    "STARTUP_PING server={} lifetime={number} begin={} end={} request={} reply={} result={result:?}",
                    c.server,
                    invoked.duration_since(origin).as_nanos(),
                    origin.elapsed().as_nanos(),
                    hex(&bytes),
                    hex(&raw)
                ),
            );
            match result
                .map_err(|error| format!("{} startup PING transport/protocol: {error}", c.server))?
            {
                Frame::Simple(value) if value == "PONG" => break,
                Frame::Error(error) if error.starts_with("LOADING ") => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                other => {
                    return Err(format!(
                        "{} startup PING unexpected reply: {other:?}",
                        c.server
                    ));
                }
            }
        }
        child
            .ensure_running(lease.port())
            .map_err(|error| error.to_string())
    })();
    if let Err(reason) = ready {
        // Stop/reap first: direct-file tails include the final diagnostic, and
        // no still-running startup process survives a typed early return.
        drop(child);
        return Err(StartupFailure::capture(c, shared, number, reason));
    }
    note(
        log,
        origin,
        &format!(
            "READY server={} lifetime={number} pid={}",
            c.server,
            child.id()
        ),
    );
    Ok(child)
}

fn kill(
    mut child: support::ProgramChild,
    shared: &Shared,
    epoch: usize,
    label: &str,
    trigger: Option<(bool, bool)>,
) {
    let intended = Instant::now();
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // Exclude replies published after the signal from the adjacency metric.
    // Hold only across the syscall and timestamp, never across network or file I/O.
    let progress = locked(&shared.ack_progress);
    assert_eq!(unsafe { kill(child.id() as i32, 9) }, 0, "send SIGKILL");
    let sent = Instant::now();
    let last_ack_age = progress.last.map(|last| sent.duration_since(last));
    drop(progress);
    {
        let mut coverage = locked(&shared.fault_coverage);
        coverage.sigkill_sent += 1;
        coverage.faults.push(FaultRecord {
            kind: "SIGKILL sent",
            epoch,
            at_ns: sent.duration_since(shared.origin).as_nanos(),
        });
        if last_ack_age.is_some_and(|age| age <= Duration::from_millis(10)) {
            coverage.ack_adjacent_sigkills_within_10ms += 1;
        }
    }
    note(
        &shared.wire,
        shared.origin,
        &format!(
            "SIGKILL sent epoch={epoch} {label} pid={} intended_ns={} at_ns={} last_ack_age_ns={:?} trigger_fresh_ack_and_disk_cycle={trigger:?}",
            child.id(),
            intended.duration_since(shared.origin).as_nanos(),
            sent.duration_since(shared.origin).as_nanos(),
            last_ack_age.map(|age| age.as_nanos())
        ),
    );
    let (status, reaped) = loop {
        if let Some(status) = child.try_wait().expect("reap killed server") {
            // First harness observation of exit, before collecting captured
            // output. This is not a claim about the kernel's exact exit time.
            break (status, Instant::now());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(9),
        "server exited before injected kill: {:?}",
        status
    );
    {
        let mut coverage = locked(&shared.fault_coverage);
        coverage.sigkill_reaped += 1;
        coverage.faults.push(FaultRecord {
            kind: "SIGKILL reaped",
            epoch,
            at_ns: reaped.duration_since(shared.origin).as_nanos(),
        });
    }
    let _output = child.wait_with_output().expect("collect killed server output");
    note(
        &shared.wire,
        shared.origin,
        &format!(
            "REAP epoch={epoch} {label} status={} at_ns={} timestamp=first_harness_exit_observation",
            status, reaped.duration_since(shared.origin).as_nanos()
        ),
    );
}

#[derive(Default)]
struct AckProgress {
    sequence: u64,
    last: Option<Instant>,
}
impl AckProgress {
    fn record(&mut self, completed: Instant) {
        self.sequence += 1;
        self.last = Some(
            self.last
                .map_or(completed, |previous| previous.max(completed)),
        );
    }
}
fn wait_for_ack_after(
    progress: &Mutex<AckProgress>,
    changed: &Condvar,
    sequence: u64,
    completed_after: Instant,
    limit: Duration,
) -> bool {
    let deadline = Instant::now() + limit;
    let mut state = locked(progress);
    loop {
        if state.sequence > sequence && state.last.is_some_and(|time| time >= completed_after) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        state = changed
            .wait_timeout(state, remaining)
            .unwrap_or_else(|error| error.into_inner())
            .0;
    }
}
fn prepare_kill(shared: &Shared, epoch: usize, disk_cycle: bool) -> (bool, bool) {
    prepare_kill_with_limit(shared, epoch, disk_cycle, IO_LIMIT)
}
fn prepare_kill_with_limit(
    shared: &Shared,
    epoch: usize,
    disk_cycle: bool,
    limit: Duration,
) -> (bool, bool) {
    let progress = locked(&shared.ack_progress);
    let previous = progress.sequence;
    let completed_after = Instant::now();
    drop(progress);
    let fresh = wait_for_ack_after(
        &shared.ack_progress,
        &shared.ack_changed,
        previous,
        completed_after,
        limit,
    );
    // On success, do no artifact I/O between the new reply and the signal.
    if !fresh {
        note(
            &shared.wire,
            shared.origin,
            &format!(
                "KILL_TRIGGER epoch={epoch} fresh_ack=false disk_cycle={disk_cycle} reason=no fresh acknowledgment within {} seconds; kill fallback",
                limit.as_secs_f64()
            ),
        );
        if !disk_cycle {
            problem(
                shared,
                &format!("NO_PROGRESS epoch={epoch} no fresh acknowledgment at kill trigger"),
            );
        }
    }
    (fresh, disk_cycle)
}

#[derive(Clone)]
struct RunPlan {
    seconds: u64,
    device_configured: bool,
    fault_skip_reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckKind {
    Full,
    Incremental,
    Volatile,
}
impl CheckKind {
    fn index(self) -> usize {
        match self {
            Self::Full => 0,
            Self::Incremental => 1,
            Self::Volatile => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CheckPlan {
    first: usize,
    last: usize,
    kind: CheckKind,
}
struct CheckRun {
    finished: Instant,
    duration: Duration,
}
#[derive(Default)]
struct CheckScheduler {
    last_full_epoch: usize,
    last_full_finished: Option<Instant>,
    last_full_duration: Duration,
}
impl CheckScheduler {
    fn interval(&self) -> Duration {
        self.last_full_duration
            .max(Duration::from_millis(1))
            .saturating_mul(19)
            .max(Duration::from_secs(60))
    }
    fn next(&self, newest: usize, now: Instant) -> CheckPlan {
        let full = self.last_full_finished.is_none_or(|finished| {
            now.saturating_duration_since(finished) >= self.interval()
        });
        CheckPlan {
            first: if full { 1 } else { self.last_full_epoch + 1 },
            last: newest,
            kind: if full {
                CheckKind::Full
            } else {
                CheckKind::Incremental
            },
        }
    }
    fn finish_plan(&self, newest: usize) -> Option<CheckPlan> {
        (newest > 0 && self.last_full_epoch < newest).then_some(CheckPlan {
            first: 1,
            last: newest,
            kind: CheckKind::Full,
        })
    }
    fn completed(&mut self, plan: CheckPlan, run: &CheckRun) {
        if plan.kind == CheckKind::Full {
            self.last_full_epoch = plan.last;
            self.last_full_finished = Some(run.finished);
            self.last_full_duration = run.duration.max(Duration::from_millis(1));
        }
    }
}

#[derive(Default)]
struct Timings {
    workload: Duration,
    check: Duration,
    checks_started: [usize; 3],
    checks_completed: [usize; 3],
    checked_epochs: [usize; 3],
}
impl Timings {
    fn fraction(&self, wall: Duration) -> f64 {
        if wall.is_zero() {
            0.0
        } else {
            self.check.as_secs_f64() / wall.as_secs_f64()
        }
    }
}
#[derive(Clone, Copy)]
enum TimedPhase {
    Workload,
    Check(CheckKind),
}
struct PhaseTimer<'a> {
    shared: &'a Shared,
    phase: TimedPhase,
    started: Instant,
    finished: bool,
}
impl<'a> PhaseTimer<'a> {
    fn start(shared: &'a Shared, phase: TimedPhase, started: Instant) -> Self {
        if let TimedPhase::Check(kind) = phase {
            locked(&shared.timings).checks_started[kind.index()] += 1;
        }
        Self {
            shared,
            phase,
            started,
            finished: false,
        }
    }
    fn finish_at(&mut self, finished: Instant, complete: bool) {
        if self.finished {
            return;
        }
        // Stop issuing requests before recording workload time, including on
        // unwind. Joining clients and all later work lie outside this interval.
        if matches!(self.phase, TimedPhase::Workload) {
            self.shared.stop.store(true, Ordering::Release);
        }
        let elapsed = finished.saturating_duration_since(self.started);
        let mut timings = locked(&self.shared.timings);
        match self.phase {
            TimedPhase::Workload => timings.workload += elapsed,
            TimedPhase::Check(kind) => {
                timings.check += elapsed;
                if complete {
                    timings.checks_completed[kind.index()] += 1;
                }
            }
        }
        self.finished = true;
    }
}
impl Drop for PhaseTimer<'_> {
    fn drop(&mut self) {
        self.finish_at(Instant::now(), false);
    }
}

struct Shared {
    stop: AtomicBool,
    killing: AtomicBool,
    disk: AtomicBool,
    drops: AtomicUsize,
    errors: AtomicUsize,
    repeats: AtomicUsize,
    seen: Mutex<HashSet<String>>,
    coverage: Mutex<Vec<[usize; 4]>>,
    fault_coverage: Mutex<Coverage>,
    server_failures: Mutex<Vec<StartupFailure>>,
    run_plan: Mutex<Option<RunPlan>>,
    timings: Mutex<Timings>,
    ack_progress: Mutex<AckProgress>,
    ack_changed: Condvar,
    acks: AtomicUsize,
    wire: Mutex<File>,
    findings: Mutex<File>,
    origin: Instant,
}
impl Shared {
    fn new(out: &Path, origin: Instant) -> Self {
        Self {
            stop: AtomicBool::new(false),
            killing: AtomicBool::new(false),
            disk: AtomicBool::new(false),
            drops: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            repeats: AtomicUsize::new(0),
            seen: Mutex::new(HashSet::new()),
            coverage: Mutex::new(Vec::new()),
            fault_coverage: Mutex::new(Coverage::default()),
            server_failures: Mutex::new(Vec::new()),
            run_plan: Mutex::new(None),
            timings: Mutex::new(Timings::default()),
            ack_progress: Mutex::new(AckProgress::default()),
            ack_changed: Condvar::new(),
            acks: AtomicUsize::new(0),
            wire: Mutex::new(append_file(&out.join("history.log"))),
            findings: Mutex::new(append_file(&out.join("findings.log"))),
            origin,
        }
    }
}

fn problem(shared: &Shared, message: &str) {
    shared.errors.fetch_add(1, Ordering::Relaxed);
    note(&shared.findings, shared.origin, message);
}
fn unique_problem(shared: &Shared, key: String, message: &str) {
    if locked(&shared.seen).insert(key) {
        problem(shared, message);
    } else {
        shared.repeats.fetch_add(1, Ordering::Relaxed);
    }
}
fn writer(c: &Config, shared: &Shared, port: u16, epoch: usize, client: usize) -> Vec<Event> {
    let mut events = Vec::new();
    let mut seed = c.seed ^ ((epoch as u64) << 32) ^ client as u64;
    let mut stream: Option<TcpStream> = None;
    let mut seq = 0_u32;
    while !shared.stop.load(Ordering::Acquire) {
        let key = (random(&mut seed) % KEYS as u64) as usize;
        let kind = kind_from((random(&mut seed) % 4) as u8);
        let dropped = random(&mut seed) % 16 == 0;
        let token = format!("{:06x}{:02x}{:08x}", epoch, client, seq);
        seq = seq.checked_add(1).expect("token overflow");
        assert_eq!(token.len(), 16, "token width");
        let (bytes, count) = request(epoch, key, &token, kind);
        let invoked = Instant::now();
        let mut raw = Vec::new();
        let disk_before = shared.disk.load(Ordering::Acquire);
        let result = (|| -> io::Result<Option<Vec<Frame>>> {
            if stream.is_none() {
                stream = Some(connect(port)?);
            }
            let socket = stream.as_mut().unwrap();
            socket.write_all(&bytes)?;
            if dropped {
                socket.shutdown(Shutdown::Both)?;
                shared.drops.fetch_add(1, Ordering::Relaxed);
                locked(&shared.fault_coverage).connection_drops += 1;
                return Ok(None);
            }
            let frames = (0..count)
                .map(|_| frame(socket, &mut raw, 0))
                .collect::<io::Result<Vec<_>>>()?;
            Ok(Some(frames))
        })();
        let completed = Instant::now();
        let outcome = match result {
            Ok(Some(frames)) => match classify(
                kind,
                &frames,
                disk_before || shared.disk.load(Ordering::Acquire),
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    problem(
                        shared,
                        &format!("PROTOCOL epoch={epoch} token={token} {error}"),
                    );
                    stream = None;
                    Outcome::Unknown
                }
            },
            Ok(None) => {
                stream = None;
                Outcome::Unknown
            }
            Err(error) => {
                if error.kind() == io::ErrorKind::InvalidData
                    || !(shared.killing.load(Ordering::Acquire)
                        || disk_before
                        || shared.disk.load(Ordering::Acquire))
                {
                    problem(
                        shared,
                        &format!("TRANSPORT epoch={epoch} token={token} error={error:?}"),
                    );
                }
                note(
                    &shared.wire,
                    shared.origin,
                    &format!("IO_ERROR token={token} {error:?}"),
                );
                stream = None;
                Outcome::Unknown
            }
        };
        if matches!(outcome, Outcome::Applied { .. }) {
            shared.acks.fetch_add(1, Ordering::Relaxed);
            {
                let mut progress = locked(&shared.ack_progress);
                progress.record(completed);
            }
            shared.ack_changed.notify_one();
            locked(&shared.coverage)[client][kind_code(kind) as usize] += 1;
        }
        // A single artifact line contains complete wire bytes, including partial replies.
        note(
            &shared.wire,
            shared.origin,
            &format!(
                "OP epoch={epoch} client={client} token={token} key={key} kind={} begin={} end={} drop={dropped} request={} reply={} outcome={outcome:?}",
                kind_code(kind),
                invoked.duration_since(shared.origin).as_nanos(),
                completed.duration_since(shared.origin).as_nanos(),
                hex(&bytes),
                hex(&raw)
            ),
        );
        events.push(Event {
            key,
            token,
            kind,
            invoked,
            completed,
            outcome,
        });
        std::thread::sleep(Duration::from_millis(250));
    }
    events
}
fn archive(path: &Path, events: &[Event], origin: Instant) {
    let mut file = File::create(path).expect("create epoch archive");
    writeln!(file, "CHAOS1\t{}", events.len()).unwrap();
    for e in events {
        let (outcome, length, counter) = match e.outcome {
            Outcome::Applied { length, counter } => {
                ("A", length, counter.map_or("-".into(), |v| v.to_string()))
            }
            Outcome::Unknown => ("U", 0, "-".into()),
            Outcome::Rejected => ("R", 0, "-".into()),
        };
        writeln!(
            file,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            e.key,
            e.token,
            kind_code(e.kind),
            e.invoked.duration_since(origin).as_nanos(),
            e.completed.duration_since(origin).as_nanos(),
            outcome,
            length,
            counter
        )
        .unwrap();
    }
    file.flush().expect("flush epoch archive");
}
fn load_archive(reader: impl BufRead, origin: Instant) -> Result<Vec<Event>, String> {
    let mut lines = reader.lines();
    let header = lines
        .next()
        .ok_or("missing archive header")?
        .map_err(|e| e.to_string())?;
    let count = header
        .strip_prefix("CHAOS1\t")
        .ok_or("bad archive header")?
        .parse::<usize>()
        .map_err(|_| "bad archive count")?;
    if count > 1_000_000 {
        return Err("archive count exceeds epoch bound".into());
    }
    let mut events = Vec::new();
    for line in lines {
        let line = line.map_err(|e| e.to_string())?;
        let f: Vec<_> = line.split('\t').collect();
        if f.len() != 8 {
            return Err("bad archive field count".into());
        }
        let key = f[0].parse::<usize>().map_err(|_| "bad archive key")?;
        let kind = f[2].parse::<u8>().map_err(|_| "bad archive kind")?;
        let begin = f[3].parse::<u64>().map_err(|_| "bad archive begin")?;
        let end = f[4].parse::<u64>().map_err(|_| "bad archive end")?;
        let length = f[6].parse::<i64>().map_err(|_| "bad archive length")?;
        let counter = if f[7] == "-" {
            None
        } else {
            Some(f[7].parse::<i64>().map_err(|_| "bad archive counter")?)
        };
        if key >= KEYS
            || kind > 3
            || f[1].len() != 16
            || !f[1].bytes().all(|b| b.is_ascii_hexdigit())
            || begin > end
        {
            return Err("invalid archive event".into());
        }
        let outcome = match f[5] {
            "A" => Outcome::Applied { length, counter },
            "U" if length == 0 && counter.is_none() => Outcome::Unknown,
            "R" if length == 0 && counter.is_none() => Outcome::Rejected,
            _ => return Err("bad archive outcome".into()),
        };
        events.push(Event {
            key,
            token: f[1].into(),
            kind: kind_from(kind),
            invoked: origin + Duration::from_nanos(begin),
            completed: origin + Duration::from_nanos(end),
            outcome,
        });
        if events.len() > count {
            return Err("extra archived event".into());
        }
    }
    if events.len() != count {
        return Err("truncated archive".into());
    }
    Ok(events)
}
struct ObservedSnapshot {
    state: State,
    request: Vec<u8>,
    raw: Vec<u8>,
}

fn snapshot_digest(state: &State) -> u64 {
    // Diagnostic only: neither model validation nor continuity uses this hash.
    let mut digest = DefaultHasher::new();
    (&state.logs, &state.sets, &state.counters).hash(&mut digest);
    digest.finish()
}

fn dump_snapshot(
    out: &Path,
    epoch: usize,
    request: &[u8],
    raw: &[u8],
    state: Option<&State>,
    reason: &str,
) -> io::Result<PathBuf> {
    static NEXT_DUMP: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_DUMP.fetch_add(1, Ordering::Relaxed);
    let stem = out.join(format!("snapshot-failure-{epoch}-{id}"));
    fs::write(stem.with_extension("request"), request)?;
    fs::write(stem.with_extension("resp"), raw)?;
    fs::write(stem.with_extension("txt"), format!("{reason}\nstate={state:?}\n"))?;
    Ok(stem)
}

fn retain_failed_snapshot(
    out: &Path,
    epoch: usize,
    observed: &ObservedSnapshot,
    findings: &Findings,
    baseline_mismatch: bool,
) -> io::Result<Option<PathBuf>> {
    if findings.lost.is_empty() && findings.inconsistent.is_empty() && !baseline_mismatch {
        return Ok(None);
    }
    dump_snapshot(
        out,
        epoch,
        &observed.request,
        &observed.raw,
        Some(&observed.state),
        &format!("baseline_mismatch={baseline_mismatch} findings={findings:?}"),
    )
    .map(Some)
}

fn snapshot_state(parsed: Frame) -> Result<State, String> {
    let Frame::Array(Some(frames)) = parsed else {
        return Err("snapshot MGET is not an array".into());
    };
    if frames.len() != KEYS * 3 {
        return Err("snapshot MGET arity".into());
    }
    let values: Vec<Option<String>> = frames
        .into_iter()
        .map(|f| match f {
            Frame::Bulk(bytes) => bytes
                .map(String::from_utf8)
                .transpose()
                .map_err(|error| format!("snapshot UTF8: {error}")),
            other => Err(format!("snapshot MGET element {other:?}")),
        })
        .collect::<Result<_, _>>()?;
    let mut state = State {
        logs: Vec::new(),
        sets: Vec::new(),
        counters: Vec::new(),
    };
    for group in values.chunks_exact(3) {
        state.logs.push(group[0].clone().unwrap_or_default());
        state.sets.push(group[1].clone());
        state.counters.push(
            group[2].as_ref().map(|v| v.parse::<i64>())
                .transpose().map_err(|error| format!("snapshot counter: {error}"))?,
        );
    }
    Ok(state)
}

fn snapshot_summary(epoch: usize, state: Option<&State>, begin_ns: u128, end_ns: u128) -> String {
    let digest = state
        .map(|state| format!("{:016x}", snapshot_digest(state)))
        .unwrap_or_else(|| "unavailable".into());
    format!(
        "SNAPSHOT e={epoch} keys={} digest=default-hasher64:{digest} begin={begin_ns} end={end_ns}",
        KEYS * 3,
    )
}

fn snapshot(c: &Config, shared: &Shared, port: u16, epoch: usize) -> ObservedSnapshot {
    let names: Vec<String> = (0..KEYS).flat_map(|key| keys(epoch, key)).collect();
    let mut args = vec!["MGET"];
    args.extend(names.iter().map(String::as_str));
    let bytes = resp(&args);
    let invoked = Instant::now();
    let mut raw = Vec::new();
    let parsed = (|| {
        let mut stream = connect(port)?;
        stream.write_all(&bytes)?;
        frame(&mut stream, &mut raw, 0)
    })()
    .map_err(|error: io::Error| error.to_string())
    .and_then(snapshot_state);
    let ended = Instant::now();
    note(
        &shared.wire,
        shared.origin,
        &snapshot_summary(
            epoch,
            parsed.as_ref().ok(),
            invoked.duration_since(shared.origin).as_nanos(),
            ended.duration_since(shared.origin).as_nanos(),
        ),
    );
    match parsed {
        Ok(state) => ObservedSnapshot { state, request: bytes, raw },
        Err(error) => {
            dump_snapshot(&c.out, epoch, &bytes, &raw, None, &error)
                .expect("retain malformed snapshot evidence");
            panic!("snapshot: {error}");
        }
    }
}

fn matches_validated_snapshot(previous: &str, current: &State) -> bool {
    previous == format!("{current:?}")
}

const KNOWN_ACK_WINDOW_NS: u128 = 10_000_000;
const KNOWN_ACK_LOSS: &str = "known-acknowledgment-before-append-#97-still-fails";
const UNVERIFIED_ACK_LOSS: &str = "firn-loss-attribution-unverified-still-fails";
const OVERLAPPING_EIO_LOSS: &str = "post-eio-loss-attribution-ambiguous-still-fails";
const OUTSIDE_ACK_WINDOW_LOSS: &str = "loss-outside-nominal-10ms-window-investigate-still-fails";

#[derive(Clone, Debug, Eq, PartialEq)]
struct EioWindow {
    epoch: usize,
    active_ns: u128,
    healed_ns: Option<u128>,
}
#[derive(Debug, Eq, PartialEq)]
enum EioEvidence {
    NoOverlap,
    Overlap(Vec<EioWindow>),
    Unverified(&'static str),
}
#[derive(Debug)]
struct LossAttribution {
    label: &'static str,
    invoked_ns: Option<u128>,
    ack_completed_ns: Option<u128>,
    sigkill_at_ns: Option<u128>,
    sigkill_reaped_ns: Option<u128>,
    // Signed completed-minus-sent offset; positive means the reply completed
    // after signal submission. The matched reap separately bounds attribution.
    ack_offset_ns: Option<i128>,
    ack_age_ns: Option<u128>,
    eio_overlap: EioEvidence,
    interpretation: &'static str,
}

/// Pair actual activation/healing records by epoch. An open interval remains
/// active through the observed kill; a prior healed interval is not a veto.
fn eio_overlap(faults: &[FaultRecord], invoked_ns: u128, sigkill_ns: u128) -> EioEvidence {
    let mut active = HashMap::new();
    let mut windows = Vec::new();
    let mut last_time = None;
    for fault in faults {
        if !matches!(
            fault.kind,
            "combined write/flush EIO active" | "combined write/flush EIO healed"
        ) {
            continue;
        }
        if last_time.is_some_and(|previous| previous > fault.at_ns) {
            return EioEvidence::Unverified("EIO timestamps are not monotonic");
        }
        last_time = Some(fault.at_ns);
        match fault.kind {
            "combined write/flush EIO active" => {
                if active.insert(fault.epoch, fault.at_ns).is_some() {
                    return EioEvidence::Unverified("duplicate EIO activation before healing");
                }
            }
            "combined write/flush EIO healed" => {
                let Some(start) = active.remove(&fault.epoch) else {
                    return EioEvidence::Unverified("EIO healing has no preceding activation");
                };
                windows.push(EioWindow {
                    epoch: fault.epoch,
                    active_ns: start,
                    healed_ns: Some(fault.at_ns),
                });
            }
            _ => unreachable!(),
        }
    }
    // Signal delivery is asynchronous: a request can start after submission.
    // Its invocation-to-send interval is empty. Validate the complete EIO
    // metadata above before declaring that empty interval free of overlap.
    if invoked_ns > sigkill_ns {
        return EioEvidence::NoOverlap;
    }
    windows.extend(active.into_iter().map(|(epoch, start)| EioWindow {
        epoch,
        active_ns: start,
        healed_ns: None,
    }));
    // Active intervals are half-open: a heal completed at invocation time
    // has already ended the failure; activation at kill time still overlaps.
    windows.retain(|window| {
        window.active_ns <= sigkill_ns && window.healed_ns.is_none_or(|end| end > invoked_ns)
    });
    windows.sort_by_key(|window| (window.active_ns, window.epoch));
    if windows.is_empty() {
        EioEvidence::NoOverlap
    } else {
        EioEvidence::Overlap(windows)
    }
}

fn current_sigkill_window(faults: &[FaultRecord]) -> Result<(u128, u128), &'static str> {
    let mut previous = None;
    for fault in faults.iter().filter(|fault| {
        matches!(fault.kind, "SIGKILL sent" | "SIGKILL reaped")
    }) {
        if previous.is_some_and(|time| time > fault.at_ns) {
            return Err("SIGKILL send/reap timestamps are not monotonic");
        }
        previous = Some(fault.at_ns);
    }
    let (sent_index, sent) = faults.iter().enumerate().rev()
        .find(|(_, fault)| fault.kind == "SIGKILL sent")
        .ok_or("missing current SIGKILL send")?;
    if faults.iter().filter(|fault| fault.kind == "SIGKILL sent" && fault.epoch == sent.epoch).count() != 1 {
        return Err("ambiguous current SIGKILL send");
    }
    let mut reaps = faults.iter().enumerate()
        .filter(|(_, fault)| fault.kind == "SIGKILL reaped" && fault.epoch == sent.epoch);
    let (reaped_index, reaped) = reaps.next().ok_or("missing current SIGKILL reap")?;
    if reaps.next().is_some() {
        return Err("ambiguous current SIGKILL reap");
    }
    if reaped_index <= sent_index || reaped.at_ns < sent.at_ns {
        return Err("SIGKILL reap precedes its send");
    }
    Ok((sent.at_ns, reaped.at_ns))
}

fn classify_lost_write(
    server: &str,
    fix: &str,
    lost: &LostWrite,
    events: &[Event],
    origin: Instant,
    faults: &[FaultRecord],
) -> LossAttribution {
    let sigkill_at_ns = faults
        .iter()
        .rev()
        .find(|fault| fault.kind == "SIGKILL sent")
        .map(|fault| fault.at_ns);
    let kill_window = current_sigkill_window(faults);
    let mut attribution = LossAttribution {
        label: UNVERIFIED_ACK_LOSS,
        invoked_ns: None,
        ack_completed_ns: None,
        sigkill_at_ns,
        sigkill_reaped_ns: kill_window.as_ref().ok().map(|(_, reaped)| *reaped),
        ack_offset_ns: None,
        ack_age_ns: None,
        eio_overlap: EioEvidence::Unverified(
            "missing or invalid acknowledged event/signal metadata",
        ),
        interpretation: "attribution unverified; an acknowledged write is still lost",
    };
    let mut matches = events.iter().filter(|event| event.token == lost.token);
    if let Some(event) = matches.next() {
        attribution.invoked_ns = event
            .invoked
            .checked_duration_since(origin)
            .map(|d| d.as_nanos());
        attribution.ack_completed_ns = event
            .completed
            .checked_duration_since(origin)
            .map(|d| d.as_nanos());
        if matches.next().is_none()
            && event.key == lost.key
            && event.key < KEYS
            && event.token.len() == 16
            && event.token.is_ascii()
            && matches!(event.outcome,Outcome::Applied {length,counter} if length>0 && length%16==0 && counter.is_some()==has_incr(event.kind))
            && event.invoked <= event.completed
        {
            if let (Some(invoked), Some(completed), Some(killed)) = (
                attribution.invoked_ns,
                attribution.ack_completed_ns,
                attribution.sigkill_at_ns,
            ) {
                attribution.ack_age_ns = killed.checked_sub(completed);
                attribution.ack_offset_ns = i128::try_from(completed).ok()
                    .zip(i128::try_from(killed).ok())
                    .and_then(|(completed, killed)| completed.checked_sub(killed));
                match kill_window {
                    Err(reason) => {
                        attribution.eio_overlap = EioEvidence::Unverified(reason);
                        attribution.interpretation = reason;
                    }
                    Ok((_, reaped)) if completed <= reaped => {
                        attribution.eio_overlap = eio_overlap(faults, invoked, killed);
                    }
                    Ok(_) => {
                        attribution.interpretation = "acknowledgment follows the matched SIGKILL reap; attribution unverified";
                    }
                }
            }
        }
    }
    // Policy labels describe competitors without changing the loss finding.
    match server {
        "redis" | "valkey" => {
            attribution.label = "observation-under-everysec-not-always-sync";
            attribution.interpretation = "lost acknowledgment observed under the everysec policy";
            return attribution;
        }
        "garnet" => {
            attribution.label = "aof-commit-semantics-unverified";
            attribution.interpretation =
                "lost acknowledgment; installed Garnet commit semantics remain unverified";
            return attribution;
        }
        "firn" => {}
        _ => {
            attribution.label = "lost-acknowledged-write";
            return attribution;
        }
    }
    if fix == "unknown" {
        attribution.label = "firn-fix-status-unknown-still-fails";
        attribution.interpretation = "fix status is unverified; no known-cause attribution";
    } else if attribution.ack_offset_ns.is_none()
        || attribution.sigkill_reaped_ns.is_none()
        || matches!(attribution.eio_overlap, EioEvidence::Unverified(_))
    {
        // No inferred offset, missing request/reap, reversed time, or malformed fault
        // record can establish the known acknowledgment-before-append window.
    } else if fix == "present" {
        attribution.label = "lost-acknowledged-write";
        attribution.interpretation =
            "append-before-reply fix reported present; investigate this loss";
    } else if fix != "absent" {
        attribution.interpretation = "invalid fix-status metadata; attribution unverified";
    } else if matches!(attribution.eio_overlap, EioEvidence::Overlap(_)) {
        attribution.label = OVERLAPPING_EIO_LOSS;
        attribution.interpretation = "EIO overlapped the request-to-kill interval; cannot attribute solely to acknowledgment-before-append";
    } else if attribution
        .ack_offset_ns
        .is_some_and(|offset| offset >= -(KNOWN_ACK_WINDOW_NS as i128))
    {
        attribution.label = KNOWN_ACK_LOSS;
        attribution.interpretation = "consistent with the known pre-fix window from 10ms before SIGKILL send through its reap; still fails acceptance, not proof of sole cause";
    } else {
        attribution.label = OUTSIDE_ACK_WINDOW_LOSS;
        attribution.interpretation =
            "outside the nominal 10ms window; investigate, not proof of a separate firn defect";
    }
    attribution
}

fn report_lost_write(
    shared: &Shared,
    epoch: usize,
    finding: &LostWrite,
    attribution: &LossAttribution,
) {
    // Attribution is diagnostic only: every first-observed lost acknowledgment
    // increments the same failure count, including a known pre-fix window.
    unique_problem(
        shared,
        format!("LOST-{epoch}-{finding}"),
        &format!(
            "LOST epoch={epoch} class={} {finding} invoked_ns={:?} ack_completed_ns={:?} sigkill_at_ns={:?} sigkill_reaped_ns={:?} ack_to_kill_offset_ns={:?} ack_age_ns={:?} nominal_ack_window_ns={} EIO_overlap={:?} interpretation={}",
            attribution.label,
            attribution.invoked_ns,
            attribution.ack_completed_ns,
            attribution.sigkill_at_ns,
            attribution.sigkill_reaped_ns,
            attribution.ack_offset_ns,
            attribution.ack_age_ns,
            KNOWN_ACK_WINDOW_NS,
            attribution.eio_overlap,
            attribution.interpretation
        ),
    );
}

fn checkpoint(
    c: &Config,
    shared: &Shared,
    port: u16,
    plan: CheckPlan,
) -> CheckRun {
    let begin = Instant::now();
    let mut timer = PhaseTimer::start(shared, TimedPhase::Check(plan.kind), begin);
    let mut failures = 0;
    for epoch in plan.first..=plan.last {
        let events = load_archive(
            BufReader::new(
                File::open(c.out.join(format!("epoch-{epoch}.tsv"))).expect("open epoch archive"),
            ),
            shared.origin,
        )
        .expect("validate complete archived history");
        let observed = snapshot(c, shared, port, epoch);
        let state = &observed.state;
        let findings = chaos_check::check(&events, state, Instant::now());
        let baseline = c.out.join(format!("validated-{epoch}.txt"));
        let serialized = format!("{state:?}");
        let mut baseline_mismatch = false;
        if baseline.exists() {
            if !matches_validated_snapshot(
                &fs::read_to_string(&baseline).expect("read validated state"),
                state,
            ) {
                baseline_mismatch = true;
                unique_problem(
                    shared,
                    format!("CHANGED-{epoch}"),
                    &format!(
                        "INCONSISTENT epoch={epoch} closed epoch changed after an independently validated checkpoint; includes unknown effects, not all are acknowledged losses"
                    ),
                );
                failures += 1;
            }
        } else if findings.lost.is_empty() && findings.inconsistent.is_empty() {
            // This first independently validated exact state is expectation input
            // for unknown-effect continuity, not a repeated observed-state dump.
            fs::write(&baseline, serialized).expect("archive first independently validated state");
        }
        retain_failed_snapshot(&c.out, epoch, &observed, &findings, baseline_mismatch)
            .expect("retain failing snapshot evidence");
        let Findings { lost, inconsistent } = findings;
        for finding in lost {
            let attribution = classify_lost_write(
                &c.server,
                &c.fix,
                &finding,
                &events,
                shared.origin,
                &locked(&shared.fault_coverage).faults,
            );
            failures += 1;
            report_lost_write(shared, epoch, &finding, &attribution);
        }
        for finding in inconsistent {
            failures += 1;
            unique_problem(
                shared,
                format!("INCONSISTENT-{epoch}-{finding}"),
                &format!("INCONSISTENT epoch={epoch} {finding}"),
            );
        }
        locked(&shared.timings).checked_epochs[plan.kind.index()] += 1;
    }
    note(
        &shared.wire,
        shared.origin,
        &format!(
            "CHECK kind={:?} first={} last={} seconds={} findings={failures}",
            plan.kind, plan.first, plan.last, begin.elapsed().as_secs_f64()
        ),
    );
    let finished = Instant::now();
    timer.finish_at(finished, true);
    CheckRun {
        finished,
        duration: finished.duration_since(begin),
    }
}

fn execute(c: &Config, shared: &Shared) -> String {
    let Some(binary) = c.binary() else {
        locked(&shared.fault_coverage).skipped_faults.push(format!(
            "all planned faults skipped: {} executable missing",
            c.server
        ));
        if matches!(c.server.as_str(), "firn" | "redis") {
            panic!("required {} binary missing", c.server);
        }
        note(
            &shared.findings,
            shared.origin,
            "SKIPPED missing competitor; comparison incomplete",
        );
        return "skipped".into();
    };
    let meta = fs::metadata(&binary).expect("binary metadata");
    note(
        &shared.wire,
        shared.origin,
        &format!(
            "IDENTITY binary={binary:?} bytes={} modified={:?}",
            meta.len(),
            meta.modified()
        ),
    );
    if c.server == "redis" {
        let output = support::run_command(Command::new(&binary).arg("--version"));
        note(
            &shared.wire,
            shared.origin,
            &format!(
                "VERSION stdout={} stderr={}",
                hex(&output.stdout),
                hex(&output.stderr)
            ),
        );
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(" v=7.2.16 "),
            "Redis must be pinned 7.2.16"
        );
    }
    let data_parent = c.mount.as_deref().unwrap_or(&c.out);
    let data = data_parent.join(format!("chaos-data-{}-{}", c.server, std::process::id()));
    fs::create_dir(&data).expect("new server data directory");
    let data = fs::canonicalize(data).unwrap();
    let out = fs::canonicalize(&c.out).unwrap();
    if let Some(mount) = &c.mount {
        let mount = fs::canonicalize(mount).unwrap();
        assert!(
            !out.starts_with(&mount),
            "CHAOS_OUT must be outside faulty mount"
        );
    }
    let device = Device {
        enabled: c.mount.is_some(),
        mount: c.mount.clone(),
        shared,
        script: Path::new(env!("CARGO_MANIFEST_DIR")).join("aof_sync_fault_device.sh"),
    };
    device.run("heal");
    if !device.enabled {
        locked(&shared.fault_coverage).skipped_faults.push(format!(
            "write EIO and flush EIO skipped: {}",
            c.fault_skip_reason
        ));
        note(
            &shared.findings,
            shared.origin,
            "SKIPPED disk failure: CHAOS_FAULT_MOUNT absent; full acceptance unavailable",
        );
    }
    if c.server == "dragonfly" {
        locked(&shared.fault_coverage).skipped_faults.push("restart durability not applicable: Dragonfly has no AOF; SIGKILL follows the volatile checkpoint".into());
        note(
            &shared.findings,
            shared.origin,
            "NOT_APPLICABLE restart durability: Dragonfly has no AOF; check before kill, start new volatile lifetime",
        );
    }
    if c.server == "garnet" {
        note(
            &shared.findings,
            shared.origin,
            "UNVERIFIED Garnet AOF commit semantics; startup has no fallback",
        );
    }
    let lease = support::lease_port();
    let mut child = Some(match start(c, &binary, &data, &lease, 0, shared) {
        Ok(child) => child,
        Err(failure) => return record_startup_failure(shared, failure),
    });
    let deadline = shared.origin + Duration::from_secs(c.seconds);
    let mut schedule = c.seed;
    let mut epoch = 0;
    let mut disks = 0;
    let mut checks = CheckScheduler::default();
    while Instant::now() < deadline {
        epoch += 1;
        locked(&shared.fault_coverage).epochs_started += 1;
        shared.stop.store(false, Ordering::Release);
        shared.killing.store(false, Ordering::Release);
        shared.disk.store(false, Ordering::Release);
        let seconds = epoch_seconds(&mut schedule);
        let active = Instant::now();
        let end = (active + Duration::from_secs(seconds)).min(deadline);
        let disk_cycle = epoch % 3 == 0
            && device.enabled
            && end.saturating_duration_since(active) >= Duration::from_secs(6);
        let acks_before = shared.acks.load(Ordering::Relaxed);
        let events = std::thread::scope(|scope| {
            let mut workload = PhaseTimer::start(shared, TimedPhase::Workload, active);
            let handles: Vec<_> = (0..c.clients)
                .map(|client| {
                    let port = lease.port();
                    scope.spawn(move || writer(c, shared, port, epoch, client))
                })
                .collect();
            let mut failed = false;
            let mut failed_at = None;
            let mut healed = false;
            while Instant::now() < end || (failed && !healed) {
                let status = child.as_mut().unwrap().try_wait().expect("monitor server");
                assert!(status.is_none(), "unrequested server exit: {status:?}");
                if disk_cycle && !failed && Instant::now() >= end - Duration::from_secs(5) {
                    shared.disk.store(true, Ordering::Release);
                    note(&shared.wire, shared.origin, "DISK_FAIL intention");
                    device.run("fail");
                    {
                        let at_ns = shared.origin.elapsed().as_nanos();
                        let mut coverage = locked(&shared.fault_coverage);
                        coverage.write_eio_injected_windows += 1;
                        coverage.flush_eio_injected_windows += 1;
                        coverage.faults.push(FaultRecord {
                            kind: "combined write/flush EIO active",
                            epoch,
                            at_ns,
                        });
                    }
                    note(&shared.wire, shared.origin, "DISK_FAIL active");
                    failed = true;
                    failed_at = Some(Instant::now());
                    disks += 1;
                }
                if failed && !healed && failed_at.unwrap().elapsed() >= Duration::from_secs(3) {
                    device.run("heal");
                    locked(&shared.fault_coverage).faults.push(FaultRecord {
                        kind: "combined write/flush EIO healed",
                        epoch,
                        at_ns: shared.origin.elapsed().as_nanos(),
                    });
                    note(
                        &shared.wire,
                        shared.origin,
                        "DISK_HEAL complete; refusal recovery window until restart",
                    );
                    healed = true;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if c.server != "dragonfly" {
                let trigger = prepare_kill(shared, epoch, disk_cycle);
                shared.killing.store(true, Ordering::Release);
                kill(
                    child.take().unwrap(),
                    shared,
                    epoch,
                    "injected while writers active",
                    Some(trigger),
                );
            }
            shared.stop.store(true, Ordering::Release);
            workload.finish_at(Instant::now(), true);
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("writer panic"))
                .collect::<Vec<_>>()
        });
        if shared.acks.load(Ordering::Relaxed) == acks_before {
            problem(
                shared,
                &format!("NO_PROGRESS epoch={epoch} no successful writes"),
            );
        }
        archive(
            &c.out.join(format!("epoch-{epoch}.tsv")),
            &events,
            shared.origin,
        );
        drop(events);
        fs::write(
            c.out.join(format!("epoch-{epoch}.disk")),
            if disk_cycle { "true" } else { "false" },
        )
        .expect("archive disk attribution");
        if c.server == "dragonfly" {
            checkpoint(c, shared, lease.port(), CheckPlan {
                first: epoch, last: epoch, kind: CheckKind::Volatile,
            });
            kill(
                child.take().unwrap(),
                shared,
                epoch,
                "injected after volatile checkpoint",
                None,
            );
        }
        // Device is healed without resetting its files; retained data remains evidence.
        device.run("heal");
        locked(&shared.fault_coverage).restart_attempts += 1;
        child = Some(match start(c, &binary, &data, &lease, epoch, shared) {
            Ok(child) => {
                locked(&shared.fault_coverage).sigkill_restarts_completed += 1;
                child
            }
            Err(failure) => return record_startup_failure(shared, failure),
        });
        if c.server != "dragonfly" {
            let plan = checks.next(epoch, Instant::now());
            let run = checkpoint(c, shared, lease.port(), plan);
            checks.completed(plan, &run);
        }
        locked(&shared.fault_coverage).epochs_completed += 1;
        note(
            &shared.wire,
            shared.origin,
            &format!(
                "EPOCH_DONE epoch={epoch} disk={disk_cycle} workload_seconds={}",
                locked(&shared.timings).workload.as_secs_f64()
            ),
        );
    }
    if c.server != "dragonfly" {
        // An incremental check crossing the deadline did not revisit old epochs.
        // A full check already covering this last epoch needs no duplicate pass.
        if let Some(plan) = checks.finish_plan(epoch) {
            let run = checkpoint(c, shared, lease.port(), plan);
            checks.completed(plan, &run);
        }
    }
    if epoch == 0 {
        problem(shared, "NO_PROGRESS startup consumed the wall-clock budget; no workload epoch started");
    }
    let gaps = locked(&shared.fault_coverage).smoke_gaps(c.seconds, device.enabled);
    for gap in gaps {
        problem(shared, &format!("COVERAGE {gap}"));
    }
    for (client, counts) in locked(&shared.coverage).iter().enumerate() {
        note(
            &shared.findings,
            shared.origin,
            &format!("CLIENT_COVERAGE client={client} applied_append_set_incr_both={counts:?}"),
        );
        if counts.iter().any(|count| *count == 0) {
            problem(
                shared,
                &format!(
                    "COVERAGE client={client} has an operation kind with zero acknowledgments"
                ),
            );
        }
    }
    note(
        &shared.findings,
        shared.origin,
        &format!(
            "COVERAGE epochs={epoch} disk_cycles={disks} drops={} acknowledged={} workload_seconds={} elapsed_seconds={}",
            shared.drops.load(Ordering::Relaxed),
            shared.acks.load(Ordering::Relaxed),
            locked(&shared.timings).workload.as_secs_f64(),
            shared.origin.elapsed().as_secs_f64()
        ),
    );
    note(
        &shared.wire,
        shared.origin,
        "CLEANUP normal end (not an unrequested crash)",
    );
    drop(child);
    // Hardware identity is collected separately by CI; this harness never asserts it from a label.
    note(
        &shared.findings,
        shared.origin,
        "ACCEPTANCE not established automatically: requires reviewed 86400-second i9-14900K run, all requested coverage, and zero failures",
    );
    if shared.errors.load(Ordering::Relaxed) == 0 {
        "passed-functional-probe".into()
    } else {
        "failed".into()
    }
}

fn render_report(shared: &Shared, status: &str, panic: Option<&str>) -> String {
    render_report_at(shared, status, panic, shared.origin.elapsed())
}

fn enforce_check_budget(shared: &Shared, wall: Duration) {
    let long_run = locked(&shared.run_plan)
        .as_ref()
        .is_some_and(|p| p.seconds >= 86400);
    let fraction = locked(&shared.timings).fraction(wall);
    if long_run && fraction > 0.10 {
        problem(shared, &format!(
            "CHECK_BUDGET check_fraction={fraction} exceeds 0.10 of wall time for a requested day run"
        ));
    }
}

fn render_report_at(shared: &Shared, status: &str, panic: Option<&str>, wall: Duration) -> String {
    let mut report = format!(
        "status={status}\nelapsed_seconds={}\nwall_seconds={}\nfindings={}\nacknowledged={}\nconnection_drops={}\npanic={panic:?}\nacceptance=requires_review_of_full_duration_hardware_coverage_and_findings\ndeadline_semantics=wall_clock_budget_no_new_epochs_after_deadline_final_fault_and_verification_may_overrun\n",
        wall.as_secs_f64(),
        wall.as_secs_f64(),
        shared.errors.load(Ordering::Relaxed),
        shared.acks.load(Ordering::Relaxed),
        shared.drops.load(Ordering::Relaxed)
    );
    {
        let timings = locked(&shared.timings);
        report.push_str(&format!(
            "workload_seconds={}\ncheck_seconds={}\ncheck_fraction={}\n",
            timings.workload.as_secs_f64(), timings.check.as_secs_f64(), timings.fraction(wall)
        ));
        for (index, kind) in ["full", "incremental", "volatile"].iter().enumerate() {
            report.push_str(&format!(
                "{kind}_checks_started={}\n{kind}_checks_completed={}\n{kind}_epochs_checked={}\n",
                timings.checks_started[index], timings.checks_completed[index], timings.checked_epochs[index]
            ));
        }
    }
    let plan = locked(&shared.run_plan).clone();
    report.push_str(&format!(
        "requested_seconds={:?}\nconfigured_fault_device={:?}\n",
        plan.as_ref().map(|p| p.seconds),
        plan.as_ref().map(|p| p.device_configured)
    ));
    let failures = locked(&shared.server_failures);
    let coverage = locked(&shared.fault_coverage);
    let mut unexecuted = Vec::new();
    if panic.is_some() {
        unexecuted.push(
            "remaining planned faults and checks aborted by harness error; see panic field".into(),
        );
    }
    if coverage.write_eio_injected_windows == 0 || coverage.flush_eio_injected_windows == 0 {
        let missing = match (
            coverage.write_eio_injected_windows == 0,
            coverage.flush_eio_injected_windows == 0,
        ) {
            (true, true) => "combined write/flush EIO",
            (true, false) => "write EIO",
            _ => "flush EIO",
        };
        let reason = if panic.is_some() {
            "harness error stopped execution before this injection; see panic field".into()
        } else if let Some(failure) = failures.last() {
            format!(
                "{} stopped execution for {} lifetime {}",
                failure.kind, failure.server, failure.lifetime
            )
        } else if status == "skipped" {
            "requested server unavailable; all planned faults were skipped".into()
        } else if let Some(plan) = &plan {
            if !plan.device_configured {
                plan.fault_skip_reason.clone()
            } else {
                format!(
                    "no eligible third disk-failure epoch reached before the {}-second run deadline (checkpoint and restart time included)",
                    plan.seconds
                )
            }
        } else {
            "run configuration unavailable; setup did not reach the injection schedule".into()
        };
        unexecuted.push(format!("{missing} not injected: {reason}"));
    }
    report.push_str(&coverage.report_with_skips(&unexecuted));
    drop(coverage);
    drop(failures);
    for (index, failure) in locked(&shared.server_failures).iter().enumerate() {
        report.push_str(&format!(
            "server_finding[{index}].recovery_check=not_executed_server_unavailable\n"
        ));
        report.push_str(&format!("server_finding[{index}].kind={:?}\nserver_finding[{index}].server={:?}\nserver_finding[{index}].lifetime={}\nserver_finding[{index}].reason={:?}\nserver_finding[{index}].preceding_faults={:?}\nserver_finding[{index}].stdout_tail={:?}\nserver_finding[{index}].stderr_tail={:?}\n",failure.kind,failure.server,failure.lifetime,failure.reason,failure.preceding_faults,failure.stdout_tail,failure.stderr_tail));
    }
    report
}

#[test]
#[ignore = "explicit CI chaos experiment; see chaos.yml"]
fn chaos_run() -> Result<(), String> {
    // Establish reporting before parsing all settings, so setup panics leave a report too.
    let out = PathBuf::from(
        std::env::var_os("CHAOS_OUT").expect("CHAOS_OUT must name healthy artifact directory"),
    );
    fs::create_dir_all(&out).expect("create artifact directory");
    let origin = Instant::now();
    let shared = Shared::new(&out, origin);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let c = Config::read();
        *locked(&shared.run_plan) = Some(RunPlan {
            seconds: c.seconds,
            device_configured: c.mount.is_some(),
            fault_skip_reason: c.fault_skip_reason.clone(),
        });
        *locked(&shared.coverage) = vec![[0; 4]; c.clients];
        note(
            &shared.wire,
            origin,
            &format!(
                "CONFIG server={} seconds={} restart_seconds={} seed={} clients={} mount={:?} firn_append_before_reply={}",
                c.server, c.seconds, c.restart_seconds, c.seed, c.clients, c.mount, c.fix
            ),
        );
        execute(&c, &shared)
    }));
    let (mut status, panic) = match result {
        Ok(status) => (status, None),
        Err(payload) => {
            let text = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or("non-string panic".into());
            ("failed".into(), Some(text))
        }
    };
    // execute's child/device cleanup and final full checkpoint have finished.
    // Freeze one wall value for the budget decision and the published fraction.
    let wall = shared.origin.elapsed();
    enforce_check_budget(&shared, wall);
    if shared.errors.load(Ordering::Relaxed) > 0 {
        status = "failed".into();
    }
    note(
        &shared.findings,
        origin,
        &format!(
            "REPEATED_CHECKER_OBSERVATIONS {} (first attribution retained)",
            shared.repeats.load(Ordering::Relaxed)
        ),
    );
    let report = render_report_at(&shared, &status, panic.as_deref(), wall);
    fs::write(out.join("report.txt"), report).expect("write final report");
    if status == "failed" {
        Err(format!("chaos failed; see {}", out.display()))
    } else {
        Ok(())
    }
}

#[test]
fn resp_parser_rejects_truncated_malformed_and_oversized_frames() {
    assert_eq!(
        frame(&mut &b"*2\r\n:16\r\n+OK\r\n"[..], &mut Vec::new(), 0).unwrap(),
        Frame::Array(Some(vec![Frame::Integer(16), Frame::Simple("OK".into())]))
    );
    for bad in [
        b"$3\r\nab".as_slice(),
        b"+OK\n",
        b":no\r\n",
        b"$-2\r\n",
        b"*999999\r\n",
        b"$999999999\r\n",
        b"$1\r\nx\rX",
    ] {
        assert!(
            frame(&mut &bad[..], &mut Vec::new(), 0).is_err(),
            "accepted {bad:?}"
        );
    }
}
#[test]
fn protocol_rejects_partial_transactions_bad_set_and_unexpected_errors() {
    let good = vec![
        Frame::Simple("OK".into()),
        Frame::Simple("QUEUED".into()),
        Frame::Simple("QUEUED".into()),
        Frame::Array(Some(vec![Frame::Integer(16), Frame::Simple("OK".into())])),
    ];
    assert!(matches!(
        classify(Kind::Set, &good, false),
        Ok(Outcome::Applied { .. })
    ));
    let mut bad = good;
    bad[3] = Frame::Array(Some(vec![
        Frame::Integer(16),
        Frame::Error("ERR write failed".into()),
    ]));
    assert!(classify(Kind::Set, &bad, true).is_err());
    bad[3] = Frame::Array(Some(vec![Frame::Integer(16)]));
    assert!(classify(Kind::Set, &bad, false).is_err());
    let refused = vec![Frame::Error("MISCONF disk failed".into())];
    assert!(classify(Kind::Append, &refused, false).is_err());
    assert!(matches!(
        classify(Kind::Append, &refused, true),
        Ok(Outcome::Rejected)
    ));
}
#[test]
fn archive_parser_rejects_missing_truncated_and_extra_history() {
    let origin = Instant::now();
    let event = "0\t0000000000000001\t0\t0\t1\tA\t16\t-\n";
    assert_eq!(
        load_archive(
            BufReader::new(format!("CHAOS1\t1\n{event}").as_bytes()),
            origin
        )
        .unwrap()
        .len(),
        1
    );
    for bad in [
        String::new(),
        "CHAOS1\t1\n".into(),
        format!("CHAOS1\t0\n{event}"),
        "CHAOS1\t1\n0\ttoken\n".into(),
    ] {
        assert!(load_archive(BufReader::new(bad.as_bytes()), origin).is_err());
    }
}

#[test]
fn retained_snapshot_detects_a_previously_witnessed_unknown_effect_disappearing() {
    let now = Instant::now();
    let event = Event {
        key: 0,
        token: "0000000000000001".into(),
        kind: Kind::Append,
        invoked: now,
        completed: now,
        outcome: Outcome::Unknown,
    };
    let empty = State {
        logs: vec![String::new(); KEYS],
        sets: vec![None; KEYS],
        counters: vec![None; KEYS],
    };
    let mut present = empty.clone();
    present.logs[0] = event.token.clone();
    // Each snapshot alone permits either outcome of an unread reply. Once its
    // effect was independently witnessed, a later checkpoint cannot erase it.
    assert_eq!(
        chaos_check::check(std::slice::from_ref(&event), &present, now),
        Findings::default()
    );
    assert_eq!(
        chaos_check::check(&[event], &empty, now),
        Findings::default()
    );
    let validated = format!("{present:?}");
    assert!(matches_validated_snapshot(&validated, &present));
    assert!(!matches_validated_snapshot(&validated, &empty));
}

#[test]
fn snapshot_dumps_only_failures_and_preserves_raw_evidence() {
    // Reply-reference machinery is retired: clean observations produce only a
    // diagnostic digest, while exact validated expectations remain unchanged.
    let directory = support::fixture_directory();
    let out = directory.path();
    let now = Instant::now();
    let event = Event {
        key: 0, token: "0000000000000001".into(), kind: Kind::Append,
        invoked: now, completed: now, outcome: Outcome::Applied { length: 16, counter: None },
    };
    let mut observed = ObservedSnapshot {
        state: State { logs: vec![String::new(); KEYS], sets: vec![None; KEYS], counters: vec![None; KEYS] },
        request: resp(&["MGET", "fixture"]),
        raw: b"*1\r\n$-1\r\n".to_vec(),
    };
    observed.state.logs[0] = event.token.clone();
    let good = chaos_check::check(std::slice::from_ref(&event), &observed.state, now);
    assert_eq!(good, Findings::default());
    assert!(retain_failed_snapshot(out, 1, &observed, &good, false).unwrap().is_none());
    assert_eq!(fs::read_dir(out).unwrap().count(), 0, "clean snapshots must not dump values");

    let mut large = State {
        logs: vec![String::new(); KEYS],
        sets: vec![None; KEYS],
        counters: vec![None; KEYS],
    };
    let history: Vec<_> = (1..=256).map(|index| {
        let token = format!("feedface{index:08x}");
        large.logs[0].push_str(&token);
        Event {
            key: 0, token, kind: Kind::Append, invoked: now, completed: now,
            outcome: Outcome::Applied { length: index * 16, counter: None },
        }
    }).collect();
    assert_eq!(chaos_check::check(&history, &large, now), Findings::default());
    assert_eq!(large.logs[0].len(), 4096);
    let end_ns = Duration::from_secs(86400).as_nanos();
    let summary = snapshot_summary(6000, Some(&large), end_ns - 1_000_000, end_ns);
    let line = format!("{end_ns}\t{summary}\n");
    assert!(line.len() <= 160, "snapshot summary expanded to {} bytes", line.len());
    assert!(summary.contains("keys=12 "));
    assert!(summary.contains(&format!("digest=default-hasher64:{:016x}", snapshot_digest(&large))));
    assert!(!summary.contains("feedface"), "token values must not enter clean snapshot summaries");
    assert!(!summary.contains(&large.logs[0]));
    assert!(!summary.contains("State {"));

    observed.state.logs[0].clear();
    let lost = chaos_check::check(&[event], &observed.state, now);
    assert_eq!(lost.lost.len(), 1);
    let path = retain_failed_snapshot(out, 1, &observed, &lost, false).unwrap().unwrap();
    assert_eq!(fs::read(path.with_extension("resp")).unwrap(), observed.raw);
    assert_eq!(fs::read(path.with_extension("request")).unwrap(), observed.request);
    assert!(fs::read_to_string(path.with_extension("txt")).unwrap().contains("0000000000000001"));

    let continuity = retain_failed_snapshot(out, 2, &observed, &Findings::default(), true)
        .unwrap().expect("unknown-effect continuity mismatch must also retain evidence");
    assert!(fs::read_to_string(continuity.with_extension("txt")).unwrap().contains("baseline_mismatch=true"));
    let inconsistent = Findings { lost: vec![], inconsistent: vec!["duplicated effect".into()] };
    assert!(retain_failed_snapshot(out, 3, &observed, &inconsistent, false).unwrap().is_some());

    let mut partial = Vec::new();
    assert!(frame(&mut &b"$3\r\nab"[..], &mut partial, 0).is_err());
    let malformed = dump_snapshot(out, 4, &observed.request, &partial, None, "truncated RESP").unwrap();
    assert_eq!(fs::read(malformed.with_extension("resp")).unwrap(), b"$3\r\nab");
    assert!(snapshot_state(Frame::Array(Some(vec![]))).is_err());
    assert!(snapshot_state(Frame::Error("ERR fixture".into())).is_err());
}

#[test]
fn periodic_full_check_detects_corruption_in_an_old_epoch() {
    let origin = Instant::now();
    let mut states = Vec::new();
    let mut histories = Vec::new();
    for epoch in 1..=4 {
        let token = format!("{epoch:016x}");
        let mut state = State {
            logs: vec![String::new(); KEYS], sets: vec![None; KEYS], counters: vec![None; KEYS],
        };
        state.logs[0] = token.clone();
        histories.push(vec![Event {
            key: 0, token, kind: Kind::Append, invoked: origin, completed: origin,
            outcome: Outcome::Applied { length: 16, counter: None },
        }]);
        states.push(state);
    }
    // Run the real independent checker over exactly the scheduled range. A
    // scheduler that never returns to old epochs cannot pass the last assertion.
    let inspect = |plan: CheckPlan, states: &[State]| {
        (plan.first..=plan.last).flat_map(|epoch| {
            chaos_check::check(&histories[epoch - 1], &states[epoch - 1], origin).lost
        }).collect::<Vec<_>>()
    };
    let mut schedule = CheckScheduler::default();
    let first = schedule.next(1, origin);
    assert!(inspect(first, &states).is_empty());
    let full_finished = origin + Duration::from_secs(1);
    schedule.completed(first, &CheckRun { finished: full_finished, duration: Duration::from_secs(1) });
    states[0].logs[0].clear();
    let incremental = schedule.next(2, full_finished + Duration::from_secs(20));
    assert!(inspect(incremental, &states).is_empty());
    schedule.completed(incremental, &CheckRun {
        finished: full_finished + Duration::from_secs(21), duration: Duration::from_secs(1),
    });
    let later = schedule.next(3, full_finished + Duration::from_secs(40));
    assert_eq!(later.first, 2, "every epoch since the last full remains in incremental checks");
    assert!(inspect(later, &states).is_empty());
    schedule.completed(later, &CheckRun {
        finished: full_finished + Duration::from_secs(41), duration: Duration::from_secs(1),
    });
    let due = schedule.next(4, full_finished + Duration::from_secs(60));
    let losses = inspect(due, &states);
    assert_eq!(losses, vec![LostWrite { key: 0, token: "0000000000000001".into() }]);
}

#[test]
fn final_full_check_is_not_repeated_and_interval_tracks_cost() {
    let origin = Instant::now();
    let mut schedule = CheckScheduler::default();
    assert!(schedule.finish_plan(0).is_none(), "startup-only runs have no imaginary epochs");
    let first = schedule.next(1, origin);
    let finished = origin + Duration::from_secs(10);
    schedule.completed(first, &CheckRun { finished, duration: Duration::from_secs(10) });
    assert!(schedule.finish_plan(1).is_none(), "last full already covers the final epoch");
    assert_eq!(schedule.interval(), Duration::from_secs(190));
    assert_eq!(schedule.next(2, finished + Duration::from_secs(189)).kind, CheckKind::Incremental);
    assert_eq!(schedule.next(2, finished + Duration::from_secs(190)).kind, CheckKind::Full);
    let incremental = schedule.next(2, finished + Duration::from_secs(20));
    schedule.completed(incremental, &CheckRun {
        finished: finished + Duration::from_secs(21), duration: Duration::from_secs(1),
    });
    let final_plan = schedule.finish_plan(2).expect("deadline after an incremental needs a final full");
    assert_eq!(final_plan, CheckPlan { first: 1, last: 2, kind: CheckKind::Full });
    schedule.completed(final_plan, &CheckRun {
        finished: finished + Duration::from_secs(23), duration: Duration::from_secs(2),
    });
    assert!(schedule.finish_plan(2).is_none(), "final full executes exactly once");
    assert_eq!(schedule.interval(), Duration::from_secs(60), "short checks keep a 60-second floor");
    schedule.completed(final_plan, &CheckRun { finished, duration: Duration::ZERO });
    assert_eq!(schedule.last_full_duration, Duration::from_millis(1));
}

#[test]
fn phase_timings_preserve_partial_work_without_join_time() {
    let directory = support::fixture_directory();
    let origin = Instant::now();
    let shared = Shared::new(directory.path(), origin);
    let mut work = PhaseTimer::start(&shared, TimedPhase::Workload, origin);
    work.finish_at(origin + Duration::from_secs(5), true);
    assert!(shared.stop.load(Ordering::Acquire));
    work.finish_at(origin + Duration::from_secs(8), true); // Joining must not add time.
    let mut check = PhaseTimer::start(&shared, TimedPhase::Check(CheckKind::Full), origin + Duration::from_secs(8));
    check.finish_at(origin + Duration::from_secs(10), false); // Unwound partial check.
    drop(work);
    drop(check);
    let report = render_report_at(&shared, "failed", Some("fixture interrupted check"), Duration::from_secs(20));
    for expected in ["wall_seconds=20\n", "elapsed_seconds=20\n", "workload_seconds=5\n",
        "check_seconds=2\n", "check_fraction=0.1\n", "full_checks_started=1\n", "full_checks_completed=0\n"] {
        assert!(report.contains(expected), "missing {expected}: {report}");
    }
    // Exercise the actual unfinished guard's Drop during unwinding, without
    // sleeping or relying on an exact real-clock duration.
    *locked(&shared.timings) = Timings::default();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _check = PhaseTimer::start(
            &shared,
            TimedPhase::Check(CheckKind::Full),
            Instant::now() - Duration::from_secs(2),
        );
        panic!("fixture interrupts a checkpoint before explicit completion");
    }));
    assert!(interrupted.is_err());
    let timings = locked(&shared.timings);
    assert!(timings.check >= Duration::from_secs(2), "unwind must retain elapsed check work");
    assert_eq!(timings.checks_started[CheckKind::Full.index()], 1);
    assert_eq!(timings.checks_completed[CheckKind::Full.index()], 0);
}

#[test]
fn day_check_budget_rejects_more_than_ten_percent_but_not_smoke_or_boundary() {
    let directory = support::fixture_directory();
    let shared = Shared::new(directory.path(), Instant::now());
    *locked(&shared.run_plan) = Some(RunPlan {
        seconds: 120,
        device_configured: false,
        fault_skip_reason: "fixture".into(),
    });
    locked(&shared.timings).check = Duration::from_secs(9000);
    let wall = Duration::from_secs(86400);
    enforce_check_budget(&shared, wall);
    assert_eq!(shared.errors.load(Ordering::Relaxed), 0, "smoke runs do not enforce a day budget");
    locked(&shared.run_plan).as_mut().unwrap().seconds = 86400;
    locked(&shared.timings).check = Duration::from_secs(8640);
    enforce_check_budget(&shared, wall);
    assert_eq!(shared.errors.load(Ordering::Relaxed), 0, "ten percent is inclusive");
    locked(&shared.timings).check += Duration::from_nanos(1);
    enforce_check_budget(&shared, wall);
    assert_eq!(shared.errors.load(Ordering::Relaxed), 1, "excess checking must fail acceptance");
    let findings = fs::read_to_string(directory.path().join("findings.log")).unwrap();
    assert!(findings.contains("CHECK_BUDGET"));
}

#[test]
fn refused_restart_report_retains_partial_coverage_fault_context_and_bounded_tails() {
    let directory = support::fixture_directory();
    let out = directory.path();
    let shared = Shared::new(out, Instant::now());
    let c = Config {
        server: "redis".into(),
        seconds: 120,
        restart_seconds: 300,
        seed: 1,
        clients: 32,
        out: out.to_path_buf(),
        mount: None,
        fix: "unknown".into(),
        fault_skip_reason: "CHAOS_FAULT_MOUNT not provided".into(),
    };
    let mut stderr = vec![b'x'; 5000];
    stderr.extend_from_slice(b"\nREDIS_AOF_FAILURE\n");
    fs::write(out.join("server-3.stdout"), b"REDIS_STARTUP_STDOUT").unwrap();
    fs::write(out.join("server-3.stderr"), &stderr).unwrap();
    {
        let mut coverage = locked(&shared.fault_coverage);
        coverage.epochs_started = 3;
        coverage.epochs_completed = 2;
        coverage.sigkill_sent = 3;
        coverage.sigkill_reaped = 3;
        coverage.restart_attempts = 3;
        coverage.sigkill_restarts_completed = 2;
        coverage.write_eio_injected_windows = 1;
        coverage.flush_eio_injected_windows = 1;
        coverage.connection_drops = 5;
        coverage.faults.push(FaultRecord {
            kind: "combined write/flush EIO active",
            epoch: 3,
            at_ns: 100,
        });
        coverage.faults.push(FaultRecord {
            kind: "SIGKILL sent",
            epoch: 3,
            at_ns: 200,
        });
    }
    let failure = StartupFailure::capture(
        &c,
        &shared,
        3,
        "redis refused to load its append-only file".into(),
    );
    assert_eq!(
        failure.stderr_tail.len(),
        4096,
        "tail must be bounded, not the whole file"
    );
    assert!(failure.stderr_tail.ends_with("REDIS_AOF_FAILURE\n"));
    let status = record_startup_failure(&shared, failure);
    let report = render_report(&shared, &status, None);
    for expected in [
        "status=failed",
        "panic=None",
        "findings=1",
        "kind=\"refused restart\"",
        "recovery_check=not_executed_server_unavailable",
        "server=\"redis\"",
        "coverage.epochs_started=3",
        "coverage.epochs_completed=2",
        "coverage.sigkill_sent=3",
        "coverage.sigkill_reaped=3",
        "coverage.restart_attempts=3",
        "coverage.sigkill_restarts_completed=2",
        "coverage.write_eio_injected_windows=1",
        "coverage.flush_eio_injected_windows=1",
        "coverage.enospc_injected_windows=0",
        "combined write/flush EIO active",
        "SIGKILL sent",
        "REDIS_AOF_FAILURE",
        "REDIS_STARTUP_STDOUT",
    ] {
        assert!(report.contains(expected), "missing {expected}: {report}");
    }
    assert!(
        !report.contains("firn exited"),
        "Redis failure must retain its identity"
    );
    {
        let mut coverage = locked(&shared.fault_coverage);
        coverage.write_eio_injected_windows = 0;
        coverage.flush_eio_injected_windows = 0;
    }
    let before_disk = render_report(&shared, &status, None);
    assert!(before_disk.contains("combined write/flush EIO not injected: refused restart stopped execution for redis lifetime 3"));
    assert!(
        !before_disk.contains("no eligible third disk-failure epoch"),
        "an aborted run must not be labeled normal schedule expiry"
    );
}

#[test]
fn smoke_coverage_rejects_missing_fault_kinds_and_preserves_unwind_progress() {
    let directory = support::fixture_directory();
    let shared = Shared::new(directory.path(), Instant::now());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut coverage = locked(&shared.fault_coverage);
        coverage.epochs_started = 1;
        coverage.sigkill_sent = 1;
        coverage.sigkill_reaped = 1;
        coverage.restart_attempts = 1;
        coverage.connection_drops = 1;
        panic!("injected harness error after first kill");
    }));
    assert!(result.is_err());
    let report = render_report(
        &shared,
        "failed",
        Some("injected harness error after first kill"),
    );
    assert!(report.contains("coverage.sigkill_sent=1"));
    assert!(report.contains("coverage.sigkill_restarts_completed=0"));
    assert!(report.contains("panic=Some("));
    assert!(report.contains("remaining planned faults and checks aborted by harness error"));
    assert!(report.contains("combined write/flush EIO not injected: harness error"));
    assert!(report.contains("coverage.write_eio_injected_windows=0"));
    assert!(report.contains("coverage.flush_eio_injected_windows=0"));
    *locked(&shared.run_plan) = Some(RunPlan {
        seconds: 20,
        device_configured: true,
        fault_skip_reason: String::new(),
    });
    let short = render_report(&shared, "passed-functional-probe", None);
    assert!(short.contains("combined write/flush EIO not injected: no eligible third disk-failure epoch reached before the 20-second run deadline"));
    let mut coverage = locked(&shared.fault_coverage);
    assert_eq!(
        coverage.smoke_gaps(120, true).len(),
        3,
        "kill alone is not a restart or either EIO window"
    );
    coverage.sigkill_restarts_completed = 1;
    coverage.write_eio_injected_windows = 1;
    assert_eq!(
        coverage.smoke_gaps(120, true),
        vec!["no flush-EIO injection window"]
    );
    coverage.flush_eio_injected_windows = 1;
    assert!(coverage.smoke_gaps(120, true).is_empty());
    drop(coverage);
    let injected = render_report(&shared, "passed-functional-probe", None);
    assert!(
        !injected.contains("EIO not injected:"),
        "actual injected windows must not be called skipped"
    );
    let mut coverage = locked(&shared.fault_coverage);
    coverage.write_eio_injected_windows = 0;
    coverage.flush_eio_injected_windows = 0;
    assert!(
        coverage.smoke_gaps(120, false).is_empty(),
        "missing device is an explicit skip, not a synthetic injection"
    );
    coverage.connection_drops = 0;
    assert_eq!(
        coverage.smoke_gaps(120, false),
        vec!["no completed injected connection drop"]
    );
}

#[test]
fn seeded_schedule_reaches_first_combined_eio_epoch_within_75_active_seconds() {
    for seed in [0, 1, 2, 42, 147, u64::MAX] {
        let mut schedule = seed;
        let mut seconds = 0;
        for _ in 1..=3 {
            let epoch = epoch_seconds(&mut schedule);
            assert!((15..=25).contains(&epoch));
            seconds += epoch;
        }
        assert!(
            seconds <= 75,
            "120-second smoke has room for the third epoch before checkpoint overhead"
        );
    }
}

#[test]
fn kill_trigger_requires_fresh_ack_and_timeout_retains_prior_faults() {
    let now = Instant::now();
    let progress = Mutex::new(AckProgress {
        sequence: 7,
        last: Some(now),
    });
    let changed = Condvar::new();
    assert!(
        !wait_for_ack_after(&progress, &changed, 7, now, Duration::ZERO),
        "old acknowledgment is not fresh"
    );
    locked(&progress).sequence = 8;
    assert!(
        !wait_for_ack_after(
            &progress,
            &changed,
            7,
            now + Duration::from_millis(1),
            Duration::ZERO
        ),
        "a newly published reply completed before the cutoff is still stale"
    );
    assert!(
        wait_for_ack_after(&progress, &changed, 7, now, Duration::ZERO),
        "new acknowledgment should trigger immediately"
    );
    let directory = support::fixture_directory();
    let shared = Shared::new(directory.path(), now);
    locked(&shared.fault_coverage).faults.push(FaultRecord {
        kind: "combined write/flush EIO active",
        epoch: 3,
        at_ns: 100,
    });
    prepare_kill_with_limit(&shared, 3, true, Duration::ZERO);
    assert_eq!(
        shared.errors.load(Ordering::Relaxed),
        0,
        "disk refusal permits the explicit kill fallback"
    );
    let coverage = locked(&shared.fault_coverage);
    assert_eq!(
        coverage.sigkill_sent, 0,
        "waiting does not count a signal that was never sent"
    );
    assert_eq!(coverage.sigkill_restarts_completed, 0);
    assert!(
        coverage
            .report()
            .contains("combined write/flush EIO active")
    );
    drop(coverage);
    prepare_kill_with_limit(&shared, 4, false, Duration::ZERO);
    assert_eq!(
        shared.errors.load(Ordering::Relaxed),
        1,
        "healthy silence at kill trigger is no progress"
    );
}

#[test]
fn ack_publication_keeps_latest_completion_when_an_older_reply_is_published_last() {
    let now = Instant::now();
    let mut progress = AckProgress::default();
    progress.record(now + Duration::from_millis(2));
    progress.record(now);
    assert_eq!(
        progress.sequence, 2,
        "every complete acknowledgment advances freshness"
    );
    assert_eq!(
        progress.last,
        Some(now + Duration::from_millis(2)),
        "late publication must not move the latest completion backwards"
    );
}

#[test]
fn loss_attribution_uses_ack_age_and_actual_eio_overlap_instead_of_ever_failed_disk() {
    let origin = Instant::now();
    let killed = 10_000_000_000_u128;
    let event = Event {
        key: 0,
        token: "0000000000000001".into(),
        kind: Kind::Append,
        invoked: origin + Duration::from_nanos(9_999_000_000),
        completed: origin + Duration::from_nanos(9_999_500_000),
        outcome: Outcome::Applied {
            length: 16,
            counter: None,
        },
    };
    let lost = LostWrite {
        key: event.key,
        token: event.token.clone(),
    };
    let kill = FaultRecord {
        kind: "SIGKILL sent",
        epoch: 6,
        at_ns: killed,
    };
    let reap = FaultRecord {
        kind: "SIGKILL reaped",
        epoch: 6,
        at_ns: killed + 1_000_000,
    };
    let past = vec![
        FaultRecord {
            kind: "combined write/flush EIO active",
            epoch: 3,
            at_ns: 5_000_000_000,
        },
        FaultRecord {
            kind: "combined write/flush EIO healed",
            epoch: 3,
            at_ns: 8_000_000_000,
        },
        kill.clone(),
        reap.clone(),
    ];
    let classify = |event: &Event, faults: &[FaultRecord], fix: &str| {
        classify_lost_write(
            "firn",
            fix,
            &lost,
            std::slice::from_ref(event),
            origin,
            faults,
        )
    };
    let after_heal = classify(&event, &past, "absent");
    assert_eq!(
        after_heal.label, KNOWN_ACK_LOSS,
        "old healed EIO must not veto a new request"
    );
    assert_eq!(after_heal.ack_age_ns, Some(500_000));
    assert_eq!(after_heal.eio_overlap, EioEvidence::NoOverlap);
    for current_epoch in [3, 4, 5, 6] {
        let mut faults = past.clone();
        for fault in &mut faults {
            if matches!(fault.kind, "SIGKILL sent" | "SIGKILL reaped") {
                fault.epoch = current_epoch;
            }
        }
        assert_eq!(
            classify(&event, &faults, "absent").label,
            KNOWN_ACK_LOSS,
            "attribution uses times, not lifetime numbers or cumulative EIO"
        );
    }
    let during = vec![
        FaultRecord {
            kind: "combined write/flush EIO active",
            epoch: 6,
            at_ns: 9_998_000_000,
        },
        FaultRecord {
            kind: "combined write/flush EIO healed",
            epoch: 6,
            at_ns: 9_999_750_000,
        },
        kill.clone(),
        reap.clone(),
    ];
    assert_eq!(
        classify(&event, &during, "absent").label,
        OVERLAPPING_EIO_LOSS
    );
    assert!(
        matches!(classify(&event,&during,"absent").eio_overlap,EioEvidence::Overlap(ref windows) if windows.len()==1)
    );
    let intervening = vec![
        FaultRecord {
            kind: "combined write/flush EIO active",
            epoch: 6,
            at_ns: 9_999_750_000,
        },
        kill.clone(),
        reap.clone(),
    ];
    assert_eq!(
        classify(&event, &intervening, "absent").label,
        OVERLAPPING_EIO_LOSS,
        "EIO after acknowledgment but before kill also vetoes sole known-cause attribution"
    );
    assert!(
        matches!(classify(&event,&intervening,"absent").eio_overlap,EioEvidence::Overlap(ref windows) if windows[0].healed_ns.is_none())
    );
    let mut boundary = event.clone();
    boundary.invoked = origin + Duration::from_nanos(9_980_000_000);
    boundary.completed = origin + Duration::from_nanos(9_990_000_000);
    assert_eq!(
        classify(&boundary, &[kill.clone(), reap.clone()], "absent").label,
        KNOWN_ACK_LOSS,
        "nominal 10ms boundary is inclusive"
    );
    boundary.completed -= Duration::from_nanos(1);
    let outside = classify(&boundary, &[kill.clone(), reap.clone()], "absent");
    assert_eq!(outside.label, OUTSIDE_ACK_WINDOW_LOSS);
    assert!(
        outside
            .interpretation
            .contains("not proof of a separate firn defect")
    );
    assert_eq!(
        classify(&event, &past, "present").label,
        "lost-acknowledged-write"
    );
    assert_eq!(
        classify(&event, &past, "unknown").label,
        "firn-fix-status-unknown-still-fails"
    );
    let mut later = past.clone();
    later.push(FaultRecord {
        kind: "SIGKILL sent",
        epoch: 7,
        at_ns: killed + 1_000_000_000,
    });
    later.push(FaultRecord {
        kind: "SIGKILL reaped",
        epoch: 7,
        at_ns: killed + 1_001_000_000,
    });
    assert_eq!(
        classify(&event, &later, "absent").label,
        OUTSIDE_ACK_WINDOW_LOSS,
        "a loss first observed later cannot use the source epoch's older close kill"
    );
}

#[test]
fn loss_attribution_missing_or_reversed_metadata_cannot_establish_a_known_window() {
    let origin = Instant::now();
    let event = Event {
        key: 0,
        token: "0000000000000001".into(),
        kind: Kind::Append,
        invoked: origin + Duration::from_nanos(10),
        completed: origin + Duration::from_nanos(20),
        outcome: Outcome::Applied {
            length: 16,
            counter: None,
        },
    };
    let lost = LostWrite {
        key: 0,
        token: event.token.clone(),
    };
    let kill = FaultRecord {
        kind: "SIGKILL sent",
        epoch: 1,
        at_ns: 30,
    };
    let reap = FaultRecord {
        kind: "SIGKILL reaped",
        epoch: 1,
        at_ns: 40,
    };
    let paired = [kill.clone(), reap.clone()];
    let classify = |events: &[Event], faults: &[FaultRecord]| {
        classify_lost_write("firn", "absent", &lost, events, origin, faults)
    };
    assert_eq!(classify(&[], &paired).label, UNVERIFIED_ACK_LOSS);
    assert_eq!(
        classify(std::slice::from_ref(&event), &[]).label,
        UNVERIFIED_ACK_LOSS
    );
    let mut after_reap = event.clone();
    after_reap.completed = origin + Duration::from_nanos(41);
    assert_eq!(
        classify(&[after_reap], &paired).label,
        UNVERIFIED_ACK_LOSS
    );
    let mut reversed = event.clone();
    reversed.invoked = origin + Duration::from_nanos(21);
    assert_eq!(
        classify(&[reversed], &paired).label,
        UNVERIFIED_ACK_LOSS
    );
    let mut unknown = event.clone();
    unknown.outcome = Outcome::Unknown;
    assert_eq!(
        classify(&[unknown], &paired).label,
        UNVERIFIED_ACK_LOSS
    );
    assert_eq!(
        classify(&[event.clone(), event.clone()], &paired).label,
        UNVERIFIED_ACK_LOSS
    );
    let malformed = vec![
        FaultRecord {
            kind: "combined write/flush EIO healed",
            epoch: 1,
            at_ns: 5,
        },
        kill.clone(),
        reap.clone(),
    ];
    assert_eq!(
        classify(std::slice::from_ref(&event), &malformed).label,
        UNVERIFIED_ACK_LOSS
    );
    for (server, label) in [
        ("redis", "observation-under-everysec-not-always-sync"),
        ("valkey", "observation-under-everysec-not-always-sync"),
        ("garnet", "aof-commit-semantics-unverified"),
    ] {
        assert_eq!(
            classify_lost_write(
                server,
                "absent",
                &lost,
                std::slice::from_ref(&event),
                origin,
                &paired
            )
            .label,
            label
        );
    }
}

#[test]
fn post_signal_ack_before_reap_is_known_and_still_fails() {
    let directory = support::fixture_directory();
    let (origin, mut event, lost, faults) = post_signal_loss_fixture();
    let shared = Shared::new(directory.path(), origin);
    let classify = |event: &Event, fix: &str, faults: &[FaultRecord]| {
        classify_lost_write("firn", fix, &lost, std::slice::from_ref(event), origin, faults)
    };
    let attribution = classify(&event, "absent", &faults);
    assert_eq!(attribution.label, KNOWN_ACK_LOSS);
    assert_eq!(attribution.ack_offset_ns, Some(68_000));
    assert_eq!(attribution.ack_age_ns, None);
    assert_eq!(attribution.sigkill_reaped_ns, Some(1_001_000_000));
    assert_eq!(attribution.eio_overlap, EioEvidence::NoOverlap);
    let empty = State {
        logs: vec![String::new(); KEYS], sets: vec![None; KEYS], counters: vec![None; KEYS],
    };
    let findings = chaos_check::check(std::slice::from_ref(&event), &empty,
        origin + Duration::from_nanos(1_001_000_001));
    assert_eq!(findings.lost, vec![lost.clone()]);
    report_lost_write(&shared, 1, &lost, &attribution);
    assert_eq!(shared.errors.load(Ordering::Relaxed), 1);
    let log = fs::read_to_string(directory.path().join("findings.log")).unwrap();
    assert!(log.contains("ack_to_kill_offset_ns=Some(68000)"));
    assert!(log.contains("sigkill_reaped_ns=Some(1001000000)"));
    assert!(render_report(&shared, "failed", None).contains("findings=1"));
    assert_eq!(classify(&event, "present", &faults).label, "lost-acknowledged-write");
    assert_eq!(classify(&event, "unknown", &faults).label, "firn-fix-status-unknown-still-fails");

    for completed in [1_000_000_000, 1_001_000_000] {
        event.completed = origin + Duration::from_nanos(completed);
        assert_eq!(classify(&event, "absent", &faults).label, KNOWN_ACK_LOSS,
            "send and reap endpoints are inclusive");
    }
    let mut overlapping = vec![FaultRecord {
        kind: "combined write/flush EIO active", epoch: 1, at_ns: 999_000_000,
    }];
    overlapping.extend(faults.iter().cloned());
    assert_eq!(classify(&event, "absent", &overlapping).label, OVERLAPPING_EIO_LOSS);
}

fn post_signal_loss_fixture() -> (Instant, Event, LostWrite, [FaultRecord; 2]) {
    let origin = Instant::now();
    let event = Event {
        key: 0,
        token: "0000000000000001".into(),
        kind: Kind::Append,
        invoked: origin + Duration::from_nanos(999_999_000),
        completed: origin + Duration::from_nanos(1_000_068_000),
        outcome: Outcome::Applied { length: 16, counter: None },
    };
    let lost = LostWrite { key: 0, token: event.token.clone() };
    let faults = [
        FaultRecord { kind: "SIGKILL sent", epoch: 1, at_ns: 1_000_000_000 },
        FaultRecord { kind: "SIGKILL reaped", epoch: 1, at_ns: 1_001_000_000 },
    ];
    (origin, event, lost, faults)
}

#[test]
fn ack_after_sigkill_reap_is_unverified() {
    let (origin, mut event, lost, faults) = post_signal_loss_fixture();
    event.completed = origin + Duration::from_nanos(1_001_000_001);
    let attribution = classify_lost_write("firn", "absent", &lost,
        std::slice::from_ref(&event), origin, &faults);
    assert_eq!(attribution.label, UNVERIFIED_ACK_LOSS);
    assert_eq!(attribution.ack_offset_ns, Some(1_000_001));
    assert_eq!(attribution.sigkill_reaped_ns, Some(1_001_000_000));
    event.invoked = origin + Duration::from_nanos(1_000_000_001);
    event.completed = origin + Duration::from_nanos(1_000_068_000);
    assert_eq!(classify_lost_write("firn", "absent", &lost,
        std::slice::from_ref(&event), origin, &faults).label,
        KNOWN_ACK_LOSS, "asynchronous delivery permits invocation after send and completion before reap");
    let mut active = vec![FaultRecord {
        kind: "combined write/flush EIO active", epoch: 1, at_ns: 999_000_000,
    }];
    active.extend(faults.iter().cloned());
    assert_eq!(classify_lost_write("firn", "absent", &lost,
        std::slice::from_ref(&event), origin, &active).label,
        KNOWN_ACK_LOSS, "the requested invocation-to-send EIO interval is empty");
    active[0].kind = "combined write/flush EIO healed";
    assert_eq!(classify_lost_write("firn", "absent", &lost,
        std::slice::from_ref(&event), origin, &active).label,
        UNVERIFIED_ACK_LOSS, "empty intervals must not bypass malformed EIO metadata");
    active[0].at_ns = 1_000_010_000;
    assert_eq!(classify_lost_write("firn", "absent", &lost,
        std::slice::from_ref(&event), origin, &active).label,
        UNVERIFIED_ACK_LOSS, "metadata validation also covers EIO records after signal submission");
}

#[test]
fn missing_or_ambiguous_sigkill_reap_is_unverified() {
    let (origin, mut event, lost, faults) = post_signal_loss_fixture();
    let [sent, reaped] = faults;
    let invalid = [
        vec![sent.clone()],
        vec![sent.clone(), FaultRecord { epoch: 2, ..reaped.clone() }],
        vec![sent.clone(), FaultRecord { at_ns: sent.at_ns - 1, ..reaped.clone() }],
        vec![reaped.clone(), sent.clone()],
        vec![sent.clone(), reaped.clone(), reaped.clone()],
        vec![sent.clone(), sent.clone(), reaped.clone()],
        vec![sent.clone(), reaped.clone(), FaultRecord {
            kind: "SIGKILL sent", epoch: 2, at_ns: 2_000_000_000,
        }],
    ];
    // Both a post-signal reply and an ordinary pre-signal reply require the
    // matching current reap; neither can borrow an earlier epoch's boundary.
    for completed in [1_000_068_000, 999_999_500] {
        event.completed = origin + Duration::from_nanos(completed);
        for records in &invalid {
            let attribution = classify_lost_write("firn", "absent", &lost,
                std::slice::from_ref(&event), origin, records);
            assert_eq!(attribution.label, UNVERIFIED_ACK_LOSS, "accepted malformed pair {records:?}");
        }
    }
}

#[test]
fn six_known_window_losses_remain_six_failures_after_diagnostic_reclassification() {
    let directory = support::fixture_directory();
    let origin = Instant::now();
    let shared = Shared::new(directory.path(), origin);
    let gaps = [261_800_u64, 63_465, 503_733, 159_404, 256_673, 166_939];
    let mut faults = Vec::new();
    for (index, gap) in gaps.into_iter().enumerate() {
        let epoch = index + 1;
        let killed = epoch as u64 * 10_000_000_000;
        if epoch % 3 == 0 {
            faults.push(FaultRecord {
                kind: "combined write/flush EIO active",
                epoch,
                at_ns: (killed - 5_000_000_000) as u128,
            });
            faults.push(FaultRecord {
                kind: "combined write/flush EIO healed",
                epoch,
                at_ns: (killed - 2_000_000_000) as u128,
            });
        }
        faults.push(FaultRecord {
            kind: "SIGKILL sent",
            epoch,
            at_ns: killed as u128,
        });
        faults.push(FaultRecord {
            kind: "SIGKILL reaped",
            epoch,
            at_ns: (killed + 1_000_000) as u128,
        });
        let event = Event {
            key: 0,
            token: format!("{epoch:016x}"),
            kind: Kind::Append,
            invoked: origin + Duration::from_nanos(killed - gap - 10_000),
            completed: origin + Duration::from_nanos(killed - gap),
            outcome: Outcome::Applied {
                length: 16,
                counter: None,
            },
        };
        let empty = State {
            logs: vec![String::new(); KEYS],
            sets: vec![None; KEYS],
            counters: vec![None; KEYS],
        };
        let findings = chaos_check::check(
            std::slice::from_ref(&event),
            &empty,
            origin + Duration::from_nanos(killed + 1),
        );
        assert_eq!(
            findings.lost.len(),
            1,
            "classification cannot erase checker evidence"
        );
        let attribution = classify_lost_write(
            "firn",
            "absent",
            &findings.lost[0],
            std::slice::from_ref(&event),
            origin,
            &faults,
        );
        assert_eq!(attribution.label, KNOWN_ACK_LOSS);
        assert_eq!(attribution.ack_age_ns, Some(gap as u128));
        assert_eq!(attribution.ack_offset_ns, Some(-(gap as i128)));
        report_lost_write(&shared, epoch, &findings.lost[0], &attribution);
    }
    assert_eq!(
        shared.errors.load(Ordering::Relaxed),
        6,
        "known cause never converts acknowledged losses to acceptance"
    );
    let report = render_report(&shared, "failed", None);
    assert!(report.contains("status=failed"));
    assert!(report.contains("findings=6"));
    let log = fs::read_to_string(directory.path().join("findings.log")).unwrap();
    assert!(log.contains("ack_to_kill_offset_ns=Some(-261800)"));
    assert!(log.contains("sigkill_reaped_ns=Some(10001000000)"));
}
