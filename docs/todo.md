# Defects and follow-up work

Known defects, capability gaps, unresolved costs and improvement
opportunities in firn, including unverified ones, and the Whitefoot changes
firn needs. An unverified opportunity is a validation task: state its
expected benefit, uncertainty and criterion for deciding whether to pursue
it. Entries do not select a design. Remove an item when its implementation
and checks land, or its validation concludes with a recorded disposition.
Add an item at the end of the section that owns its topic.

A gap in Whitefoot itself, stated as its minimal semantic example, goes
under *Whitefoot requirements* until Whitefoot resolves it
([AGENTS.md](../AGENTS.md#the-whitefoot-boundary)). The items below were
written while firn lived in the Whitefoot repository; a path such as
`compiler/...` they name is Whitefoot's.

## Server

- **Complete firn's standalone deployment workloads.** The
  [deployment direction](../research/investigations/firn/DESIGN.md#deployment-direction)
  requires usable cache/session storage and scripted conditional updates,
  with an existing application using its ordinary client and unchanged
  business logic. Select the consumers and their required command behavior
  before treating a feature inventory as release coverage; a leaderboard or
  queue is a candidate additional scenario, not yet a selected dependency.
  Complete the following work and remove this item when the deployment
  evidence meets that boundary:
  - Complete the selected clients' connection behavior, RESP3, command
    metadata, ordinary pipelines, scans and application command gaps. Add
    `MULTI`/`EXEC`/`DISCARD` and `WATCH`/`UNWATCH`, including queue-time and
    execution-time errors, expiry/eviction invalidation and Redis's lack of
    transaction rollback. Provide command semantics that transactions and
    the Lua work below can compose without separately committing each call.
  - Make AOF persistence usable through write/sync error handling, rewrite
    and orderly `SHUTDOWN`/signal handling; verify
    a practical data migration path. File replacement and signal delivery
    may require Whitefoot library/runtime work; AOF presence alone is not
    durable-recovery evidence. RDB compatibility is not assumed by this item.
  - Add memory accounting, `maxmemory` and the eviction behavior the selected
    deployments need, including large-value reclamation and slow-client
    pressure. Complete authentication, configuration, logs, connection limits,
    core `INFO` metrics and build/run/recovery instructions for that scope.
  - Validate with independent Redis behavior and tests, real applications,
    concurrent histories, restart/crash cases and sustained memory-bounded
    runs. Measure throughput, tail latency, memory and scaling with matched
    durability settings, including scripts and AOF-enabled operation. Report
    compiler/runtime support separately from Whitefoot implementation and
    record language gaps as minimal witnesses. Neither the benchmark nor an
    aggregate compatibility pass count replaces these observations.
  Replication, Sentinel, Cluster, modules and unused feature families such
  as Streams, GEO and HyperLogLog remain outside the first release. Blocking
  lists and Pub/Sub are needed if the selected application requires them;
  select them with that consumer rather than promising a partial queue.
  Reopen now with the command integration and Lua vertical slice, and revisit
  deferred families when a real consumer makes them necessary.

- **Implement firn's Redis-compatible Lua interpreter in Whitefoot.** Missing
  scripting prevents applications from composing conditional multi-command
  operations through `EVAL` and `EVALSHA`. Target the Redis Lua 5.1 execution
  environment ([Lua semantics](https://www.lua.org/manual/5.1/manual.html),
  [Redis Lua API](https://redis.io/docs/latest/develop/programmability/lua-api/)),
  pinning the Redis reference version for tests; this is not a JIT, a native
  Lua embedding, or a recognizer for selected script templates. Important
  work, with representation and algorithms still to be investigated:
  - Parse and compile scripts to an executable form; implement dynamic
    values, tables, lexical scopes, closures/upvalues, multiple returns,
    varargs, calls/tail calls, metatables and protected error propagation.
    Implement the library behavior Redis exposes, including strings/patterns,
    tables, math, bit operations, JSON and MessagePack, rather than importing
    the standalone interpreter's host I/O or native-module facilities.
  - Establish object identity, roots, cyclic-object reclamation and bounded
    resource behavior within Whitefoot's checked ownership and access rules.
    Compare viable VM/GC representations and their dependent accesses before
    selecting one; do not assume reference counting alone collects cycles.
  - Add `KEYS`/`ARGV`, `redis.call`/`redis.pcall`, Lua/RESP conversions,
    script caching and `NOSCRIPT`, `SCRIPT LOAD`/`EXISTS`/`FLUSH`/`KILL`, and
    the sandbox and busy-script behavior of the pinned reference. Distinguish
    script errors and allowed termination from rollback; compare the state
    left by an error after a write. Persistent `FUNCTION`/`FCALL` support is
    later work unless the selected consumer requires it.
  - Execute each script with Redis-compatible atomic visibility, expiry
    behavior and AOF effects. Share command semantics with transactions;
    identify the true conflicting key/state dependencies, retain independent
    work where semantics permit it, and investigate key discovery rather
    than silently restricting scripts to make locking convenient.
  - Start with a vertical slice spanning compilation, closures/tables,
    reclamation and real command calls. State its falsifiers before measuring;
    use Lua/Redis differential cases and representative application scripts,
    including pure computation, command-heavy work, disjoint keys and hot
    keys. Measure execution, GC, memory, command and waiting costs separately.
    Redis's interpreter is the primary compatibility/performance reference;
    LuaJIT is an additional comparison with JIT mode and warmup reported.
    Expose a Whitefoot limitation as a language/library/compiler requirement
    rather than excluding a failing workload or claiming foreign-engine work
    as Whitefoot's result.
  Reopen with that vertical slice; remove this item only when the selected
  Redis scripting surface and application workloads have correctness and
  performance evidence, recording any remaining incompatibilities separately.

- **A script can hold the whole keyspace without limit.** A firn script
  runs inside one atomic statement holding every key and the keyspace's
  metadata (`script_command` in `firn/commands/script.wf`), and the
  first scripting version stops no script: one that loops forever stalls
  every client and the append-only file's writer until firn is killed, where
  Redis 7.0.15 answers other clients `BUSY` once `busy-reply-threshold`
  (5 seconds) passes and lets `SCRIPT KILL` stop a script that has not
  written. The change: count the engine's steps and, past the threshold,
  answer other clients `BUSY` and accept `SCRIPT KILL` and `SHUTDOWN NOSAVE`,
  which needs a way for the clients' contexts to run while the script's
  statement holds the keyspace. Validate with Redis's `unit/scripting` busy
  tests. Reopen when the Lua engine runs real scripts, before any deployment
  that accepts scripts from clients it does not control.
- **Replay still differs from Redis's loader in two cases.** A file that
  does not parse, cannot be read or holds a block larger than the input
  window's ceiling now stops firn with status 4, as Redis 7.0.15 exits
  (`firn/persistence/persistence.wf`). Two differences remain: an error
  opening the file other than its absence is treated as no file, where Redis
  exits on `AOF_OPEN_ERR`; and replay accepts inline commands, where Redis's
  loader requires every record to start with `*` and stops at any other
  byte. A block larger than the window's ceiling, 2 GiB, also stops firn
  where Redis loads it; replaying a block by re-reading it from its file
  offset instead of holding it would lift that limit. The change: tell an
  absent file from an open failure, and refuse a record not starting with
  `*`. Validate with an unreadable file and a file holding an inline
  command. Reopen before firn is offered to a deployment that keeps an
  append-only file.
- **Close the current main-line Redis compatibility gaps.** The following
  gaps remain after the command integration of
  [PR #212](https://github.com/mbbill/Whitefoot/pull/212). Missing, among
  others: `CONFIG SET` of `appendonly`, `port` or `bind` to a value other
  than the one firn started with, and of a nonempty `save` schedule, which
  firn refuses where Redis applies them, since the append-only file's writer
  and the listener would have to change while firn runs, `appendonly yes`
  with keys present needs a rewrite that visits every key, and firn saves no
  snapshot; the parameters beyond firn's 22, refused as unknown; `FUNCTION`
  subcommands beyond `FLUSH` and `DEBUG` subcommands beyond `LOG`, answered
  as unknown; a score of negative
  zero in a sorted set Redis encodes as a skiplist, one of more than 128
  members or with a member longer than 64 bytes, which Redis keeps and writes
  as `-0` where firn keeps and writes 0, as Redis does in a smaller set; more
  than one database, where
  `SELECT` takes 0 alone; a listening address in IPv6, or several, where
  `--bind` takes one IPv4 address or `*`, and users other than `default`;
  `CLIENT` subcommands beyond `ID`, `GETNAME` and `SETNAME`, which firn
  answers as unknown, and a command table, which `COMMAND` and
  `COMMAND COUNT` report empty and `COMMAND DOCS`, `INFO`, `LIST` and
  `GETKEYS` answer as unknown subcommands; `KEYS` and `SCAN`, which can match
  with `glob_match` (`firn/bytes/bytes.wf`), RESP3, which
  `HELLO 3` refuses, `LMPOP` and the blocking list commands, `SSCAN`,
  `MULTI` and `EXEC`, publish and subscribe, and a random hash seed; and
  `RANDOMKEY`, `SORT`, `LCS`, `OBJECT`, `DUMP`, `RESTORE`, `MOVE`, `MIGRATE`,
  `WAIT`, `HSCAN`, `ZSCAN`, `ZRANGESTORE`, `ZRANDMEMBER`, `ZMPOP` and
  `BZMPOP`, `BZPOPMIN` and `BZPOPMAX`, and `ZDIFF`, `ZINTER`, `ZUNION`,
  `ZINTERCARD` and their stores, which firn answers as unknown commands.
  A relative expiry whose sum with the current time passes 2^63 - 1
  milliseconds (`SETEX k 9223372036854775 v`, and `SET`'s, `GETEX`'s and
  `PSETEX`'s equivalents) is refused by firn as an invalid expire time, where
  a redis-server 7.0.15 built on arm64 macOS answered `OK`: Redis adds the
  time in signed arithmetic before its nonpositive check (`t_string.c`), an
  overflow C leaves undefined, so the reference to match needs Redis's x86-64
  build observed first. These come from a differential run of firn at
  `18d8637ab` against that redis-server over about 160,000 requests on
  strings, keys, connection commands, lists, sets, hashes and sorted sets,
  which left out `INCRBYFLOAT` and `HINCRBYFLOAT`, whose long double is a
  double on that platform, random replies and replies that depend on the
  clock, and sent no request before authentication with a password set. firn answers the calls the suite's
  framework makes around its tests: `FLUSHALL` and `FUNCTION FLUSH` at the
  start of each block, `CONFIG GET` and `CONFIG SET` of the block's
  overrides, `INFO`'s `aof_rewrite_in_progress` after an `appendonly yes`
  override, and `DEBUG LOG` before each test. In a run on 2026-10-03 of firn
  at `acca9c9d5` without `--tolerant`, its output not kept, tests ran in 36
  of the 39 units in which Redis passes any, and firn passed 157 of the
  1,994 tests Redis passes. Blocks stopped where a test sent `HELLO 3`,
  which left 217 tests of `unit/type/list` and 95 of `unit/type/zset`
  unreached, or `MEMORY`, 71 of `unit/type/hash`, and where an override
  named a parameter firn refuses, `appendonly yes`, which ends
  `unit/type/stream`, or does not know, `slowlog-log-slower-than`, which
  ends `unit/slowlog`; past those, the commands firn lacks that fail the
  most tests are `FUNCTION LOAD`, `EVAL`, `XADD`, `SORT` and `GEOADD`. The
  measurement in redis-compat is of firn at `cea9188d4`, before those
  commands, when every unit stopped at the first `FLUSHALL`; with the calls
  allowed to fail it passed 107 of the 1,994 tests Redis passes there, 25 of
  which a server knowing only `PING` passes too, and redis-py 8.1.0 fails
  every call made with its defaults, since it opens each connection with
  `HELLO 3` ([redis-compat](../research/experiments/redis-compat/README.md));
  a new run belongs in that record. Reopen during deployment command
  integration; remove each gap only after independent behavior checks on
  the integrated implementation, or record its explicit release exclusion.

- **firn writes a removal and the command that made it as two records
  where Redis wraps them in `MULTI` and `EXEC`, and no `SELECT 0`.** When
  one command propagates more than one record, a key it found expired and
  the command itself, or several expired keys, Redis 7.0.15 brackets them
  in `MULTI` and `EXEC`; firn has no transactions to replay, so it writes
  the records alone, which replays to the same state. Nor does firn, with
  one database, write the `SELECT 0` Redis writes before its first record.
  `firn_records_its_writes_as_redis_propagates_them` compares firn's file
  with Redis's but for both. The change: write each where Redis does. Reopen
  when firn answers `MULTI` and `EXEC`, or `SELECT` with more than one
  database.

- **No case checks that the expiring context keeps a key through its
  expiry's millisecond.** `take_due` (`firn/store/store.wf`) leaves a
  queued expiry equal to the time in the queue, so that the expiring context
  keeps the key through that millisecond, as Redis's
  `activeExpireCycleTryExpire` does (`now > t`). Were it to take the key,
  only a command whose reading of the clock falls in that millisecond and
  that runs after the context's tick would find it absent, and no case can
  place a command there, since the context ticks when the program chooses;
  nor does the file show it, recording the removal as `DEL` whichever
  context makes it. The change: a check entry in firn's module graph that
  queues an expiry and calls `expire_batch` with `now` equal to it, which
  `compile_app` can build as a second entry, as module graphs with two
  entries do (Whitefoot's `research/investigations/modular-compilation/demo/modules.wfg`).
  Reopen when `take_due` changes, or when firn gains such entries.

- **firn converts decimals to binary twice, by one algorithm.**
  `firn/scores/decimal.wf` reads a double and
  `firn/extended/extended.wf` a long double by the same Simple Decimal
  Conversion, shifts of a decimal by powers of two, each with its own digit
  room (800 and 12,000 digits), significand width (53 and 64 bits),
  exponent range and subnormal rounding. One reader taking those as
  parameters, its decimal generic over the digit room, would serve both.
  Reopen when either reader next changes, or a third format needs one.

- **firn reads a slow request again from its start at every read.**
  `parse_request` keeps no state between reads, so a request arriving in
  many reads is scanned from its first byte each time: quadratic in its
  length, and since firn takes arrays of up to 2,147,483,647 elements, as
  Redis 7.0 does, a client sending a huge array slowly costs the server
  work out of proportion to its bytes, where Redis resumes at the element it
  stopped at. Keeping the parse position and the spans found so far in the
  client between reads would remove it; reopen when firn faces clients it
  does not trust, or a profile shows parsing past a few percent.

- **Writing a score far from 1 is slow, and the commands that answer scores
  write them while the key is held.** The `scores` module writes a score
  from its exact decimal expansion, one digit per byte, so the cost grows
  with the score's binary exponent; with two drivers and
  `redis-benchmark -c 50 -P 16 -n 200000`, `ZSCORE` answered 3.85M requests
  per second for a score of 7, 2.94M for 0.1, 0.36M for 1e300 and 0.28M for
  4.9e-324 on the i9-14900K, in one run of one server. The visitors of
  `ZSCORE`, `ZMSCORE`, `ZPOPMIN`, `ZPOPMAX` and `ZRANGE` with `WITHSCORES`
  (`member_score`, `member_scores` and `pop_ranked` in
  `firn/commands/sorted.wf`, `walk_visit` in `ranges.wf`) write the
  reply inside the key's atomic statement, so that time is also time the key
  is held. Carrying the scores out in the client, as a command's other
  results are carried, and writing the reply after the statement would take
  the cost out of the statement; an exact writer that works in base-10^9
  words would shrink it. Reopen when those visitors or the atomic statements
  that call them next change, or when a workload stores scores far from 1.

- **A set never shrinks, so `SPOP` walks ever sparser buckets.** `SPOP`
  picks the first filled bucket from a random position, and a hash map keeps
  its buckets after its members are removed, so after most of a large set is
  popped each pop scans many empty buckets inside the atomic statement;
  Redis shrinks its table as it empties. A library hash map that rebuilds at
  four times its pairs when a removal leaves it under an eighth full was
  measured and refused by its criterion
  ([firn](../research/investigations/firn/DESIGN.md#list-elements-inline-and-maps-that-shrink-results)):
  `SPOP` at depth 1 gained 14% on two CPUs but only 4% on one, while its p99
  fell to 0.32 and 0.58 of the unshrinking map's. The full suite then left
  `SPOP` at depth 1 on two CPUs at 0.87 of Valkey with I/O threads and 0.94
  of Dragonfly
  ([firn](../research/investigations/firn/DESIGN.md#the-full-suite-results)).
  Reopen with firn's next performance work after the scaling stage, or when a
  criterion weighs depth-1 tail latency; the change is small
  (`hash_map_rebuild` accepting fewer buckets than it had while they
  outnumber the pairs, and a check after each removal) and would join the
  growth decision of `hash-map-storage` in the design tree.

- **`SPOP`, `SRANDMEMBER`, `HRANDFIELD` and the set operations read the hash map's
  buckets.** The library has no entry that returns a member at random, so
  firn's `pick_member` (`firn/commands/sets.wf`) reads the map's public
  bucket array and matches its slot variants, which ties firn to the
  library's representation, and takes the first filled bucket from a random
  position, which favors a member that follows a run of empty buckets; Redis
  samples buckets instead. `random_members` walks the same array to choose
  distinct members, and `combine`, which `SINTER`, `SUNION`, `SDIFF`, their
  stores and `SINTERCARD` share, walks it too, because `hash_map_each` lends
  each pair to a visitor whose one environment cannot also reach the other
  sets it must look the member up in. A library entry that picks a filled
  bucket, as uniformly as its layout allows, and a visit that lends pairs to
  a caller-chosen step, would remove the three. Reopen with the library's
  next hash map change or when a second program needs a random member.
  `random_field` and `distinct_fields` in `firn/commands/hashes.wf`
  also read buckets: the former favors fields after empty runs and the
  latter scans every bucket even for a small count. A library entry that
  picks several distinct entries would remove that scan; reopen with the
  same library change.

- **A connection that waits with no idle limit misses a limit `CONFIG SET`
  sets.** firn's `serve` (`firn/server/server.wf`) gives a receive a
  deadline, at most a second away, only while an idle limit is set, and reads
  the limit again when a deadline passes and at most once a second while the
  client sends; a client waiting with no limit has no deadline, so after
  `CONFIG SET timeout 5` it stays open until it sends, where Redis's
  `clientsCron` closes every client silent past the new limit. Every receive
  with a deadline of at most a second would close the gap; a receive that
  parks then pays a timer insertion and removal on its driver's heap
  (`wf_context_arm_deadline` in Whitefoot's `compiler/src/backend/completion/bridge.c`),
  which an unpipelined benchmark pays on every request. Measure that cost with
  `redis-bench.sh quick` before choosing. Nor is a client closed while firn's
  send to it waits on replies it leaves unread, where Redis closes one that
  nothing has been written to for the limit; `flush` could pass `send_once`
  (`lib/std/net/module.wfm`) a deadline while a limit is set. Reopen when a
  deployment changes the limit while it runs or relies on it to drop clients
  that stop reading.

- **`INFO` leaves out what firn does not measure.** `run_info`
  (`firn/commands/info.wf`) reports real values for the port, the
  calendar time, the uptime, the clients connected, whether the append-only
  file is kept, the connections accepted and the keys held, and constants
  that are true of firn, but no memory used, processor time, commands or
  errors counted, keyspace hits or misses, keys expired or changes since a
  save, so its CPU, Commandstats, Errorstats and Latencystats sections are
  empty; the keyspace line's `expires` and `avg_ttl` are 0 whatever the keys
  hold. Tests of Redis's suite that read those fields fail on firn: all three
  of `unit/info-command`, which expect `rejected_calls` in Commandstats, and
  those reading `used_memory`, `total_error_replies` or `expired_keys`.
  Counting `expires` needs the statements that set, clear or remove an
  expiry to keep a count beside the table, or the table to count entries by a
  property; per-command counts need per-connection counters merged without a
  shared statement on each command, which would serialize every connection.
  Reopen when firn is monitored through `INFO`, or with the next work on
  firn's statistics.

- **A signal stops firn without writing its pending append-only bytes.**
  firn handles no signal, and the standard library's `std::process`
  delivers none, so SIGTERM or SIGINT ends it at once, losing the changes its
  writer (`write_log` in `firn/persistence/persistence.wf`) has not yet
  appended, up to one 10-millisecond cycle, and the bytes not yet synced,
  where Redis on SIGTERM appends and syncs its file before it exits, as its
  `SHUTDOWN` command does, which firn lacks. Seen on 2026-10-03: a `SET` sent
  a few milliseconds before a SIGTERM was absent after the replay. A signal
  delivered to a context could set the keyspace's `stopping`, which makes the
  writer append, sync and close, as it does once the client limit is
  reached. Reopen with `SHUTDOWN`, or when firn runs under a service manager
  that stops it with SIGTERM.

- **firn writes decimals and reads `CONFIG SET`'s integers in repeated
  code.** `text_reserve` and `text_number` in `firn/commands/info.wf`
  copy `log_reserve` and `log_number` in `firn/store/store.wf`, the one
  for `INFO`'s text and the other for the append-only file's, and
  `run_config_set` (`firn/commands/server.wf`) writes the same reading
  three times, for the port, the idle limit and the encoding parameters:
  `read_integer`, the error for a value that does not parse, the range check
  and `reply_bounds`. A fix to one copy can miss the others. Writing into a
  growing byte buffer belongs in one public function of the `bytes` or
  `protocol` module that both callers use, and the three readings in one
  helper that takes the parameter's range from a table like
  `encoding_lower` and `encoding_upper`. Reopen with the next change to
  either writer or to `CONFIG SET`'s parameters.

- **firn kept a key past its expiry once in Redis's suite.** In a
  redis-compat run on 2026-10-03 of firn at `c2dca4616`, made while the host
  built and tested in parallel, `unit/expire`'s "EXPIRE - After 2.1 seconds
  the key should no longer be here" found the key still there at least 2.1
  seconds after `EXPIRE x 2`: `GET` answered its value and `EXISTS` 1. The
  run before, the run after and five runs of the unit alone on the same
  binary passed it. A key's expiry was then an instant of the monotonic
  clock, which `serve` (`firn/server/server.wf`) read once after each
  receive, so no wait should have left a key past it; the cause is unknown,
  and expiries have since become calendar milliseconds (`8f8b43b48`).
  Reopen when it recurs, with each `EXPIRE`'s instant and each read's clock
  logged in a run under load.

- **The framework commands' reviews left four small items open.**
  `memory_value`, `c_space` and `save_token_valid`, firn's readings of
  Redis's memtoull and strtoll, sit in `firn/commands/server.wf`, where
  the README's layout puts such readings in `bytes`, beside `read_integer`
  and `glob_match`. No case gives `CONFIG SET port` a value that does not
  parse, so that branch of `run_config_set` is unchecked. The idle case in
  `tests/network.rs` pins a sending client's
  once-a-second reading of the limit only loosely: its pings end about 1.9
  seconds after the client's last reading, so a period up to that passes.
  And its waits make it firn's longest case, about 20 seconds with the
  build, where the whole group took 11.2 seconds at `51bc58e13`, while the
  corpus stage is over its budget on ubuntu. Moving the readings to
  `bytes`, adding the case, ending the pings 1.2 seconds after the last
  reading, and running the first route's checks beside the second's would
  settle them. Reopen with the next change to `CONFIG SET` or the idle
  limit, or when the corpus stage needs the time back.

- **Sorted-set ranks and counts walk the order element by element.** The
  library's `OrderedMap` keeps no subtree sizes, so `ZRANK` and `ZREVRANK`
  count the members before the one asked, `ZRANGE` by position and
  `ZREMRANGEBYRANK` skip the members before the range, and `ZCOUNT` and
  `ZLEXCOUNT` count the members in it (`rank_before` in
  `firn/commands/sorted.wf`, `walk_node` in `ranges.wf`): linear in the
  rank or the count, where Redis's skiplist spans make them logarithmic. A
  range by score or member does find its start by comparison. An ordered map
  that keeps each node's subtree size would make ranks and counts
  logarithmic; it changes the library's representation, a design decision.
  Reopen when a workload ranks or counts in large sorted sets, or with the
  library's next ordered map change.

## Tests

- **firn's network cases now and then lose their first connection when many
  cases run at once on a 32-CPU host.** `cargo test --test corpus` on
  the 14900K under WSL2, every case at once, failed one of firn's cases
  (now in `tests/network.rs`), a different one each time, with
  "Connection reset by peer" on the first batch's reply, or once with the
  server never listening: 1 run of 3 at `cea9188d4`, 3 of 4 at `f8ca277a9`,
  and 4 of 8 at `a008b01ef` and after; a case run alone passed 6 times of 6.
  The programs that abort during those runs are the same two in passing and
  failing runs alike, so firn is not seen to crash, and ports chosen below the
  ephemeral range, one per case, changed nothing. The gate's hosted runners
  showed the like once, on macOS at `cea9188d4`
  (`a_loopback_echo_preserves_all_bytes_and_half_close_on_both_routes`). On
  branch `firn/strings` the reset came in about 1 group run of 3 with
  firn's cases alone and in 1 run of the whole corpus at `44f10c449` (the
  replay case, on the native ring), while 440 servers of that branch and
  440 of `51bc58e13`, started 20 and 30 at a time outside the harness, each
  answering the first case's batch, saw no reset and all exited 0. Giving
  each of `connect_to_when_ready`'s attempts 5 seconds instead of 100
  milliseconds still left a reset in 1 group run of 8. A scratch patch to
  `ProgramChild`'s drop that reports the program when its case panics saw
  7 resets in 78 group runs at `44f10c449`, none in the last 24. Each time
  firn was running with nothing on its standard error; in the 6 examined
  further, firn held no socket (2) or no process held one on firn's port
  (4), and firn was still running a second (2) or ten seconds (4) later,
  where 1,000 firn servers started 40 at a time outside the harness each
  accepted a first connection within 62 milliseconds. Three of the four
  had not reached `tcp_listen`: they still held the working directory
  `main` closes before it, and 3 of the 96 anonymous descriptors a started
  firn holds on this host, 3 for each of its 32 drivers; the fourth held
  30. So firn stalls in its start, and the harness's connection must have
  reached some other socket on the port meanwhile, which a port
  `free_port` released allows and the fixed ports above should have ruled
  out. Run alternately with `51bc58e13`'s group, the branch's longer one
  failed this way in 3 of 5 runs at `abcb3127d`, once with the server never
  listening, against none of the base's 5, and in 1 of 4 at `474bb9da3`,
  against 1 of the base's 4, two cases at once; 8 runs at `abcb3127d` with a
  monitor that reported any program left more than 1.5 seconds without a
  socket neither failed nor showed one. The change: keep that report in the
  harness for a case that panics, and find what stalls firn's start there.
  Reopen
  when a hosted run of the corpus fails this way, or before the corpus gates
  on a large host.

- **firn's replay case allows the restart two seconds, which a loaded host
  exceeds.** `firn_replays_its_append_only_file_after_a_restart_on_both_routes`
  expects `PTTL` of a key set with `PX 60000` before the restart to answer at
  least 58,000 after it; with firn's cases running at once on the 14900K
  under WSL2 it answered 57,830 in 1 run of 8 at `51bc58e13` and 57,532 once
  on branch `firn/strings`, the first run's stop, the 400-millisecond pause
  and the second start taking about 2.4 seconds. The case now also compares
  each expiry's `PEXPIRETIME` before and after the restart, which no load
  changes; the window could give way to that comparison, or widen. Reopen
  when the case fails this way in CI.

## Whitefoot requirements

None filed since firn left the Whitefoot repository.
