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
//! the device heals, as the case observes. firn keeps syncing once a second
//! while idle and recovers without a restart, the deliberate difference
//! design/firn/append-only-file.md records.
//!
//! When Linux fails to write a file's dirty pages back, it leaves them clean
//! and records the error for the next fsync of the file to report once; a
//! later fsync that succeeds does not write those pages again. So bytes a
//! server acknowledged before a failed sync reach the device afterwards only
//! if a later write dirties their page (with a filesystem block smaller than
//! a page, their block) again. firn therefore does not take a successful
//! sync after a failed one as recovery: it rewrites the file from memory into
//! a new base, incremental file and manifest, and accepts writes again only
//! once that rewrite is installed and its directory synced. Until then it
//! keeps refusing writes with the same MISCONF and INFO keeps reporting
//! aof_last_write_status:err. The first case also holds the recovery rewrite
//! back, by refusing its start through maxmemory, and checks that firn keeps
//! refusing for as long as the rewrite cannot run. The second case,
//! aof_sync_failure_keeps_a_write_acknowledged_at_a_page_end, pads the file
//! so that the write acknowledged just before the failure ends at a page end
//! and the write after recovery starts on the next page, where no later write
//! dirties the failed page again, then reads what a restart finds once the
//! filesystem is remounted. It records Redis 7.2.16's outcome as an
//! observation and requires firn to keep every write it acknowledged. Both
//! servers sync only after new bytes, so the case fails the device while they
//! are idle, sends the write, which both acknowledge, and lets the sync of
//! that write fail.
//!
//! The cases are ignored in `make check`, which has no such device; the
//! `aof-sync-fault` job of .github/workflows/check.yml makes it and runs both
//! cases one after the other through `make aof-sync-fault`, failing when the
//! device cannot be made. They share the device, so they never run at once,
//! and a case that fails while the device fails heals it for the next.

use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const POLL_LIMIT: Duration = Duration::from_secs(10);
/// Two of firn's and Redis's once-a-second syncs and a margin, so the seed is
/// on the device before it fails.
const SETTLE: Duration = Duration::from_millis(2500);
/// How long Redis is watched still refusing after the device heals, and firn
/// while its recovery rewrite cannot start: three of firn's once-a-second
/// syncs, each of which succeeds once the device heals and so starts another
/// recovery attempt.
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
/// The write whose record the page-end case ends at a page end, and which the
/// first case writes once the device fails, before the server can know it has.
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

/// Heals the device when a case fails, so that a case failing while the
/// device fails leaves the next case of the module a working device.
struct HealOnPanic<'a>(&'a Device);

impl Drop for HealOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = Command::new("sudo").arg("-n").arg(&self.0.script).arg("heal").status();
        }
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

/// Polls until the status is `expected` with no background sync pending and
/// nothing buffered. Redis's bio thread sets the status before it leaves the
/// pending count, and firn reports the pending count as always 0 and takes
/// its buffer a little before its sync in the same tick, so a write
/// acknowledged while the sync fails can still be buffered, and not yet in
/// aof_current_size, when the status turns err. Redis writes its buffer in
/// beforeSleep with nothing buffered once no sync is pending, so waiting for
/// aof_buffer_length 0 holds for both servers.
fn wait_status(client: &mut TcpStream, expected: &str) -> String {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        if info_field(&info, "aof_last_write_status").as_deref() == Some(expected)
            && info_field(&info, "aof_pending_bio_fsync").as_deref() == Some("0")
            && info_field(&info, "aof_buffer_length").as_deref() == Some("0") {
            return info;
        }
        assert!(Instant::now() < deadline, "wanted AOF status {expected} with nothing buffered: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until firn has recovered from the failed sync, checking on every
/// INFO sample that it does not report the AOF healthy while a rewrite is
/// still running: the recovery rewrite clears the saved error in the same
/// metadata statement that ends it, so a server that accepted writes again
/// before its new files were installed would show both.
fn wait_recovered(client: &mut TcpStream, route: &str) -> String {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        let healthy = info_field(&info, "aof_last_write_status").as_deref() == Some("ok");
        let rewriting = info_field(&info, "aof_rewrite_in_progress").as_deref() == Some("1");
        assert!(!(healthy && rewriting), "{route}: writes accepted again before the recovery rewrite ended: {info}");
        if healthy && info_field(&info, "aof_buffer_length").as_deref() == Some("0") {
            return info;
        }
        assert!(Instant::now() < deadline, "{route}: no recovery: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn count(info: &str, name: &str) -> u64 {
    info_field(info, name).unwrap_or_else(|| panic!("INFO lacks {name}: {info}")).parse().unwrap()
}

/// The branch a server took for the write made once the device failed, named
/// in the log so a run shows which path its assertions followed.
fn branch(during: &str) -> String {
    if during == "+OK\r\n" {
        "acknowledged".to_string()
    } else {
        format!("refused with {during:?}")
    }
}

fn stop(client: &mut TcpStream, child: ProgramChild) {
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    shutdown_finished(child);
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
}

/// Writes the seed and lets the server sync it.
fn seed(client: &mut TcpStream) -> String {
    for command in SEED {
        assert!(!memory_request(client, command).starts_with('-'), "seed {command:?}");
    }
    std::thread::sleep(SETTLE);
    wait_status(client, "ok")
}

/// Seeds the server, lets it sync, fails the device, writes once and reads
/// what the server shows once its sync has failed.
fn fail(device: &Device, client: &mut TcpStream) -> Failure {
    let before = seed(client);
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

fn spawn_redis(binary: &Path, directory: &Path, log: &Path) -> (ProgramChild, u16) {
    let port = free_port().to_string();
    let mut command = Command::new(binary);
    command.current_dir(directory)
        .args(["--port", port.as_str(), "--bind", "127.0.0.1", "--save", "", "--appendonly", "yes",
            "--appendfilename", "sync.aof", "--appendfsync", "everysec", "--dir"])
        .arg(directory).arg("--logfile").arg(log)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = ProgramChild::spawn(&mut command).expect("spawn redis-server");
    (child, port.parse().unwrap())
}

fn start_redis(binary: &Path, directory: &Path, log: &Path) -> (ProgramChild, TcpStream) {
    let (child, port) = spawn_redis(binary, directory, log);
    (child, connect_when_ready(port))
}

fn spawn_firn(program: &CompiledProgram, native: bool) -> (ProgramChild, u16) {
    let port = free_port();
    let text = port.to_string();
    (program.spawn_on_route(native, &[text.as_bytes(), b"0", b"sync.aof"]), port)
}

fn start_firn(program: &CompiledProgram, native: bool) -> (ProgramChild, TcpStream) {
    let (child, port) = spawn_firn(program, native);
    (child, connect_when_ready(port))
}

/// The redis-server REDIS_SERVER names, checked to be 7.2.16, the oracle.
fn oracle() -> PathBuf {
    let binary = std::env::var_os("REDIS_SERVER").map(PathBuf::from)
        .expect("REDIS_SERVER must name redis-server 7.2.16, the oracle");
    let version = support::run_command(Command::new(&binary).arg("--version"));
    assert!(String::from_utf8_lossy(&version.stdout).contains(" v=7.2.16 "), "{version:?}");
    binary
}

/// Redis 7.2.16 through the failure, then watched staying refused while
/// idle once the device heals.
fn redis_failure(device: &Device) -> Failure {
    let binary = oracle();
    let directory = device.mount.join(format!("redis-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let log = std::env::temp_dir().join(format!("firn-sync-fault-redis-{}.log", std::process::id()));
    let (child, mut client) = start_redis(&binary, &directory, &log);
    let failure = fail(device, &mut client);
    eprintln!("redis 7.2.16: the write during the failure was {}", branch(&failure.during));
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
/// fields; (c) once the device heals it keeps refusing writes for as long as
/// its recovery rewrite cannot start, here because maxmemory 1 refuses the
/// rewrite's admission; (d) it accepts writes again once that rewrite is
/// installed, with no restart and no client write to prompt it; (e)
/// everything it acknowledged is in the file a restart reads from the
/// device.
///
/// (c) is what a server that took its successful retried sync as recovery
/// fails: its sync succeeds within a second of the heal, so it would accept
/// writes during the watch. (e) reads after a remount, so only what reached
/// the device counts. Here the write after recovery would also dirty the
/// page of the write acknowledged during the failure again; the page-end
/// case below takes that rescue away.
#[test]
#[ignore = "needs the faulty device the aof-sync-fault job of .github/workflows/check.yml makes"]
fn aof_sync_failure_refuses_writes_and_recovers_on_a_failing_device() {
    let device = Device::from_environment();
    let _heal = HealOnPanic(&device);
    let redis = redis_failure(&device);
    for native in [false, true] {
        let route = if native { "native ring" } else { "file adapter" };
        let program = CompiledProgram::from_environment_under(&device.mount);
        let (child, mut client) = start_firn(&program, native);
        let failure = fail(&device, &mut client);
        // firn also syncs while idle, so its sync can fail before this write
        // arrives, and then the write is refused as the ones below are.
        // Redis syncs only after new bytes, so it always acknowledges this
        // write (asserted in redis_failure), and the two branches may
        // legitimately differ: the case asserts what each branch requires
        // and logs the one taken.
        let acknowledged = failure.during == "+OK\r\n";
        eprintln!("firn, {route}: the write during the failure was {}", branch(&failure.during));
        if !acknowledged {
            assert_eq!(&failure.during, &redis.refused[0], "{route}");
        }
        assert_eq!(failure.refused, redis.refused, "{route}: (a) writes and PING refused as Redis refuses them");
        assert_eq!(failure.read, redis.read, "{route}: (a) reads answered as Redis answers them");
        assert_eq!(failure.fields, redis.fields, "{route}: (b) INFO reports the failed sync as Redis does");
        let info = persistence(&mut client);
        assert_eq!(info_field(&info, "firn_aof_rewrite_abort_reason").as_deref(), Some(""),
            "{route}: (b) a failed sync with no rewrite running aborts none: {info}");
        let rewrites = count(&info, "aof_rewrites");
        // maxmemory 1 refuses every rewrite's admission (R plus its headroom
        // exceeds it) without touching a key: the policy stays noeviction,
        // and PING and DEL are not DENYOOM, so they meet MISCONF, not OOM.
        assert_eq!(memory_request(&mut client, &["CONFIG", "SET", "maxmemory", "1"]), "+OK\r\n", "{route}");
        let misconf = &redis.refused[0];
        device.run("heal");
        let deadline = Instant::now() + POLL_LIMIT;
        loop {
            let info = persistence(&mut client);
            assert_eq!(info_field(&info, "aof_last_write_status").as_deref(), Some("err"),
                "{route}: (c) no recovery before its rewrite: {info}");
            if info_field(&info, "firn_aof_rewrite_abort_reason").as_deref() == Some("admission") {
                assert_eq!(info_field(&info, "aof_last_bgrewrite_status").as_deref(), Some("err"), "{route}: {info}");
                assert_eq!(info_field(&info, "aof_rewrite_in_progress").as_deref(), Some("0"), "{route}: {info}");
                assert_eq!(count(&info, "aof_rewrites"), rewrites, "{route}: a refused start is not a rewrite: {info}");
                break;
            }
            assert!(Instant::now() < deadline, "{route}: the healed sync started no recovery rewrite: {info}");
            std::thread::sleep(Duration::from_millis(20));
        }
        let watch = Instant::now() + IDLE_WATCH;
        while Instant::now() < watch {
            assert_eq!(&memory_request(&mut client, &["PING"]), misconf, "{route}: (c) PING refused while recovery cannot run");
            assert_eq!(&memory_request(&mut client, &["DEL", "k1"]), misconf, "{route}: (c) DEL refused while recovery cannot run");
            let info = persistence(&mut client);
            assert_eq!(info_field(&info, "aof_last_write_status").as_deref(), Some("err"), "{route}: (c) {info}");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(memory_request(&mut client, &["CONFIG", "SET", "maxmemory", "0"]), "+OK\r\n", "{route}");
        let recovered = wait_recovered(&mut client, route);
        assert_eq!(info_field(&recovered, "aof_last_bgrewrite_status").as_deref(), Some("ok"), "{route}: (d) {recovered}");
        assert_eq!(info_field(&recovered, "firn_aof_rewrite_abort_reason").as_deref(), Some(""), "{route}: (d) {recovered}");
        assert!(count(&recovered, "aof_rewrites") > rewrites, "{route}: (d) recovery rewrote the file: {recovered}");
        assert_eq!(memory_request(&mut client, &["SET", "after", "p"]), "+OK\r\n", "{route}: (d) writes accepted again");
        assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n", "{route}: (d)");
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
            assert_eq!(memory_request(&mut client, command), expected, "{route}: (e) {command:?} after a restart");
        }
        stop(&mut client, child);
    }
}

/// The page size of the x86-64 hosts the job runs on, checked at the start
/// of the case: the page cache tracks dirty data per page, and per block of
/// the filesystem within one.
const PAGE: u64 = 4096;
/// Written once the device has healed and the server syncs again.
const AFTER: &[&str] = &["SET", "after", "p"];
/// What a restart is asked, by the name each observation gives it.
const KEPT: &[(&str, &[&str])] = &[
    ("during", &["GET", "during"]), ("after", &["GET", "after"]), ("k1", &["GET", "k1"]),
    ("k2", &["GET", "k2"]), ("l", &["LRANGE", "l", "0", "-1"]), ("pad", &["STRLEN", "pad"]),
];

/// What one server did in the page-end case, and what it kept.
struct PageEnd {
    server: String,
    /// The reply to DURING, sent before the device failed.
    during: String,
    /// The incremental file's size once DURING's record was in it, a page
    /// end, or why the layout the case is for was not reached.
    layout: Result<u64, String>,
    /// The length of the padding value.
    pad: usize,
    /// The reply to AFTER.
    after: String,
    /// The replies to KEPT after the remount and restart, or how the server
    /// exited instead of answering.
    restart: Result<BTreeMap<String, String>, String>,
}

impl PageEnd {
    /// One line per try of a server, in a stable form for the research record.
    fn observation(&self) -> String {
        let layout = match &self.layout {
            Ok(end) => format!("page-end@{end}"),
            Err(why) => format!("missed({why:?})"),
        };
        let mut line = format!("observation: {} during-write={:?} layout={layout} pad-length={} after-write={:?}",
            self.server, self.during, self.pad, self.after);
        match &self.restart {
            Ok(replies) => {
                line.push_str(" loaded=yes");
                for (name, _) in KEPT {
                    line.push_str(&format!(" {name}={:?}", replies[*name]));
                }
            }
            Err(why) => line.push_str(&format!(" loaded=no exit={why:?}")),
        }
        line
    }
}

/// The incremental file's size on the device's filesystem.
///
/// The layout is measured on the file itself rather than through INFO's
/// aof_current_size, which counts the base file too once a server has
/// loaded one (firn's startup sums every part; Redis 7.2.16 sets it to all
/// the files' size, aof.c:1759) and equals the incremental file's size only
/// because this case starts both servers on an empty directory. The size is
/// the inode's, which a write(2) of the file adapter and a completed write of
/// the native ring both extend before they report, so once nothing is
/// buffered it holds every byte the server wrote. INFO's account of DURING
/// is still checked against it.
fn on_disk(incremental: &Path) -> u64 {
    std::fs::metadata(incremental)
        .unwrap_or_else(|error| panic!("the incremental file {}: {error}", incremental.display()))
        .len()
}

/// The incremental file the manifest in `directory`'s appendonlydir names
/// last. firn's recovery rewrite moves appends to a new one and removes the
/// file the failure happened in.
fn active_incremental(directory: &Path) -> PathBuf {
    let directory = directory.join("appendonlydir");
    let manifest = std::fs::read_to_string(directory.join("sync.aof.manifest")).expect("the manifest");
    let name = manifest.lines().rev().find(|line| line.ends_with(" type i"))
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_else(|| panic!("no incremental file in the manifest {manifest:?}"));
    directory.join(name)
}

/// The value of `SET pad VALUE` whose record, appended to a file of `size`
/// bytes, leaves exactly DURING's record before a page end. A value one byte
/// longer can lengthen the record by two, when its length gains a digit, so
/// a gap no record fits is moved one page on.
fn padding(size: u64) -> String {
    let record = |length: usize| resp(&["SET", "pad", ""]).len() + length + length.to_string().len() - 1;
    let during = resp(DURING).len() as u64;
    let gap = (PAGE - (size + during) % PAGE) % PAGE;
    for target in [gap, gap + PAGE, gap + 2 * PAGE] {
        if let Some(length) = (0..=target as usize).find(|length| record(*length) as u64 == target) {
            let value = "x".repeat(length);
            assert_eq!(resp(&["SET", "pad", &value]).len() as u64, target);
            return value;
        }
    }
    unreachable!("some record fits one of three successive page ends");
}

/// The page size and the filesystem's block size, which must divide the
/// page: then a page end is also a block end, and a write starting there
/// dirties neither the page nor the block before it.
fn check_page_layout(device: &Device) {
    let page = support::run_command(Command::new("getconf").arg("PAGESIZE"));
    let page = String::from_utf8_lossy(&page.stdout).trim().to_string();
    let block = support::run_command(Command::new("stat").args(["-f", "-c", "%S"]).arg(&device.mount));
    let block = String::from_utf8_lossy(&block.stdout).trim().to_string();
    eprintln!("page size {page}, filesystem block size {block}");
    assert_eq!(page, PAGE.to_string(), "the case assumes {PAGE}-byte pages");
    let block: u64 = block.parse().unwrap_or_else(|_| panic!("block size {block:?}"));
    assert!(block > 0 && PAGE % block == 0, "block size {block} does not divide the page");
}

/// Seeds the server and pads its incremental file so that DURING's record
/// will end at a page end, lets both reach the device, fails the device,
/// sends DURING and requires +OK, waits until a sync has failed, waits again
/// after SETTLE so that a sync covering DURING has failed, and reads the
/// layout. Returns the reply to DURING, the layout reached and the padding's
/// length.
///
/// Both servers sync only after new bytes (Redis 7.2.16's everysec does; firn
/// does as well, or while a sync error is saved), so with the device failed
/// and nothing new written neither syncs, and the write is acknowledged: the
/// status is still ok when it arrives. Its sync then fails writing DURING's
/// page back, and Linux leaves the page clean and reports the error once,
/// which is the case the page-end layout is for. The first err can come from
/// a sync that began before the bytes were written, so the case waits for
/// SETTLE and the status err again, by which time a sync that covers them
/// has failed.
fn fail_at_page_end(device: &Device, client: &mut TcpStream, incremental: &Path, server: &str)
    -> (String, Result<u64, String>, usize) {
    seed(client);
    let seeded = on_disk(incremental);
    let pad = padding(seeded);
    let pad_record = resp(&["SET", "pad", &pad]).len() as u64;
    assert_eq!(memory_request(client, &["SET", "pad", &pad]), "+OK\r\n", "{server}: the padding");
    std::thread::sleep(SETTLE);
    let padded_info = wait_status(client, "ok");
    let padded = on_disk(incremental);
    device.run("fail");
    let during = memory_request(client, DURING);
    // Nothing new was written since the last successful sync, so a server
    // that syncs only after new bytes has not noticed the fault yet.
    assert_eq!(during, "+OK\r\n", "{server}: the write after the device failed is acknowledged");
    wait_status(client, "err");
    std::thread::sleep(SETTLE);
    let failed = wait_status(client, "err");
    let written = on_disk(incremental);
    let during_record = resp(DURING).len() as u64;
    let counted = size(&failed) as i128 - size(&padded_info) as i128;
    let layout = if padded != seeded + pad_record {
        Err(format!("the padding of {pad_record} bytes took the file from {seeded} to {padded} bytes"))
    } else if written != padded + during_record || written % PAGE != 0 {
        Err(format!("the write before the failure took the file from {padded} to {written} bytes, not to a page end"))
    } else if counted != during_record as i128 {
        Err(format!("INFO counts {counted} bytes for the write before the failure, the file {during_record}"))
    } else {
        Ok(written)
    };
    eprintln!("layout: {server} seed ends at {seeded}, padding at {padded}, the write before the failure at {written}");
    (during, layout, pad.len())
}

/// The restarted server once it answers PING, or how it exited: a server
/// whose append-only file does not load exits, Redis 7.2.16 after it has
/// begun listening (server.c:7321-7331, 6773-6776) and firn before.
fn answering(mut child: ProgramChild, port: u16, server: &str) -> Result<(ProgramChild, TcpStream), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().expect("poll the restarted server").is_some() {
            let output = child.wait_with_output().expect("the exited server's output");
            eprintln!("{server}: exited at restart with {}; stdout {:?}; stderr {:?}", output.status,
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return Err(output.status.to_string());
        }
        assert!(Instant::now() < deadline, "{server} neither answered nor exited after its restart");
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            let mut stream = bounded_stream(stream);
            if stream.write_all(&resp(&["PING"])).is_ok() {
                let mut line = Vec::new();
                let mut byte = [0_u8; 1];
                while !line.ends_with(b"\r\n") && matches!(stream.read(&mut byte), Ok(1)) {
                    line.push(byte[0]);
                }
                // -LOADING, a reset or EOF: ask again until it answers or exits.
                if line == b"+PONG\r\n" {
                    return Ok((child, stream));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Restarts a server on what reached the device and asks it KEPT.
fn restart(child: ProgramChild, port: u16, server: &str) -> Result<BTreeMap<String, String>, String> {
    let (child, mut client) = answering(child, port, server)?;
    let replies: BTreeMap<String, String> = KEPT.iter()
        .map(|(name, command)| (name.to_string(), memory_request(&mut client, command)))
        .collect();
    stop(&mut client, child);
    Ok(replies)
}

/// Redis 7.2.16 through the page-end case. It refuses AFTER while idle once
/// the device heals, so its restart shows only what became of DURING.
fn redis_page_end(device: &Device) -> PageEnd {
    let server = "redis-7.2.16";
    let binary = oracle();
    let directory = device.mount.join(format!("redis-page-end-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let log = std::env::temp_dir().join(format!("firn-sync-fault-redis-page-end-{}.log", std::process::id()));
    let incremental = aof_incremental_path(&directory, "sync.aof");
    let (child, mut client) = start_redis(&binary, &directory, &log);
    let (during, layout, pad) = fail_at_page_end(device, &mut client, &incremental, server);
    device.run("heal");
    std::thread::sleep(IDLE_WATCH);
    let after = memory_request(&mut client, AFTER);
    stop(&mut client, child);
    device.run("remount");
    let (child, port) = spawn_redis(&binary, &directory, &log);
    let restart = restart(child, port, server);
    if restart.is_err() {
        eprintln!("{server} log: {}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    let _ = std::fs::remove_dir_all(&directory);
    PageEnd { server: server.to_string(), during, layout, pad, after, restart }
}

/// firn on one route through the page-end case. Every call runs a fresh
/// server process on a fresh directory.
fn firn_page_end(device: &Device, native: bool) -> PageEnd {
    let server = if native { "firn-native-ring" } else { "firn-file-adapter" };
    let program = CompiledProgram::from_environment_under(&device.mount);
    let incremental = aof_incremental_path(program.working_directory(), "sync.aof");
    let (child, mut client) = start_firn(&program, native);
    let (during, layout, pad) = fail_at_page_end(device, &mut client, &incremental, server);
    device.run("heal");
    wait_recovered(&mut client, server);
    // Recovery rewrote the file, so AFTER goes to the incremental file the
    // new manifest names.
    let active = active_incremental(program.working_directory());
    let before = on_disk(&active);
    let after = memory_request(&mut client, AFTER);
    // The periodic sync, not the one at shutdown, carries AFTER to the
    // device, so a lost DURING cannot be blamed on the stop.
    std::thread::sleep(SETTLE);
    wait_status(&mut client, "ok");
    eprintln!("layout: {server} recovery moved appends from {} to {}; the write after recovery took it from {before} to {} bytes",
        incremental.display(), active.display(), on_disk(&active));
    stop(&mut client, child);
    device.run("remount");
    let (child, port) = spawn_firn(&program, native);
    let restart = restart(child, port, server);
    PageEnd { server: server.to_string(), during, layout, pad, after, restart }
}

/// A write acknowledged just before the device fails, whose record ends
/// exactly at a page end and whose sync then fails, is in the file a restart
/// reads from the device once the device has healed, the server has recovered
/// and a later write has started on the next page.
///
/// Linux leaves the pages whose writeback failed clean and reports the
/// error once to the next fsync, so the first successful sync after the
/// device heals does not write them again: only a later write that dirties
/// the same page (or filesystem block) brings their bytes to the device. In
/// the case above AFTER dirties DURING's page; here the seed is padded so
/// that DURING's record ends at a page end and AFTER starts on the next
/// page, which leaves DURING's page as the failed writeback left it, so the
/// file the failure happened in has lost DURING and only firn's recovery
/// rewrite, which writes DURING from memory into a new base, keeps it. The
/// case then stops the server, remounts so that only what reached the
/// device counts, restarts it and reads every key.
///
/// For each server the order is: seed and pad the file and let them sync;
/// fail the device; send DURING and require +OK; wait for the failed status,
/// then SETTLE and the failed status again; heal the device and go on as
/// above. Both servers sync only after new bytes (Redis 7.2.16's everysec
/// does; firn does as well, or while a sync error is saved), so neither has
/// noticed the fault when DURING arrives and both acknowledge it, which the
/// case also requires of firn as Redis does. Earlier runs failed (CI runs
/// 38327248463 and 38328422734) because firn synced while idle with nothing
/// new, so its sync had already failed and it refused DURING with MISCONF.
///
/// Redis 7.2.16 goes through the same steps as an observation for the
/// research record, not as an oracle: it refuses AFTER while idle, and its
/// restart shows whether DURING survived. firn runs on both routes, and the
/// case prints every server's observation before it asserts firn's: the
/// file loads, DURING and AFTER read back, and the seed and padding are
/// intact.
#[test]
#[ignore = "needs the faulty device the aof-sync-fault job of .github/workflows/check.yml makes"]
fn aof_sync_failure_keeps_a_write_acknowledged_at_a_page_end() {
    let device = Device::from_environment();
    let _heal = HealOnPanic(&device);
    check_page_layout(&device);
    // Each observation is printed as its run ends, so a run that fails
    // part way still leaves the earlier ones in the log.
    let mut outcomes = Vec::new();
    let mut record = |outcome: PageEnd| {
        eprintln!("{}", outcome.observation());
        outcomes.push(outcome);
    };
    record(redis_page_end(&device));
    record(firn_page_end(&device, false));
    record(firn_page_end(&device, true));
    let redis_during = outcomes[0].during.clone();
    for outcome in outcomes.iter().filter(|outcome| outcome.server.starts_with("firn")) {
        let server = &outcome.server;
        assert_eq!(outcome.during, redis_during,
            "{server}: the write after the device failed gets Redis's reply; Redis syncs only after new bytes, so it acknowledges the first write after a device fault");
        if let Err(why) = &outcome.layout {
            panic!("{server}: the page-end layout was not exercised, so the case tested nothing: {why}");
        }
        assert_eq!(outcome.after, "+OK\r\n", "{server}: the write after recovery is accepted");
        let replies = outcome.restart.as_ref()
            .unwrap_or_else(|why| panic!("{server}: the append-only file does not load after a remount: {why}"));
        let pad = format!(":{}\r\n", outcome.pad);
        for (name, expected) in [
            ("during", "$1\r\nw\r\n"), ("after", "$1\r\np\r\n"), ("k1", "$2\r\nv1\r\n"),
            ("k2", "$2\r\nv2\r\n"), ("l", "*2\r\n$1\r\na\r\n$1\r\nb\r\n"), ("pad", pad.as_str()),
        ] {
            assert_eq!(replies[name], expected, "{server}: {name} after a remount and restart");
        }
    }
}
