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
//! `aof-sync-fault` job of .github/workflows/check.yml makes it and runs these
//! two sync cases and the three rewrite cases below, one after the other,
//! through `make aof-sync-fault`, failing when the
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
        let device = Self { mount, script };
        // A new filesystem, so that metadata an earlier case lost cannot
        // break this one.
        device.run("reset");
        device
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
    let port_lease = support::lease_port();
    let port = port_lease.port().to_string();
    let mut command = Command::new(binary);
    command.current_dir(directory)
        .args(["--port", port.as_str(), "--bind", "127.0.0.1", "--save", "", "--appendonly", "yes",
            "--appendfilename", "sync.aof", "--appendfsync", "everysec", "--dir"])
        .arg(directory).arg("--logfile").arg(log)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = ProgramChild::spawn(&mut command).expect("spawn redis-server").with_port(&port_lease);
    (child, port.parse().unwrap())
}

fn start_redis(binary: &Path, directory: &Path, log: &Path) -> (ProgramChild, TcpStream) {
    let (child, port) = spawn_redis(binary, directory, log);
    let client = connect_when_ready(port, &child);
    (child, client)
}

fn spawn_firn(program: &CompiledProgram, native: bool) -> (ProgramChild, u16) {
    let port_lease = support::lease_port();
    let port = port_lease.port();
    let text = port.to_string();
    (program.spawn_on_route(native, &[text.as_bytes(), b"0", b"sync.aof"]).with_port(&port_lease), port)
}

fn start_firn(program: &CompiledProgram, native: bool) -> (ProgramChild, TcpStream) {
    let (child, port) = spawn_firn(program, native);
    let client = connect_when_ready(port, &child);
    (child, client)
}

/// Redis 7.2.16 through the failure, then watched staying refused while
/// idle once the device heals.
fn redis_failure(device: &Device) -> Failure {
    let binary = support::redis_oracle();
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
    let mut client = None;
    loop {
        if child.try_wait().expect("poll the restarted server").is_some() {
            return Err(restart_exit(child, server));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "{server} neither answered nor exited after its restart{}",
            child.monitor().diagnostic_block(Some(port)));
        if client.is_none() {
            match TcpStream::connect_timeout(&address, remaining) {
                Ok(stream) => client = Some(bounded_stream(stream)),
                Err(error) if retry_refused_connection(&error, deadline) => {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(error) => panic!("{server}: connect at restart failed: {error}{}",
                    child.monitor().diagnostic_block(Some(port))),
            }
        }
        let stream = client.as_mut().expect("one restart connection");
        let line = match restart_ping(stream, deadline) {
            Ok(line) => line,
            Err(error) => return restart_connection_failed(child, port, server, error, deadline),
        };
        if line == b"+PONG\r\n" {
            return Ok((child, bounded_stream(client.take().expect("the answering connection"))));
        }
        assert!(line.starts_with(b"-LOADING "), "{server}: unexpected restart PING reply {:?}{}",
            String::from_utf8_lossy(&line), child.monitor().diagnostic_block(Some(port)));
        // Redis can answer LOADING while replaying. Keep its admitted socket
        // and ask again, rather than spending another connection slot.
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn restart_io_timeout(deadline: Instant) -> std::io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut,
            "restart PING exceeded the 20-second deadline"))
    } else {
        Ok(remaining.min(Duration::from_secs(10)))
    }
}

fn restart_ping(stream: &mut TcpStream, deadline: Instant) -> std::io::Result<Vec<u8>> {
    stream.set_write_timeout(Some(restart_io_timeout(deadline)?))?;
    stream.write_all(&resp(&["PING"]))?;
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while !line.ends_with(b"\r\n") {
        // A slow reply must not extend the absolute restart deadline one
        // byte at a time, nor get a fresh timeout after a delayed connect.
        stream.set_read_timeout(Some(restart_io_timeout(deadline)?))?;
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
    }
    Ok(line)
}

fn restart_exit(child: ProgramChild, server: &str) -> String {
    let output = child.wait_with_output().expect("the exited server's output");
    eprintln!("{server}: exited at restart with {}; stdout {:?}; stderr {:?}", output.status,
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    output.status.to_string()
}

/// Redis listens before replay and may then exit on a corrupt AOF. Observe
/// that exit after a closed connection without making another connection.
fn restart_connection_failed(
    mut child: ProgramChild,
    port: u16,
    server: &str,
    error: std::io::Error,
    deadline: Instant,
) -> Result<(ProgramChild, TcpStream), String> {
    let closed = matches!(error.kind(), std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe);
    loop {
        if child.try_wait().expect("poll restart after connection failure").is_some() {
            return Err(restart_exit(child, server));
        }
        if !closed || Instant::now() >= deadline {
            panic!("{server}: restart PING failed: {error}{}",
                child.monitor().diagnostic_block(Some(port)));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn restart_readiness_keeps_one_connection_while_loading() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("restart listener");
    let port = listener.local_addr().expect("restart listener address").port();
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one restart connection");
        drop(listener);
        let mut stream = bounded_stream(stream);
        let ping = resp(&["PING"]);
        let mut request = vec![0_u8; ping.len()];
        for reply in [b"-LOADING Redis is loading the dataset in memory\r\n".as_slice(), b"+PONG\r\n"] {
            stream.read_exact(&mut request).expect("PING on the same connection");
            assert_eq!(request, ping);
            stream.write_all(reply).expect("restart PING reply");
        }
    });
    let child = ProgramChild::spawn(Command::new("/bin/sh").args(["-c", "exec sleep 30"])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))
        .expect("spawn loading witness");
    let (child, client) = answering(child, port, "loading witness").expect("PONG after LOADING");
    drop(client);
    drop(child);
    peer.join().expect("single-connection loading witness");
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
    let binary = support::redis_oracle();
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

// Rewrite faults use syscall-visible file-adapter writes. io_uring submits
// writes through shared memory, so strace cannot inject its per-file result;
// the existing sync cases above separately cover both runtime routes.
// Redis 7.2.16 aof.c:2171-2226 aborts temporary-file write failures before
// publication. aof.c:2390-2495 publishes a successful child independently of
// the current append's saved error; its retained buffer retries after repair.
const REWRITE_KEYS: usize = 512;
const REWRITE_VALUE_BYTES: usize = 8192;
const REWRITE_WAIT: Duration = Duration::from_secs(30);
const WRITE_SYSCALLS: &str = "write,writev,pwrite64,pwritev,pwritev2";

/// One lifetime directory, reused at restart, and an independently checked
/// reference binary. Both servers use commands, not an RDB base preamble.
struct RewriteServer {
    program: CompiledProgram,
    redis: Option<PathBuf>,
    log: PathBuf,
}

impl RewriteServer {
    fn new(device: &Device, redis: bool, scenario: &str) -> Self {
        let program = CompiledProgram::from_environment_under(&device.mount);
        let label = if redis { "redis" } else { "firn" };
        let log = std::env::temp_dir().join(format!(
            "firn-sync-fault-redis-rewrite-{scenario}-{label}-{}.log", std::process::id()));
        Self { program, redis: redis.then(support::redis_oracle), log }
    }

    fn directory(&self) -> &Path { self.program.working_directory() }

    fn label(&self) -> &str {
        if self.redis.is_some() { "redis-7.2.16" } else { "firn-file-adapter" }
    }

    fn start(&self) -> (ProgramChild, TcpStream) {
        if let Some(binary) = &self.redis {
            let lease = support::lease_port();
            let mut command = Command::new(binary);
            command.current_dir(self.directory())
                .args(["--port", &lease.port().to_string(), "--bind", "127.0.0.1",
                    "--save", "", "--appendonly", "yes", "--appendfilename", "sync.aof",
                    "--appendfsync", "everysec", "--aof-use-rdb-preamble", "no",
                    "--auto-aof-rewrite-percentage", "0", "--dir"])
                .arg(self.directory()).arg("--logfile").arg(&self.log)
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            let child = ProgramChild::spawn(&mut command).unwrap().with_port(&lease);
            let (child, client) = answering(child, lease.port(), self.label()).unwrap();
            (child, client)
        } else {
            let (child, port) = spawn_firn(&self.program, false);
            answering(child, port, self.label()).unwrap()
        }
    }
}

fn rewrite_signal(pid: u32, signal: &str) {
    let output = support::run_command(Command::new("/bin/kill")
        .arg(signal).arg(pid.to_string()));
    assert!(output.status.success(), "{signal} {pid}: {output:?}");
}

/// A stopped worker cannot finish between locating its temporary file and
/// attaching the tracer/filling the disk. Never leave it stopped on panic.
struct PausedRewrite(u32);

impl PausedRewrite {
    fn new(pid: u32) -> Self {
        rewrite_signal(pid, "-STOP");
        let deadline = Instant::now() + POLL_LIMIT;
        loop {
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            if status.lines().any(|line| line.starts_with("State:") && line.contains('T')) {
                return Self(pid);
            }
            assert!(Instant::now() < deadline, "rewrite worker never stopped: {status}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for PausedRewrite {
    fn drop(&mut self) {
        let _ = Command::new("/bin/kill").arg("-CONT").arg(self.0.to_string()).status();
    }
}

struct FullRewriteDisk<'a>(&'a Device);

impl Drop for FullRewriteDisk<'_> {
    fn drop(&mut self) {
        let _ = Command::new("sudo").arg("-n").arg(&self.0.script).arg("unfill").status();
    }
}

/// -P restricts both tracing and injection to this file, -f includes all
/// firn worker threads. The kernel supplies the ENOSPC result; only EIO is
/// injected. The trace is evidence, and a missed pathname never passes.
struct RewriteTrace {
    child: Option<ProgramChild>,
    log: PathBuf,
    target: PathBuf,
}

impl RewriteTrace {
    fn attach(pid: u32, target: &Path, inject: bool, label: &str) -> Self {
        let log = std::env::temp_dir().join(format!(
            "firn-rewrite-fault-{label}-{pid}-{}.trace", std::process::id()));
        let mut command = Command::new("sudo");
        // The root-owned watchdog also bounds a tracer if cleanup itself
        // loses sudo access; --foreground preserves our separate group.
        command.args(["-n", "timeout", "--foreground", "--signal=KILL", "40s",
            "strace", "-f", "-qq", "-yy", "-s", "80", "-o"])
            .arg(&log).arg("-e").arg(format!("trace={WRITE_SYSCALLS}"))
            .arg("-P").arg(target).arg("-p").arg(pid.to_string());
        if inject { command.arg("-e").arg(format!("inject={WRITE_SYSCALLS}:error=EIO")); }
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = ProgramChild::spawn(&mut command).expect("attach strace to rewrite process");
        // Own privilege-aware cleanup immediately, including an attachment
        // failure or a vanished worker before this function returns.
        let mut trace = Self { child: Some(child), log, target: target.to_path_buf() };
        let deadline = Instant::now() + POLL_LIMIT;
        loop {
            assert!(trace.child.as_mut().unwrap().try_wait().unwrap().is_none(), "strace exited before attachment");
            let attached = std::fs::read_dir(format!("/proc/{pid}/task")).unwrap().all(|entry| {
                let status = std::fs::read_to_string(entry.unwrap().path().join("status")).unwrap();
                status.lines().find_map(|line| line.strip_prefix("TracerPid:"))
                    .is_some_and(|value| value.trim() != "0")
            });
            if attached { break; }
            assert!(Instant::now() < deadline, "strace did not attach every thread of {pid}");
            std::thread::sleep(Duration::from_millis(5));
        }
        trace
    }

    fn assert_fault(&self, errno: &str, injected: bool) {
        let text = std::fs::read_to_string(&self.log).unwrap();
        let target = self.target.to_str().unwrap();
        let failed: Vec<_> = text.lines().filter(|line| line.contains(&format!("= -1 {errno}"))).collect();
        assert!(!failed.is_empty(), "no {errno} write to {}: {text}", self.target.display());
        for line in failed {
            assert!(line.contains(target), "fault was not attributed to {target}: {line}");
            assert_eq!(line.contains("(INJECTED)"), injected, "unexpected injection source: {line}");
            eprintln!("rewrite-fault syscall: {line}");
        }
    }

    fn detach(mut self) {
        assert!(self.cleanup(), "could not detach rewrite fault tracer");
    }

    fn cleanup(&mut self) -> bool {
        let Some(child) = self.child.as_mut() else { return true; };
        for signal in ["-INT", "-KILL"] {
            if child.try_wait().ok().flatten().is_none() {
                // sudo, timeout and strace own this separate group, never
                // the server's. An unprivileged Child::kill is insufficient.
                let _ = support::output_within(Command::new("sudo")
                    .args(["-n", "/bin/kill", signal, "--"])
                    .arg(format!("-{}", child.id())), Duration::from_secs(2));
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if child.try_wait().ok().flatten().is_some() {
                    let child = self.child.take().unwrap();
                    if let Ok(output) = child.wait_with_output() {
                        eprintln!("rewrite-fault tracer detached: {}; stderr={}", output.status,
                            String::from_utf8_lossy(&output.stderr));
                    }
                    return true;
                }
                if Instant::now() >= deadline { break; }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // Do not enter ProgramChild's unprivileged normal Drop, which could
        // wait on a root process it cannot kill. The root watchdog expires
        // after 40 seconds even on this cleanup-failure path.
        if let Some(child) = self.child.take() { std::mem::forget(child); }
        eprintln!("rewrite-fault: privileged tracer cleanup failed; watchdog remains active; trace={}", self.log.display());
        false
    }
}

impl Drop for RewriteTrace {
    fn drop(&mut self) { let _ = self.cleanup(); }
}

/// Expected values are built from acknowledged client commands, never by
/// snapshotting the server's answer. A wrong value, missing write, duplicate
/// INCR replay or any extra key fails this independent recovery check.
fn rewrite_seed(client: &mut TcpStream) -> BTreeMap<String, String> {
    let mut expected = BTreeMap::new();
    for index in 0..REWRITE_KEYS {
        let key = format!("rewrite:key:{index}");
        let value = format!("{index:08}:{}", "v".repeat(REWRITE_VALUE_BYTES - 9));
        assert_eq!(memory_request(client, &["SET", &key, &value]), "+OK\r\n");
        expected.insert(key, value);
    }
    for value in 1..=2 {
        assert_eq!(memory_request(client, &["INCR", "rewrite:counter"]), format!(":{value}\r\n"));
    }
    expected.insert("rewrite:counter".to_string(), "2".to_string());
    std::thread::sleep(SETTLE);
    wait_status(client, "ok");
    expected
}

fn rewrite_increment(client: &mut TcpStream, expected: &mut BTreeMap<String, String>) {
    let value = expected["rewrite:counter"].parse::<u64>().unwrap() + 1;
    assert_eq!(memory_request(client, &["INCR", "rewrite:counter"]), format!(":{value}\r\n"));
    expected.insert("rewrite:counter".to_string(), value.to_string());
}

fn rewrite_verify(client: &mut TcpStream, expected: &BTreeMap<String, String>, server: &str) {
    for (key, value) in expected {
        assert_eq!(memory_request(client, &["GET", key]), format!("${}\r\n{value}\r\n", value.len()),
            "{server}: acknowledged value of {key} after restart");
    }
    assert_eq!(memory_request(client, &["GET", "rewrite:refused"]), "$-1\r\n",
        "{server}: a refused write must not reappear");
    assert_eq!(memory_request(client, &["DBSIZE"]), format!(":{}\r\n", expected.len()),
        "{server}: no unacknowledged extra keys");
}

/// No quoting is needed for this fixture's fixed filenames. Validate every
/// active reference; unlisted temporary files left by a crash are permitted.
fn rewrite_active_files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    let directory = directory.join("appendonlydir");
    let manifest = std::fs::read_to_string(directory.join("sync.aof.manifest")).unwrap();
    let mut files = BTreeMap::new();
    let mut bases = 0;
    let mut increment_sequence = 0;
    for line in manifest.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        assert_eq!(words.len(), 6, "manifest row: {line}");
        assert_eq!((words[0], words[2], words[4]), ("file", "seq", "type"));
        assert!(matches!(words[5], "b" | "i" | "h"));
        if words[5] == "h" { continue; }
        assert!(!words[1].contains('/') && !words[1].starts_with("temp-"), "active temporary/path: {line}");
        let sequence: u64 = words[3].parse().unwrap();
        assert!(sequence > 0);
        if words[5] == "b" { bases += 1; } else {
            assert!(sequence > increment_sequence, "unordered increments: {manifest}");
            increment_sequence = sequence;
        }
        assert!(files.insert(words[1].to_string(), std::fs::read(directory.join(words[1])).unwrap()).is_none());
    }
    assert_eq!(bases, 1, "exactly one complete base: {manifest}");
    assert!(increment_sequence > 0, "one current increment: {manifest}");
    files.insert("sync.aof.manifest".to_string(), manifest.into_bytes());
    files
}

fn rewrite_base_name(directory: &Path) -> String {
    let manifest = std::fs::read_to_string(directory.join("appendonlydir/sync.aof.manifest")).unwrap();
    manifest.lines().find(|line| line.ends_with(" type b")).unwrap()
        .split_whitespace().nth(1).unwrap().to_string()
}

/// Load precisely the old manifest and its active files, without any
/// temporary/new base that could accidentally rescue an invalid old AOF.
fn rewrite_verify_old(server: &RewriteServer, files: &BTreeMap<String, Vec<u8>>,
    expected: &BTreeMap<String, String>) {
    let copy = RewriteServer {
        program: CompiledProgram::from_environment(),
        redis: server.redis.clone(),
        log: server.log.with_extension("old.log"),
    };
    let directory = copy.directory().join("appendonlydir");
    std::fs::create_dir(&directory).unwrap();
    for (name, bytes) in files { std::fs::write(directory.join(name), bytes).unwrap(); }
    let (child, mut client) = copy.start();
    rewrite_verify(&mut client, expected, server.label());
    stop(&mut client, child);
    eprintln!("rewrite-fault observation: server={} old-active-files-alone=exact keys={}",
        server.label(), expected.len());
}

/// 20 ms per key leaves a ten-second scan window on both servers. We
/// require an actual nonempty temporary base, then freeze its writer before
/// attaching a fault; missing the phase is a failure rather than a skip.
fn rewrite_begin(server: &RewriteServer, child: &ProgramChild, client: &mut TcpStream) -> (u32, PathBuf) {
    assert_eq!(memory_request(client, &["CONFIG", "SET", "auto-aof-rewrite-percentage", "0"]), "+OK\r\n");
    assert_eq!(memory_request(client, &["CONFIG", "SET", "rdb-key-save-delay", "20000"]), "+OK\r\n");
    assert_eq!(memory_request(client, &["BGREWRITEAOF"]), "+Background append only file rewriting started\r\n");
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        assert_eq!(info_field(&info, "aof_rewrite_in_progress").as_deref(), Some("1"), "missed rewrite: {info}");
        let candidate = if server.redis.is_some() {
            let children = std::fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id())).unwrap();
            children.split_whitespace().find_map(|pid| {
                let path = server.directory().join(format!("temp-rewriteaof-{pid}.aof"));
                path.exists().then(|| (pid.parse().unwrap(), path))
            })
        } else {
            Some((child.id(), server.directory().join("appendonlydir/temp-sync.aof.base")))
        };
        if let Some((pid, path)) = candidate {
            if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0) { return (pid, path); }
        }
        assert!(Instant::now() < deadline, "no nonempty temporary rewrite base: {info}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn rewrite_finished(client: &mut TcpStream) -> String {
    let deadline = Instant::now() + REWRITE_WAIT;
    loop {
        let info = persistence(client);
        if info_field(&info, "aof_rewrite_in_progress").as_deref() == Some("0") { return info; }
        assert!(Instant::now() < deadline, "rewrite did not finish: {info}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn rewrite_restart(device: &Device, server: &RewriteServer, child: ProgramChild,
    client: TcpStream, expected: &BTreeMap<String, String>) {
    // No SHUTDOWN flush may rescue missing bytes. The acknowledged stream
    // was drained and allowed everysec to sync before this SIGKILL.
    drop(client);
    drop(child);
    device.run("remount");
    rewrite_active_files(server.directory());
    let (child, mut client) = server.start();
    rewrite_verify(&mut client, expected, server.label());
    rewrite_active_files(server.directory());
    stop(&mut client, child);
}

fn rewrite_temporary_fault(enospc: bool) {
    let device = Device::from_environment();
    let scenario = if enospc { "temporary-enospc" } else { "temporary-eio" };
    for redis in [true, false] {
        device.run("reset");
        let server = RewriteServer::new(&device, redis, scenario);
        let (child, mut client) = server.start();
        let mut expected = rewrite_seed(&mut client);
        let (worker, temporary) = rewrite_begin(&server, &child, &mut client);
        rewrite_increment(&mut client, &mut expected);
        // Especially in Redis's just-rotated empty increment, wait for
        // allocation/sync before filling the device. Otherwise ENOSPC could
        // first hit delayed allocation for this healthy ordinary append.
        std::thread::sleep(SETTLE);
        wait_status(&mut client, "ok");
        let old = rewrite_active_files(server.directory());
        let paused = PausedRewrite::new(worker);
        let temporary_size = std::fs::metadata(&temporary).expect("paused rewrite still has its temporary base").len();
        assert!(temporary_size > 0 && temporary_size < (REWRITE_KEYS * REWRITE_VALUE_BYTES) as u64,
            "missed incomplete scan: temporary base already has {temporary_size} bytes");
        let trace = RewriteTrace::attach(worker, &temporary, !enospc, scenario);
        let full = enospc.then(|| FullRewriteDisk(&device));
        if enospc { device.run("fill"); }
        drop(paused);
        let failed = rewrite_finished(&mut client);
        trace.assert_fault(if enospc { "ENOSPC" } else { "EIO" }, !enospc);
        assert_eq!(info_field(&failed, "aof_last_bgrewrite_status").as_deref(), Some("err"), "{failed}");
        assert_eq!(info_field(&failed, "aof_last_write_status").as_deref(), Some("ok"),
            "temporary failure must not be a current-append failure: {failed}");
        if !redis {
            assert_eq!(info_field(&failed, "firn_aof_rewrite_abort_reason").as_deref(), Some("base_io"), "{failed}");
        }
        assert_eq!(rewrite_active_files(server.directory()), old,
            "{} {scenario}: failure must preserve every old active byte and its manifest", server.label());
        trace.detach();
        drop(full);
        rewrite_verify_old(&server, &old, &expected);
        rewrite_increment(&mut client, &mut expected);
        std::thread::sleep(SETTLE);
        wait_status(&mut client, "ok");
        eprintln!("rewrite-fault observation: server={} case={scenario} rewrite=err append=ok old-files=unchanged acknowledged-counter={} restart=pending",
            server.label(), expected["rewrite:counter"]);
        rewrite_restart(&device, &server, child, client, &expected);
        eprintln!("rewrite-fault observation: server={} case={scenario} restart=exact keys={}", server.label(), expected.len());
    }
}

#[test]
#[ignore = "needs the aof-sync-fault device, strace and pinned Redis oracle"]
fn aof_rewrite_temporary_enospc_keeps_acknowledged_state() {
    rewrite_temporary_fault(true);
}

#[test]
#[ignore = "needs the aof-sync-fault device, strace and pinned Redis oracle"]
fn aof_rewrite_temporary_eio_keeps_acknowledged_state() {
    rewrite_temporary_fault(false);
}

/// Keep the current increment unwritable until the rewrite finishes, then
/// measure publication while the write is still retained. Redis's forked
/// rewrite succeeds; firn aborts. Repair, drain and crash/restart both, without
/// treating an everysec acknowledgement as already durable during the fault.
#[test]
#[ignore = "needs the aof-sync-fault device, strace and pinned Redis oracle"]
fn aof_rewrite_current_append_failure_observes_publication() {
    let device = Device::from_environment();
    for redis in [true, false] {
        device.run("reset");
        let server = RewriteServer::new(&device, redis, "current-eio");
        let (child, mut client) = server.start();
        let mut expected = rewrite_seed(&mut client);
        let old_base = rewrite_base_name(server.directory());
        let (_worker, _temporary) = rewrite_begin(&server, &child, &mut client);
        let current = active_incremental(server.directory());
        let before = std::fs::read(&current).unwrap();
        let old = rewrite_active_files(server.directory());
        let before_failure = expected.clone();
        let trace = RewriteTrace::attach(child.id(), &current, true, "current-eio");
        assert_eq!(info_field(&persistence(&mut client), "aof_rewrite_in_progress").as_deref(), Some("1"),
            "current append fault must happen during the rewrite");
        rewrite_increment(&mut client, &mut expected);
        let deadline = Instant::now() + POLL_LIMIT;
        loop {
            let info = persistence(&mut client);
            if info_field(&info, "aof_last_write_status").as_deref() == Some("err") { break; }
            assert!(Instant::now() < deadline, "current append never failed: {info}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let refused = memory_request(&mut client, &["SET", "rewrite:refused", "must-not-reappear"]);
        assert_eq!(refused, format!("{MISCONF}Input/output error\r\n"));
        let outcome = rewrite_finished(&mut client);
        trace.assert_fault("EIO", true);
        assert_eq!(info_field(&outcome, "aof_last_write_status").as_deref(), Some("err"), "{outcome}");
        assert!(count(&outcome, "aof_buffer_length") > 0, "failed acknowledged bytes must remain buffered: {outcome}");
        assert_eq!(std::fs::read(&current).unwrap(), before, "the failing write must not reach the current increment");
        let published = rewrite_base_name(server.directory()) != old_base;
        assert_eq!(published, redis, "Redis publishes the completed child while append still fails; firn aborts: {outcome}");
        assert_eq!(info_field(&outcome, "aof_last_bgrewrite_status").as_deref(), Some(if redis { "ok" } else { "err" }), "{outcome}");
        if !redis {
            // The actual failed append latches `drain` inside aof_rw_drain,
            // before aof_rw_flush returns failure to the rewrite loop.
            // capture.md's first-cause rule keeps it when that outer loop
            // subsequently reports `writer_io`, as the latched-append-error
            // regression in aof_write_errors also requires.
            assert_eq!(info_field(&outcome, "firn_aof_rewrite_abort_reason").as_deref(), Some("drain"), "{outcome}");
        }
        rewrite_active_files(server.directory());
        eprintln!("rewrite-fault observation: server={} case=current-eio rewrite={} append=err buffered={} published-before-append-repair={published} refused={refused:?}",
            server.label(), if redis { "ok" } else { "err" }, count(&outcome, "aof_buffer_length"));
        trace.detach();
        rewrite_verify_old(&server, &old, &before_failure);
        wait_status(&mut client, "ok");
        rewrite_increment(&mut client, &mut expected);
        std::thread::sleep(SETTLE);
        wait_status(&mut client, "ok");
        rewrite_restart(&device, &server, child, client, &expected);
        eprintln!("rewrite-fault observation: server={} case=current-eio repaired=yes restart=exact keys={}", server.label(), expected.len());
    }
}
