//! The append-only file's sync failing on a real device, with Redis 7.2.16
//! taken through the same steps first as the oracle.
//!
//! tests/aof_sync_fault_device.sh mounts an ext4 filesystem without a
//! journal on a device-mapper device that `fail` swaps to dm-flakey with
//! error_writes: a write(2) to the file still lands in the page cache, and
//! the next fsync fails with EIO. Redis 7.2.16 latches that in its
//! background fsync (bio.c:259-276) as aof_bio_fsync_status and
//! aof_bio_fsync_errno; processCommand (server.c:4033-4055) then refuses
//! WRITE commands and PING with writeCommandsDeniedByDiskError's MISCONF and
//! strerror of that errno (server.c:4483-4514), and INFO reports
//! aof_last_write_status:err for it (server.c:5812-5813), leaving
//! aof_last_bgrewrite_status alone. Redis 7.0.15 takes the same path here
//! (aof.c:1062-1263, bio.c:231-248, server.c:4324-4355,5631-5632).
//!
//! Redis resubmits a sync only after new bytes reach the file (aof.c:1071-
//! 1094,1261-1267) and serverCron retries only failed writes (server.c:1447-
//! 1457), so once writes are refused an idle Redis keeps refusing them after
//! the device heals, as the case observes. firn retries its sync once a
//! second while idle, the deliberate difference
//! design/firn/append-only-file.md records, and so accepts writes again
//! without a restart.
//!
//! The case is ignored in `make check`, which has no such device; the
//! `aof-sync-fault` job of .github/workflows/check.yml makes it and runs the
//! case alone with `--ignored`, failing when the device cannot be made.

use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const POLL_LIMIT: Duration = Duration::from_secs(10);
/// Two of firn's and Redis's once-a-second syncs and a margin, so the seed is
/// on the device before it fails.
const SETTLE: Duration = Duration::from_millis(2500);
/// How long Redis is watched still refusing after the device heals: three of
/// firn's idle sync retries.
const IDLE_WATCH: Duration = Duration::from_secs(3);
const MISCONF: &str = "-MISCONF Errors writing to the AOF file: ";
/// The persistence fields a failed sync sets or must leave as they were, the
/// same in Redis and firn.
const FIELDS: &[&str] = &[
    "aof_enabled", "aof_rewrite_in_progress", "aof_last_bgrewrite_status",
    "aof_last_write_status", "aof_buffer_length", "aof_pending_bio_fsync",
    "aof_delayed_fsync",
];
const SEED: &[&[&str]] = &[
    &["SET", "k1", "v1"], &["SET", "k2", "v2"], &["RPUSH", "l", "a", "b"],
];
/// Written once the device fails, before the server can know it has.
const DURING: &[&str] = &["SET", "during", "w"];
/// Write commands and PING, each refused while the sync is failing.
const REFUSED: &[&[&str]] = &[
    &["SET", "k1", "x"], &["INCR", "n"], &["DEL", "k1"], &["RPUSH", "l", "c"],
    &["EXPIRE", "k1", "100"], &["PING"],
];
/// Reads of the seed, each answered while the sync is failing.
const READS: &[&[&str]] = &[
    &["GET", "k1"], &["MGET", "k1", "k2"], &["LRANGE", "l", "0", "-1"],
    &["EXISTS", "k1", "k2", "n"], &["TTL", "k1"], &["STRLEN", "k2"],
];

/// The faulty device and the mount the workflow made on it.
struct Device {
    mount: PathBuf,
    script: PathBuf,
}

impl Device {
    fn from_environment() -> Self {
        let mount = std::env::var_os("FIRN_SYNC_FAULT_MOUNT").map(PathBuf::from).expect(
            "FIRN_SYNC_FAULT_MOUNT must name the mount aof_sync_fault_device.sh setup made");
        assert!(mount.is_absolute() && mount.is_dir(), "no mount at {}", mount.display());
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("aof_sync_fault_device.sh");
        Self { mount, script }
    }

    fn run(&self, action: &str) {
        let output = support::run_command(Command::new("sudo").arg("-n").arg(&self.script).arg(action));
        assert!(output.status.success(), "device {action}: {output:?}");
    }
}

/// What a server showed while its sync was failing.
struct Failure {
    during: String,
    refused: Vec<String>,
    read: Vec<String>,
    fields: BTreeMap<String, String>,
}

fn persistence(client: &mut TcpStream) -> String {
    client.write_all(&resp(&["INFO", "persistence"])).unwrap();
    bulk_reply(client, "INFO persistence")
}

fn fields(info: &str) -> BTreeMap<String, String> {
    FIELDS.iter().map(|name| {
        let value = info_field(info, name).unwrap_or_else(|| panic!("INFO lacks {name}: {info}"));
        (name.to_string(), value)
    }).collect()
}

fn size(info: &str) -> u64 {
    info_field(info, "aof_current_size").expect("aof_current_size").parse().unwrap()
}

/// Polls until the status is `expected` with no background sync pending;
/// Redis's bio thread sets the status before it leaves the pending count.
fn wait_status(client: &mut TcpStream, expected: &str) -> String {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        if info_field(&info, "aof_last_write_status").as_deref() == Some(expected)
            && info_field(&info, "aof_pending_bio_fsync").as_deref() == Some("0") {
            return info;
        }
        assert!(Instant::now() < deadline, "wanted AOF status {expected}: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop(client: &mut TcpStream, child: ProgramChild) {
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    shutdown_finished(child);
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
}

/// Seeds the server, lets it sync, fails the device, writes once and reads
/// what the server shows once its sync has failed.
fn fail(device: &Device, client: &mut TcpStream) -> Failure {
    for command in SEED {
        assert!(!memory_request(client, command).starts_with('-'), "seed {command:?}");
    }
    std::thread::sleep(SETTLE);
    let before = wait_status(client, "ok");
    device.run("fail");
    let during = memory_request(client, DURING);
    let failed = wait_status(client, "err");
    if during == "+OK\r\n" {
        // The write reached the file and only its sync failed: Redis adds a
        // write's bytes to aof_current_size once write(2) has taken them all
        // (aof.c:1224-1225), and nothing stays buffered.
        assert_eq!(size(&failed) - size(&before), resp(DURING).len() as u64, "{failed}");
    }
    let refused = REFUSED.iter().map(|command| memory_request(client, command)).collect();
    let read = READS.iter().map(|command| memory_request(client, command)).collect();
    // The refused commands added nothing to the file.
    assert_eq!(size(&persistence(client)), size(&failed));
    Failure { during, refused, read, fields: fields(&failed) }
}

fn start_redis(binary: &Path, directory: &Path, log: &Path) -> (ProgramChild, TcpStream) {
    let port = free_port().to_string();
    let mut command = Command::new(binary);
    command.current_dir(directory)
        .args(["--port", port.as_str(), "--bind", "127.0.0.1", "--save", "", "--appendonly", "yes",
            "--appendfilename", "sync.aof", "--appendfsync", "everysec", "--dir"])
        .arg(directory).arg("--logfile").arg(log)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = ProgramChild::spawn(&mut command).expect("spawn redis-server");
    (child, connect_when_ready(port.parse().unwrap()))
}

fn start_firn(program: &CompiledProgram, native: bool) -> (ProgramChild, TcpStream) {
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(native, &[text.as_bytes(), b"0", b"sync.aof"]);
    (child, connect_when_ready(port))
}

/// Redis 7.2.16 through the failure, then watched staying refused while
/// idle once the device heals.
fn redis_failure(device: &Device) -> Failure {
    let binary = std::env::var_os("REDIS_SERVER").map(PathBuf::from)
        .expect("REDIS_SERVER must name redis-server 7.2.16, the oracle");
    let version = support::run_command(Command::new(&binary).arg("--version"));
    assert!(String::from_utf8_lossy(&version.stdout).contains(" v=7.2.16 "), "{version:?}");
    let directory = device.mount.join(format!("redis-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let log = std::env::temp_dir().join(format!("firn-sync-fault-redis-{}.log", std::process::id()));
    let (child, mut client) = start_redis(&binary, &directory, &log);
    let failure = fail(device, &mut client);
    assert_eq!(failure.during, "+OK\r\n", "Redis syncs only after new bytes, so it acknowledges them");
    let misconf = &failure.refused[0];
    assert!(misconf.starts_with(MISCONF) && misconf.len() > MISCONF.len() + 2, "{misconf:?}");
    for reply in &failure.refused {
        assert_eq!(reply, misconf, "every refusal names the failed sync's error");
    }
    for reply in &failure.read {
        assert!(!reply.starts_with('-'), "reads still answer: {reply:?}");
    }
    assert_eq!(failure.fields["aof_last_write_status"], "err");
    assert_eq!(failure.fields["aof_last_bgrewrite_status"], "ok");
    assert_eq!(failure.fields["aof_buffer_length"], "0");
    device.run("heal");
    let deadline = Instant::now() + IDLE_WATCH;
    while Instant::now() < deadline {
        let info = persistence(&mut client);
        assert_eq!(info_field(&info, "aof_last_write_status").as_deref(), Some("err"),
            "an idle Redis resubmits no sync once the device heals: {info}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(&memory_request(&mut client, &["SET", "after", "p"]), misconf);
    // Redis is not restarted: it accepted no write after the device healed,
    // so it has no recovery of its own to compare, and its write `during`
    // lies in pages Linux stopped counting as dirty when their writeback
    // failed.
    stop(&mut client, child);
    std::fs::remove_dir_all(&directory).unwrap();
    failure
}

/// (a) firn refuses writes and PING with Redis's MISCONF while answering
/// reads as Redis does; (b) its INFO reports the failure with Redis's
/// fields; (c) it accepts writes again once the device heals, with no
/// restart and no client write to prompt it; (d) everything it acknowledged
/// is in the file a restart reads from the device.
///
/// (d) reads after a remount, so only what reached the device counts. The
/// write acknowledged while the device was failing reaches it only because
/// the write after recovery dirties the same page again: Linux leaves the
/// pages whose writeback failed clean, so firn's successful retry alone
/// would not rewrite them.
#[test]
#[ignore = "needs the faulty device the aof-sync-fault job of .github/workflows/check.yml makes"]
fn aof_sync_failure_refuses_writes_and_recovers_on_a_failing_device() {
    let device = Device::from_environment();
    let redis = redis_failure(&device);
    for native in [false, true] {
        let route = if native { "native ring" } else { "file adapter" };
        let program = CompiledProgram::from_environment_under(&device.mount);
        let (child, mut client) = start_firn(&program, native);
        let failure = fail(&device, &mut client);
        // firn also syncs while idle, so its sync can fail before this write
        // arrives, and then the write is refused as the ones below are.
        let acknowledged = failure.during == "+OK\r\n";
        if !acknowledged {
            assert_eq!(&failure.during, &redis.refused[0], "{route}");
        }
        assert_eq!(failure.refused, redis.refused, "{route}: (a) writes and PING refused as Redis refuses them");
        assert_eq!(failure.read, redis.read, "{route}: (a) reads answered as Redis answers them");
        assert_eq!(failure.fields, redis.fields, "{route}: (b) INFO reports the failed sync as Redis does");
        let info = persistence(&mut client);
        assert_eq!(info_field(&info, "firn_aof_rewrite_abort_reason").as_deref(), Some(""),
            "{route}: (b) a failed sync with no rewrite running aborts none: {info}");
        device.run("heal");
        let recovered = wait_status(&mut client, "ok");
        assert_eq!(info_field(&recovered, "aof_buffer_length").as_deref(), Some("0"), "{route}");
        assert_eq!(memory_request(&mut client, &["SET", "after", "p"]), "+OK\r\n", "{route}: (c) writes accepted again");
        assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n", "{route}: (c)");
        stop(&mut client, child);
        device.run("remount");
        let (child, mut client) = start_firn(&program, native);
        let during = if acknowledged { "$1\r\nw\r\n" } else { "$-1\r\n" };
        for (command, expected) in [
            (&["GET", "k1"][..], "$2\r\nv1\r\n"), (&["GET", "k2"][..], "$2\r\nv2\r\n"),
            (&["LRANGE", "l", "0", "-1"][..], "*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            (&["GET", "during"][..], during), (&["GET", "after"][..], "$1\r\np\r\n"),
            (&["EXISTS", "n"][..], ":0\r\n"),
        ] {
            assert_eq!(memory_request(&mut client, command), expected, "{route}: (d) {command:?} after a restart");
        }
        stop(&mut client, child);
    }
}
