//! Redis 7.0.15 evict.c, server.c processCommand, multi.c and script.c are
//! the oracle, including RESETSTAT's retained peak and both EXECABORT paths.
//! Values are heap allocated even before Whitefoot counts shared-map nodes.

use super::*;

const OOM: &str = "-OOM command not allowed when used memory > 'maxmemory'.\r\n";
const PREVIOUS_ERRORS_ABORT: &str = "-EXECABORT Transaction discarded because of previous errors.\r\n";
const OOM_ABORT: &str = "-EXECABORT Transaction discarded because of: OOM command not allowed when used memory > 'maxmemory'.\r\n";
const VALUE_BYTES: usize = 64 * 1024;

fn info(client: &mut TcpStream, section: &str) -> String {
    client.write_all(&resp(&["INFO", section])).unwrap();
    bulk_reply(client, "memory enforcement INFO")
}

fn number(text: &str, field: &str) -> u64 {
    info_field(text, field).unwrap_or_else(|| panic!("missing {field}: {text}"))
        .parse().unwrap_or_else(|_| panic!("invalid {field}: {text}"))
}

fn configure(client: &mut TcpStream, pairs: &[&str]) {
    let mut args = vec!["CONFIG", "SET"];
    args.extend_from_slice(pairs);
    assert_eq!(memory_request(client, &args), "+OK\r\n");
}

fn start(clients: &[u8]) -> (ProgramChild, TcpStream, u16) {
    let port = free_port();
    let text = port.to_string();
    let child = firn().spawn_on_route(true, &[text.as_bytes(), clients]);
    (child, connect_when_ready(port), port)
}

/// processCommand rejects DENYOOM even for a write that would be a no-op,
/// but lets reads and memory-releasing writes run. Lookup/arity/auth win.
#[test]
fn noeviction_refuses_denyoom_but_allows_reads_and_deletes() {
    let (child, mut client, _) = start(b"1");
    let value = "v".repeat(VALUE_BYTES);
    assert_eq!(memory_request(&mut client, &["SET", "retained", &value]), "+OK\r\n");
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "noeviction"]);
    for command in [
        vec!["SET", "new", &value], vec!["SETNX", "retained", &value],
        vec!["MSET", "new", &value], vec!["MSETNX", "retained", &value],
        vec!["SETEX", "new", "30", &value], vec!["PSETEX", "new", "30000", &value],
        vec!["GETSET", "new", &value], vec!["INCR", "counter"], vec!["DECR", "counter"],
        vec!["INCRBY", "counter", "1"], vec!["DECRBY", "counter", "1"],
        vec!["INCRBYFLOAT", "counter", "1"], vec!["APPEND", "new", &value],
        vec!["SETRANGE", "new", "0", &value], vec!["COPY", "absent", "copy"],
        vec!["HSET", "hash", "f", &value], vec!["HMSET", "hash", "f", &value],
        vec!["HSETNX", "hash", "f", &value], vec!["HINCRBY", "hash", "f", "1"],
        vec!["HINCRBYFLOAT", "hash", "f", "1"], vec!["LPUSH", "list", &value],
        vec!["RPUSH", "list", &value], vec!["LPUSHX", "list", &value],
        vec!["RPUSHX", "list", &value], vec!["LSET", "list", "0", &value],
        vec!["LINSERT", "list", "BEFORE", "pivot", &value],
        vec!["LMOVE", "list", "other", "LEFT", "RIGHT"], vec!["RPOPLPUSH", "list", "other"],
        vec!["SADD", "set", &value], vec!["SINTERSTORE", "out", "a", "b"],
        vec!["SUNIONSTORE", "out", "a", "b"], vec!["SDIFFSTORE", "out", "a", "b"],
        vec!["ZADD", "sorted", "1", &value], vec!["ZINCRBY", "sorted", "1", &value],
    ] {
        assert_eq!(memory_request(&mut client, &command), OOM, "{} DENYOOM", command[0]);
    }
    assert_eq!(memory_request(&mut client, &["SET", "short"]), "-ERR wrong number of arguments for 'set' command\r\n");
    assert!(memory_request(&mut client, &["NO-SUCH-COMMAND"]).starts_with("-ERR unknown command"));
    assert_eq!(memory_request(&mut client, &["GET", "retained"]), format!("${VALUE_BYTES}\r\n{value}\r\n"));
    assert_eq!(memory_request(&mut client, &["GETEX", "absent"]), "$-1\r\n");
    assert_eq!(memory_request(&mut client, &["EXPIRE", "retained", "3600"]), ":1\r\n");
    assert_eq!(memory_request(&mut client, &["DEL", "retained"]), ":1\r\n");
    assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), 0);
    configure(&mut client, &["requirepass", "secret"]);
    assert_eq!(memory_request(&mut client, &["RESET"]), "+RESET\r\n");
    // networking.c refuses an unauthenticated bulk over 16384 bytes before
    // lookup, so the authentication order is shown with a short value.
    assert_eq!(memory_request(&mut client, &["SET", "new", "v"]), "-NOAUTH Authentication required.\r\n");
    assert_eq!(memory_request(&mut client, &["AUTH", "secret"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["SET", "new", "v"]), OOM);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// performEvictions frees value storage before later writes. This is a
/// bound on observed heap, not an eviction-quality or throughput claim.
#[test]
fn allkeys_lru_bounds_heap_and_counts_evictions() {
    let (child, mut client, _) = start(b"1");
    let value = "l".repeat(VALUE_BYTES);
    assert_eq!(memory_request(&mut client, &["SET", "warm-input", &value]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["DEL", "warm-input"]), ":1\r\n");
    let baseline = number(&info(&mut client, "memory"), "used_memory");
    let limit = baseline + (8 * VALUE_BYTES) as u64;
    configure(&mut client, &["maxmemory", &limit.to_string(), "maxmemory-policy", "allkeys-lru", "maxmemory-eviction-tenacity", "100"]);
    for index in 0..48 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("lru:{index}"), &value]), "+OK\r\n");
    }
    let measured = info(&mut client, "memory");
    let counted = number(&measured, "used_memory").saturating_sub(number(&measured, "mem_not_counted_for_evict"));
    assert!(counted <= limit + (2 * VALUE_BYTES) as u64, "limit={limit}: {measured}");
    assert!(number(&info(&mut client, "stats"), "evicted_keys") > 0);
    assert!(memory_request(&mut client, &["DBSIZE"]).trim().trim_start_matches(':').parse::<u64>().unwrap() < 48);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// The expires dictionary is Redis's volatile-policy candidate set. A
/// sparse unsuccessful batch must not be mistaken for an empty set.
#[test]
fn volatile_lru_preserves_persistent_keys_and_refuses_when_exhausted() {
    let (child, mut client, _) = start(b"1");
    let value = "t".repeat(VALUE_BYTES);
    for index in 0..32 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("permanent:{index}"), &value]), "+OK\r\n");
    }
    for index in 0..4 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("volatile:{index}"), &value, "EX", "3600"]), "+OK\r\n");
    }
    let limit = number(&info(&mut client, "memory"), "used_memory") - (2 * VALUE_BYTES) as u64;
    configure(&mut client, &["maxmemory", &limit.to_string(), "maxmemory-policy", "volatile-lru", "maxmemory-samples", "1", "maxmemory-eviction-tenacity", "100"]);
    assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n");
    assert!(number(&info(&mut client, "stats"), "evicted_keys") > 0);
    for index in 0..32 {
        assert_eq!(memory_request(&mut client, &["EXISTS", &format!("permanent:{index}")]), ":1\r\n");
    }
    configure(&mut client, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["SET", "refused", &value]), OOM);
    for index in 0..4 {
        assert_eq!(memory_request(&mut client, &["EXISTS", &format!("volatile:{index}")]), ":0\r\n");
    }
    for index in 0..32 {
        assert_eq!(memory_request(&mut client, &["EXISTS", &format!("permanent:{index}")]), ":1\r\n");
    }
    assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), 4);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// Smoke every other evicting policy. With tenacity 100 and an impossible
/// limit, all eligible keys must be removed; nonvolatile keys must survive.
#[test]
fn random_lfu_and_ttl_policies_evict_their_eligible_keys() {
    for policy in ["allkeys-random", "volatile-random", "allkeys-lfu", "volatile-lfu", "volatile-ttl"] {
        let (child, mut client, _) = start(b"1");
        let value = "p".repeat(VALUE_BYTES);
        configure(&mut client, &["maxmemory-policy", policy, "maxmemory-eviction-tenacity", "100"]);
        assert_eq!(memory_request(&mut client, &["SET", "permanent", &value]), "+OK\r\n");
        for key in ["ttl:1", "ttl:2", "ttl:3"] {
            assert_eq!(memory_request(&mut client, &["SET", key, &value, "EX", "3600"]), "+OK\r\n");
        }
        configure(&mut client, &["maxmemory", "1"]);
        assert_eq!(memory_request(&mut client, &["SET", "refused", &value]), OOM, "{policy}");
        for key in ["ttl:1", "ttl:2", "ttl:3"] {
            assert_eq!(memory_request(&mut client, &["EXISTS", key]), ":0\r\n", "{policy}");
        }
        let volatile = policy.starts_with("volatile");
        assert_eq!(memory_request(&mut client, &["EXISTS", "permanent"]), if volatile { ":1\r\n" } else { ":0\r\n" });
        assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), if volatile { 3 } else { 4 });
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// rejectCommand flags a queue-time OOM as DIRTY_EXEC; an admitted EXEC
/// returns shared.execaborterr, including after memory recovers. Each
/// command starts a clean transaction so reads must themselves dirty it.
#[test]
fn multi_queue_time_oom_aborts_with_previous_errors() {
    let (child, mut client, port) = start(b"2");
    let mut other = connect_when_ready(port);
    configure(&mut other, &["maxmemory", "1"]);
    for command in [
        vec!["SET", "queued", "q"], vec!["GET", "queued"],
        vec!["DEL", "queued"], vec!["PING"], vec!["INFO", "memory"],
        vec!["CONFIG", "GET", "maxmemory"], vec!["MULTI"],
        vec!["WATCH", "queued"],
    ] {
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &command), OOM, "{} inside MULTI", command[0]);
        assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS_ABORT, "{} dirties MULTI", command[0]);
        assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
    }
    // queueMultiCommand ignores later accepted commands once dirty: they
    // reply QUEUED but must not add DENYOOM flags that could change EXEC's
    // error if memory becomes unavailable again.
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["GET", "ignored"]), OOM);
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["SET", "ignored", "q"]), "+QUEUED\r\n");
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS_ABORT);
    assert_eq!(memory_request(&mut client, &["GET", "ignored"]), "$-1\r\n");
    // A queued DENYOOM command must not replace the previous-errors reply
    // when memory recovers before EXEC admission, and must not execute.
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["SET", "queued", "q"]), "+QUEUED\r\n");
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), OOM);
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["EXEC"]), PREVIOUS_ERRORS_ABORT);
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
    drop(client);
    drop(other);
    assert_eq!(finished(child).0, 0);
}

/// processCommand uses the queued flag union for EXEC. Only OOM at EXEC
/// admission uses execCommandAbort's explicit cause. EXEC, DISCARD, QUIT
/// and RESET are exempt from the blanket refusal of MULTI queueing.
#[test]
fn multi_oom_refusal_exec_flags_and_control_exemptions() {
    let (child, mut client, port) = start(b"2");
    let mut other = connect_when_ready(port);
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["EXEC"]), "*0\r\n");
    for control in ["DISCARD", "RESET"] {
        assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
        assert_eq!(memory_request(&mut client, &["GET", "queued"]), OOM);
        assert_eq!(memory_request(&mut client, &[control]), if control == "RESET" { "+RESET\r\n" } else { "+OK\r\n" });
    }
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["SET", "queued", "q"]), "+QUEUED\r\n");
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["EXEC"]), OOM_ABORT);
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
    // EXEC admission precedes the dirty-transaction check: an already
    // queued DENYOOM command still causes the explicit OOM abort here.
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["SET", "queued", "q"]), "+QUEUED\r\n");
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), OOM);
    assert_eq!(memory_request(&mut client, &["EXEC"]), OOM_ABORT);
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), "$-1\r\n");
    // Queue buffers are already counted when the limit is chosen. The
    // execution's fresh value allocations cross it, but EXEC must finish.
    configure(&mut other, &["maxmemory", "0"]);
    let value = "x".repeat(VALUE_BYTES);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    for key in ["exec:1", "exec:2", "exec:3"] {
        assert_eq!(memory_request(&mut client, &["SET", key, &value]), "+QUEUED\r\n");
    }
    let executing_limit = number(&info(&mut other, "memory"), "used_memory") + VALUE_BYTES as u64;
    configure(&mut other, &["maxmemory", &executing_limit.to_string()]);
    assert_eq!(memory_request(&mut client, &["EXEC"]), "*3\r\n+OK\r\n+OK\r\n+OK\r\n");
    assert_eq!(memory_request(&mut client, &["SET", "after-exec", &value]), OOM);
    configure(&mut other, &["maxmemory", "0"]);
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), "+QUEUED\r\n");
    configure(&mut other, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["EXEC"]), "*1\r\n$-1\r\n");
    assert_eq!(memory_request(&mut client, &["MULTI"]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["GET", "queued"]), OOM);
    assert_eq!(memory_request(&mut client, &["QUIT"]), "+OK\r\n");
    drop(client);
    drop(other);
    assert_eq!(finished(child).0, 0);
}

/// scriptVerifyOOM uses state at script start; scriptCall sets WRITE_DIRTY
/// before invoking an accepted write even when DEL removes no key. A retry
/// restarts that state independently of the effects flag used for rollback.
#[test]
fn scripts_refuse_before_a_write_and_continue_after_a_noop_write() {
    let (child, mut client, _) = start(b"1");
    let value = "s".repeat(VALUE_BYTES);
    configure(&mut client, &["maxmemory", "1"]);
    assert_eq!(memory_request(&mut client, &["EVAL", "return redis.pcall('SET','script-key',ARGV[1])", "0", &value]), OOM);
    let written = "redis.call('DEL','missing'); local n=0; for i=1,20000 do n=n+i end; return redis.call('SET','script-key',ARGV[1])";
    assert_eq!(memory_request(&mut client, &["EVAL", written, "0", &value]), "+OK\r\n");
    assert_eq!(memory_request(&mut client, &["STRLEN", "script-key"]), format!(":{VALUE_BYTES}\r\n"));
    let refused = memory_request(&mut client, &["EVAL", "return redis.call('SET','script-key','new')", "0"]);
    assert!(refused.contains("OOM command not allowed when used memory > 'maxmemory'."), "{refused}");
    assert_eq!(memory_request(&mut client, &["STRLEN", "script-key"]), format!(":{VALUE_BYTES}\r\n"));
    for script in ["#!lua\nreturn 1", "#!lua flags=allow-oom\nreturn 1", "#!lua flags=no-writes\nreturn 1"] {
        for prefix in ["EVAL", "SCRIPT"] {
            let args = if prefix == "EVAL" { vec!["EVAL", script, "0"] } else { vec!["SCRIPT", "LOAD", script] };
            assert_eq!(memory_request(&mut client, &args), "-ERR firn does not support Redis script shebangs or flags\r\n");
        }
    }
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// A small single write supplies CONFIG and following commands together to
/// exercise the read snapshot refresh, including lifting a previous limit.
#[test]
fn pipelined_config_refreshes_admission() {
    let (child, mut client, _) = start(b"1");
    let mut pipeline = resp(&["CONFIG", "SET", "maxmemory", "1"]);
    pipeline.extend(resp(&["SET", "pipelined", "v"]));
    pipeline.extend(resp(&["CONFIG", "SET", "maxmemory", "0"]));
    pipeline.extend(resp(&["SET", "pipelined", "v"]));
    client.write_all(&pipeline).unwrap();
    expect_replies(&mut client, format!("+OK\r\n{OOM}+OK\r\n+OK\r\n").as_bytes(), "same-read CONFIG refresh");
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// Tenacity zero still makes progress for 16 deletions, then permits the
/// command with EVICT_RUNNING. A later command continues the work.
#[test]
fn bounded_eviction_continues_on_later_commands() {
    let (child, mut client, _) = start(b"1");
    let value = "b".repeat(VALUE_BYTES);
    for index in 0..48 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("batch:{index}"), &value]), "+OK\r\n");
    }
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "allkeys-random", "maxmemory-eviction-tenacity", "0"]);
    assert_eq!(memory_request(&mut client, &["SET", "allowed-while-running", &value]), "+OK\r\n");
    for _ in 0..5 {
        assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n");
    }
    assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), 49);
    assert_eq!(memory_request(&mut client, &["SET", "now-refused", &value]), OOM);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// genRedisInfoString reports raw heap and exclusions separately.
/// configResetStatCommand/resetServerStats zero eviction and connection
/// counts while preserving stat_peak_memory after the live heap shrinks.
#[test]
fn info_memory_fields_peak_and_resetstat_are_live() {
    let (child, mut client, _) = start(b"1");
    let initial = info(&mut client, "memory");
    assert!(number(&initial, "used_memory") > 0);
    assert!(number(&initial, "used_memory_rss") > 0);
    assert!(number(&initial, "used_memory_peak") >= number(&initial, "used_memory"));
    assert_eq!(number(&initial, "mem_not_counted_for_evict"), 0);
    assert_eq!(info_field(&initial, "maxmemory_human").as_deref(), Some("0B"));
    assert!(info_field(&initial, "used_memory_human").is_some());
    let value = "m".repeat(VALUE_BYTES);
    for index in 0..32 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("peak:{index}"), &value]), "+OK\r\n");
    }
    let high = number(&info(&mut client, "memory"), "used_memory_peak");
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "allkeys-lru", "maxmemory-eviction-tenacity", "100"]);
    assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n");
    assert!(number(&info(&mut client, "stats"), "evicted_keys") > 0);
    let evicted = info(&mut client, "memory");
    assert!(number(&evicted, "used_memory") < high, "heap must shrink before RESETSTAT: {evicted}");
    let peak_before_reset = number(&evicted, "used_memory_peak");
    assert!(peak_before_reset >= high);
    assert_eq!(memory_request(&mut client, &["CONFIG", "RESETSTAT"]), "+OK\r\n");
    let reset_stats = info(&mut client, "stats");
    assert_eq!(number(&reset_stats, "evicted_keys"), 0);
    assert_eq!(number(&reset_stats, "total_connections_received"), 0);
    let reset = info(&mut client, "memory");
    assert!(number(&reset, "used_memory_peak") >= peak_before_reset, "{reset}");
    assert!(number(&reset, "used_memory_peak") >= number(&reset, "used_memory"));
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// AOF eviction propagates DEL, so a restart must not resurrect a victim.
/// Even a drained writer retains allocated capacity, excluded in INFO.
#[test]
fn aof_capacity_is_excluded_and_eviction_survives_replay() {
    let program = CompiledProgram::from_environment();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", b"memory.aof"]);
    let mut client = connect_when_ready(port);
    let value = "a".repeat(VALUE_BYTES);
    assert_eq!(memory_request(&mut client, &["SET", "victim", &value]), "+OK\r\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let persisted = info(&mut client, "persistence");
        if number(&persisted, "aof_current_size") >= resp(&["SET", "victim", &value]).len() as u64 { break; }
        assert!(Instant::now() < deadline, "writer did not drain the SET");
        std::thread::sleep(Duration::from_millis(10));
    }
    let measured = info(&mut client, "memory");
    assert!(number(&measured, "mem_not_counted_for_evict") >= 2 * 65536, "empty AOF buffers still own capacity: {measured}");
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "allkeys-random", "maxmemory-eviction-tenacity", "100"]);
    for args in [vec!["NOT-A-COMMAND"], vec!["SET", "bad"], vec!["CONFIG", "BAD"], vec!["CONFIG", "GET"], vec!["OBJECT", "FREQ"]] {
        assert!(memory_request(&mut client, &args).starts_with("-ERR "));
    }
    // Closing the connection drains AOF without another command's eviction.
    drop(client);
    assert_eq!(finished(child).0, 0);
    let file = std::fs::read(aof_incremental_path(program.working_directory(), "memory.aof")).unwrap();
    let removal = resp(&["DEL", "victim"]);
    assert!(!file.windows(removal.len()).any(|window| window == removal), "preflight errors must not evict");
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", b"memory.aof"]);
    let mut client = connect_when_ready(port);
    assert_eq!(memory_request(&mut client, &["STRLEN", "victim"]), format!(":{VALUE_BYTES}\r\n"));
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "allkeys-random", "maxmemory-eviction-tenacity", "100"]);
    assert_eq!(memory_request(&mut client, &["PING"]), "+PONG\r\n");
    assert_eq!(memory_request(&mut client, &["EXISTS", "victim"]), ":0\r\n");
    assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), 1);
    drop(client);
    assert_eq!(finished(child).0, 0);
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", b"memory.aof"]);
    let mut client = connect_when_ready(port);
    assert_eq!(memory_request(&mut client, &["GET", "victim"]), "$-1\r\n");
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// Correct the existing constant cached-script count while changing INFO.
#[test]
fn info_counts_registered_scripts_and_flush() {
    let (child, mut client, _) = start(b"1");
    assert_eq!(number(&info(&mut client, "memory"), "number_of_cached_scripts"), 0);
    for expected in [1, 1] {
        assert!(memory_request(&mut client, &["SCRIPT", "LOAD", "return 101"]).starts_with("$40\r\n"));
        assert_eq!(number(&info(&mut client, "memory"), "number_of_cached_scripts"), expected);
    }
    assert_eq!(memory_request(&mut client, &["EVAL", "return 102", "0"]), ":102\r\n");
    assert_eq!(number(&info(&mut client, "memory"), "number_of_cached_scripts"), 2);
    assert_eq!(memory_request(&mut client, &["SCRIPT", "FLUSH"]), "+OK\r\n");
    assert_eq!(number(&info(&mut client, "memory"), "number_of_cached_scripts"), 0);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// A read-only script creates no volatile eligibility. Even unlimited
/// eviction must complete the empty scan and let SCRIPT KILL run while
/// that script is still retrying, without waiting for it to finish.
#[test]
fn unlimited_eviction_does_not_block_script_kill() {
    let (child, mut looping, port) = start(b"2");
    let mut other = connect_when_ready(port);
    let value = "k".repeat(VALUE_BYTES);
    assert_eq!(memory_request(&mut other, &["SET", "permanent", &value]), "+OK\r\n");
    configure(&mut other, &["maxmemory", "1", "maxmemory-policy", "volatile-lru", "maxmemory-eviction-tenacity", "100"]);
    assert_eq!(memory_request(&mut other, &["SET", "refused", &value]), OOM);
    looping.write_all(&resp(&["EVAL", "while true do end", "0"])).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let reply = memory_request(&mut other, &["SCRIPT", "KILL"]);
        if reply == "+OK\r\n" { break; }
        assert_eq!(reply, "-NOTBUSY No scripts in execution right now.\r\n");
        assert!(Instant::now() < until, "script did not start");
        std::thread::sleep(Duration::from_millis(1));
    }
    let killed = whole_reply(&mut looping, "killed script under unlimited eviction");
    assert!(killed.starts_with("-ERR Script killed by user with SCRIPT KILL... script: "), "{killed}");
    assert_eq!(memory_request(&mut other, &["EXISTS", "permanent"]), ":1\r\n");
    assert_eq!(memory_request(&mut other, &["SET", "still-refused", &value]), OOM);
    drop(looping);
    drop(other);
    assert_eq!(finished(child).0, 0);
}

/// A volatile-policy empty search can span time slices. Permanent SETs
/// must not discard that progress: they add no eviction eligibility.
#[test]
fn permanent_writes_do_not_restart_volatile_exhaustion() {
    let (child, mut client, _) = start(b"1");
    let value = "e".repeat(VALUE_BYTES);
    for index in 0..512 {
        assert_eq!(memory_request(&mut client, &["SET", &format!("permanent:{index}"), &value]), "+OK\r\n");
    }
    configure(&mut client, &["maxmemory", "1", "maxmemory-policy", "volatile-lru", "maxmemory-samples", "1", "maxmemory-eviction-tenacity", "0"]);
    let mut refused = false;
    for _ in 0..128 {
        let reply = memory_request(&mut client, &["SET", "permanent:0", &value]);
        if reply == OOM {
            refused = true;
            break;
        }
        assert_eq!(reply, "+OK\r\n", "a time slice permits the command");
    }
    assert!(refused, "permanent writes kept restarting the empty volatile scan");
    assert_eq!(number(&info(&mut client, "stats"), "evicted_keys"), 0);
    assert_eq!(memory_request(&mut client, &["DBSIZE"]), ":512\r\n");
    drop(client);
    assert_eq!(finished(child).0, 0);
}
