# INFO counters without a shared write on every command

Board item: `firn-scope-info`. The counter study below remains a proposed
design, not a counter implementation or performance result. The later
[INFO field audit](#info-field-audit) records the separate field corrections
and their network cases.

## Question and conclusion

Can firn report Redis 7.2.16's `total_commands_processed`, `keyspace_hits`,
`keyspace_misses`, `expired_keys` and `total_net_input_bytes`, including
`CONFIG RESETSTAT`, without all connections writing one contended location?

Recommend runtime-selected shards, with a whole-shards observation for INFO
and RESETSTAT. Ordinary commands update only the selected shard; an INFO
observation sums all shards and subtracts a reset baseline. Keep the existing
expiry counter in Meta, where actual expiry deletion already needs a write.
Do not delay publication until a timer, another request or connection closure.
This preserves exact observations without a global counter write per command.

The Whitefoot release inspected in this counter study cannot express this
mechanism: it has independent
`Shared<T>` objects but no `Shards<T>` or runtime-selected shard target.
Whitefoot's [sharded-state PR #385](https://github.com/Ming-Research/Whitefoot/pull/385)
contains the appropriate proposed surface and implementation. Its presence on
a work branch is not availability in firn's release. Stop implementation at
this dependency; do not build a hand-indexed approximation to hide it.

The competing connection-owned design is expressible for individual counters,
but a dynamic collection cannot be held as one atomic observation using the
pinned target grammar. Retirement and reset make that distinction substantive.
The alternatives below explain both the usable part and the remaining gap.

## Evidence boundary

Inspected firn revision `fcd4d5c3688667522e301185c98f2a36dbfad12d`, pin
`wf-d75f318d5631`, and the local Whitefoot object at
`d75f318d56318834785322bde3a3656fb93116a6`. Language claims below refer to that
commit's specification and standard-library interfaces, not the kit's newer
examples. The handshake comparison is the local remote-tracking revision
`de6d7b53a2352cb8139d8c65b74867467fe5468d` of `origin/claude/client-handshake`.
The inspected shards proposal is
`591fe54ba0dfb0b6c044490c1b1e195329ab0a42` of
`origin/claude/shared-counter-study`; its implementation commit is not an
ancestor of the pin. Remote PR/CI/release status was not polled or established.

Redis evidence is the source and tests at tag **7.2.16**. The local source
archive used for inspection matches `tests/redis-suite/reference.sh`'s SHA-256
`960a8ec15e34ff40e57ff16837b26b33bd81f2da6d24497bb63de532a323a18e`.
Source links below use the tag. The repository's `tests/redis-suite/` contains
the runner and ratchet records, not a vendored Tcl suite; its runner obtains
that suite from the pinned archive. No claim here uses firn's old replies as
an oracle.

## Counting rules

### Processed commands

[Redis call()](https://github.com/redis/redis/blob/7.2.16/src/server.c#L3476)
increments `stat_numcommands` after invoking the handler, provided the client
is not blocked. It does not simply count received requests or successful
replies. Unknown commands, wrong table arity, authentication/ACL/OOM and other
admission failures before `call()` do not count. An executed handler that
returns an error does count. QUIT is an ordinary executed handler and counts
([quitCommand](https://github.com/redis/redis/blob/7.2.16/src/networking.c#L3007)).
INFO reads before its own increment; CONFIG RESETSTAT clears inside its
handler, then its completion increments the new total to one.

[MULTI queueing](https://github.com/redis/redis/blob/7.2.16/src/server.c#L4156)
does not invoke `call()` for queued members. MULTI counts when executed;
[EXEC](https://github.com/redis/redis/blob/7.2.16/src/multi.c#L148) counts each
member actually called, then itself. Aborted transactions do not count their
unexecuted members. Distinguish an EXEC handler's abort from an EXEC rejected
before `call()`. An ACL rejection during EXEC skips that member's call.

[Script commands](https://github.com/redis/redis/blob/7.2.16/src/script.c#L520)
entered through `redis.call` or `redis.pcall` count individually, followed by
the outer EVAL/EVALSHA. Pre-call script validation failures do not count an
inner invocation; handler errors do. Lua execution steps are not commands.
Firn must distinguish a discarded interpreter attempt from an accepted
execution ending in a Lua error: discard the former's counters, preserve the
latter's already executed commands and lookups.

Blocking attempts do not increment the total until final reprocessing or
[updateStatsOnUnblock](https://github.com/redis/redis/blob/7.2.16/src/blocked.c#L100).
The timeout path of blocking list/zset/stream operations also accounts for
completion. A disconnected unfinished operation is not a completed command.
This is a rule for any future blocking support, not a claim firn has it.

Loading is not a blanket exclusion: [direct AOF replay](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1552)
calls the handler without `call()`, but members of a replayed EXEC go through
`call(CMD_CALL_NONE)` ([multi.c](https://github.com/redis/redis/blob/7.2.16/src/multi.c#L229)).
The global command increment is outside the loading guard that suppresses
commandstats. Preserve that distinction in the implementation audit.

### Keyspace hits and misses

[lookupKey](https://github.com/redis/redis/blob/7.2.16/src/db.c#L63) resolves
expiry, then counts one hit for a live value or one miss for absence, unless
`LOOKUP_NOSTATS` or `LOOKUP_WRITE` suppresses statistics. The write wrappers
add `LOOKUP_WRITE`. `LOOKUP_NOTOUCH` suppresses access stamps, not statistics;
`LOOKUP_NONOTIFY` suppresses notifications, not statistics.

| Lookup path | What contributes |
|---|---|
| GET, MGET and other read helpers | Each lookup actually reached, before a subsequent type error; a present wrong-type key is a hit. |
| [EXISTS](https://github.com/redis/redis/blob/7.2.16/src/db.c#L746) | One lookup per argument, including repeated keys; NOTOUCH still counts. |
| [TYPE](https://github.com/redis/redis/blob/7.2.16/src/db.c#L1176) | One NOTOUCH read lookup. |
| [OBJECT](https://github.com/redis/redis/blob/7.2.16/src/object.c#L1448) ENCODING/REFCOUNT/IDLETIME/FREQ | NOTOUCH plus NONOTIFY; still counts. Invalid options/arity must follow the actual helper reachability. |
| [TTL family](https://github.com/redis/redis/blob/7.2.16/src/expire.c#L689) | TTL/PTTL/EXPIRETIME/PEXPIRETIME use a counted NOTOUCH lookup. |
| Ordinary SET or INCR write lookup | No hit or miss. |
| [GETEX](https://github.com/redis/redis/blob/7.2.16/src/t_string.c#L326) | A counted read lookup, although the command can write. |
| Explicit NOSTATS callers | Cluster migration checks and the module open-key option suppress counts; neither makes OBJECT uncounted. |

Audit command lookup sites, including short circuits, duplicate operands,
scripts and EXEC, against Redis. Firn's existing `update_access` is not an
adequate counting hook: NOTOUCH callers omit it and write lookups may use it.
Neither command READONLY/WRITE flags nor the number of distinct locked keys
determines the count. Internal scans, expiry probes and eviction probes must
not accidentally become Redis read lookups.

### Expiration

[deleteExpiredKeyAndPropagate](https://github.com/redis/redis/blob/7.2.16/src/db.c#L1679)
increments `expired_keys` on actual active or lazy expiration deletion. A
deadline passing alone does not increment it. A lazy read of an expired key
can therefore cause both one expiration and one lookup miss. Ordinary DEL,
eviction, and [setting an already elapsed expiry on a live key](https://github.com/redis/redis/blob/7.2.16/src/expire.c#L631)
are not expiration-counter increments; the last path deletes directly.

Firn maintains `Meta.expired_keys` in active/lazy removal helpers and eviction's
expired-entry path, but INFO currently omits it. Merely exposing that field
does not establish complete compatibility: audit direct entry removals and
script rollback as well. A key whose deadline was replaced before the queued
expiry is processed must not count; deletion and increment must commit
together, once. Keep this counter in Meta rather than count it again in a shard.

### Received bytes

[readQueryFromClient](https://github.com/redis/redis/blob/7.2.16/src/networking.c#L2695)
adds every positive `connRead` result before parsing or command validation.
Count protocol framing, incomplete requests, invalid requests, already-read
pipeline suffixes and bytes from connections later closed. Zero/error reads
add nothing. Lua-generated arguments and AOF file bytes are not network input.
[INFO](https://github.com/redis/redis/blob/7.2.16/src/server.c#L5952) adds normal
and replication input counters; firn has no replication input today.

In `serve`, add `next - held` for successful `receive_next(start: held, ...)`,
before parsing, not `next` and not each consumed command's length. Buffer
compaction contributes zero. Publish each receive contribution before
processing its first command. RESETSTAT removes all bytes already received,
including later commands in the same read. An INFO after reset has no fixed
byte count unless the experiment controls receive boundaries; TCP writes do
not guarantee those boundaries.

### RESETSTAT and the other fields firn has

[configResetStatCommand](https://github.com/redis/redis/blob/7.2.16/src/config.c#L3409)
calls `resetServerStats`, `resetCommandTableStats` and `resetErrorTableStats`.
The [complete server reset](https://github.com/redis/redis/blob/7.2.16/src/server.c#L2495)
clears command/connection, expiry cycle, eviction, hit/miss, defragmentation,
fork, rejected-connection, replication synchronization, I/O operation/byte,
instantaneous sample, AOF rewrite/RDB save/consecutive rewrite failure, error,
sanitization, delayed-fsync, reply-buffer, event-loop-duration/max-command and
completed-lazyfree statistics. Command table calls, durations and latency
histograms and the error table reset through the other two functions.

| Firn state | Required treatment |
|---|---|
| Four new hot counters | Set their baseline to the current whole-shards totals. |
| `ServerState.connections` | Clear, as today. This is a total, unlike live clients. |
| `Meta.expired_keys`, `Meta.evicted_keys` | Clear, as today. |
| `Meta.aof_rewrites` | **Also clear. The current reset omits it.** |
| Fixed-zero unsupported-feature statistics | Remain zero; do not invent new state. |
| `Meta.memory_peak`, live clients, startup time, AOF sizes/status/buffers, rewrite operational state | Preserve. They are not reset statistics. |

The omission of `aof_rewrites` is an inferred source defect, not a locally
observed failed test. Include its correction and a CI witness when this item
is implemented. The empty commandstats/errorstats sections do not acquire
new implementations through this five-counter task.

## Suite coverage, consumers and whether lag is acceptable

Search result: no occurrence of `keyspace_hits`, `keyspace_misses`, or `total_net_input_bytes` in the Redis 7.2.16 tests directory. `total_commands_processed` occurs once in TCL; `expired_keys` has four direct assertion scenarios, plus cluster function names which refer to expiration behavior rather than this INFO counter. Therefore the ratchet alone cannot establish the three absent fields or exact `total_commands_processed` counting. These need dedicated oracle cases.

### Direct checks of the five requested fields

| Unit / exact test | Assertion | Connection relationship and Firn external gate |
| --- | --- | --- |
| [integration/failover: failover command to specific replica works](https://github.com/redis/redis/blob/7.2.16/tests/integration/failover.tcl#L63-L92) | `wait_for_condition 50 100 { [s 0 total_commands_processed] > 100 }` while another replica process is paused. This is a lower bound, never an exact total. | `start_write_load` starts a separate TCL writer process and connection; `s 0` sends INFO on the server block's control connection. Thus it observes other connections. Outer `external:skip` excludes it from Firn's external-mode gate. |
| [unit/pause: Test both active and passive expires are skipped during client pause](https://github.com/redis/redis/blob/7.2.16/tests/unit/pause.tcl#L263-L288) | Save `expired_keys`, create two PX=10 keys and CLIENT PAUSE WRITE inside MULTI/EXEC; after they become logically expired, assert unchanged count; unpause and GET both; assert exactly original +2. | `r` and `s 0` use the same control connection. Active expiration is a server background producer. This block has no external:skip. Exact named test is absent from current passing.tsv and neither unstable.tsv nor hung.tsv names it. |
| [unit/expire: expired key which is created in writeable replicas should be deleted by active expiry](https://github.com/redis/redis/blob/7.2.16/tests/unit/expire.tcl#L577-L594) | For appendonly=yes and no, save replica `expired_keys`, SET PX=1 on writable replica, poll 100×10ms for original +1 and GET empty. | `$replica` and `s` identify the inner replica's control connection, so the SET is same-connection. Background active expiry produces the increment. Enclosing `needs:repl external:skip` excludes it from external gate. |
| [unit/moduleapi/keyspace_events: Test expired key space event](https://github.com/redis/redis/blob/7.2.16/tests/unit/moduleapi/keyspace_events.tcl#L86-L96) | Save `expired_keys`; SET PX=10; poll 100×10ms for original +1; assert module's `testkeyspace:expired` key equals 1. | SET, INFO and GET use `r`/`s`, same control connection; expiration and module notification are background paths. Requires MODULE LOAD, not an implemented Firn workload; no current ratchet row. |
| [unit/moduleapi/infotest: module reading info](https://github.com/redis/redis/blob/7.2.16/tests/unit/moduleapi/infotest.tcl#L12-L38) | Module API command `info.geti stats expired_keys` returns 0; also tests string/float/unsigned INFO access. | Module reads INFO internally on a command from the control connection; no workload on another connection. Requires MODULE LOAD, no current ratchet row. |

`r` and `s` connection equivalence follows [tests/test_helper.tcl](https://github.com/redis/redis/blob/7.2.16/tests/test_helper.tcl#L206-L213) (`r` calls `[srv level client]`) and [the `s` definition](https://github.com/redis/redis/blob/7.2.16/tests/test_helper.tcl#L295-L305) (`status [srv level client]`). [status](https://github.com/redis/redis/blob/7.2.16/tests/support/util.tcl#L74-L83) issues INFO. The [external skip rule](https://github.com/redis/redis/blob/7.2.16/tests/support/server.tcl#L209-L220) returns early for an `external:skip` block. No direct requested-counter tests appear in Firn's current passing.tsv; none are selected in unstable.tsv or hung.tsv.

### RESETSTAT tests: what they really establish

RESETSTAT appears throughout the suite as setup for other counters; it does not imply an assertion of the five requested fields. Exact test-name inventory follows below. Relevant checks and ownership:

- [introspection-2](https://github.com/redis/redis/blob/7.2.16/tests/unit/introspection-2.tcl#L61-L119): GEOADD calls=1 and no ZADD; GEOADD WRONGTYPE calls=1/rejected=0/failed=1; EXPIRE calls=1 and no DEL; BRPOP calls=1 and no RPOP; MULTI and EXEC and each queued command calls=1; scripts EVAL=1, SET=2, EXPIRE=1 and GEOADD=1. All command requests and INFO share `r`. Internal script calls and EXEC entries are separate counted executions, not separate network connections. These six exact names have no current ratchet rows.
- [info](https://github.com/redis/redis/blob/7.2.16/tests/unit/info.tcl): six latencystats tests check disabled/empty and enabled percentiles; RESETSTAT removes percentiles; errorstats tests check global `total_error_replies` and per-error counts and per-command calls/rejected_calls/failed_calls. The Lua and MULTI/EXEC tests pin attribution without asserting global total_commands_processed. `latencystats: blocking commands` reads BLPOP percentiles after two BLPOPs on `$rd`; `errorstats: blocking commands` reads UNBLOCKED count=1, BLPOP calls=1/failed=1 and total_error_replies=1 after `$rd` is unblocked by `r`. Those read other-connection facts, including immediately after `$rd read` returns. `stats: instantaneous metrics` polls eventloop cycles/duration metrics. Entire info unit is external:skip.
- [latency-monitor](https://github.com/redis/redis/blob/7.2.16/tests/unit/latency-monitor.tcl#L6-L75): RESETSTAT itself remains a histogram command with calls=1; empty-histogram case dictionary has exactly one entry, config|resetstat. Subsequent cases pin SET=2, CLIENT ID/LIST=1, HSET/GET/HGETALL=1, dictionary lengths, and invalid-command omission. Same control connection, no other-client workload.
- [type/list](https://github.com/redis/redis/blob/7.2.16/tests/unit/type/list.tcl#L2356-L2403): both exact blocking-stat tests assert BLPOP calls=1/rejected_calls=0/failed_calls=0 after wake or CLIENT UNBLOCK TIMEOUT; `r` reads stats for `$rd`'s execution, the second immediately after CLIENT UNBLOCK. No current ratchet rows for these exact names.
- [acl](https://github.com/redis/redis/blob/7.2.16/tests/unit/acl.tcl#L326-L343): reprocessed blocked BLPOP on `$rd` becomes NOPERM after ACL change on `r`, then `r` reads BLPOP calls=0/usec=0/rejected=1/failed=0 after `$rd` closes. Cross-connection, including retired facts.
- [type/stream-cgroups](https://github.com/redis/redis/blob/7.2.16/tests/unit/type/stream-cgroups.tcl#L529-L545): after `$rd` XREADGROUP fails from `r`'s XGROUP DESTROY and `$rd` closes, `r` asserts NOGROUP count=1, XREADGROUP calls=1/rejected=0/failed=1, total_error_replies=1. Cross-connection and retired facts.
- [scripting](https://github.com/redis/redis/blob/7.2.16/tests/unit/scripting.tcl#L2188-L2340): `Consistent eval error reporting` repeatedly resets and checks OOM/ERR/WRONGTYPE count=1 and global errors=1, and exact nested SET/SELECT/GEOADD and EVAL/EVAL_RO rejected/failed attribution. Error API tests pin MY_ERR_CODE=1, empty error ERR=1; status API leaves MY_ERR_CODE absent. Same control connection, internal calls.
- [lazyfree](https://github.com/redis/redis/blob/7.2.16/tests/unit/lazyfree.tcl#L43-L90): waits background pending objects=0 then asserts lazyfreed_objects exactly 1 for complex stream, exactly 0 for stream with deleted cgroup. Same control client, background reclamation counter. Neither requested five nor other connections.
- [integration/redis-benchmark](https://github.com/redis/redis/blob/7.2.16/tests/integration/redis-benchmark.tcl): helper resets then runs external benchmark subprocess, and INFO on control connection checks exact commandstats from all benchmark clients, which have finished and disconnected. Basic SET/GET=10; default suite PING=200, LPUSH=200, LRANGE=400, all remaining listed commands=100; pipelined SET/GET=10010; arbitrary INCRBYFLOAT=150; keyspace test SET=1000; TLS SET=1000. Entire block external:skip. No requested field is asserted.
- [integration/replication](https://github.com/redis/redis/blob/7.2.16/tests/integration/replication.tcl#L88-L163): RESETSTAT on replica A then workload on primary B. A's INFO verifies replication rewritten commands: SET calls=3/no GETSET; BRPOPLPUSH replicated as RPOPLPUSH calls=1/no LMOVE; BLMOVE replicated as LMOVE calls=1/no RPOPLPUSH. Cross-connection/system producer; replication unit external:skip.
- [integration/psync2-master-restart](https://github.com/redis/redis/blob/7.2.16/tests/integration/psync2-master-restart.tcl): RESETSTAT on replica, restart upstream server, then INFO tests sync_partial_ok=1 for successful partial sync in three tests; final test sync_partial_ok=0 and sync_full=1. Replication connection generates facts read by control connection; external:skip.
- [maxmemory](https://github.com/redis/redis/blob/7.2.16/tests/unit/maxmemory.tcl#L6-L47): reusable init_test resets eviction counters and fills 50 keys; consumers then verify client-eviction mode evicted_clients>0/evicted_keys=0/dbsize=50 versus key-eviction mode evicted_clients=0/evicted_keys>0/dbsize<50. Tests exercise other deferring clients; external:skip.
- [memefficiency](https://github.com/redis/redis/blob/7.2.16/tests/unit/memefficiency.tcl): resets active-defrag counters before AOF load or chosen fragmentation workload, observes active_defrag_hits/misses and completion/memory effects; the names containing hits/misses here are not keyspace_hits/misses. Background defragmentation, not requested fields. external:skip.
- Module test RESETSTAT callers check module-auth AUTH/HELLO calls and failed calls, module errors (ERR/NULL/WRONGTYPE), blocked-command timing, eviction callbacks, and commandstat exclusion during AOF load. Moduleauth reads several `$rd`/other-client executions after completion or close, including killed AUTH leaving no commandstat row. Propagate tests read evicted_keys=3 or =2 generated by notifications/timer; AOF-load test SET calls=8 before load and no calls row after RESETSTAT and load. blockonbackground tests command calls=1 plus time thresholds 100000/250000/60000 microseconds, not the requested global counter. They require native modules outside current Firn scope.
- [cluster/tests/18-info](https://github.com/redis/redis/blob/7.2.16/tests/cluster/tests/18-info.tcl): resets both primaries then verifies MOVED errorstats and rejected per-command accounting. Separate cluster harness, not Firn's external runner.

No lag allowance follows from an absence of a named test. A counter design that preserves only a connection's private unpublished increments until its next request can lose a completed command from another connection indefinitely and can lose facts when it disconnects. Existing cross-connection commandstat/errorstat tests demonstrate the same visibility/retirement issue if Firn later applies this mechanism to those stats.

### Consumers checked

Firn's maintained consumer runner currently pins Django/connect-redis/etc., not Sidekiq or redis_exporter; Sidekiq is a queue candidate in research/investigations/consumers/README.md. Therefore these are inspected consumer examples, not a claim of coverage of a selected pinned workload.

redis_exporter local read-only clone is clean at `589a1cd930d62ba449a5207179ab44252e359c66`. [exporter.go](https://github.com/oliver006/redis_exporter/blob/589a1cd930d62ba449a5207179ab44252e359c66/exporter/exporter.go#L408-L430) maps all five requested INFO values to Prometheus counters: commands_processed_total, keyspace_hits_total, keyspace_misses_total, expired_keys_total, net_input_bytes_total. [Scrape](https://github.com/oliver006/redis_exporter/blob/589a1cd930d62ba449a5207179ab44252e359c66/exporter/exporter.go#L889-L902) requests INFO ALL and falls back to INFO. [info.go](https://github.com/oliver006/redis_exporter/blob/589a1cd930d62ba449a5207179ab44252e359c66/exporter/info.go#L73-L174) parses each field, and [metrics.go](https://github.com/oliver006/redis_exporter/blob/589a1cd930d62ba449a5207179ab44252e359c66/exporter/metrics.go#L39-L95) emits it as CounterValue without checking freshness. Inference: lag changes the exported sample and derived request/traffic rates and hit ratios; a long-lived idle connection whose last increments never publish remains undercounted across scrapes. A bounded delay might merely shift activity between scrape intervals, but the consumer supplies no permission to weaken exact Redis counts.

Sidekiq v7.3.9 [Config#redis_info](https://github.com/sidekiq/sidekiq/blob/v7.3.9/lib/sidekiq/config.rb) requests INFO and parses every `key:value` line to a hash. Its [dashboard](https://github.com/sidekiq/sidekiq/blob/v7.3.9/web/views/dashboard.erb) displays Redis version, uptime, connections, used memory and peak memory, not these five fields. Inference restricted to these two inspected paths: delay in these five does not change that dashboard's display or Config#redis_info parsing, while callers/extensions reading the returned hash can observe it. This does not establish absence of uses in Sidekiq Pro/Enterprise or every Sidekiq path; those were not available/read. Do not turn this limited observation into 'Sidekiq never notices lag'.

### Exact RESETSTAT name inventory

The source inventory below includes every upstream TCL literal `CONFIG RESETSTAT` call (deduplicated within its nearest named test); shared helper callers are named in the narrative above. All listed names concern non-requested counters unless the direct field table says otherwise. It is an inventory, not an executed-test report.

- `cluster/tests/18-info.tcl`: `errorstats: rejected call due to MOVED Redirection`.

- `integration/psync2-master-restart.tcl`: `PSYNC2: Partial resync after Master restart using RDB aux fields when offset is 0`; `PSYNC2: Partial resync after Master restart using RDB aux fields with data`; `PSYNC2: Partial resync after Master restart using RDB aux fields with expire`; `PSYNC2: Full resync after Master restart when too many key expired`.

- `integration/redis-benchmark.tcl`: through `common_bench_setup`, `benchmark: set,get`; `benchmark: connecting using URI set,get`; `benchmark: connecting using URI with authentication set,get`; `benchmark: full test suite`; `benchmark: multi-thread set,get`; `benchmark: pipelined full set,get`; `benchmark: arbitrary command`; `benchmark: keyspace length`; `benchmark: specific tls-ciphers`; `benchmark: tls connecting using URI with authentication set,get`. Directly: `benchmark: specific tls-ciphersuites`. The idle-mode/maxclients test does not reset.

- `integration/replication.tcl`: `GETSET replication`; `BRPOPLPUSH replication, when blocking against empty list`; `BRPOPLPUSH replication, list exists`; `BLMOVE ($wherefrom, $whereto) replication, when blocking against empty list`; `BLMOVE ($wherefrom, $whereto) replication, list exists`.

- `unit/acl.tcl`: `blocked command gets rejected when reprocessed after permission change`.

- `unit/info.tcl`: `latencystats: disable/enable`; `latencystats: configure percentiles`; `latencystats: bad configure percentiles`; `latencystats: blocking commands`; `latencystats: subcommands`; `latencystats: measure latency`; `errorstats: failed call authentication error`; `errorstats: failed call within MULTI/EXEC`; `errorstats: failed call within LUA`; `errorstats: failed call NOSCRIPT error`; `errorstats: failed call NOGROUP error`; `errorstats: rejected call unknown command`; `errorstats: rejected call within MULTI/EXEC`; `errorstats: rejected call due to wrong arity`; `errorstats: rejected call by OOM error`; `errorstats: rejected call by authorization error`; `errorstats: blocking commands`; `stats: instantaneous metrics`.

- `unit/introspection-2.tcl`: `command stats for GEOADD`; `errors stats for GEOADD`; `command stats for EXPIRE`; `command stats for BRPOP`; `command stats for MULTI`; `command stats for scripts`.

- `unit/latency-monitor.tcl`: `LATENCY HISTOGRAM with empty histogram`; `LATENCY HISTOGRAM all commands`; `LATENCY HISTOGRAM sub commands`; `LATENCY HISTOGRAM with a subset of commands`; `LATENCY HISTOGRAM command`; `LATENCY HISTOGRAM with wrong command name skips the invalid one`.

- `unit/lazyfree.tcl`: `lazy free a stream with all types of metadata`; `lazy free a stream with deleted cgroup`.

- `unit/maxmemory.tcl`: through `init_test`, `eviction due to output buffers of many MGET clients, client eviction: $client_eviction`; `eviction due to input buffer of a dead client, client eviction: $client_eviction`; `eviction due to output buffers of pubsub, client eviction: $client_eviction`, each instantiated for `false` and `true`.

- `unit/memefficiency.tcl`: `Active defrag - AOF loading`; `Active defrag eval scripts`; `Active defrag big keys`; `Active defrag big list`; `Active defrag edge case`.

- `unit/moduleapi/blockedclient.tcl`: `module client error stats`.

- `unit/moduleapi/blockonbackground.tcl`: ` blocked clients time tracking - check blocked command that uses RedisModule_BlockedClientMeasureTimeStart() is tracking background time`; ` blocked clients time tracking - check blocked command that uses RedisModule_BlockedClientMeasureTimeStart() is tracking background time even in timeout `; ` blocked clients time tracking - check blocked command with multiple calls RedisModule_BlockedClientMeasureTimeStart()  is tracking the total background time `.

- `unit/moduleapi/moduleauth.tcl`: `test module AUTH for non existing / disabled users`; `test non blocking module AUTH`; `test non blocking module HELLO AUTH`; `test non blocking module HELLO AUTH SETNAME`; `test blocking module AUTH`; `test blocking module HELLO AUTH`; `test blocking module HELLO AUTH SETNAME`; `test AUTH after registering multiple module auth callbacks`; `module auth during blocking module auth`; `module auth inside MULTI EXEC`; `Disabling Redis User during blocking module auth`; `Killing a client in the middle of blocking module auth`; `test RM_AbortBlock Module API during blocking module auth`; `test RM_RegisterAuthCallback Module API during blocking module auth`; `Module unload during blocking module auth`.

- `unit/moduleapi/propagate.tcl`: `module propagation with notifications with eviction case 2`; `module propagation with timer and CONFIG SET maxmemory`; `Modules RM_Call does not update stats during aof load: AOF-load type $aofload_type`.

- `unit/scripting.tcl`: `Consistent eval error reporting`; `LUA redis.error_reply API`; `LUA redis.error_reply API with empty string`; `LUA redis.status_reply API`.

- `unit/type/list.tcl`: `Blocking command accounted only once in commandstats`; `Blocking command accounted only once in commandstats after timeout`.

- `unit/type/stream-cgroups.tcl`: `XGROUP DESTROY should unblock XREADGROUP with -NOGROUP`.

Recommendation: INFO must include all contributions published before its
whole-snapshot point, across connections, including completed work whose
connection is now idle or closed. Concurrent unfinished operations may land
on either side of that point. No deliberate time-based lag is proposed.
The suite alone does not prove that weaker behavior is acceptable; the exporter
would expose it. RESETSTAT must define the same boundary for every shard.

## Whitefoot facilities and the missing natural form

At the pin, [SHARE-1 through SHARE-3](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/spec/kernel-spec.md#L2289-L2357)
give each `Shared<T>` its own object. `shared_read` retains the object with
read-only authority; it still requires an atomic statement. `Frozen<T>` can
be read without an atomic statement, but cannot change and cannot contain
Shared/SharedRead or host handles. No exposed scalar atomic load/store/RMW or
mutable lock-free publication cell exists in the prelude or standard-module
inventory. `Inputs.drivers` reports a count, not the running driver's identity;
WAIT-2 deliberately makes placement unobservable.

The following complete illustrative program uses only released forms. The
writer and reader can overlap; only the writer changes the payload. Its final
read follows a join. Source acceptance remains for CI; no compiler diagnostic
is claimed here.

```whitefoot
fn write_count(counter: Shared<u64>) -> result: unit pure waits {
  atomic value = &counter {
    set value^ = value^ +wrap 1_u64;
  }
  return unit;
}

fn main() -> status: std::process::ExitStatus pure waits {
  let counter = shared_new::<u64>(value: 0_u64);
  let reader = shared_read::<u64>(shared: &counter);
  let writer = shared_share::<u64>(shared: &counter);
  let completed = spawn write_count(counter: move writer);
  let before_join = 0_u64;
  atomic value = &reader {
    set before_join = value^;
  }
  let joined = completed;
  let after_join = 0_u64;
  atomic value = &reader {
    set after_join = value^;
  }
  if after_join == 1_u64 {
    return std::process::exit_status(code: 0_u8);
  }
  return std::process::exit_status(code: 1_u8);
}
```

This is synchronized access, not a lock-free cell. Per-connection objects
separate writers, but INFO briefly contends with each owner. The pinned
compiler lowers ordinary whole-object statements to exclusive holds; it may
also retain/release value handles around statements. A read-only handle
restricts authority, not lock cost. Inspect the actual emitted path in CI:
ordinary Shared ownership alone promises neither cache-line isolation nor
absence of runtime bookkeeping contention.

### Gap G1: runtime-selected partitions with an exact whole observation

The natural semantic witness is one writer-selected partition per update,
with one atomic all-partitions observation. Below is a complete **proposed
language** program using #385's surface, unavailable at the study's inspected pin:

```whitefoot
struct Counts {
  commands: u64;
}

fn empty_counts() -> made: Counts pure {
  return Counts(commands: 0_u64);
}

fn bump(stats: &Shared<Shards<Counts>>) -> result: unit reads(stats) waits {
  let selected = any_shard();
  atomic part = &stats^[selected] {
    set part^.commands = part^.commands +wrap 1_u64;
  }
  return unit;
}

fn main() -> status: std::process::ExitStatus pure waits {
  let stats = shared_shards_new::<Counts, fn empty_counts>();
  bump(stats: &stats);
  let total = 0_u64;
  atomic all = &stats {
    for (index in 0_u64..all^.len) {
      set total = total +wrap all^[index].commands;
    }
  }
  if total == 1_u64 {
    return std::process::exit_status(code: 0_u8);
  }
  return std::process::exit_status(code: 1_u8);
}
```

The missing declarations/target form are `Shards`, `shared_shards_new`,
`any_shard` and the shard atomic target, not integer arithmetic. The existing
[Whitefoot study](https://github.com/Ming-Research/Whitefoot/blob/591fe54ba0dfb0b6c044490c1b1e195329ab0a42/research/investigations/shared-counters/DESIGN.md#54-c-a-sharded-state-one-part-per-driver-all-parts-for-a-read-recommended)
and [branch specification](https://github.com/Ming-Research/Whitefoot/blob/591fe54ba0dfb0b6c044490c1b1e195329ab0a42/spec/kernel-spec.md)
cover this need, including owned values made once per shard. Counts themselves
need only copyable values; no owned buffer capability is required here.

The specification does not promise a persistent connection-to-driver mapping
or a particular shard count. The inspected implementation proposal uses
cache-line-separated thread partitions, including spare threads, so driver
handoff cannot give two threads unsynchronized access to one partition. A
shard hold excludes a whole observation; it is not an unsynchronized plain
write. Its performance depends on the lean handle path discussed with
[Whitefoot #409](https://github.com/Ming-Research/Whitefoot/pull/409).
The branch study records pending acceptance measurements; this study does
not turn those estimates into a release or performance claim.

No new Whitefoot request was filed: this session has no callable board MCP
tools. Attach this witness and firn's acceptance requirements to the existing
shards request from `firn-scope-info`, rather than file duplicate language work.

## Proposed mechanism

### State, publication and snapshot

One server-lifetime `Shared<Shards<Counts>>` holds cumulative command, hit,
miss and receive-byte counters. Each shard writer changes its own selected
payload under the shard target. Selection is per atomic statement, not cached
as a connection index. A separate reset-baseline object is touched only by
INFO and RESETSTAT. It holds four aggregate values, initially zero.

Ordinary key commands add the shard target to their existing key statement;
shared command parts receive an ordinary reference to statement-local deltas,
not a capability to open another atomic statement. Commit the deltas before
releasing the statement and before publishing its reply. Handle completion
accounting at the actual equivalent of `call()`, once per executed command,
including keyless commands and errors. Where a handler spans several
statements, preserve its event order and finish its command count before the
reply; do not use received/queued/sent as interchangeable states.

Publish receive-byte deltas in a short shard statement immediately after each
successful receive, before parsing. Receive batching amortizes this cost;
buffering command completions until the end of that batch is not allowed.

INFO takes the whole shards object, baseline, Meta and ServerState in one
statement for these statistics, copies out their values, and constructs the
reply outside the hold. For each hot field, report `sum(shards) - baseline`.
Copy the pre-increment values, then increment INFO's command count through the
whole target before releasing that same statement. Thus its own reply excludes
INFO, but a later snapshot/reset includes it. Keep gauges and counters distinct.
This does not assert that every other existing INFO section already has one
global snapshot.

RESETSTAT takes those same targets, replaces the baseline with the four current
sums, resets the existing counters listed above, then increments its own
command count before releasing the same statement. Under a whole target, one
known shard (index zero, with the specified nonempty shard set) can receive
this rare increment; no separate selected-shard statement is needed. Exclude
these already-accounted handlers from a second generic completion increment.
No updater reads an epoch or writes the baseline on the hot path. No pre-reset
delta may remain unpublished and later be resurrected as
post-reset work. Arithmetic representation and decimal output must follow the
reference fields; use explicitly defined modular unsigned accumulation for
the implementation, not saturation that loses future increments. Redis stores
all five fields as signed `long long` and prints them with `%lld`
([declarations](https://github.com/redis/redis/blob/7.2.16/src/server.h#L1643-L1678),
[format](https://github.com/redis/redis/blob/7.2.16/src/server.c#L5893-L5916)).
Its ordinary signed increments have no defined C signed-overflow guarantee;
do not claim compatibility beyond the reference's defined range. Modular
internal lifetime totals still permit subtraction across internal wraps while
the count since reset remains in that range.

These combined boundaries are necessary. If A copies an INFO count C greater
than one, B resets, and A only then publishes its increment, a later INFO can
return two. Neither Redis order fits: A before B leaves one, whereas B before
A makes A see one rather than C. Likewise, splitting reset from its own
increment would let another INFO see zero. Copying the reported values and
publishing the handler's completion in one hold avoids both impossible views.

The per-shard values live until server shutdown. Connection closure therefore
needs no retirement transfer: every completed contribution is already in
server-lifetime state. This avoids a growing list of dead connections and the
double-count/lost-count race between a live slot and a retired accumulator.

Expected cost, to be checked rather than assumed: four 64-bit words (32 payload
bytes) per runtime shard, plus its lock/alignment and a four-word baseline.
Each ordinary execution statement adds one shard hold and constant-size delta
publication; lookup arithmetic scales with actual Redis lookup occurrences.
A keyless command may need its own short statement. Each positive socket read
adds one short statement, independent of how many frames it contains. INFO and
RESETSTAT acquire D shard locks and sum 4D words, where D is the actual shard
count, not the connection count or an assumed driver count. The uncontended
ticket-lock path can still execute a hardware read-modify-write on its own
line. The requirement is no common contended write between ordinary writers,
not zero atomic instructions, zero reader interference or a measured zero cost.

### Transactions, scripts and expiry

EXEC's one held statement accumulates each executed member in order, then
EXEC. A discarded queue contributes no member deltas. Script attempts retain
their counters alongside the existing undo/effect state; a retry restores or
discards all attempt deltas. Accepted Lua errors keep preceding work. Publish
the accepted deltas under the outer statement's shard target. Do not add a
waiting counter call inside a nonwaiting command part or interpreter callback.
Expiry deletions remain paired with `Meta.expired_keys`, including rollback;
an abandoned attempt must not leave an increment after restoring the key.

Redis permits INFO in both EXEC and scripts, and RESETSTAT in EXEC, while
RESETSTAT is NOSCRIPT. For reference, on an isolated server:

* `CONFIG RESETSTAT`; `EVAL "redis.call('PING'); return redis.call('INFO','stats')" 0`
  gives **2** in nested INFO; a following INFO gives **4**.
* `MULTI`; `PING`; `CONFIG RESETSTAT`; `INFO stats`; `EXEC`
  gives **1** in the nested INFO; a following INFO gives **3**.

These counts follow call order from source. They are not recorded executions.
Firn's current held-command dispatch does not implement nested INFO/CONFIG.
That existing compatibility boundary must remain explicit, not be mistaken for
proof that an INFO snapshot can always be acquired by the network shell.
When those commands are admitted, preflight EXEC to take a whole stats target
when needed; member INFO sees preceding deltas but excludes itself and EXEC.
A dynamic script INFO needs the same whole target before the attempt runs,
with a retry/promotion protocol analogous to the existing whole-keyspace
promotion, discarding the abandoned attempt's counters. This is additional
design work, not an implemented facility or permission to nest atomic blocks.
Do not solve it by taking a whole stats hold for every ordinary script, or by
returning a cached total. If admission of nested INFO belongs to this item's
final acceptance scope, settle that protocol before implementing it.

### Why the connection-owned alternative is incomplete

A workable individual cell is one `Shared<Counts>` per connection, with only
the handler retaining write authority and the registry retaining SharedRead
handles. The handshake branch's client table can register such handles at
open/close without publishing hot values into the table. Its existing
`client_sync` publishes only changed metadata; replacing that with a table
write after every command violates [reported facts](https://github.com/Ming-Research/Firn-wf/blob/de6d7b53a2352cb8139d8c65b74867467fe5468d/design/firn/reported-facts.md).

However, a reader cannot dynamically open all those cells while holding the
registry: SHARE-2 forbids nested atomic statements and waiting calls inside
one. A copied registry followed by sequential cell reads is not an atomic
vector snapshot. At close, adding a final cell to `retired` and removing it
from the live set must be one ownership transition seen by readers, or they
count both/neither. Keeping handles alive prevents use-after-free, but does
not prevent double accounting.

Even with retirement fixed, a reset that scans cells one at a time has no
single boundary: it can clear a later event in one cell while preserving an
earlier event in another. A global baseline over a torn sum does not repair
that ordering. Epoch checks by owners do not fix idle owners or events racing
reset without another synchronization protocol. A fixed, manually enumerated
set of targets can be held together today, but fixes a capacity and does not
provide runtime driver selection or a dynamic all-cells hold. This is why the
released single-cell example is not a complete implementation recommendation.

## Alternatives and falsifiers

| Alternative | Disposition |
|---|---|
| Put every counter in Meta or the client table | Reject: all commands, including reads, acquire the common write path already rejected by reported-facts and memory-limit. |
| One process-wide hardware fetch-add per field | Reject: lock-free still means every core writes the same cache line; it also has no exposed API at the pin. |
| Private counters flushed every N requests, timer tick or disconnect | Reject: a completed request on an idle live connection can be absent from another connection's INFO; reset can resurrect its old work. |
| Request messages to every connection on INFO | Reject: receive-blocked owners require interruption and acknowledgment; not an uncontended observation, and a polling workaround is forbidden. |
| Independent Shared cells, sequential INFO sums | Keep only as the explicit weaker-observation alternative above; reject for the exact reset/snapshot requirement. |
| Concurrent map keyed by connection ID | Does not establish the required cost: entry access still has shared runtime bookkeeping; a different key is not proof of no common write. Do not substitute this for the missing native facility without a separately approved and measured design. |
| Shards, whole observation only when requested | Recommend, conditional on a released compiler, exactness witnesses, generated-path inspection and the comparison below. Readers can contend briefly with writers; independent ordinary writers must not contend on one stats line. |

## Measurement registered before implementation

Question: does exact accounting preserve firn's scaling and throughput when
the counter update is added to every command? Falsifier: the implementation
writes any process-wide counter/lock/refcount on each update, loses or
duplicates a deterministic contribution, or exceeds the following throughput
criterion. No number below is a measured result or an owner-approved budget.

Use CI on an idle 14900K, through the existing measurement-session cleanup and
physical-core placement rules. Build with `make firn-lto`. Record exact source,
compiler and submodule revisions, flags, binary hashes, driver count, CPU lists,
kernel, AOF setting, key range, data size, connections and pipeline depth.
First adopt a compiler that supplies the chosen facility; keep that compiler
identical across the attribution arms so compiler changes are not attributed
to counters. Reuse the pre-change source and the same source with only this
counter change. Build the unchanged base twice, as base and twin.

Use `research/experiments/redis-bench/redis-bench.sh` compare mode with
`IMAGES` for base/twin/counters and the full `SUITE_TESTS` default roster as
`COMPARE_TESTS`, not compare mode's narrower default of mset/set/get. The
roster is ping_inline, ping_mbulk, set, get, incr, lpush, rpush, lpop, rpop,
sadd, hset, spop, zadd, zpopmin, lrange_100, lrange_300, lrange_500,
lrange_600 and mset. Keep Redis 7.2.16's benchmark client identical for every
arm. Use pipeline depths 1 and 16 and 1, 2, 4 and 8 eligible physical P-cores;
the topology rule reserves the OS core, so report/omit an unavailable count
rather than count a sibling or efficiency core as a P-core. Preserve identical
client placement across compared arms at each server size.

Start with one smallest useful cell, timed by CI, then inspect spread before
choosing duration. The existing compare pilot uses one million requests per
arm/cell to size equal request counts; do not open with the whole matrix.
After calibration, propose six interleaved passes, reversing arm order on
even passes, with approximately ten seconds for the fastest arm per cell.
Measure requests/second, CPU time/request, p50 and p99, and retain raw samples.
Use fresh equal server state and the same setup for each arm.

For each test/depth/core cell, let T be the median paired twin/base rate ratio,
C the median paired counters/base ratio, and E the median absolute deviation
of the paired twin/base ratios. Treat the cell as inconclusive when
`abs(T - 1) > 0.03` or `E > 0.03`; investigate noise before longer runs.
Proposed throughput acceptance is `C >= 0.95` in every conclusive cell: at
most 5% slower than the same-source base. Report every cell rather than hide
a GET regression in an aggregate. Repeat only inconclusive cells or cells
affected by a code change. Failure reopens the mechanism/compiler cost; it
does not relax counting or redefine the workload.

Run the full comparison without an observer first, then a separately labeled
observer case with another connection requesting `INFO stats` once per second
and a stress case at 100 requests/second. Price these on both base and changed
builds; report snapshot latency and the added throughput loss separately.
RESETSTAT correctness races are not mixed into throughput attribution.
Inspect lock ledger and emitted code for representative GET, SET, receive,
EXEC and script paths; look for global retain/release or lock writes and
false sharing. A source shard target alone is not evidence of a cheap path.

## Implementation and acceptance steps

1. Resolve the shard dependency through Whitefoot's existing request. Obtain
   the released main-line compiler and validate the witness in CI; a pin move
   needs the kit's independent upgrade comparison. No pin is moved here.
2. Approve the counting/publication/snapshot design and its throughput budget.
   Put the surviving decisions in `design/firn/reported-facts.md` or a child
   statistics node, with the relevant script/transaction amendments if their
   support is extended. This study changes no live design node.
3. Audit every supported Redis lookup site and firn execution path. Record
   explicit deltas and admission/completion distinctions; include network,
   transaction, script, expiry and AOF replay paths. Fix the existing rewrite
   reset omission in the implementation change. Do not instrument access-stamp
   refreshes as a proxy for lookups.
4. Add the stats owner to the store interface and pass held delta references
   through command parts, EXEC and scripts. Wire receive-byte updates to
   successful socket reads, common snapshot construction to INFO, and the
   whole baseline operation to RESETSTAT. Dispose of retry-local deltas on
   every abandoned attempt and preserve accepted error-path deltas.
5. In CI, run the focused oracle cases below and existing suite
   ratchet, then the gate and AOF fault job appropriate to the implementation.
   Expected values must come from 7.2.16 source or captured oracle output.
6. Run the registered 14900K comparison. Perform a separate completion review
   of the implementation, contracts, full diff, actual validation and design
   correspondence.

### Focused cases to add in the implementation, not tests added by this study

| Case | Failure it must detect |
|---|---|
| Reset, PING, INFO; handler error versus wrong arity/unknown/NOAUTH; QUIT observed from another connection | Counting receipt/success rather than execution; counting INFO too early; dropping a closing connection's count. |
| GET present/absent/wrong type, MGET duplicates, EXISTS duplicates, TYPE, OBJECT, TTL family, SET/INCR and GETEX | Confusing NOTOUCH with NOSTATS, writes with write lookups, unique keys with lookup occurrences, or errors with misses. |
| MULTI with execution, DISCARD and aborted EXEC; nested EVAL calls and pcall error | Counting queued/abandoned work or missing inner calls. |
| Script forced to abandon a declared-key or budget attempt, compared with one accepted execution | Leaking retries' hits/misses/commands/expiry increments. |
| Lazy expiry with active expiry controlled; active expiry alone; overwritten deadlines; immediate expired EXPIRE | Counting deadlines or ordinary deletion instead of actual expiry removal, or double counting. |
| Successful nonzero-offset short receives, partial requests, malformed input, pipeline reset and buffer compaction | Counting cumulative endpoints, parser bytes, reread buffers, or reconstructing receive boundaries from commands. |
| A completes a command and remains idle; B immediately requests INFO; repeat after A closes and during connection churn | Lagged publication and lost/double retired contributions; test without sleep-to-flush. |
| Concurrent updates and RESETSTAT with acknowledged before/after batches; two fields incremented together | Torn baselines/snapshots, resurrection, or cross-field inconsistency; no assumption about unsynchronized in-flight order. |
| Race INFO against RESETSTAT, starting above one, then read another INFO | Separating an INFO snapshot or reset baseline from that handler's own completion count, allowing observations no Redis serial order can produce. |
| AOF rewrite completed, RESETSTAT, INFO persistence | Existing `aof_rewrites` omission; sizes/status/peak must remain intact. |
| Startup replay of direct commands versus MULTI/EXEC records | Blanket exclusion or double counting of replayed commands. |

No executable expected-reply fixture is added by this study. The deterministic
nested command totals above are source-derived. The following unrun oracle
experiments have deliberately unresolved expectations; record them in CI
before turning them into executable assertions. `ORACLE` is a placeholder,
not an accepted expected reply.

| Experiment | Exact command batch and setup | Unresolved expectation |
|---|---|---|
| Transport, separate sends | Fresh raw client, no concurrent workload: send RESP `CONFIG RESETSTAT`, wait for OK; send RESP `PING`, wait for PONG; send RESP `INFO stats`. | `ORACLE`: capture requested fields and wire lengths; the final INFO's own request bytes are included. |
| Transport, pipelined/fragmented | Fresh raw client: send RESP `CONFIG RESETSTAT`, `PING`, `INFO stats` in one write; repeat with the last INFO frame split between two writes. | `ORACLE`: capture received-byte totals together with actual receive grouping. TCP grouping varies, so do not bless one count for every execution; the assertion must follow the positive-read sums and reset boundary. |
| Startup replay | Two fresh isolated servers, each using a valid pure-command AOF: first contains `SET k v`; second contains `MULTI`, `SET k v`, `EXEC`. On each, the first client sends only `INFO stats`. | `ORACLE`: capture the complete five-field result and exact fixture/configuration; distinguish inner replayed calls from direct replay and first-network-read bytes. |

Open uncertainties are source acceptance and cost of the future shard path,
full lookup-site coverage during implementation, script/EXEC nested INFO scope,
and the unrun oracle/CI/14900K evidence. The language gap is G1 above, covered
by the existing Whitefoot shard work. No compiler failure was manufactured,
no workaround implemented, and no live design node, pin or submodule changed.

## INFO field audit

These are the INFO field corrections of board item `firn-info-fields`,
separate from the counter proposal above. The baseline is firn before the
field changes.
The reference is [Redis 7.2.16's INFO implementation](https://github.com/redis/redis/blob/7.2.16/src/server.c#L5465-L6268)
and the raw RESP replies recorded in [the INFO oracle CI run](https://github.com/Ming-Research/Firn-wf/actions/runs/38396764389).
Batch 0 uses RESP2 without AOF; batch 1 uses RESP3 with AOF. Both first send
`SET k v` and `EXPIRE k 100`. The field names, their relative order,
headers, CRLF delimiters and conditional AOF fields come from those
recordings. Values that depend on the host or elapsed time are not copied.
Redis source supplies the value rules and selections not recorded, including
case folding, embedded NUL termination, duplicate elimination and `debug`.
The repository's Redis-suite runner obtains the release's tests; it does
not hold the Tcl sources directly.

The [reported-facts rule](../../../design/firn/reported-facts.md) permits
omitting unavailable measurements, so matching here means that every Redis
field firn emits has its reference name, section, relative order and format,
and a real or genuinely fixed firn value. It does not mean copying every
Redis field or copying Redis's allocator usage, build hash or host values.
CLIENT and Clients fields belong to the client-handshake work and are
unchanged. The sharded command, hit, miss and input counters and the
expiration-counter semantics remain outside this step.

In the table, **bytes** is decimal byte count; **signed** is Redis's signed
64-bit decimal rendering; **human** is `bytesToHuman`, integer `B` below
1024 and at least 2^60, otherwise binary `K/M/G/T/P` with two decimals.
Zero values listed as additions follow absent facilities, not a guessed
measurement. A field group has the same disposition for every named field.

| Field | Redis 7.2.16 section + format / value rule | Firn before this step | Action |
|---|---|---|---|
| `redis_version` | Server; version string | Compatibility version `7.2.16` | Retain declared compatibility identity. |
| `redis_git_sha1` | Server; source hash | Invented `00000000` | Omit: no firn build revision is supplied. |
| `redis_mode`, `arch_bits` | Server; mode string, decimal bits | `standalone`, `64` | Retain fixed facts. |
| `process_supervised` | Server; supervision mode | Absent | Add `no`: no supervision integration. |
| `tcp_port`, `server_time_usec` | Server; port, signed epoch microseconds | Configured port and actual clock | Retain. |
| `uptime_in_seconds`, `uptime_in_days` | Server; signed difference of whole-second calendar timestamps; days truncate toward zero | Millisecond subtraction before division, clamped at zero | Divide each timestamp first; retain signed difference and signed days. |
| `hz`, `configured_hz` | Server; decimal cadence | `10`, `10` | Retain: expiry worker sleeps 100 ms between ticks. |
| `lru_clock` | Server; decimal low 24 bits of epoch seconds | Absent | Add from current calendar time, same clock rule as access stamps. |
| `config_file` | Server; path, empty when absent | Empty | Retain: no configuration file. |
| `redis_git_dirty`, `redis_build_id`, `os`, `monotonic_clock`, `multiplexing_api`, `atomicvar_api`, `gcc_version`, `process_id`, `run_id`, `executable`, `io_threads_active`, `listener0` | Server; build/host strings, identifiers or decimal facts | Absent | Keep omitted; no existing published measurement maps them to Redis's meanings. Whitefoot contexts are not Redis I/O threads. |
| `shutdown_in_milliseconds` | Server; signed delay, only during scheduled shutdown | Absent | Keep omitted: no measured Redis-style scheduled delay. |
| Clients fields | Clients; decimal counts | Existing measured/fixed subset | Leave all fields untouched for the client-handshake session. |
| `used_memory`, `used_memory_human` | Memory; bytes, human | Actual requested live heap bytes, placed after limits | Move to the start; retain the approved heap-meter meaning. |
| `used_memory_rss`, `used_memory_rss_human` | Memory; bytes, human | RSS optional and late; human absent | Place after heap/human; add human from the same RSS reading, omit both when host returns None. |
| `used_memory_peak`, `used_memory_peak_human` | Memory; bytes, human | Observed heap peak late; human absent | Place after RSS; add human from the same peak. |
| `used_memory_peak_perc` | Memory; two decimals plus `%` | Absent | Keep omitted in this step; existing heap/peak can support a later ratio with exact reference rounding. |
| `used_memory_overhead`, `used_memory_startup`, `used_memory_dataset`, `used_memory_dataset_perc` | Memory; bytes, two decimals plus `%` | Absent | Keep omitted: no separate overhead/startup/dataset accounting. |
| `allocator_allocated`, `allocator_active`, `allocator_resident`, `total_system_memory`, `total_system_memory_human` | Memory; bytes or human | Absent | Keep omitted: heap requested bytes are not allocator/physical-system measurements. |
| `used_memory_lua`, `used_memory_vm_eval`, `used_memory_lua_human`, `used_memory_scripts_eval` | Memory; signed bytes or human | Absent | Keep omitted: the Halo VM/cache allocations are not separately measured. |
| `number_of_cached_scripts` | Memory; decimal registry size | Actual script registry length before heap fields | Retain after measured heap fields. |
| `number_of_functions`, `number_of_libraries` | Memory; decimal counts | `0`, `0` | Retain: FUNCTION is unsupported. |
| `used_memory_vm_functions`, `used_memory_functions` | Memory; signed bytes | Absent | Add fixed `0`: no function VM or function library cache. |
| `used_memory_vm_total`, `used_memory_vm_total_human`, `used_memory_scripts`, `used_memory_scripts_human` | Memory; signed bytes or human | Absent | Keep omitted: evaluation VM and script-cache components are unmeasured. |
| `maxmemory`, `maxmemory_human`, `maxmemory_policy` | Memory; signed limit, human, policy name | Actual settings, emitted too early | Move after scripts/functions, preserving signed conversion and human rounding. |
| `allocator_frag_ratio`, `allocator_frag_bytes`, `allocator_rss_ratio`, `allocator_rss_bytes`, `rss_overhead_ratio`, `rss_overhead_bytes`, `mem_fragmentation_ratio`, `mem_fragmentation_bytes` | Memory; two decimals or signed/unsigned bytes | Absent | Keep omitted: required allocator measurements are absent. |
| `mem_not_counted_for_evict` | Memory; bytes | Actual collecting and writer-pending allocated capacities | Retain after memory policy. |
| `mem_replication_backlog`, `mem_total_replication_buffers`, `mem_clients_slaves`, `mem_cluster_links` | Memory; bytes | Absent | Add fixed `0`: no replication/cluster storage. |
| `mem_clients_normal`, `mem_allocator` | Memory; bytes, allocator name | Absent | Keep omitted: no separately measured client allocation total or exposed allocator identity. |
| `mem_aof_buffer` | Memory; bytes | Absent | Add from the same actual allocated AOF capacities as eviction exclusion, zero without AOF. |
| `active_defrag_running` | Memory; decimal state | Absent | Add fixed `0`: no active defragmenter. |
| `lazyfree_pending_objects`, `lazyfreed_objects` | Memory; decimal counts | `0`, `0` | Retain: no lazyfree queue. |
| `loading`, `async_loading` | Persistence; `0`/`1` | `0`, `0` | Retain: listening starts after replay. |
| `current_cow_peak`, `current_cow_size`, `current_cow_size_age`, `current_fork_perc` | Persistence; bytes, seconds, two decimals | Absent | Add fixed `0`, `0`, `0`, `0.00`: no fork or copy-on-write child. |
| `current_save_keys_processed`, `current_save_keys_total` | Persistence; decimal child-save progress | Absent | Keep omitted; do not map rewrite scan steps to child-save key counts. |
| `rdb_changes_since_last_save`, `rdb_last_save_time` | Persistence; signed mutations, epoch seconds | Absent | Keep omitted: no dirty-since-save counter or saved timestamp. |
| `rdb_bgsave_in_progress`, `rdb_last_bgsave_status`, `rdb_saves` | Persistence; `0`/`1`, `ok`/`err`, signed count | `0`, `ok`, `0` | Retain fixed no-save state. |
| `rdb_last_bgsave_time_sec`, `rdb_current_bgsave_time_sec` | Persistence; signed seconds, `-1` when no save | Absent | Add genuine absence sentinel `-1`, `-1`. |
| `rdb_last_cow_size`, `rdb_last_load_keys_expired`, `rdb_last_load_keys_loaded` | Persistence; bytes or signed counts | Absent | Add fixed `0`: no RDB fork or RDB loading. |
| `aof_enabled`, `aof_rewrite_in_progress`, `aof_rewrite_scheduled` | Persistence; `0`/`1` | Real logging/progress, fixed unscheduled `0` | Retain. |
| `aof_last_rewrite_time_sec`, `aof_current_rewrite_time_sec`, `aof_rewrites_consecutive_failures` | Persistence; signed durations/count | Absent | Keep omitted: timings/failure streak are not recorded; a last-status Boolean cannot supply them. |
| `aof_last_bgrewrite_status`, `aof_rewrites`, `aof_last_write_status` | Persistence; `ok`/`err`, signed count, `ok`/`err` | Actual rewrite status/attempts and combined append/sync health | Retain; render attempts signed. Approved sync-recovery differences still apply. |
| `aof_last_cow_size`, `module_fork_last_cow_size` | Persistence; bytes | Absent | Add fixed `0`: no fork. |
| `module_fork_in_progress` | Persistence; `0`/`1` | `0`, after enabled-only AOF sizes | Move before enabled-only fields. |
| `aof_current_size`, `aof_base_size` | Persistence; signed bytes, only when AOF enabled | Real active-file/baseline bytes | Retain conditional output, use signed rendering. |
| `aof_pending_rewrite` | Persistence; `0`/`1`, only when AOF enabled | Absent | Add `0` between base size and buffer: no scheduled rewrite queue. |
| `aof_buffer_length` | Persistence; bytes, only when AOF enabled | Collecting plus retained unwritten lengths | Retain actual sum after sizes/pending rewrite. |
| `aof_pending_bio_fsync`, `aof_delayed_fsync` | Persistence; decimal counts, only when AOF enabled | `0`, `0` | Retain: sole writer syncs itself, with no BIO jobs or defer-behind-BIO mechanism. |
| Loading progress fields | Persistence; conditional epoch/bytes/percent/ETA fields | Absent | Keep omitted: INFO is not served while startup loads. |
| `firn_aof_rewrite_*` | Firn extension, no Redis field | Real documented diagnostics interleaved | Preserve after every Redis Persistence field; the design node records that boundary. Commit sequence stays AOF-only. |
| `total_connections_received` | Stats; signed accepted-connection count | Actual total | Retain. |
| `total_net_repl_input_bytes`, `total_net_repl_output_bytes`, `instantaneous_input_repl_kbps`, `instantaneous_output_repl_kbps` | Stats; signed bytes, two decimals | Absent | Add `0`, `0`, `0.00`, `0.00`: no replication traffic. |
| `rejected_connections`, `sync_full`, `sync_partial_ok`, `sync_partial_err` | Stats; signed counts | Fixed `0`, after eviction count | Place before `evicted_keys`, retaining unsupported-feature zeros. |
| `evicted_keys` | Stats; signed count | Real counter before rejected/sync fields | Move after sync fields and omitted expiry statistics; render signed. |
| `evicted_clients`, `pubsub_channels`, `pubsub_patterns`, `pubsubshard_channels`, `latest_fork_usec`, `total_forks`, `migrate_cached_sockets` | Stats; decimal counts/duration | Fixed `0` | Retain in Redis order: unsupported facilities. |
| `slave_expires_tracked_keys`, `active_defrag_hits`, `active_defrag_misses`, `active_defrag_key_hits`, `active_defrag_key_misses`, `total_active_defrag_time`, `current_active_defrag_time` | Stats; decimal counts or milliseconds | Absent | Add fixed `0` after migration: no replica expiry tracking or defragmenter. |
| `tracking_total_keys`, `tracking_total_items`, `tracking_total_prefixes`, `dump_payload_sanitizations` | Stats; signed counts | Fixed `0` | Retain: no tracking or DUMP/RESTORE sanitation. |
| `io_threaded_reads_processed`, `io_threaded_writes_processed` | Stats; signed counts | Absent | Add fixed `0`: firn has no Redis I/O-thread processing mechanism. |
| `total_commands_processed`, `keyspace_hits`, `keyspace_misses`, `total_net_input_bytes` | Stats; signed counts/bytes | Absent | Remain absent: excluded sharded counters. |
| `expired_keys` | Stats; signed expiration count | Absent | Remain absent: excluded expiration-semantics work. |
| `instantaneous_ops_per_sec`, `total_net_output_bytes`, `instantaneous_input_kbps`, `instantaneous_output_kbps`, expiry-cycle/eviction-time/error/read/write/reply-buffer/event-loop/ACL statistics | Stats; signed/unsigned counts, times or two decimals | Absent | Keep omitted: no corresponding published measurements. No hot-path bookkeeping is added. |
| `role`, `connected_slaves`, `master_failover_state`, `master_repl_offset`, `second_repl_offset`, `repl_backlog_active`, `repl_backlog_first_byte_offset`, `repl_backlog_histlen` | Replication; mode/state or signed decimal | `master`, `0`, `no-failover`, `0`, `-1`, `0`, `0`, `0` | Retain genuine standalone no-replication state. |
| `master_replid`, `master_replid2`, `repl_backlog_size` | Replication; 40-digit hex identity, signed configured bytes | Absent | Keep omitted: no replication identity/backlog setting. |
| `used_cpu_sys_children`, `used_cpu_user_children` | CPU; seconds with six fractional digits | Absent | Add `0.000000`: server creates no child processes. |
| `used_cpu_sys`, `used_cpu_user`, `used_cpu_sys_main_thread`, `used_cpu_user_main_thread` | CPU; seconds with six fractional digits | Absent | Keep omitted: no process/thread CPU meter. |
| Module records, `cmdstat_*`, `errorstat_*`, `latency_percentiles_usec_*` | Modules/Commandstats/Errorstats/Latencystats; dynamic records | Empty headers | Retain headers with no invented records; modules unsupported, remaining measurements unavailable. |
| `cluster_enabled` | Cluster; `0`/`1` | `0` | Retain. |
| `db0` | Keyspace; `keys=N,expires=N,avg_ttl=N`, only nonempty DB | Actual `keys=N` only | Retain supported subfield and empty-DB header; expiry population/Redis's average-TTL sampling are not measured. |
| Debug fields | Debug; unsigned durations, signed maximum commands | Section absent | Add explicit Debug header only; omit unmeasured facts. |

### Selection, structure and additions

No argument and `default` select Server, Clients, Memory, Persistence,
Stats, Replication, CPU, Modules, Errorstats, Cluster and Keyspace, in that
order. `all` and `everything` also select Commandstats and Latencystats,
at their Redis positions. With no loaded modules their distinction adds
no output. Explicit names combine in canonical output order, independent
of argument order and duplicates; case and bytes after the first NUL do
not change the selected name. Unknown names add nothing. `module_list`
and `modules` both select Modules. `debug` adds Debug last only when
explicitly selected, including alongside `all` or `default`.

Each section has `# Name\r\n` and each field has `name:value\r\n`.
One additional CRLF separates sections. There is no extra blank line
at the end of INFO's text. A fieldless section still has its header;
the empty unknown selection has zero-length text. RESP2 returns a bulk
string; RESP3 a `txt` verbatim string whose framing length includes the
four-byte `txt:` prefix. A final CRLF terminating RESP framing is not a
blank line inside the INFO text.

The two network cases cover no-AOF RESP2 and AOF RESP3; they assert exact
supported field lists against ordered Redis recording projections, fixed
values, integer/human formatting, AOF conditions, diagnostic-tail order,
section selection and separators. The cases allow unavailable RSS as a
pairwise omission. They distinguish missing measurement from fixed zero
and reject the excluded counters. Empty and populated keyspaces and
case/duplicate/unknown requests exercise selection without relying on
unstable recorded pid, times, memory or command timings. Existing
human-limit boundary cases already cover signed maxmemory and nearest-even
formatting, and existing rewrite/fault cases cover operational sizes and
health.

#### Evidence that the cases fail on a wrong server

A temporary CI workflow (removed after the run) ran the INFO cases against
firn with one line of `firn/commands/info.wf` broken at a time, and against
the `info.wf` from before the field changes, at commit `f4f85d0`
([run](https://github.com/Ming-Research/Firn-wf/actions/runs/39498283007)).
Each row fails with the message of the condition it protects, and the
unchanged server passes all four cases (the two structure cases, the uptime
case below and the existing maxmemory case).

| Server | Case that failed | Failure |
|---|---|---|
| unchanged | none (4 passed) | |
| `info.wf` before the field changes | both structure cases | Server fields include `redis_git_sha1`, lack `process_supervised` and `lru_clock` |
| `rdb_last_bgsave_time_sec` and `rdb_current_bgsave_time_sec` swapped | RESP2 structure | Persistence field order |
| `uptime_in_days` written twice | RESP2 structure | `duplicate INFO field: uptime_in_days` |
| section gap `\r\n` replaced by `\n\n` | RESP2 structure | `INFO line:` holds a line break |
| `aof_current_size` and the AOF-only fields written with AOF off | RESP2 structure | Persistence field names |
| `all` also selects Debug | RESP2 structure | section list gains `"Debug"` |
| default also selects Commandstats | RESP2 structure | section list gains `"Commandstats"` |
| human-size units lowercased | RESP2 structure | `Memory.used_memory_human` is `8.57m`, not `8.57M` |
| `redis_version` 7.2.15 | RESP2 structure | `Server.redis_version` |
| LRU clock modulo 16777215 | RESP2 structure | `LRU clock uses Redis's 24-bit seconds` |
| uptime from the millisecond difference | uptime case | `uptime_in_seconds left the calendar second` |
| minus sign of the signed writer replaced by `+` | maxmemory case | `maxmemory` reads `+1`, not `-1` |

The uptime case reads `server_time_usec` and `uptime_in_seconds` for 2.3 s
and requires the calendar second less the uptime to stay one value while the
uptime steps at least twice. Redis 7.2.16 subtracts two whole-second clocks
(`server.c` `genRedisInfoString`), so the uptime steps with the calendar
second; a millisecond difference steps a start-time fraction later and
leaves the value constant only when that fraction is under about the 5 ms
polling step. The `uptime_in_days` divisor has no case: at any reachable uptime the
value is 0.

Further real-value additions worth considering are `used_memory_peak_perc`
from the existing used/peak readings, and `db0`'s `expires` from a whole-map
expiry-population observation. The first needs reference decimal rounding;
the second needs a separate choice of observation cost and expired-but-not-
yet-removed population semantics. This step does not add their accounting.
Startup allocation, CPU/host metadata, VM/script memory splits and rewrite
timings require measurements before any field is emitted. Existing shard
and expiration requests remain the owners of the excluded counters.

### Design, unresolved evidence and oracle placeholders

The [diagnostic-tail decision](../../../design/firn/info-diagnostics.md)
keeps documented rewrite measurements available in Persistence after the
Redis fields, rather than moving them to a new selection or removing them.
It changes no CLIENT/Clients decision. The existing memory-limit and
append-only-file decisions continue to own heap accounting and sync
recovery, including their deliberate differences from Redis.

No new `ORACLE` placeholder is needed for the field cases: their structural
and fixed-value expectations are recorded or source-derived. The three
`ORACLE` rows in the preceding counter study remain unresolved and outside
this implementation: separate transport sends; pipelined/fragmented sends;
and direct-command versus MULTI/EXEC startup AOF replay. Their exact batches
remain in that study's table.
