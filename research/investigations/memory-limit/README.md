# Bounding firn's memory as Redis's maxmemory does

## The question

The deployment milestone needs memory-bounded operation: a cache that
evicts under a limit, and a session or rate-limit store that refuses writes
rather than lose data when full
([TODO](../../../docs/todo.md#server), "Complete firn's standalone
deployment workloads"). Redis 7.0.15 does both with `maxmemory`,
`maxmemory-policy` and its `OOM` refusal. At this investigation's start,
firn refused `CONFIG SET maxmemory` and reported constant `maxmemory:0`.
The implementation records below state the draft's current scope.
Whitefoot now lets a program read the bytes its heap holds and its resident
set ([memory statistics](https://github.com/Ming-Research/Whitefoot/pull/277)).
What should firn count, when should it check and evict, and which keys may
it evict, so that the selected consumers behave as they do on Redis?

## What Redis 7.0.15 does

From its source at the 7.0.15 tag (`evict.c`, `server.c`, `object.c`,
`db.c`, `script.c`, `multi.c`, `src/commands/*.json`):

- **What is compared.** `used_memory`, the allocator's usable bytes of every
  live allocation, less the append-only buffer and replica buffers beyond
  the backlog (`mem_not_counted_for_evict`), against `maxmemory`; 0, the
  default, means no limit.
- **When.** `processCommand` calls `performEvictions` before every command
  while `maxmemory` is set. It evicts until under the limit or until a time
  limit set by `maxmemory-eviction-tenacity` (default 10, about 500 µs),
  continuing from the event loop after that.
- **Refusal.** When nothing more can be evicted, a command flagged
  `denyoom` gets `-OOM command not allowed when used memory >
  'maxmemory'.`. Among the consumers' commands `SET`, `MSET`, `SETEX`,
  `INCR`, `INCRBY` are `denyoom`; `GET`, `MGET`, `EXPIRE`, `DEL`, `PTTL`,
  `SCAN` are not. Every command queued after `MULTI` is refused, and `EXEC`
  is refused when a queued command is `denyoom`; `DISCARD` never is. A
  script without a shebang runs, and its first `denyoom` call before any
  write is refused with the same error.
- **Policies.** `noeviction` (default), `allkeys-lru`, `allkeys-lfu`,
  `allkeys-random`, `volatile-lru`, `volatile-lfu`, `volatile-random`,
  `volatile-ttl`; `volatile-*` consider only keys with an expiry and act as
  `noeviction` when there are none.
- **Approximation.** Each round samples `maxmemory-samples` (default 5) keys
  with `dictGetSomeKeys`, keeps the best 16 candidates in a pool, and evicts
  the best. Each object carries 24 bits: a seconds clock for LRU, or a
  minute stamp and an 8-bit logarithmic counter for LFU. Every lookup except
  `EXISTS`, `TYPE`, `TTL`, `PTTL`, `OBJECT` and their kin refreshes it, under
  every policy, so `OBJECT IDLETIME` works under `noeviction`.
- **Report.** `INFO` gives `used_memory`, `used_memory_rss` (refreshed every
  100 ms), `used_memory_peak`, `maxmemory`, `maxmemory_policy`,
  `mem_fragmentation_ratio`, `mem_not_counted_for_evict`, and `evicted_keys`
  in its stats.

## What can be observed

Redis's suite runs `unit/maxmemory` only against a server it starts
itself, so the ratchet skips it. Tests elsewhere that run against firn need
`CONFIG SET maxmemory`, the `OOM` refusals of `MULTI`, `EXEC` and scripts,
`OBJECT IDLETIME` and `FREQ`, `RESTORE ... IDLETIME` and `FREQ`, and the
`used_memory` and `mem_not_counted_for_evict` fields (`unit/multi`,
`unit/scripting`, `unit/info`, `unit/dump`, `unit/introspection-2`,
`unit/tracking`). No test checks which of several candidates is evicted.
Which key goes first is observable to an application only as a cache's hit
rate.

The consumers say little: Django warns that a cache that evicts can log
users out of cache-backed sessions; connect-redis and rate-limiter-flexible
give every key an expiry and document no policy.

## Proposals

These were the initial direction-setting proposals. The recorded outcomes
and step-2 record below supersede them where the owner's rulings differ;
the historical alternatives explain what the owner selected.

1. **What is counted.** `used_memory` is Whitefoot's `heap_in_use`, the
   requested bytes of live allocations, and `used_memory_rss` its resident
   set; the limit compares `heap_in_use` less firn's append-only pending
   bytes. Redis counts usable sizes, so firn's count for the same data is
   lower by the allocator's rounding. This follows the memory-statistics
   proposal the owner approved; it is recorded here, not reopened.
2. **Which policies.** All eight, against the subset the consumers' likely
   deployments use (`noeviction`, `allkeys-lru`, `volatile-lru`). The
   sampling, the refusal and the reporting are shared; `random` needs
   nothing more, `ttl` can take the nearest expiry from the expiry queue
   firn already keeps, and LFU adds an 8-bit counter beside the stamp.
3. **The access stamp.** Each entry gains 32 bits holding Redis's 24-bit
   LRU clock or LFU stamp and counter, refreshed by every lookup Redis
   refreshes, under every policy, as Redis does. That makes `GET`'s
   statement write the entry it reads. The alternative, stamping only under
   an LRU or LFU policy, leaves `OBJECT IDLETIME` wrong under `noeviction`.
4. **When to check.** Before every command while `maxmemory` is nonzero,
   as Redis does, with one comparison of a cached setting when it is zero.
   The command's own context evicts, so several contexts may evict at
   once; each evicts until the count it reads is under the limit.
5. **Sampling.** `maxmemory-samples` keys from a random cursor of the
   keyspace's `map_scan`, keeping Redis's pool of 16 best candidates, for
   the `lru`, `lfu` and `random` policies; `volatile-*` skip keys without an
   expiry.

## Measurements, stated before implementing

- **Cost of the stamp.** firn's `redis-benchmark` `get` and `set` at
  depths 1 and 16 on one and two CPUs, with and without the stamp, same
  source, interleaved with twins on the i9-14900K. The stamp is rejected in
  that form if `get` loses more than its twins' spread, and then the next
  step is a stamp written only when the clock it holds has advanced.
- **Cost of the check.** The same comparison with `maxmemory` set far above
  the dataset, against unset.
- **Memory per key.** The resident set after one million sessions, as in
  the [deployment measurement](../deployment-performance/README.md#results-1),
  with and without the stamp.
- **Eviction quality.** A Zipf-distributed `GET`/`SET` workload under
  `allkeys-lru` with a limit at half the dataset: firn's hit rate against
  Redis 7.0.15's. firn's is rejected if it is more than two percentage
  points below Redis's.

## Step 1 implementation record

The [access/configuration draft record](step-1.md) lists every lookup site,
its Redis oracle, validation still required for the current working changes,
and the owner's two additional A rulings: snapshot memory settings once per
request read, and restore first-refresh stamps and LFU random state before
abandoning an unwritten script attempt. The owner selected all eight policies and
the all-policy access stamp in the board rulings `firn-maxmemory-policies` A
and `firn-access-stamp` A. The step-2 record below covers eviction and OOM refusal; the access-stamp
representation and its outstanding lock design remain unchanged.

## Results: cost of the access stamp

Measured on the i9-14900K through the `redis-bench.yml` workflow, mode
compare, LTO builds, two interleaved passes of five seconds:
`base` is main's firn (`exp/memlimit-base`, 6585531) built with the same
experiment compiler (`wf-exp-81609eda3c84`) and Halo-wf (d116d6a) as `head`,
this branch at 66ff776; each has a `-twin` measuring its image again. Median
throughput relative to `base`, with the twins' ratio as the noise control
([Firn-wf run 37888136395](https://github.com/Ming-Research/Firn-wf/actions/runs/37888136395)):

| CPUs | test | depth | base-twin | head | head-twin |
|---|---|---|---|---|---|
| 1 | get | 16 | 1.031 | 0.860 | 0.905 |
| 1 | get | 1 | 0.993 | 1.006 | 0.982 |
| 1 | set | 16 | 1.011 | 1.009 | 1.024 |
| 1 | set | 1 | 1.005 | 1.005 | 1.007 |
| 2 | get | 16 | 0.973 | 0.898 | 0.909 |
| 2 | get | 1 | 1.022 | 0.995 | 0.976 |
| 2 | set | 16 | 1.008 | 0.998 | 0.961 |
| 2 | set | 1 | 1.012 | 0.996 | 1.016 |

By the criterion stated before measuring, the stamp in this form is
rejected: `get` at depth 16 loses 9 to 14%, beyond its twins' spread, on one
and two CPUs. A profiled rerun of `get` at depth 16 on one CPU
([Firn-wf run 37889202069](https://github.com/Ming-Research/Firn-wf/actions/runs/37889202069);
head 0.912 and head-twin 0.912 of base, base-twin 1.004) attributes the loss
to the entry lock: head adds `acquire_entry` (6.5% of samples) and
`wf__table_unlock_entry` (1.8%), and `wf__table_lock_entry` rises from 1.6%
to 3.5%, while `run_get`, where the stamp is computed, stays at 0.7%. Before
the stamp, GET's statement only read its entry, and Whitefoot's shared map
serves a statement that only reads its entry on its lock-free read path
(`wf_cmap_read_entry`); a statement that may write the entry takes the entry
lock (`wf_cmap_lock_entry`) whether or not it writes. The stated fallback, a
stamp written only when the clock it holds has advanced, therefore does not
remove the cost: the statement still may write. Which change closes it is the
owner's ruling on the board card `firn-stamp-lock`; the profile used clock
sampling only, so cache effects are not separated.

## Recorded outcome: who evicts

The owner chose **A** on board card `firn-evict-admission` at 2026-10-09
06:01 UTC: each command's own context checks the limit before running and
evicts there. Option B, one dedicated evictor, and option C, serializing
admission through execution while a limit is set, were rejected for the
reasons recorded in [design/firn/memory-limit.md](../../../design/firn/memory-limit.md).
The admission-to-execution interval this leaves open, a command checking
below the limit while another command's allocation takes the heap over it,
is the accepted difference; it shows only as overshoot and in when
`evicted_keys` grows. Its size is to be measured with the eviction-quality
workload.

## Recorded outcome: the access stamp's lock

The owner chose **A** on board card `firn-stamp-lock` at 2026-10-09 06:01
UTC: Whitefoot is to provide a field of a basic type that can be updated
atomically under a read-only hold, so that GET keeps the shared map's
lock-free read path. The owner asked that its design first settle which
types qualify on which platforms and how it relates to a shared object
holding one value; that design is a Whitefoot investigation and card, and
this step waits for it.


## Step 2 implementation record

Working-tree implementation on `claude/maxmemory` for
[draft PR #35, maxmemory](https://github.com/Ming-Research/Firn-wf/pull/35),
2026-10-09. This record describes source, not a passing implementation:
no build, compiler, test, checker or performance run was made on the owner's
machine. The owner will commit; CI must validate that revision. No pin or
submodule was moved and no new Whitefoot gap has been established.

### Admission and policy behavior

`commands/admission.wf` records Redis 7.0.15's DENYOOM and WRITE flags for
all commands firn implements as parts; `command_kind` extends the existing
identity lookup to commands outside those parts. `execute_client` checks
existence, command-table arity, authentication and SHUTDOWN's NO_MULTI
restriction before `evict_before`. Outside a transaction an unknown name is
refused before admission. Inside one, a name `command_kind` does not
identify keeps the established foreign-transaction path with known commands
without parts: it is queued and EXEC refuses the whole transaction. A draft
that refused such names when sent regressed 94 ratchet rows in
[Firn-wf run 37904672454](https://github.com/Ming-Research/Firn-wf/actions/runs/37904672454):
Redis's suite sends commands firn does not run, such as BRPOPLPUSH and XADD,
inside MULTI, stops at the immediate error, and leaves its connection inside
the transaction for every later test, the failure the transactions node's
rejection already records. firn cannot tell a name Redis lacks from a Redis
command it does not run without a table of Redis's names. No command body is
specialized for a client, test or benchmark.

`eviction.wf` follows `evict.c performEvictions`, `evictionPoolPopulate` and
`evictionTimeLimitUs`, and `object.c LFUDecrAndReturn` via the unchanged
access functions. All eight policies are implemented; volatile eligibility
requires a nonzero expiry. Meta owns the single pool of 16 owned names and
scores: it already orders removal with propagation, so another shared object
would add a hold without removing the existing one. A per-context pool would
multiply retained candidates and history across connections. Sampling holds
the map and Meta; deletion releases that hold and acquires only the selected
entry and Meta, rechecking existence and volatile eligibility. DEL propagation,
removal and the eviction counter share that statement. A key expired at
revalidation follows active expiry's DEL path and increments the expiry
counter instead. The new internal expiry counter covers active expiry and
the store's removal helper; it is not advertised as a complete `expired_keys`
metric, since command-local lazy expiry still lacks complete accounting.

`map_scan` takes a random cursor and the snapshotted sample count. Its count
is a hint, so the whole returned batch is consumed and a reservoir retains
at most that many eligible keys. Random policies draw uniformly within that
reservoir with rejection of the modulo remainder; ranking policies retain
the best 16 scores across rounds. No sampling lookup refreshes a stamp.
The pool is cleared when the sampling policy changes, since its scores have
different meanings. A stale pool name is revalidated, not blindly removed.
An empty batch does not establish exhaustion: scanning continues through the
end and back to its starting position. The empty-search cursor lives in
Meta across time slices, so an unchanged volatile keyspace eventually
reaches FAIL even when proving absence needs several commands. To make
that proof valid under concurrency, `ExpiryQueue` groups the existing active
expiry queue with an `eligibility_changed` boolean. `queue_due` sets it in
the same entry-and-Meta statement that creates or transfers a nonzero
expiry, even if the queue cannot accept the active-expiry record. Sampling
consumes the flag and restarts its saved search. SET with expiry, the EXPIRE
family, GETEX with expiry, and RENAME/COPY with a transferred expiry all
reach this helper on their network, script and EXEC paths. SET KEEPTTL
preserves the same key's eligibility; plain SET, PERSIST, FLUSH, lazy/active
expiry, stamp restoration and eviction cannot add volatile eligibility.
FLUSH swaps only the inner queue and preserves the invalidation flag.

Thus an EXPIRE behind the cursor cannot produce false exhaustion, while
permanent writes and looping read-only scripts do not discard progress or
block SCRIPT KILL. No second index, per-command mutation counters or command
turn are added. Allkeys policies establish absence by live `map_count==0`
under the sampling hold; a nonempty map cannot return FAIL merely because
concurrent insertion put a key behind the cursor. The final heap/exhaustion
comparison shares that map-and-Meta statement, which is its admission point
if another command creates eligibility after the hold ends. Continuously
creating new expiries can restart a volatile proof; this has real eligible
work to retry and remains bounded by tenacity.

The serving context rereads the heap before sampling and immediately before
each victim, using its own `meter_share` handle. No global turn extends into
a command. Tenacity 0 through 10 uses 50 times the setting in microseconds;
11 through 99 uses floor(500 times 1.15 to the power setting minus 10), using
the pinned Halo number implementation; 100 is unlimited. The monotonic clock
is checked every 16 deletions and every 16 unsuccessful batches to bound
sparse/ghost work. One scan and one deletion are indivisible. A time limit
returns RUNNING, allowing the command; only exhaustion while still over the
limit returns FAIL and sets pre-command OOM. The next command continues;
there is no Redis `evictionTimeProc` continuation while clients are idle.

### Settings, accounting and persistence

The owner rulings `firn-evict-sampling` A, `firn-evict-config` A and
`firn-aof-exclusion` A are recorded in the memory-limit design node.
`access_settings` now copies limit, policy, samples, tenacity and LFU
parameters once per read. This connection retakes that snapshot after
executing CONFIG SET, including an attempted SET rejected for its values;
other connections retain their own snapshot. Arity/unknown-subcommand
refusals that never execute SET do not refresh it.

`heap_in_use` is raw used_memory. The comparison subtracts, saturating at
zero, `Meta.log.inner.cap` plus the writer's published private capacity when
AOF is enabled. The writer publishes its allocation, swap, drain and
partial-write suffix replacement in the same Meta statement as the change.
An in-progress host write does not resize storage, so its full capacity
remains excluded until replacement; the same unwritten bytes never switch
between counted and excluded at a swap. INFO reports raw heap and exclusion
separately, RSS on demand, observed peak, limit and policy, human-size forms,
and live evicted_keys. CONFIG RESETSTAT zeroes evicted_keys and connection
and active-expiry counts while preserving used_memory_peak, following
Redis 7.0.15 [configResetStatCommand](https://github.com/redis/redis/blob/7.0.15/src/config.c)
and [resetServerStats](https://github.com/redis/redis/blob/7.0.15/src/server.c).
Every serving context receives a MemoryMeter share;
no other context evicts. A related stale constant, cached script count, is
fixed by reading the actual registry in its own short statement. Peak
recording adds a short Meta hold even with a zero limit; its overhead is
unmeasured and belongs in the nonbinding-limit comparison.

### Transactions and scripts

`processCommand` is the admission oracle: all MULTI queueing is denied under
OOM, even reads, except EXEC, DISCARD, QUIT and RESET. EXEC uses the union of
queued DENYOOM flags, then runs without further memory checks. RESET now
clears the transaction, name, authentication and protocol state firn has.
A queue-time OOM returns the plain OOM error and marks the transaction
dirty. An admitted EXEC returns `-EXECABORT Transaction discarded because
of previous errors.`, including after memory recovers. If queued DENYOOM
flags instead cause EXEC admission itself to fail under OOM, it returns
`-EXECABORT Transaction discarded because of: OOM command not allowed when
used memory > 'maxmemory'.` before the dirty-state check. No separate OOM
cause is retained. These paths follow
[server.c rejectCommand/processCommand](https://github.com/redis/redis/blob/7.0.15/src/server.c)
and [multi.c execCommand/execCommandAbort](https://github.com/redis/redis/blob/7.0.15/src/multi.c).
As `queueMultiCommand` does, an already dirty transaction answers a later
admitted command with QUEUED without extending its queue or DENYOOM union;
memory recovery followed by another OOM cannot change EXEC's error merely
because that discarded command was a write.

`script.c scriptVerifyOOM` and `scriptCall` supply legacy-script behavior:
pre-command OOM is captured once, and a separate attempt-local WRITE_DIRTY
flag is set before an accepted WRITE command runs, including a no-effect or
error result. A DEL of a missing key therefore permits a later SET even
when `held.wrote` remains false. Each abandoned attempt restarts WRITE_DIRTY
under the same captured OOM; the effects flag still controls retries and
rollback. Shebangs are explicitly unsupported: EVAL and SCRIPT LOAD refuse
plain `#!lua` and every flag, including `allow-oom`, `no-writes`,
`allow-stale`, `no-cluster` and `allow-cross-slot-keys`. No flagged script
is silently interpreted as legacy. Implementing those contracts is deferred.

### Oracle differences and open evidence

Accepted differences are requested rather than allocator-usable bytes,
RSS on demand rather than Redis's periodic sample, per-read settings with
own-SET refresh, the map_scan sampling distribution rather than Redis's
separate expires dictionary, concurrent admission overshoot, and continuation
on the next command. Eviction quality is unmeasured. The pinned Whitefoot
meter currently misses shared-map tables and nodes, pending
[Whitefoot PR #298, shared-map heap accounting](https://github.com/Ming-Research/Whitefoot/pull/298).
All new memory-growth cases use 64 KiB values. After that fix, raw memory,
peak, chosen relative limits and the number of victims needed may increase;
no new test pins those amounts or an exact victim order, and no expected
assertion is intended to change. Exact eligible-set counts under a one-byte
limit still count keys, not bytes.

`tests/memory_limit.rs`, registered by `tests/network.rs`, adds cases for
all policies, all supported DENYOOM flags, preflight-before-eviction AOF
ordering, sparse volatile exhaustion, continued exhaustion across permanent
writes, bounded continuation, queue-time OOM and its previous-errors abort
(including after memory recovery), EXEC admission's explicit OOM abort,
all four MULTI OOM exemptions, and EXEC's single admission, legacy-script
OOM before/after missing-key DEL
including a budget retry, explicit shebang refusal, same-read CONFIG,
heap/RSS/peak/eviction INFO, RESETSTAT preserving a peak above the reduced
live heap while zeroing eviction counts, drained AOF capacities and replayed
DEL, SCRIPT KILL progress with unlimited eviction and an in-flight script,
and the cached-script count repair. Without step 2 the first refusal
cases return OK/QUEUED, memory fields are absent and evicted_keys stays zero,
keys survive writing past the limit and AOF replay, and cached scripts still
report zero. The existing transaction unknown-name expectations are
unchanged, since unidentified names still queue inside a transaction; no
ratchet row is removed or weakened.

Compiler acceptance and every new case are unverified. In particular, CI
must check the new effects and interface signatures, MemoryMeter shares in
Client, whole-map `map_scan` followed by entry-plus-Meta deletion, owned
candidate moves and reservoir index proofs, and the attempt-local OOM state.
These are natural forms written directly against the pinned specification;
there is no compiler rejection yet and no fallback spelling masking one.
Partial-write/suffix capacity races, concurrent SET/eviction AOF ordering,
expiry at revalidation, configuration races, expiry-creation invalidation
behind the cursor and crash recovery still need fault/concurrency evidence;
the sequential replay case does not establish them. CI must run the gate and
Redis ratchet and commit any new ratchet passes.

The owed 14900K CI measurements remain: the specified Zipf hit-rate
comparison against Redis (reject firn more than two percentage points below
Redis), throughput and latency at the limit with and without AOF, unlimited
versus high nonbinding limit admission overhead, and peak overshoot with
multiple simultaneous writers, transactions and scripts. Use matched
versions/settings/workloads, interleaved same-source before/after runs and
base twins where attributing a cost; first time a small sample. None of the
network assertions substitutes for those measurements. The access-stamp
lock issue remains with the separate Whitefoot design, unchanged here.

### Independent source review, 2026-10-09

A separate read-only agent using the inherited session model reviewed
`bbd53a2e3007eac0dafd03c6dd150ff301accaf1..working tree`, including the three
new files, affected consumers, Redis 7.0.15 source, pinned Whitefoot rules,
and relevant design nodes and ancestors. The tool did not expose a more
specific model identifier. Every project and owner checklist group was in
scope. Its final limited review covered the replacement exhaustion proof,
ExpiryQueue interfaces and all eligibility creators, the two progress
regressions, and corresponding documentation. No build, compiler, test,
checker, Redis execution or measurement ran; review was source inspection.

The review found and resolved these issues:

- R1: cached-script INFO literal lengths were corrected to 35 and 58 bytes.
- R2: a volatile absence scan spanning holds could miss a concurrent EXPIRE
  behind its cursor. Actual expiry creation/transfer now invalidates the
  scan, and the final heap comparison shares its whole-map and Meta hold.
  Nonempty allkeys maps cannot claim exhaustion from such a scan.
- R3: transaction unknown-command and settings-snapshot documentation was
  brought into agreement with preflight refusal and own-CONFIG refresh.
- R4: an interim unlimited-scan implementation needed an explicit borrow
  for an owned option. That entire implementation was subsequently removed;
  final victim extraction explicitly moves the sample and its option.
- R5: an interim invalidation on every admitted WRITE could restart a
  volatile scan forever during permanent overwrites. The final ExpiryQueue
  flag records actual eligibility changes instead; a regression requires
  eventual OOM across repeated permanent overwrites and bounded time slices.

The reviewer reported no outstanding findings within inspected source scope.
A1, C1, T2, D1, G1, G2 and DC1-DC3 pass within that scope; performance
attribution R1 is not applicable. C2, T1, T3, G3 and DC4 remain unverified:
compiler acceptance, new ratchet passes, gate/readiness, concurrency and
performance adequacy, and runtime correspondence lack execution evidence.
Formal design-lint node/depth diagnostics are also unverified. This is not
approval or a passing CI result. No pin, submodule, workflow, Makefile or
ratchet-list change is part of this implementation; no new Whitefoot gap has
been established or filed.

### Redis oracle corrections, 2026-10-09

The earlier task prompt's peak-reset and queue-time OOM instructions were
mistakes, not owner rulings. The statistics and transaction sections above
now describe Redis's behavior, and those two entries have been removed from
the retained differences. The related already-dirty queue flag defect is
also fixed as described in the transaction section.

No before/after execution was attempted: this correction task requires
uncommitted edits and forbids local builds and tests. The changed assertions
reject the former explicit abort after a queue-only OOM and the former
reduced peak after RESETSTAT; the additional recovery case detects later
commands incorrectly extending a dirty transaction's DENYOOM flags.

A separate read-only review using the inherited GPT-6 session model (exact
serving identifier unavailable) compared this correction with the initial
dirty working tree, read the full step-2 diff and untracked files against
`bbd53a2e3007eac0dafd03c6dd150ff301accaf1` for context, and inspected the
affected consumers, Redis sources and relevant design nodes. It reported
no findings within correction scope. Source-level checklist items pass;
compiler acceptance, runtime correspondence, gate/readiness, new ratchet
passes and formal design-lint diagnostics remain unverified. No build,
test, checker or measurement ran, and no pin or submodule moved or new
Whitefoot gap was filed.
