//! Redis 7.0.15 src/aof.c:1024-1040,1146-1230, server.c:1384-1393,
//! 3881-3905,4324-4353,5596-5650 and multi.c:85-87,105-109,136-140,
//! 170-178,229-233 define the file, admission and transaction expectations.
//! script.c:349-386,547-557 and script_lua.c:988-994,1711-1722 define
//! first-write refusal, redis.call raising and redis.pcall error tables.
//! The MISCONF suffix is strerror(aof_last_write_errno) (server.c:4346-4355):
//! the text below is glibc's on Linux x86-64 in the C locale. A short write is
//! described as ENOSPC whatever the host returned (aof.c:1163-1184).

use super::*;
use std::path::Path;

const MISCONF: &str = "-MISCONF Errors writing to the AOF file: ";
const EXEC_MISCONF: &str = "-EXECABORT Transaction discarded because of: MISCONF Errors writing to the AOF file: ";
const PREVIOUS_ERRORS: &str = "-EXECABORT Transaction discarded because of previous errors.\r\n";
const POLL_LIMIT: Duration = Duration::from_secs(10);
/// glibc strerror(EFBIG). `fault` sets RLIMIT_FSIZE to the file's current
/// length, so write(2) fails before writing a byte; aofWrite returns -1
/// (aof.c:1031-1033) and flushAppendOnlyFile keeps that errno (aof.c:1157-1162).
const EFBIG_TEXT: &str = "File too large";
/// glibc strerror(ENOSPC), which aof.c:1184 reports for every short write.
const ENOSPC_TEXT: &str = "No space left on device";

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct FileLimit {
    pub(super) soft: u64,
    hard: u64,
}

unsafe extern "C" {
    fn prlimit64(pid: i32, resource: i32, new: *const FileLimit, old: *mut FileLimit) -> i32;
}

/// prlimit(2) on the child owned by this test, preserving its hard limit so
/// raising the soft limit later needs no privilege. Linux RLIMIT_FSIZE is 1.
pub(super) fn file_limit(child: &ProgramChild, soft: u64) -> FileLimit {
    let mut previous = FileLimit { soft: 0, hard: 0 };
    let read = unsafe { prlimit64(child.id() as i32, 1, std::ptr::null(), &mut previous) };
    assert_eq!(read, 0, "read child file limit: {}", std::io::Error::last_os_error());
    let next = FileLimit { soft, hard: previous.hard };
    let set = unsafe { prlimit64(child.id() as i32, 1, &next, std::ptr::null_mut()) };
    assert_eq!(set, 0, "set child file limit: {}", std::io::Error::last_os_error());
    previous
}

fn ignored_xfsz(child: &ProgramChild) {
    let status = std::fs::read_to_string(format!("/proc/{}/status", child.id())).unwrap();
    let mask = status.lines().find_map(|line| line.strip_prefix("SigIgn:"))
        .expect("Linux process status has SigIgn").trim();
    let mask = u64::from_str_radix(mask, 16).unwrap();
    assert_ne!(mask & (1_u64 << 24), 0, "firn must preserve inherited ignored SIGXFSZ");
}

fn start(program: &CompiledProgram, native: bool) -> (ProgramChild, TcpStream, u16) {
    let port_lease = support::lease_port();
    let port = port_lease.port();
    let text = port.to_string();
    let child = program.spawn_on_route_ignoring_xfsz(native, &[text.as_bytes(), b"0", b"errors.aof"]).with_port(&port_lease);
    let client = connect_when_ready(port, &child);
    ignored_xfsz(&child);
    (child, client, port)
}

fn incremental(program: &CompiledProgram) -> std::path::PathBuf {
    program.working_directory().join("appendonlydir/errors.aof.1.incr.aof")
}

fn persistence(client: &mut TcpStream) -> String {
    client.write_all(&resp(&["INFO", "persistence"])).unwrap();
    bulk_reply(client, "AOF health INFO")
}

fn wait_status(client: &mut TcpStream, expected: &str) -> String {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        let status = info_field(&info, "aof_last_write_status")
            .expect("Redis server.c:5596-5650 reports AOF write/sync health");
        if status == expected { return info; }
        assert!(Instant::now() < deadline, "wanted AOF status {expected}, got: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_empty_healthy_buffer(client: &mut TcpStream) {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let info = persistence(client);
        if info_field(&info, "aof_last_write_status").as_deref() == Some("ok")
            && info_field(&info, "aof_buffer_length").as_deref() == Some("0") {
            return;
        }
        assert!(Instant::now() < deadline, "AOF did not publish a healthy empty buffer: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_bytes(path: &Path, expected: &[u8]) {
    let deadline = Instant::now() + POLL_LIMIT;
    loop {
        let actual = std::fs::read(path).unwrap_or_default();
        if actual == expected { return; }
        assert!(Instant::now() < deadline,
            "file never matched expected RESP bytes: actual={actual:?}, expected={expected:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn misconf(reply: &str, prefix: &str) {
    assert_eq!(reply, format!("{prefix}{EFBIG_TEXT}\r\n"));
}

fn stop(client: &mut TcpStream, child: ProgramChild) {
    client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
    shutdown_finished(child);
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
}

fn seed(client: &mut TcpStream, path: &Path) -> Vec<u8> {
    assert_eq!(memory_request(client, &["SET", "readable", "v"]), "+OK\r\n");
    let bytes = resp(&["SET", "readable", "v"]);
    wait_bytes(path, &bytes);
    // Appended bytes can reach the file before Meta publishes the empty
    // retained buffer, while the original healthy status still reads ok.
    wait_empty_healthy_buffer(client);
    bytes
}

fn fault(child: &ProgramChild, client: &mut TcpStream, path: &Path) -> FileLimit {
    let previous = file_limit(child, std::fs::metadata(path).unwrap().len());
    assert_eq!(memory_request(client, &["SET", "retained", "r"]), "+OK\r\n");
    let info = wait_status(client, "err");
    let length: u64 = info_field(&info, "aof_buffer_length").expect("retained AOF buffer reported")
        .parse().unwrap();
    assert!(length >= resp(&["SET", "retained", "r"]).len() as u64, "{info}");
    ignored_xfsz(child);
    previous
}

/// CMD_WRITE includes commands without DENYOOM, and no-effect writes. GETEX
/// has CMD_WRITE even without an expiry option. Redis commands/*.json flags
/// and server.c:3881-3905 are the oracle; lookup/arity/auth still win.
#[test]
fn aof_failure_refuses_write_flags_and_ping_and_recovers_without_new_writes() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, native);
        let path = incremental(&program);
        let mut expected = seed(&mut client, &path);
        let old = fault(&child, &mut client, &path);
        let writes: &[&[&str]] = &[
            &["SET", "absent", "v"], &["SETNX", "readable", "v"],
            &["SETEX", "absent", "30", "v"], &["PSETEX", "absent", "30000", "v"],
            &["GETSET", "absent", "v"], &["MSET", "absent", "v"], &["MSETNX", "absent", "v"],
            &["INCR", "n"], &["DECR", "n"], &["INCRBY", "n", "1"], &["DECRBY", "n", "1"],
            &["INCRBYFLOAT", "n", "1"], &["APPEND", "absent", "v"], &["SETRANGE", "absent", "0", ""],
            &["DEL", "missing"], &["UNLINK", "missing"], &["GETDEL", "missing"], &["GETEX", "readable"],
            &["COPY", "missing", "other"], &["RENAME", "missing", "other"], &["RENAMENX", "missing", "other"],
            &["EXPIRE", "missing", "30"], &["PEXPIRE", "missing", "30"],
            &["EXPIREAT", "missing", "9999999999"], &["PEXPIREAT", "missing", "9999999999999"],
            &["PERSIST", "missing"], &["FLUSHDB"], &["FLUSHALL"],
            &["HSET", "h", "f", "v"], &["HMSET", "h", "f", "v"], &["HSETNX", "h", "f", "v"],
            &["HDEL", "h", "f"], &["HINCRBY", "h", "f", "1"], &["HINCRBYFLOAT", "h", "f", "1"],
            &["LPUSH", "l", "v"], &["RPUSH", "l", "v"], &["LPUSHX", "l", "v"], &["RPUSHX", "l", "v"],
            &["LPOP", "l", "0"], &["RPOP", "l", "0"], &["LSET", "l", "0", "v"],
            &["LREM", "l", "0", "v"], &["LTRIM", "l", "0", "-1"], &["LINSERT", "l", "BEFORE", "p", "v"],
            &["LMOVE", "l", "other", "LEFT", "RIGHT"], &["RPOPLPUSH", "l", "other"],
            &["SADD", "s", "v"], &["SREM", "s", "v"], &["SPOP", "s", "0"], &["SMOVE", "s", "other", "v"],
            &["SINTERSTORE", "out", "s"], &["SUNIONSTORE", "out", "s"], &["SDIFFSTORE", "out", "s"],
            &["ZADD", "z", "1", "v"], &["ZINCRBY", "z", "1", "v"], &["ZREM", "z", "v"],
            &["ZPOPMIN", "z", "0"], &["ZPOPMAX", "z", "0"],
            &["ZREMRANGEBYRANK", "z", "0", "-1"], &["ZREMRANGEBYSCORE", "z", "-inf", "+inf"],
            &["ZREMRANGEBYLEX", "z", "-", "+"], &["FUNCTION", "FLUSH"],
            &["PING"], &["PING", "message"],
        ];
        for args in writes { misconf(&memory_request(&mut client, args), MISCONF); }
        assert_eq!(memory_request(&mut client, &["GET", "readable"]), "$1\r\nv\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "retained"]), "$1\r\nr\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "absent"]), "$-1\r\n");
        assert_eq!(memory_request(&mut client, &["SET", "bad-arity"]), "-ERR wrong number of arguments for 'set' command\r\n");
        assert!(memory_request(&mut client, &["UNKNOWN"]).starts_with("-ERR unknown command"));
        assert_eq!(memory_request(&mut client, &["CONFIG", "SET", "requirepass", "secret"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["RESET"]), "+RESET\r\n");
        assert_eq!(memory_request(&mut client, &["SET", "absent", "v"]), "-NOAUTH Authentication required.\r\n");
        assert_eq!(memory_request(&mut client, &["AUTH", "secret"]), "+OK\r\n");
        misconf(&memory_request(&mut client, &["SET", "absent", "v"]), MISCONF);
        file_limit(&child, old.soft);
        // INFO is the only traffic during recovery: Redis retries a retained
        // buffer from beforeSleep (server.c:1660-1666) and serverCron
        // (server.c:1384-1393), so recovery needs no client write.
        let recovered = wait_status(&mut client, "ok");
        assert_eq!(info_field(&recovered, "aof_buffer_length").as_deref(), Some("0"));
        expected.extend(resp(&["SET", "retained", "r"]));
        wait_bytes(&path, &expected);
        assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n");
        assert_eq!(memory_request(&mut client, &["SET", "after", "a"]), "+OK\r\n");
        expected.extend(resp(&["SET", "after", "a"]));
        stop(&mut client, child);
        assert_eq!(std::fs::read(path).unwrap(), expected);
    }
}

#[test]
fn aof_failure_transaction_admission_and_legacy_script_first_write() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, port) = start(&program, native);
        let mut other = connect_when_ready(port, &child);
        let mut readonly = connect_when_ready(port, &child);
        let path = incremental(&program);
        seed(&mut other, &path);
        let read_source = "return redis.call('GET','readable')";
        other.write_all(&resp(&["SCRIPT", "LOAD", read_source])).unwrap();
        let read_sha = bulk_reply(&mut other, "load read-only legacy script");
        let call_source = "redis.call('GET','readable'); return redis.call('DEL','missing')";
        other.write_all(&resp(&["SCRIPT", "LOAD", call_source])).unwrap();
        let call_sha = bulk_reply(&mut other, "load writing legacy script");
        let pcall_source = "local e=redis.pcall('DEL','missing'); return {e.err,redis.call('GET','readable')}";
        other.write_all(&resp(&["SCRIPT", "LOAD", pcall_source])).unwrap();
        let pcall_sha = bulk_reply(&mut other, "load pcall legacy script");
        // Every key is declared, so this runs as a declared-entry attempt
        // (Firn-wf#47) rather than the whole-keyspace fallback above.
        let declared_source = "redis.call('GET',KEYS[1]); return redis.call('DEL',KEYS[2])";
        other.write_all(&resp(&["SCRIPT", "LOAD", declared_source])).unwrap();
        let declared_sha = bulk_reply(&mut other, "load declared-key legacy script");
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["INCR", "queued"]), "+QUEUED\r\n");
        // PING is refused at queue admission but is not CMD_WRITE: Redis
        // server.c:3735-3740,3881-3905 uses WRITE alone for EXEC's queue.
        assert_eq!(memory_request(&mut readonly, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut readonly, &["PING"]), "+QUEUED\r\n");
        assert_eq!(memory_request(&mut readonly, &["GET", "readable"]), "+QUEUED\r\n");
        let old = fault(&child, &mut other, &path);
        misconf(&memory_request(&mut client, &["EXEC"]), EXEC_MISCONF);
        assert_eq!(memory_request(&mut readonly, &["EXEC"]), "*2\r\n+PONG\r\n$1\r\nv\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        misconf(&memory_request(&mut client, &["SET", "queued", "v"]), MISCONF);
        assert_eq!(memory_request(&mut client, &["GET", "readable"]), "+QUEUED\r\n");
        assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS);
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        misconf(&memory_request(&mut client, &["PING"]), MISCONF);
        assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS);
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "readable"]), "+QUEUED\r\n");
        assert_eq!(memory_request(&mut client, &["EXEC"]), "*1\r\n$1\r\nv\r\n");
        for name in ["EVAL", "EVALSHA"] {
            let read = if name == "EVAL" { read_source } else { &read_sha };
            assert_eq!(memory_request(&mut client, &[name, read, "0"]), "$1\r\nv\r\n");
            let call = if name == "EVAL" { call_source } else { &call_sha };
            // script.c:383-386 stores the reply without its CR LF as the
            // error table (script_lua.c:546-570), and luaCallFunction adds
            // the script and line (script_lua.c:1711-1722).
            let raised = memory_request(&mut client, &[name, call, "0"]);
            assert_eq!(raised, format!("{MISCONF}{EFBIG_TEXT} script: {call_sha}, on @user_script:1.\r\n"), "redis.call must raise");
            let declared = if name == "EVAL" { declared_source } else { &declared_sha };
            assert_eq!(memory_request(&mut client, &[name, declared, "2", "readable", "missing"]),
                format!("{MISCONF}{EFBIG_TEXT} script: {declared_sha}, on @user_script:1.\r\n"),
                "declared-entry attempt checks the held Meta at its first write");
            let pcall = if name == "EVAL" { pcall_source } else { &pcall_sha };
            client.write_all(&resp(&[name, pcall, "0"])).unwrap();
            assert_eq!(reply_line(&mut client, "pcall result"), "*2\r\n");
            let error = bulk_reply(&mut client, "pcall err table");
            assert_eq!(error, format!("{}{EFBIG_TEXT}", &MISCONF[1..]), "redis.pcall returns err table");
            assert_eq!(bulk_reply(&mut client, "read after pcall"), "v");
        }
        assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
        file_limit(&child, old.soft);
        wait_status(&mut other, "ok");
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["INCR", "queued"]), "+QUEUED\r\n");
        assert_eq!(memory_request(&mut client, &["EXEC"]), "*1\r\n:1\r\n");
        assert_eq!(memory_request(&mut client, &["EVAL", "redis.call('DEL','missing'); redis.call('INCR','queued'); return redis.call('GET','queued')", "0"]), "$1\r\n2\r\n");
        drop(readonly);
        drop(other);
        stop(&mut client, child);
    }
}

/// RLIMIT_FSIZE stops a real append seven bytes into a RESP command. Linux
/// write(2) must return that prefix before the following append gets EFBIG.
/// aof.c:1174-1188 rolls it back; exact bytes and non-idempotent list replay
/// detect loss, duplication, retained-prefix mistakes and size-account errors.
#[test]
fn aof_short_append_rolls_back_and_replays_exactly_once_on_both_routes() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, native);
        let path = incremental(&program);
        assert_eq!(memory_request(&mut client, &["RPUSH", "events", "before"]), ":1\r\n");
        let mut expected = resp(&["RPUSH", "events", "before"]);
        wait_bytes(&path, &expected);
        wait_empty_healthy_buffer(&mut client);
        let old = file_limit(&child, expected.len() as u64 + 7);
        let retained = resp(&["RPUSH", "events", "during", "last"]);
        assert!(retained.len() > 7);
        assert_eq!(memory_request(&mut client, &["RPUSH", "events", "during", "last"]), ":3\r\n");
        let failed = wait_status(&mut client, "err");
        assert_eq!(info_field(&failed, "aof_buffer_length").unwrap().parse::<usize>().unwrap(), retained.len());
        assert_eq!(info_field(&failed, "aof_current_size").unwrap().parse::<u64>().unwrap(), expected.len() as u64);
        // A retry can briefly expose a prefix between append and truncate;
        // poll for rollback rather than racing that in-flight retry.
        wait_bytes(&path, &expected);
        ignored_xfsz(&child);
        file_limit(&child, old.soft);
        wait_status(&mut client, "ok");
        expected.extend(retained);
        wait_bytes(&path, &expected);
        stop(&mut client, child);
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        let (replayed, mut reader, _) = start(&program, native);
        assert_eq!(memory_request(&mut reader, &["LRANGE", "events", "0", "-1"]),
            "*3\r\n$6\r\nbefore\r\n$6\r\nduring\r\n$4\r\nlast\r\n");
        stop(&mut reader, replayed);
        assert_eq!(std::fs::read(path).unwrap(), expected, "replay adds no records");
    }
}

/// A rewrite admitted while an append error is latched must name the first
/// real failure (capture.md's first-cause diagnostics): the writer's next
/// attempted append fails and latches `drain`. A tick on which the append
/// retry is not yet due attempts nothing and must not abort the rewrite as
/// `writer_io`. The 5000 keys, with rdb-key-save-delay sleeping after each,
/// keep the scan running past the first ticks and the first due retry, so
/// the cut's own drain is not what latches the cause. A 1-MiB value written
/// and deleted before the fault makes the incremental file, and so the file
/// size limit, larger than the whole base, so the base write cannot latch
/// `base_io` first: an attempted drain is the only failure the rewrite meets.
#[test]
fn aof_rewrite_during_latched_append_error_reports_the_attempted_drain() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, native);
        let path = incremental(&program);
        let mut args = vec!["MSET".to_owned()];
        for index in 0..5000 {
            args.push(format!("key:{index}"));
            args.push("v".to_owned());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(memory_request(&mut client, &args), "+OK\r\n");
        let padding = "p".repeat(1 << 20);
        assert_eq!(memory_request(&mut client, &["SET", "padding", padding.as_str()]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["DEL", "padding"]), ":1\r\n");
        let mut expected = resp(&args);
        expected.extend(resp(&["SET", "padding", padding.as_str()]));
        expected.extend(resp(&["DEL", "padding"]));
        wait_bytes(&path, &expected);
        wait_empty_healthy_buffer(&mut client);
        rewrite_key_delay(&mut client, "100");
        let old = fault(&child, &mut client, &path);
        rewrite_start(&mut client);
        let info = rewrite_wait(&mut client);
        assert_eq!(info["aof_last_bgrewrite_status"], "err");
        assert_eq!(info["firn_aof_rewrite_abort_reason"], "drain", "first real failure, not a deferred retry: {info:?}");
        file_limit(&child, old.soft);
        wait_status(&mut client, "ok");
        stop(&mut client, child);
    }
}

/// A short write reports ENOSPC whatever write(2) returned next (aof.c:1163-
/// 1184): here the second write gets EFBIG, yet Redis replies with ENOSPC's
/// text, outside MULTI and at queue admission inside it.
#[test]
fn aof_short_append_reports_no_space_on_both_routes() {
    for native in [false, true] {
        let program = CompiledProgram::from_environment();
        let (child, mut client, _) = start(&program, native);
        let path = incremental(&program);
        assert_eq!(memory_request(&mut client, &["SET", "before", "b"]), "+OK\r\n");
        let expected = resp(&["SET", "before", "b"]);
        wait_bytes(&path, &expected);
        wait_empty_healthy_buffer(&mut client);
        let old = file_limit(&child, expected.len() as u64 + 7);
        assert!(resp(&["SET", "during", "d"]).len() > 7);
        assert_eq!(memory_request(&mut client, &["SET", "during", "d"]), "+OK\r\n");
        wait_status(&mut client, "err");
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["SET", "queued", "q"]),
            format!("{MISCONF}{ENOSPC_TEXT}\r\n"));
        assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS);
        assert_eq!(memory_request(&mut client, &["SET", "after", "a"]),
            format!("{MISCONF}{ENOSPC_TEXT}\r\n"));
        file_limit(&child, old.soft);
        wait_status(&mut client, "ok");
        stop(&mut client, child);
    }
}

/// Redis server.c:4239-4247,4301-4308 attempts one final flush and sync in
/// everysec and exits despite permanent failure. Old write_log never ends.
#[test]
fn aof_latched_error_shutdown_and_signals_exit_zero_on_both_routes() {
    for native in [false, true] {
        for trigger in ["SHUTDOWN", "TERM", "INT"] {
            let program = CompiledProgram::from_environment();
            let (child, mut client, _) = start(&program, native);
            let path = incremental(&program);
            let before = seed(&mut client, &path);
            fault(&child, &mut client, &path);
            if trigger == "SHUTDOWN" {
                client.write_all(&resp(&["SHUTDOWN", "NOSAVE"])).unwrap();
            } else {
                send_signal(&child, trigger);
            }
            shutdown_finished(child);
            assert_eq!(client.read(&mut [0]).unwrap(), 0);
            assert_eq!(std::fs::read(path).unwrap(), before, "permanent failure abandons retained bytes");
        }
    }
}
