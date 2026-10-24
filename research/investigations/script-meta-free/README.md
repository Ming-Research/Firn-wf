# Script execution without a full-attempt Meta hold

Board item: `firn-script-meta-free`, step 2 of the owner's two-step study.
This is research, not an implementation or an approved replacement design.

**Source-read.** Server baseline: the worktree's `origin/main` resolves to
`e91c4887f84a672ce61c9108bb6ef5932e2fac55`. All Firn `file:line` anchors below
refer to that revision, not the older revisions cited by the related studies.
The remote freshness check failed because this environment could not resolve
GitHub over SSH; “origin/main” here means that verified local remote-tracking
revision. The compiler pin is `wf-af7665be672d`, resolved in the local Whitefoot
clone to `af7665be672d29912d878dd5da8c3932bb6f296c`, specification v0.146.
The kit is `56fd13b33ff39f5c38efe4c0b7f7e9a04913110d`.

**Source-read** labels inspected source or governing documents; **judgment**
labels proposed changes, deductions and untested predictions. A table's caption
sets the label for its factual or proposed columns. **Measured:** no measurements,
compiler invocations, builds, tests, server runs or CI runs were made for this
investigation; there is no run ID. No server, design, pin or submodule changes
are part of it.

## Question, prediction and rejection criterion

**Judgment; registered before measurement.** After the engine pool lands, can
independent scripts hold their declared entries throughout Lua execution but
take Meta only to publish accepted effects, preserving Firn's approved reply,
expiry, memory, error and persistence contracts? Does removing the full-attempt
hold materially improve the real scripted limiter, or merely move waiting to
another shared resource?

Compare the same engine-pool source with its existing early Meta use against
the specific candidate selected for each AOF cell below. Use the same compiler,
Halo, workload, settings and machine, plus an independently built twin of the
baseline. An AOF-off gain does not answer the AOF-on question.

**Source-read.** The primary limiter script begins with a WRITE:
`redis.call('set', KEYS[1], 0, 'EX', ARGV[2], 'NX')`
(`research/experiments/redis-bench/workload/src/limiter.rs:21`). Option A's live
disk check therefore takes Meta at the first redis.call when AOF is on and
holds it through essentially the entire script. Removing sequence/capture
setup before that call frees no substantial read prefix in this workload.

**Judgment; predictions by option.** Option D, the AOF-off-only change, is
predicted to improve the two-core limiter's throughput by more than max(10%, N),
where N is the twin/spread noise bound defined below, without one-core throughput
or p99 regressions exceeding N. It predicts no AOF-on metadata gain because
that path is unchanged. D retains the approved KILL behavior and needs no new
KILL integration. Option A is predicted to gain about nothing on this AOF-on limiter;
its AOF-off result has the same gain criterion as D. Only option B, after a
live-health contract permits staying Meta-free past first WRITE, has the proposed
AOF-on throughput-gain criterion. These are unmeasured predictions, not results.

For cells that predict a gain, a repeatable throughput gain failing
max(10%, N), or a regression exceeding N, rejects the performance premise.
Meta wait share is attribution evidence, not an acceptance threshold: a
two-core pooled baseline can approach 50% simply from serialization, so “at
least 20%” would not distinguish useful proposals. Moving waiting elsewhere
without the required end-to-end gain also rejects the premise. A noisy result
is inconclusive. Any reply, accepted-effect, recovery or capture discrepancy
rejects the correctness proposal. AOF-on results near baseline for A and D
are expected controls, not evidence of AOF-on Meta-free execution.

## Governing decisions and scope

**Source-read.** Read together: [Firn](../../../design/firn.md),
[scripts](../../../design/firn/scripts.md),
[command parts](../../../design/firn/command-parts.md),
[memory limit](../../../design/firn/memory-limit.md),
[append-only file](../../../design/firn/append-only-file.md),
[rewrite](../../../design/firn/aof-rewrite.md),
[sequence](../../../design/firn/aof-rewrite/sequence.md) and
[capture](../../../design/firn/aof-rewrite/capture.md).
The [script-engine investigation](../script-engines/README.md) owns engine
pooling, declared-key undo and whole-map fallback. The merged
[write-meta investigation](../write-meta/README.md) owns plain commands and
the higher-throughput alternative to a globally exclusive commit object.

**Source-read.** The premise “every data-changing statement holds Meta” is no
longer true for all plain commands on this baseline: AOF-off commands without
expiry metadata already avoid it. Scripts still start both attempt forms with
`sequence_new(meta: m, ...)` and `log_budget_begin(meta: m, ...)`
(`firn/scripting/entry.wf:503-506,567-573`). The current pool still checks out
one engine (`entry.wf:54-85`). Thus neither this baseline nor this investigation
demonstrates simultaneous Lua execution.

**Source-read.** Following the kit's
[language-reading procedure](../../../whitefoot-kit/downstream.md#reading-the-language),
the pinned [SHARE-2/3 rules](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2322-L2363)
permit the runtime team's reported answer. One keys-plus-Meta statement is
expressible; SHARE-3's last sentence permits holding fewer states where outcomes
remain equivalent to exclusive execution. The following acquisition timing is
**current-compiler policy**, documented in the
[writing guide, lines 1664-1752](../../../whitefoot-kit/docs/writing.md#how-do-i-lock-an-object-only-on-paths-that-use-it),
rather than a mandatory timing rule of the specification: key-set targets
acquire early; the later Meta type group can
acquire at its first use and remains held to statement end. Helper footprints,
a later type group or a target-free guard can force acquisition early. Header
order is not lock order under this policy. The actual refactor needs the CI
lock ledger; source acceptance alone does not prove acquisition timing.

**Source-read; SHARE-2 requirement.** Nested atomic statements and waiting
calls inside the block are forbidden, independently of acquisition policy.

**Source-read.** More specifically, the pinned compiler's
[`expression_bindings`, lines 947-970](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L947-L970)
and [`take_units_for`, lines 399-421](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L399-L421)
count a bare target binding passed as a call argument as a use before that
call. Passing `m` to the Lua execution helper therefore takes Meta even if a
particular callback path never reads a field. The execution interface itself
must stop receiving that binding for fully late acquisition.

## Meta access inventory

**Source-read for current behavior; judgment for classification and changes.**
The inventory follows EVAL/EVALSHA admission, both attempt branches, both
ScriptCommands callbacks, the entire `held_entries_run` dispatcher and its
whole-map wrapper, then exit and competing maintenance paths. It includes
transitive uses and explicitly identifies absent accesses; a helper called
`*_meta` often writes a private effects buffer, not shared Meta.

Classification:

- **(a)** A local delta or deferred commit operation can replace the use;
  existing helper contracts may need redesign. This does not authorize the
  redesign or claim it has been compiled.
- **(b)** A value must be observed at its semantic decision point under the
  current contract. “Outside” marks mandatory pre/post-execution observations,
  not a claim they occur inside Lua.
- **(c)** Preparation/input can remain or move outside Lua without reading
  Meta, or the path has no Meta dependency at all.

### Entry, shared helpers and exit

| Path and baseline anchors | Actual access | Class and consequence |
|---|---|---|
| Network dispatch `firn/commands/dispatch.wf:216-238`; admission `commands/transaction.wf:395-399,429-449` | `evict_before` saves `client.pre_command_oom`. EVAL/EVALSHA are not outer WRITE/PING disk-check cases. | **(c)** dispatch and local OOM input; memory admission's separate Meta accesses are listed below. Do not add an entry MISCONF refusal for legacy EVAL. |
| Engine checkout, SHA registry, compile/prepare: `scripting/entry.wf:54-140,208-225,378-502` | `store.scripts` pool/registry/generation/kill counter; no Meta. | **(c)** keep checkout/compile outside the key statement. A separate shared pool is not Meta-free concurrency evidence. |
| Both attempt entries `entry.wf:504-505,571-572`; `store/sequence.wf:1-10` | Reads `commit_seq`, `aof_rewrite_seq_exhausted`, `rewrite.active/limit/used/start`, `log.inner.len`. | **(a)** replace early snapshot with local mutation tracking, then construct a fresh commit token. Reusing this snapshot after concurrent commits is wrong. |
| `store/store.wf:544-550`, called at those entries | `rewrite.rotating/limit/used` initialize LogBudget. | **(a)** initialize just before final append, against the current rotation and reserve. |
| Callback propagation `entry.wf:265-347,529,605`; `commands/script.wf:2033,2130-2168` | Whole callback has `writes(meta)`; narrow callback declares reads of disk-error fields. Passing the target into the call exposes this footprint before dynamic admission. | **(a)** remove bookkeeping Meta arguments from execution; **(b)** retain a real first-WRITE health boundary. Merely moving explicit field reads does not prove the callback avoids the lock. |
| Whole-mode finite dispatch `commands/script.wf:1546-1561` | `held_run` passes `due: &meta^.due` to every finite `held_entries_run` call. Expiry creation/transfer therefore writes the shared queue during Lua in whole-map fallback; it is not staged as in narrow mode. | **(a)** give whole mode a local expiry queue too, then apply insertions at accepted commit. Remove this borrowed Meta descendant from the ordinary execution boundary even for commands that queue nothing. Whole FLUSH must stage an ordered reset plus later insertions. |
| First accepted WRITE admission `commands/script.wf:2069-2084`; `commands/admission.wf:35-44`; `store/store.wf:553` onward | Reads `aof_append_error`, `aof_sync_error`, `aof_append_partial`, including error selection. Later accepted WRITEs bypass it via local WRITE_DIRTY. | **(b)** when AOF is enabled. It changes a call exception/pcall result before Lua continues. AOF off has no writer and can use the known healthy condition without Meta; this is an explicit configuration fact, not a stale snapshot. |
| OOM command check `commands/script.wf:2076-2084`; `entry.wf:439,492` | Local `pre_command_oom` and WRITE_DIRTY only. | **(c)** preserve the original captured OOM state across retries; no per-redis.call memory measurement or eviction. |
| All mutation helpers `store/sequence.wf:28-99,126-190`; `store/rewrite-capture.wf:59-167` | Entry stamps and local WriteSequence fields/images; their initial values currently came from Meta. | **(a)** track actual mutations independently of the eventual sequence. Assign final stamps/capture at commit. Do not use an inert token as the mutation detector. |
| Reply conversion and written-state detection `entry.wf:349-376,539,548-550,615,624-626`; `scripting/replies.wf` | Current `held.wrote` comes only from sequence.taken or appended > 0; conversion itself uses engine/Env/client bytes, not Meta. | **(c)** conversion needs none. D must add the logging-independent `changed` detector specified below before resuming Lua; inert tokens and disabled effect encoding provide neither current signal. |
| Done and Lua Error `entry.wf:542,552,618,628`; `record_effects:248-261` | Append private effects to `m.log`, with MULTI/EXEC when record count exceeds one. | **(a)** append exactly once on either accepted outcome, never on abandonment. |
| Accepted narrow exit `entry.wf:658`; `store/attempt.wf:267-295` | Apply local expiry insertions to `m.due`, including eligibility invalidation. | **(a)** already staged on the narrow path; preserve this atomic publication. |
| Both accepted exits `entry.wf:563,659`; `store/sequence.wf:109-136`; `store/rewrite-capture.wf:282-365` | Write `commit_seq`, exhaustion, rewrite reserve/peak/abort/journal, read appended log bytes as needed. | **(a)** single final publication while all keys remain held. Sequence exhaustion still saturates and refuses capture, not service writes. |
| Retry/rollback and engine return `entry.wf:510-520,579-595,632-653,665-689` | Undo entries/access/client/VM state; discard local effects, due and sequence scratch; pool operations outside the statement. | **(c)** no new Meta access needed for abandonment. No publication on an abandoned attempt. |

### Every admitted command family

**Source-read for roster and shared-state reachability; judgment for deferral.**
All finite families pass through the common admission row above, including
read commands because the interface has a Meta footprint. All actual dataset
mutations reach the common sequence/capture row. Those two uses are not
repeated as separate physical accesses for each command. The following table
accounts for additional family-specific work. Anchors in this table are in
`firn/commands/script.wf` unless another path is given.

| Family (including read variants) | Adapter anchors | Additional Meta dependency / classification |
|---|---|---|
| GET, SET, SETNX, SETEX, PSETEX, GETSET, GETDEL, GETEX, APPEND, SETRANGE, STRLEN, GETRANGE/SUBSTR | `460,523,2237-2530` | **(a)** effects; SET/SETEX/PSETEX/GETEX expiry insertion is `due`, already private in narrow mode. Values, options and replies use entries/local state. |
| INCR, DECR, INCRBY, DECRBY, INCRBYFLOAT | `494,574,2439` | **(a)** effects/stamps; arithmetic and overflow replies need no Meta. Float propagation uses the effective value, not a second random computation on replay. |
| MGET, MSET, MSETNX | `647,2531,2596` | **(a)** effects; all operand entries must be held. No additional shared metadata. |
| DEL, UNLINK, EXISTS, TOUCH, TYPE, PERSIST | `2178,2208,2531` | **(a)** actual deletion/stamps/effects; PERSIST removes the entry deadline, leaving stale queue records harmless. No live shared queue query decides the reply. |
| EXPIRE, PEXPIRE, EXPIREAT, PEXPIREAT; TTL, PTTL, EXPIRETIME, PEXPIRETIME | `588,621` | **(a)** expiry insertions/effects, **(c)** frozen Time input. Reply decisions read the held entry, not Meta.due. |
| RENAME, RENAMENX, COPY | `2658,2681` | **(a)** transferred expiry insertion plus effects; hold source and destination and retain their final mutation identities. |
| HSET, HMSET, HSETNX, HGET, HMGET, HDEL, HEXISTS, HSTRLEN, HLEN, HGETALL, HKEYS, HVALS, HINCRBY, HINCRBYFLOAT, HRANDFIELD | `2709-3075` | **(a)** effects/stamps only. Random-field/access draws use client-local random state, not Meta.random. |
| LPUSH, RPUSH, LPUSHX, RPUSHX, LPOP, RPOP, LRANGE, LLEN, LINDEX, LSET, LREM, LTRIM, LINSERT, LPOS, LMOVE, RPOPLPUSH | `3077-3405`; dispatch `1564-2030` | **(a)** effects/stamps, including both move operands; no direct Meta field. These are the nonblocking admitted forms. |
| SADD, SREM, SPOP, SCARD, SMEMBERS, SISMEMBER, SMISMEMBER, SRANDMEMBER, SMOVE, SINTER, SUNION, SDIFF, SINTERCARD, SINTERSTORE, SUNIONSTORE, SDIFFSTORE | `3407-3673` | **(a)** effects/stamps. SPOP selects through client.random (`firn/commands/sets.wf:1394` onward), not Meta.random despite Meta's older field description. Stores hold every source and destination. |
| ZADD, ZINCRBY, ZREM, ZPOPMIN/MAX, ZCARD, ZSCORE, ZMSCORE, ZRANK/ZREVRANK, ZRANGE and reverse/score/lex variants, ZCOUNT, ZLEXCOUNT, ZREMRANGEBYRANK/BYSCORE/BYLEX | `3674-4010` | **(a)** effects/stamps; scores, members and reply errors are entry/local calculations. No direct Meta field. |
| OBJECT supported subcommands | `434`, plus `firn/commands/object.wf` | **(c)** entry access metadata and client memory-policy snapshot; lazy expiry takes the common **(a)** path. No Meta statistics read. |
| PING, ECHO, TIME, SELECT, implemented COMMAND forms | `held_entries_run:1564-2030`, especially `2018-2029` | **(c)** client/input/table data. TIME uses the captured client clock; no Meta. Script PING does not run network PING's outer disk admission. |
| FLUSHALL, FLUSHDB | `1525-1529`; `firn/commands/server.wf:159-188` | Whole-map fallback. **(a)** clear due queue and publish flush/capture-abort with effects at commit; preserve eligibility invalidation semantics. Whole-map locking already prevents disjoint-key concurrency. |
| SCAN, KEYS, DBSIZE | `1531-1535,1541-1544`; `firn/commands/scan.wf:206,287,299,330` | Whole-map fallback. **(c)** enumeration/count; SCAN's lazy deletion has **(a)** effects/stamps. No additional Meta read. |
| RANDOMKEY | `1538-1539`; `firn/commands/scan.wf:338` | Whole-map fallback; **(b)** reads/advances Meta.random to select a reply and possibly deletes expired entries. Keep the global draw for this serial path; localizing it needs an explicit random-state contract. It is not a blocker to concurrency of finite declared-key scripts. |
| INFO, CONFIG, SCRIPT, EVAL/EVALSHA, transaction controls, blocking/unsupported families, FUNCTION/FCALL | `held_kind:1174`, admission `2033-2127` | Not an alternative route into network handlers. Unsupported/forbidden calls return their existing errors; **(c)** no hidden INFO/CONFIG Meta traversal. No keyspace notification/PUBLISH implementation is reached. |

### Expiry, memory, rewrite and controls around the script

| Path and baseline anchors | Meta access | Classification / interaction |
|---|---|---|
| Lazy expiry inside script: `commands/script.wf:483-489`, common `commands/keys.wf:249-261`, sequence deletion helpers | Removes expired held entry and records DEL locally; does **not** update Meta.expired_keys. | **(a)** effects and a future correct counter delta; entry deletion must remain immediate within the isolated attempt. The absent counter increment is a source-inferred reporting defect, not an access to invent. |
| Active expiry: `store/store.wf:103-127,142-198,203-232` | Pop `due`; read stopping; update expired_keys, log, sequence/capture under key+Meta. | Separate statements, outside Lua. **(a)** deletion publication in principle; queue selection/stopping are **(b), outside**. Existing path must serialize against held script keys and final due insertion. |
| Pre-command heap check: `commands/eviction.wf:9-79,325-380` | AOF excluded capacity (`log.cap`, `aof_pending_capacity`), memory_peak, rewrite limit/progress/install/abort state. Heap itself is a host measurement, not stored memory accounting in Meta. | **(b), outside**, when these live fields affect pressure decisions; not an inside-Lua obstacle. AOF-off under-limit path can be **(c)**. A stale exclusion snapshot is not a proven substitute. |
| Victim selection/deletion: `eviction.wf:203-318,386-437` | Eviction pool/policy/cursor/search flags, due.eligibility_changed; recheck memory; expired/evicted counters; sequence/log/capture. | **(b), outside** for choosing/refusing eviction; **(a)** for each accepted deletion's publication. Do not move eviction inside Lua. |
| Post-command peak: `eviction.wf:41-62` | Conditionally raises memory_peak and updates client.seen_peak. | **(a)** short aggregate update, already outside script; retain the sampled value/boundary rather than silently omitting peaks. |
| Capture during commands: `store/rewrite-capture.wf:59-167`, sequence initialization above | Private coalesced images and allowance copied from Meta. | **(a)** redesign to final-image capture at commit. Under the current incremental-capture implementation this early read is required for its reservation; it is not an inherently necessary Lua observation. |
| Capture start/cut: `persistence/rewrite.wf:545-606,676-693`; scan `rewrite-dataset.wf:43-132` | S0/S1 read commit_seq and update rewrite state; S1 swaps log. Scan holds whole keys+Meta and validates last_write against commit_seq. | **(b), outside** for maintenance's own cut; script can defer its participation to **(a)** commit, as argued below. |
| Writer/error publication: `persistence/rewrite.wf:285-389,892-949` | Swap log/pending sizes; publish append/sync failure and partial marker; update file sizes. | **(b), outside**. Must order accepted records and health observations correctly; not a field the script may pretend is immutable while AOF is enabled. |
| SCRIPT FLUSH/KILL/LOAD/EXISTS: `scripting/entry.wf:695-813`, take/return/ends/kill helpers above | Pool.running/generation/registry/kills/out, not Meta; current KILL observes no published mutation/WRITE verdict. | **(c)** for Meta inventory. FLUSH waits across attempts. Accepted-WRITE publication belongs to the separate deferred KILL fix at script decision line 21 and `firn-req-attempt-publish`, not D. |
| Notifications | No notification dispatch/configuration in `firn/`; `commands/info.wf:74` reports zero pubsub channels. | **(c), absent**. There is no event buffer to defer today. Adding notifications would require publication/abandonment rules; it is not part of this proposal. |
| Other Meta fields (`clients`, stopping, file/rewrite statistics and settings) | Lifecycle/maintenance/INFO owners, not an admitted redis.call handler. | **(c), absent from Lua** except the explicit maintenance rows above. This inventory does not treat every server-wide Meta access as a script access. |

**Source-read.** For the **(b)** execution cases, Redis 7.2.16
[`script.c:scriptVerifyWriteCommandAllow`](https://github.com/redis/redis/blob/7.2.16/src/script.c)
checks disk failure before setting WRITE_DIRTY on the first accepted WRITE;
`scriptCall` marks accepted WRITE before
running the command. [`server.c:writeCommandsDeniedByDiskError`](https://github.com/redis/redis/blob/7.2.16/src/server.c)
also observes background fsync status. Thus an entry snapshot and a final
commit check have different boundaries from the required first-write decision.
Redis [`db.c:dbRandomKey`](https://github.com/redis/redis/blob/7.2.16/src/db.c)
selects a live key, expiring candidates when needed; it does not promise the
same pseudorandom sequence as Firn. Whole-map serialization makes retaining
Firn's present draw safe; introducing independent PRNG state is a separate choice.

**Source-read.** For outside-execution **(b)** rows, Redis
[`server.c:processCommand`](https://github.com/redis/redis/blob/7.2.16/src/server.c)
performs eviction before execution and captures OOM;
[`evict.c:performEvictions/getMaxmemoryState/freeMemoryGetNotCountedMemory`](https://github.com/redis/redis/blob/7.2.16/src/evict.c)
own victim choice, accounting and refusal. Redis's
[`aof.c:rewriteAppendOnlyFileBackground`](https://github.com/redis/redis/blob/7.2.16/src/aof.c)
forks the base writer and opens an incremental file. Firn's live-image scan,
reserve R and capture budget have no literal Redis per-command counterpart;
their acceptance oracle is recovered dataset equivalence plus Firn's approved
reserve/failure contract, not a fabricated Redis Meta read.

## Candidate publication protocol and correctness

**Judgment.** This protocol establishes a conditional dataset/effect proof.
It does not prove that all timed and memory-sensitive replies form an exact
single-threaded Redis history. The qualifications below are part of the result.

### Premises

1. One accepted attempt holds every read and written key from before its first
   access until publication; whole-keyspace commands select the whole target
   before that attempt starts. No nested acquisition or widening occurs.
2. No ordinary observer, expiry worker, eviction victim or rewrite scanner
   sees provisional entries. All conflicting paths use the same map authority.
3. A logging-independent attempt flag tracks mutation operations (including
   deletions and the existing conservative FLUSH marker); local bookkeeping
   tracks ordered propagated effects, expiry insertions and reset actions.
   Mutation detection remains separate from
   sequence allocation, append counts and the accepted-WRITE admission flag.
4. Done **and Lua Error** accept preceding effects. Only internal abandonment
   rolls back, restoring entries, access and last-write stamps, client reply
   position/random state and engine-visible state before releasing keys.
5. With AOF on, while keys are still held, one Meta acquisition assigns one
   sequence to an actual mutating statement, stamps surviving changed entries,
   captures final images/tombstones if required and appends its entire effects
   block. With AOF off, use an inert token, with no sequence or capture publication
   and zero final write stamps. In either mode, apply staged expiry/reset changes
   in the same statement before releasing keys; Meta need not be used at all if
   no shared metadata operation is needed. Abandonment publishes none of these.
6. Plain writes, EXEC, expiry/eviction and writer cuts preserve that same
   publication invariant. No write releases keys and later queues its AOF.
7. Replies are buffered until the accepted statement completes; reads cannot
   create a causal dependency on an uncommitted mutation. The file writer
   preserves Meta append order and existing fsync/failure/truncation rules.
8. Time, memory-policy/OOM inputs and shared Lua library/random state obey
   their explicitly approved contracts. Engine pooling must settle its shared
   Lua state; separate engines alone do not make scripts independent.
9. With AOF on, health is checked at the first accepted WRITE under an approved
   live observation contract. Today this acquires Meta then and keeps it;
   full commit-only acquisition cannot simply assume it away. With AOF off,
   health is known from immutable startup configuration and needs no Meta read.

**Source-read / judgment, from premises 1-9 and SHARE-3.** SHARE-3 supplies one
effect point per atomic statement and one total order of shared effects,
including statements that acquire their targets lazily and AOF-off scripts
that never acquire Meta. All target reads and writes of an accepted attempt
take effect at that single point. Conflicting entry observations therefore
fit that order; independent dataset operations can commute. An abandoned
attempt restores its state before completion and contributes no surviving
dataset transition. This reasoning does not derive the dataset order from
the presence of a metadata object.

**Judgment.** With AOF on, Meta's publication order orders log appends, sequence
allocation and capture cuts. Because each append and its entry changes belong
to the same atomic statement, that order agrees with the restriction of
SHARE-3's total order to the logged mutations. A client that reads A's write
then invokes B cannot put B's record before A's: the shared-effect order respects
completed-before-begun effects and the causal read. This covers cross-key
causality, not merely per-key log ordering. Each complete execution-unit prefix
replays a permitted state in that order; MULTI/EXEC framing preserves units at
truncation. AOF-off scripts need no log-order proof or Meta sequence to retain
their atomic dataset transition. No new durability guarantee follows.

### Time and expiry: what the proof does and does not say

**Source-read.** Redis 7.2.16
[`server.c:enterExecutionUnit`](https://github.com/redis/redis/blob/7.2.16/src/server.c)
sets the command snapshot.
[`expire.c:ttlGenericCommand/expireGenericCommand`](https://github.com/redis/redis/blob/7.2.16/src/expire.c)
use it for relative deadlines/TTL and propagate absolute deadlines or deletion.
[`db.c:keyIsExpired/expireIfNeeded/deleteExpiredKeyAndPropagate`](https://github.com/redis/redis/blob/7.2.16/src/db.c)
handle lazy removal. Firn carries Time across attempts and keeps each entry's
absolute deadline; the queue is an index for background work, not the TTL oracle.

**Judgment.** Keep time frozen during an accepted attempt, mutate an expired
entry under its key hold, and stage the DEL/queue effects until commit.
Commit must not recompute a relative EXPIRE using the later wall time. A
deadline can already be past when it is queued; later expiry still validates
the entry's current deadline before deletion. Both deletion and recreation
need correct final stamps/tombstones.

**Judgment, limitation.** Commit order alone does not order clock snapshots.
On a nondecreasing wall clock, disjoint scripts sampled at t1 < t2 may commit
in reverse order and return increasing PTTL values for equal deadlines. Those
two disjoint executions may be reordered to explain the replies, but literal
commit-order timed execution is not thereby proved. A stronger proposed case
uses fallback: an attempt sampled at t1 abandons, another script sampled at
t2 writes b, and the whole retry retains t1, reads b and returns a TTL based
on t1. The data dependency forces the retry after the other script, while its
clock is older. Test this against both baseline and candidate; it may already
be permitted by Firn's pre-lock snapshot/retry contract. Do not report it as a
measured regression or silently change the clock contract to make a proof pass.

### Memory and persistence admission

**Source-read.** Redis
[`script.c:scriptVerifyOOM`](https://github.com/redis/redis/blob/7.2.16/src/script.c)
uses pre-command OOM before WRITE_DIRTY; it is not a live per-command heap test.
Firn's memory-limit decision explicitly permits concurrent in-flight overshoot
and per-read configuration snapshots. Undo, engine and private effects memory
remain real heap; they are not excluded AOF capacity.

**Judgment.** Preserve that admitted contract and do not add inner eviction.
Two scripts can both pass admission below a noeviction limit, then each consume
more than the remaining headroom. Exact serial Redis would reject the second;
Firn's approved concurrent admission need not. Consequently “some serial Redis
execution consistent with every OOM reply” is stronger than the present goal.
Choosing it would reopen the memory decision and could require serial admission
through execution. A late Meta sequence cannot solve it.

**Judgment.** The persistence issue is different: Lua can observe a first-WRITE
MISCONF through pcall and branch on it. With AOF enabled, an error published
after entry but before that WRITE must be handled at the required boundary;
checking only at commit cannot revise an already consumed Lua result. Keeping
the current Meta read is expressible, but serializes the Lua suffix. A dedicated
health observation channel needs a contract for error payload, recovery and
ordering relative to accepted effects; it is not established by the late-lock
answer. After WRITE_DIRTY, retain the existing no-recheck rule, including a
no-effect accepted DEL. Do not turn a late disk failure into rollback.

### Partial errors and undeclared keys

**Judgment.** `redis.call('SET',KEYS[1],'v'); error('stop')` commits SET and
returns an error. `redis.pcall` may handle an error and continue; final effect
order is the order the commands actually produced. Accepted errors and
internal retries must remain distinct even when both unwind the interpreter.

**Source-read / judgment.** The current planner and undo already provide the
required fallback structure (`commands/script.wf:255-432,2144-2168`;
`entry.wf:632-653`). The runtime team's “decide before execution” applies to
each atomic attempt: finding an undeclared operand abandons the narrow attempt
before touching that operand, then selects whole mode before a new statement.
It does not require predicting arbitrary Lua keys before the invocation.
For `INCR KEYS[1]; GET undeclared`, restore the first increment and all staged
metadata before retry; the accepted invocation increments exactly once.
Whole mode stays selected across budget retries. Do not publish dirty-key,
expiry, sequence, random-state or capture deltas from the abandoned attempt.

### AOF rewrite capture

**Judgment; A/B only.** This AOF-on redesign is unnecessary for D, which keeps
the logged path unchanged. A script need not inspect capture state while
interpreting Lua.
Track its changed entries locally; at commit inspect the current capture state
under Meta, construct one fresh token and capture final replacements, including
deletion. Scripts retain image capture, even for one declared key; this proposal
does not adopt the plain single-key effect optimization.

- If S0 precedes the script commit, include it in the active capture window,
  even if Lua began before S0. If commit precedes S0, its effects are already
  included in the starting dataset/AOF boundary.
- If S1 precedes commit, append to the post-S1 incremental stream. If commit
  precedes S1, the sealed log/journal includes it. No observer sees new entries
  with uncommitted stamps: scan steps hold whole keys and Meta.
- Inspect reserve/rotation state only at that final hold. Charge before
  allocating images and their staging copies, including growth overlap;
  initialize LogBudget against that same live state. Capture exhaustion aborts
  the rewrite while normal writes/AOF effects survive.
- A whole-map FLUSH can stage the reset/abort action, since no scan can pass
  its map hold. Preserve queue-reset ordering relative to later EXPIRE calls
  within the same script. Copying only a final expiry list would lose this.

**Judgment, design change.** This replaces per-command image construction with
final-image construction, so reserve peaks and which intermediate growth
aborts a rewrite can change. It requires an owner-approved update to capture
and sequence contracts. A dirty-key representation must retain tombstones,
avoid duplicate captures and bound/account its own storage; calling uncharged
capture scratch “ordinary effects” is not an accounting solution. During Lua,
entry undo is already ordinary heap; any new capture-specific storage must
have an explicit lifetime/charge rule. Final capture can still make the Meta
hold long for large keys. This is a plausible redesign, not a completed proof
of the existing implementation with calls reordered.

## Effect on plain commands

**Source-read / judgment.** [Write-meta](../write-meta/README.md#3-candidate-designs)
already rejects a short global Meta commit as sufficient for high-rate plain
write scaling, and owns the alternative sequence/buffer architecture. Long
Lua computation may nevertheless benefit from moving bookkeeping out of its
hold; this study must measure that rather than reuse a plain-SET conclusion.

SET/MSET and every writer need only preserve premises 5-7 while scripts change.
Option D reuses the already owned AOF-off inert-token behavior and needs no
general sequence/capture redesign. Options A/B require shared command parts
to expose mutation/effect information without depending on an already assigned
sequence; ordinary callers can retain their existing token wrapper. Do not
convert logged callers to inert tokens or defer plain AOF publication past key
release. Coordinate changed token/capture contracts with the plain-command
owner; EXEC uses `held_run` and must remain correct too. No new plain-command
benchmark or duplicate redesign belongs here.

## Whitefoot needs and minimal witnesses

**Source-read.** There is no missing nested-transaction feature for committing
keys and Meta together. The natural form is one statement. The following is a
complete illustrative program, not a compiled result; CI must check its ledger
and behavior at the pinned compiler before it becomes acceptance evidence.

```whitefoot
struct Meta {
  failed: Bool;
  commits: u64;
}

fn put(keys: Shared<ConcurrentHashMap<u64>>, meta: Shared<Meta>, turns: u64) -> result: unit pure waits {
  let name = array_filled::<u8, 1>(value: 97_u8);
  let key = &name[0_u64..1_u64];
  atomic slot = &keys[key], m = &meta {
    let total = 0_u64;
    for (i in 0_u64..turns) {
      set total = total +wrap i;
    }
    set slot^ = Some<u64>(value: total);
    set m^.commits = m^.commits +wrap 1_u64;
  }
  return unit;
}

fn main() -> status: std::process::ExitStatus pure waits {
  let keys = shared_map_new::<u64>(capacity: 8_u64);
  let healthy = False();
  let initial = Meta(failed: healthy, commits: 0_u64);
  let meta = shared_new::<Meta>(value: initial);
  put(keys: move keys, meta: move meta, turns: 1000_u64);
  return std::process::exit_status(code: 0_u8);
}
```

### Live AOF health: an unmet concurrency requirement, not an observed rejection

**Source-read / judgment.** Replace the atomic block above with this natural
fragment to model a live first-WRITE admission and the Lua suffix:

```whitefoot
atomic slot = &keys[key], m = &meta {
  if m^.failed {
    // Real Firn returns the saved MISCONF payload to Lua here.
  } else {
    let total = 0_u64;
    for (i in 0_u64..turns) {
      set total = total +wrap i;
    }
    set slot^ = Some<u64>(value: total);
    set m^.commits = m^.commits +wrap 1_u64;
  }
}
```

The early read is legal but takes Meta for the remaining computation. A
`SharedRead` handle does not promise a short independent observation or release
of that target; a separate health object held throughout still orders all
its users against its writer. The desired independent live observation is a
different semantic effect, not a lock-order spelling change.

**Judgment, proposed request under this board item.** Ask Whitefoot whether a
reusable, independently linearized health snapshot with an owned error payload
can coexist with the enclosing atomic dataset transition, including successful
recovery and publication after the first accepted WRITE. Minimal trace:
healthy -> Lua read-only prefix -> writer publishes failed(error) -> first
WRITE observes failure -> pcall handles that error -> writer recovers -> a
later eligible first WRITE observes healthy. Establish which observations may
change inside one accepted script and how each orders with the file writer;
do not generalize DecisionCell by assumption.

Acceptance for that request: disjoint script computations overlap; a failure
whose publication completes before the observation begins, with no intervening
recovery, yields the correct MISCONF payload. Overlapping publication/read
races admit only outcomes consistent with the selected independent effect
order. A later recovery can be observed; an accepted WRITE prevents inappropriate
later refusal; error/cell state cannot tear; no partial dataset is observable.
Test failure and recovery races with pcall and plain commands in CI. There is
no compiler output or diagnostic from this investigation. This is a semantic
request candidate, not a confirmed new missing API. It is needed only for an
option that keeps AOF-on scripts Meta-free past first WRITE, such as B; A and D
retain the existing AOF-on first-WRITE check.

**Source-read.** This tempting fragment is forbidden by SHARE-2, rather than a
solution (illustrative fragment; no diagnostic was run):

```whitefoot
atomic slot = &keys[key] {
  let failed = False();
  atomic m = &meta {
    set failed = m^.failed;
  }
  if bnot(failed) {
    set slot^ = Some<u64>(value: 1_u64);
  }
}
```

**Judgment.** RANDOMKEY's live global draw is the other **(b)** execution use,
but has an expressible whole-map serial path and no declared-key concurrency
benefit to unlock. It supplies no separate language request. Final capture,
expiry staging and local OOM input likewise supply no demonstrated new gap.
Strict serial OOM/time semantics are application-contract questions, not
compiler limitations to conceal.

### Fairness and previously filed needs

**Source-read.** At the pin,
[`wf__shared_take`](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L4037-L4073)
retries acquisition while retaining keyed holds; this loop supplies no FIFO
reservation. The runtime team's reported 2026-10-11 answer promises a fairness
fix; neither its landing nor its performance is verified here.

**Judgment.** The first complete program is the minimal late-acquisition shape
for that issue. CI must add competing disjoint `put` contexts and a context
repeatedly acquiring Meta alone; record each waiter's overtakes and completion
after a finite flood, then test sustained contention against the runtime team's
stated progress contract. Require zero lost updates and zero stranded waiters
after the finite flood, plus the team's declared fairness bound if one exists.
A finite stress run does not prove starvation freedom. File/reuse the team's
existing fairness work rather than invent a resolved status or another request.

**Source-read.** The old script-engine record's language blockers need current
interpretation. The pin now exposes `Inputs.drivers`
([process interface](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/process/module.wfm#L29-L39));
Firn binds it as unused at `server/server.wf:993`. The pin also provides
[`DecisionCell`, `claim_written`, `claim_killed`](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2453-L2466),
with independent effect points inside atomic blocks under SHARE-3. The former
`firn-req-driver-count` and `firn-req-attempt-publish` needs therefore have
language facilities now; Firn integration and multi-engine control policy
remain to be verified. DecisionCell's monotone Open/Written/Killed verdict has
no reusable error payload/recovery operation, so it does not answer the AOF
health question. Redis [`script.c:scriptKill`](https://github.com/redis/redis/blob/7.2.16/src/script.c)
rejects killing a dirty script; current Firn's pool-based KILL still does not
publish that accepted-WRITE state during execution.

**Source-read / judgment.** Per the kit's downstream gap rules, Whitefoot owns
any language/runtime fix; Firn keeps semantic witnesses and verifies the pinned
fix in CI. No workaround, native shim or silent contract weakening is proposed.

## Owner options

**Source-read.** AOF-off service cannot start a rewrite
(`firn/commands/aof-rewrite.wf:14-18`), and CONFIG SET cannot change whether
AOF is enabled (`commands/server.wf:862,930-938`). Plain commands already use
`sequence_inert()` on the unlogged path (`commands/keys.wf:373-390`). This
configuration therefore needs no new commit sequence, log budget or capture
representation for script execution.

**Judgment.** D still needs to remove `m` and `&m^.due` from unlogged ordinary
command dispatch and stage expiry/reset work in whole mode as well as narrow
mode. It also needs the following independent mutation detector;
neither an inert sequence nor effect counts can supply the detector. These
changes do not require the AOF-on final-image capture redesign.

### D's mutation detector and preserved continuation behavior

**Source-read; before D.** Today's serving scripts take a real sequence even
with AOF off: both attempt branches call `sequence_new(meta: m, loading:
time^.loading)` without testing logging (`entry.wf:504,571`), and serving
EVAL has loading false. `sequence_change` therefore sets `taken` on a mutation.
The call wrappers also set their internal `logged` argument to True
(`entry.wf:297,333`), independently of `store.logging`; only final file emission
is gated by the latter. Thus today's AOF-off scripts can produce private
records as well as a real sequence. This differs from the plain AOF-off inert
path. It does not justify relying on those records in D's proposed unlogged
execution path.

**Source-read; failure of the earlier D proposal.** `complete_command_call`
sets `held.wrote` only from `sequence.taken` or `appended > 0`
(`entry.wf:350-356`). An inert token never sets taken
(`store/sequence.wf:43-48`), and with the helper's `logged` argument false,
`mset_lapse_meta` and `mset_meta` return zero (`commands/strings.wf:871-887`),
as do other disabled effect encoders. D cannot disable encoding and use an
inert token while retaining this detector: a real write would appear unwritten.
Narrow budget retries would undo/replay a write; whole budget retries would
repeat it without payload undo. A retry gap would also admit a kill, and the
written-attempt budget-extension branches would not fire.

**Judgment; after D.** Add a local `changed: Bool` to the attempt's existing
WriteSequence token, initialized false by both real and inert constructors.
Set it in `sequence_change` **before** its inert early return. The token remains
inert for numbering, stamps and capture; `changed` is independent of those
fields and of whether any bytes were appended. This is one local mutation
marker, not a new shared counter or a replacement meaning for `taken`.

All handlers must mark the same mutation boundary they currently report via
sequence operations: string/hash/list/set/zset and multi-key writes reach
`sequence_stamp`/`sequence_entry`; actual removals reach `sequence_delete`;
script lazy expiry reaches `sequence_delete_key` (`script.wf:483-484`). These
converge on `sequence_change` (`sequence.wf:78-105,134-143`). Whole FLUSH calls
it directly (`commands/server.wf:166-172`), including an empty map, preserving
today's conservative no-restart marker for that operation. Replacement helpers
must still be followed by their existing stamp/capture mutation hook; audit
every accepted handler and early return against this coverage. Failed/no-effect
commands must not infer mutation from WRITE flags or reply success alone.

In `complete_command_call`, fold `sequence.changed` into `env.held.wrote`
before `fail_call`/`complete_call` resumes Lua. Keep `env.appended` only as the
AOF record count; it is no longer the source of truth for retry safety. Audit
the continuation/abandonment sites that rely on written-state detection:

| Consumer | Required D behavior |
|---|---|
| Whole Budget branch, `entry.wf:510-520` | If changed, resume the same attempt with unlimited budget, as today; never restart it, since whole mode has no payload undo. |
| Narrow Budget branch, `entry.wf:579-595` | If changed, resume the same attempt with unlimited budget, as today; do not use undo to permit write-bearing budget retries. |
| Whole/narrow HostCall continuation budgets, `entry.wf:526,602` | Grant unlimited continuation after held.wrote is set by any mutation, including lazy expiry. Do not add an accepted-WRITE condition. A first-write continuation that immediately returns Budget must already see changed. |
| Undeclared-key fallback, `entry.wf:632-653` | This remains the explicitly permitted written-attempt abandonment: restore narrow payload/access state, discard due/effects and reset local changed only for the next attempt. Done and Lua Error retain mutations. |

**Source-read; current KILL publication.** There is none for `held.wrote`.
SCRIPT KILL checks only `pool.running`, increments `pool.kills`, and answers
OK/NOTBUSY (`entry.wf:732-745`). `kill_requested` compares that epoch
(`entry.wf:216-222`), and invocation entry checks it between attempts
(`entry.wf:449-450`). The local wrote flag prevents write-bearing budget gaps
today; it is not read by KILL and cannot make KILL answer UNKILLABLE. The script
design's separate KILL decision explicitly records that existing discrepancy.

**Source-read / judgment; D's unchanged KILL contract.** The approved
[KILL decision, line 21](../../../design/firn/scripts.md) keeps this behavior
until `firn-req-attempt-publish` is integrated. D preserves it using changed:

| Case | Behavior preserved by D |
|---|---|
| Real dataset write | Set held.wrote and continue without a budget-retry gap. A KILL processed while the script is active answers OK, not UNKILLABLE; the script runs to completion as today. |
| Accepted DEL of a missing key | No mutation means changed remains false. The attempt may budget-retry and observe a kill at the gap, as today, although Redis treats the accepted WRITE as unkillable. |
| Lazy expiry during a read-only script | The actual deletion sets changed and held.wrote. Budget and HostCall continuations become unlimited, as today; D introduces neither finite chunks nor new cancellation checkpoints. |
| Reads without mutation | Budget retries restore access state and expose the existing kill-epoch check, preserving today's killability. |

**Judgment; D's detector cost.** Mutation detection adds one Boolean per attempt,
constructor and contract updates, an idempotent local store at each existing mutation hook,
and one local read/fold per completed redis.call/pcall. It adds no Meta acquisition,
allocation or log encoding. D's other work remains removing unlogged Meta
forwarding and staging expiry/reset operations in both modes. No DecisionCell,
accepted-WRITE publication, UNKILLABLE fix or new control-progress mechanism is
part of D's cost or correctness condition.

### Separate optional KILL work

**Source-read / judgment.** Accepted-WRITE publication and UNKILLABLE remain
owned by the existing [KILL decision, line 21](../../../design/firn/scripts.md)
and `firn-req-attempt-publish`; they are not a dependency of D. The pin's
DecisionCell facility may support publication, but integration is still separate
work. That work must distinguish actual mutation from accepted WRITE, including
`DEL missing`, and specify invocation lifetime, fallback, admission/kill races
and cancellation outcomes. Pooled all-or-none KILL coordination is another
part of that control design: claiming individual cells in a loop does not
establish an atomic multi-invocation policy.

**Judgment; optional continuation proposal.** Making a read-only script remain
killable after lazy-expiry mutation would replace today's unlimited continuation
with finite, non-restarting chunks and independent cancellation checks. This
changes the continuation behavior in script decisions at lines 11 and 21,
and needs separate owner approval under the KILL work. It would also need a
verified one-driver progress mechanism while retaining key isolation, plus a
dedicated killed outcome preserving the completed lazy-expiry prefix. D makes
neither that change nor the related proposal to condition unlimited HostCall
budget on mutation **and** accepted WRITE. D keeps unlimited budget after any
mutation marker, exactly as the present held.wrote branches do.

**Source-read.** The deployment-performance workload inventory
(`research/investigations/deployment-performance/README.md:192-205`) includes
one measured Lua workload, the immediate-WRITE limiter, plus rate-limiter and
Django transactions and session SET/GET. Its client loads exactly that limiter
source (`redis-bench/workload/src/main.rs:301-304`). None of these inspected
workloads has a long Lua read prefix before first WRITE. Other recorded consumer
test scripts have not all been retrieved; this is a conclusion about the
selected deployment workloads, not every possible application.

**Judgment.** General AOF-on sequence/capture redesign while retaining the
first-WRITE hold (A) would need an application with substantial Lua/read work
before that WRITE to justify its cost. No selected workload currently supplies
that reason. A synthetic prefix may verify the mechanism but cannot substitute
for application benefit. The choice is therefore between a small AOF-off study,
resolving the stronger AOF-on contract, and waiting for pooled evidence.

| Option | Deliverable / what it unblocks | Cost and risk |
|---|---|---|
| A — General staged metadata, retain live checks | Refactor expiry/effects/sequence/capture publication after pooling, retaining Meta from first WRITE with AOF on. Frees a substantial pre-WRITE prefix if a workload has one. | Moderate interface/design work, new mutation tracking and final-image/reserve validation. Expected AOF-on limiter gain is about zero; no selected deployment workload currently justifies this redesign solely for prefix concurrency. Not recommended now. |
| B — Resolve the full observation contract first | Whitefoot/Firn joint design for live persistence status, plus an explicit ruling on the timed-history claim and accepted memory exception, before implementing commit-only AOF-on execution. | Larger, uncertain language/semantic scope; strongest concurrency target, but no existing API/diagnostic proves its final shape. May uncover a fundamental shared dependency rather than an optimization. |
| C — Keep full-attempt Meta and measure only after pooling | A clean pooled baseline with no token/capture refactor yet. | Lowest immediate change cost; retains script serialization. Prefer if D's end-to-end throughput comparison rejects its benefit. |
| **D — AOF-off execution with independent mutation detection; recommended** | Use an inert token, no outer AOF log budget, known health and local expiry staging in both modes. Add changed at mutation hooks before the inert return; preserve all current held.wrote continuation and KILL behavior. Acquire Meta at accepted expiry/reset publication only when needed; retain whole RANDOMKEY's draw. Keep AOF-on execution unchanged. | Bounded script/token-interface changes, one local Boolean/store/read for detection, and budget/fallback/expiry regression cases. No accepted-WRITE publication, UNKILLABLE fix, finite-chunk continuation, pooled KILL redesign, AOF-on capture redesign or new health API. |

**Judgment; confidence 4/5 in D as the next bounded step, not in its speedup.**
The immutable AOF-off configuration eliminates the health/capture dependencies,
and existing inert-token semantics supply the sequence behavior. The limiter's
immediate WRITE means A offers no supported AOF-on application advantage over
D while requiring more design work. B remains the route to the full AOF-on
target. D's detector must reproduce current written-state and continuation
behavior; the already deferred KILL fix and the full AOF-health request do not
block D. If D fails its throughput
criterion, prefer C and reassess the measured costs. D does not complete the
AOF-on portion of `firn-script-meta-free`.

### Design decisions retained or replaced by each option

**Source-read for existing commitments; judgment for proposed dispositions.**
The [script node](../../../design/firn/scripts.md) contains three relevant
decisions: line 7 combines declared entries with full-attempt Meta and specifies
undeclared-key abandonment/whole retry; line 11 keeps Meta throughout the
correctness step and specifies budget retry behavior; line 29 requires held-Meta
first-WRITE admission and grounds its live observation in the full-attempt hold.

| Option | Declared-key/Meta decision (line 7) | Interpreter-hold decision (line 11) | First-WRITE health decision (line 29) |
|---|---|---|---|
| A | Replace the full-attempt Meta clause with acquisition at necessary first use; immutable declarations, abandonment and sticky whole retry still stand. | Replace full-interpreter Meta lifetime; budget/actual-effect retry rules still stand. | First-WRITE boundary, MISCONF call/pcall behavior and WRITE_DIRTY bypass still stand. The health equivalence stands on different grounds: live acquisition at first WRITE, then the retained hold; replace the full-attempt ground. |
| B | Replace Meta lifetime with commit-only acquisition; declaration/fallback rules still stand. | Replace full-interpreter Meta lifetime; retain budget/actual-effect retry rules. | Replace “checks held Meta” and “needs no separate changing-status channel.” Retain observable first-WRITE/error/bypass behavior, justified by the newly approved independent observation contract. |
| C | Keep the entire decision. | Keep the entire decision. | Keep the entire decision and its ground. |
| D | Narrow the full-attempt Meta clause to AOF-on execution; declaration/fallback rules still stand in both modes. Needs owner approval. | Narrow the full-interpreter Meta hold to AOF on; expiry/reset publication remains in the key statement. With changed replacing the old mutation signals, the budget rule stands unchanged: restart only without mutation, grant unlimited continuation after any mutation, and permit written rollback only for undeclared-key fallback. Meta lifetime narrowing needs owner approval; no new continuation policy is proposed. | Narrow/replace “checks held Meta before first accepted WRITE” for AOF off, with immutable known health as its new ground; needs owner approval. Keep the AOF-on check and its full-attempt ground. |

**Judgment.** A/B also replace the sequence node's token-initialization/stamping
contract and the capture node's per-command coalescing/reservation contract as
described above. D keeps their AOF-on decisions and reuses the existing
AOF-off inert/zero-stamp decision; C keeps both nodes unchanged. All options
retain frozen time, error-prefix writes, memory admission and expiry semantics.
D extends the token interface with local mutation detection, without changing
its inert numbering/capture meaning. D keeps the existing KILL decision at
line 21, including OK for an active written script and the between-attempt
kill check. Accepted-WRITE publication and any changed continuation/cancellation
policy would replace that decision only in the separate optional KILL work.
These dispositions are proposals; this investigation edits no design tree.

## Measurement and correctness plan — do not run locally

**Judgment; preregistered plan.** The owner runs CI. Begin only after the pool
lands and its shared-library state/control semantics are known. D does not
require the deferred KILL integration in either comparison arm; compare it
against the pooled baseline with the same existing approved KILL behavior.
Record exact
Firn, compiler, kit and Halo revisions, gate results, workflow/run ID, Linux,
CPU placement, driver count, client version and AOF policy. Use
`redis-bench.yml` on an idle, coordinated `14900k`, `make firn-lto` builds,
physical P-cores for server and separate E-cores for clients. Redis tools and
oracle are the pinned Redis 7.2.16 archive build, not a package image.

### Counters and attribution

Instrument all arms identically, and validate that instrumentation does not
move throughput/p99 beyond the baseline/twin noise. Use per-context counters
or runtime tracing gathered outside measured holds; one new global statistics
lock would manufacture the bottleneck being measured. Required observations:

- Invocation count, accepted attempts, budget retries, whole-map fallbacks,
  engine checkout wait, key acquisition wait and interpreter CPU/wall time.
- Meta acquisition wait and hold duration, split by pre-admission, attempt
  setup, first-WRITE health, final publication, maintenance and post-peak.
  Record elapsed Lua time before/after first WRITE, not only AOF effect count.
- Commit count, effects bytes/records, queued expiry count, dirty keys, undo
  bytes, image-copy bytes/time, capture/staging/rotation charge and abort reason.
- Retry/overtake count and longest individual Meta wait; throughput, p50/p99,
  CPU, heap peak, writer backlog, engine utilization and capture completion.

Define Meta queue share as **sum of attempt Meta-wait durations divided by
sum of in-server invocation residence durations**, reporting entry/admission
waiting separately and including abandoned attempts in their invocation.
Also report hold share of each attempt and raw totals/counts. Do not divide
summed multi-context waits by wall-clock run duration and call it a percentage
of CPU. Existing external entry/exit timing cannot separate key wait from late
Meta spin: use runtime acquisition attribution if no public clock-safe source
probe can bracket it. Instrumentation needs CI validation; it is not already
available by virtue of this plan.

### Workload cells and comparison

Use three arms in each selected comparison: `baseline`, the pooled source with
existing full-attempt Meta; `twin`, an independent build of that same source;
and the explicitly named option candidate, all with the same pin. These arm
names are distinct from owner-option letters. If a runtime fix is required,
first compare same-source baseline/twin on that runtime; do not attribute the
compiler change to Firn. Interleave baseline,twin,candidate and
candidate,twin,baseline per round, rotating the first arm between rounds.

Primary workload: the existing `limiter-script` application script, random
100,000-key domain, 8 and 50 connections, pipeline depth 1, 1 and 2 physical
cores. Preserve its script source and business logic. Add diagnostic ordinary
EVALSHA workloads with preloaded sources: per-client disjoint keys; one hot key
as a negative control; identical finite Lua computation before versus after
first WRITE; expiry on every write; error after a write; and an undeclared-key
fallback. These are workload inputs, never server-selected code paths.

The primary limiter's first call is the SET WRITE cited above. Select candidates
and expectations per persistence cell, before running it:

| AOF cell | Candidate actually compared with baseline/twin | Registered limiter prediction |
|---|---|---|
| Off; recommended next experiment | **D (`off-only`)** | Two-core throughput gain greater than max(10%, N); one-core throughput and p99 regression no greater than N. Meta remains needed only for expiry publication in this script. |
| On, no rewrite; D control | **D (`off-only`)**, whose logged execution and approved KILL behavior are unchanged | About zero gain, within N. A reproducible difference outside N requires explaining an unintended logged-path change or another cause. |
| On, rewrite active; D control | **D (`off-only`)**, whose logged capture is unchanged | About zero gain, within N; validate unchanged capture/recovery, not a rewrite scaling claim. |
| Off, only if A is separately selected | **A (`staged`)** | Same throughput criterion as D; no extra application benefit from the capture redesign is predicted for this cell. |
| On, with/without rewrite, only if A is selected | **A (`staged`)**, retaining first-WRITE Meta | About zero concurrency gain for the immediate-WRITE limiter. A long synthetic read prefix verifies mechanism only. Any rewrite copying-cost change must be attributed separately and does not establish prefix benefit for this application. |
| On, no rewrite, only after B's contract is resolved | **B (`commit-only`)**, with approved live health observation | Two-core throughput gain greater than max(10%, N), without regressions above N; the stronger AOF-on prediction applies to this candidate alone. |
| On, rewrite active, only after B's contract is resolved | **B (`commit-only`)**, with final-image capture | Same proposed gain/regression criteria; retain scan/capture progress and abort evidence because capture may falsify the performance premise. |

AOF on must use the normal recorded fsync policy and a real file. Existing
workload-image mode must be extended and verified if it cannot enable AOF;
never label its AOF-off result AOF-on. There is no AOF-off rewrite cell: the
command is refused. For rewrite-active cells use the same dataset/script and
document scan progress and complete/aborted capture. Use a fixed finite memory
limit well above the primary workload's use; add near-limit correctness cells separately.
After the 1/2-core question is resolved, 4/8-core cells can locate the remaining
global commit ceiling, without changing the primary acceptance criterion.

Start with the smallest useful hosted correctness witness, recording its
duration. On the 14900K, first sample one 5-second run per arm in the primary
two-core cell, then a reversed round to expose spread. Choose only inconclusive
cells for longer runs, provisionally three interleaved 10-second rounds. Do not
start a long matrix before inspecting duration/spread. For each cell define
noise N as the maximum of 5%, the absolute baseline/twin median ratio deviation and the
largest arm's `(maximum - minimum) / median` across interleaved rounds.
For cells predicting a gain, require throughput improvement greater than
max(10%, N), with one-core throughput loss/p99 increase at most N. Throughput
and regression checks are the rejecting criteria; Meta wait/hold share explains
the result and has no 20% acceptance gate. An unchanged-path control or A's
AOF-on limiter result near baseline meets its no-gain prediction and cannot
be reported as successful AOF-on Meta-free execution.

### Semantic acceptance and falsifiers

Register these in the eventual implementation's CI caller and demonstrate
that representative wrong publication/rollback results fail the checks. Use
Redis source/replies/files as the oracle, subject to the explicit existing
Firn memory/configuration differences; do not regenerate expected outputs
from the candidate. Run the project gate and AOF sync-failure job on the final
revision when code changes, not a local suite for this prose investigation.

1. Disjoint and overlapping scripts plus plain writes/read-then-write clients:
   zero mismatched replies/effects, no partial visibility, correct replay of
   every tested complete AOF prefix and truncated MULTI/EXEC boundary.
2. A SET followed by Lua error, pcall recovery, and no-effect accepted DEL:
   preceding writes persist; WRITE_DIRTY and mutation count remain distinct.
3. Relative/absolute deadlines, TTL around expiry, deletion/recreation and
   retries: zero unexpected values/deadlines; include the timed-history
   witness above and report baseline/candidate separately.
4. Noeviction/evicting policies with accepted pre-command OOM, concurrent
   admission, engine/undo allocations and expiry eligibility: preserve the
   approved contract and expose its difference from strict serial Redis.
5. Published append/sync failure before first WRITE, after first accepted WRITE,
   and recovery, for call and pcall: exact refusal boundary and error payload;
   no rollback of accepted effects. A snapshot mutant must fail this case.
6. Every value family mutated then undeclared read/write/whole-map fallback:
   one accepted mutation, exact restored expiry/access stamps, zero abandoned
   sequence/due/AOF/capture publication; budget/fallback composition included.
7. S0 and S1 on both sides of commit, changed/deleted/recreated keys, whole
   FLUSH, large keys, reserve exhaustion and rotation backlog: exact recovered
   dataset and absolute expiries, zero uncharged capture allocations, no
   successful installation after capture failure. A stale token mutant must
   fail; no sequence wraps at exhaustion.
8. Pool generation/FLUSH across retry gaps: no stale-source execution or lost
   running invocation. D uses the unchanged-KILL regression cases below.
   Accepted-WRITE publication, UNKILLABLE and new cancellation/pooled arbitration
   cases belong to the separate optional KILL work, not D's gate.
9. Late-lock fairness stress: zero stranded waiters after a finite flood;
   report maximum overtakes and the runtime contract tested. Throughput gains
   cannot excuse starvation or an unbounded key-hold regression.

### D's required AOF-off detector and unchanged-KILL cases

**Judgment; unrun acceptance cases.** Disable AOF and use the candidate's
unlogged helper path; verify that effect counts remain zero so a passing case
cannot conceal dependence on internal RESP encoding. Retain diagnostic traces
of mutation, Budget, retry and continuation for all arms. A correct final value
alone cannot detect a narrow write that was undone and replayed.

| Case | Check that decides completion |
|---|---|
| Narrow write then Budget | Declare the counter, INCR it, then execute enough finite Lua work to exhaust the initial budget. Observe Budget after mutation, zero write-bearing budget restarts, continuation of that same attempt and exactly one increment. A mutant omitting changed must fail the restart trace even if undo/replay preserves the final value. |
| Whole-mode write then Budget | Use an undeclared operand to enter sticky whole mode before its INCR, then exhaust the initial budget with finite Lua work. Observe zero write-bearing budget restarts and exactly one increment; the old whole branch has no payload undo, so the missing-detector mutant must expose repeated writes. |
| SCRIPT KILL after real WRITE | Run a finite long script and establish from mutation tracing that held.wrote was set before a KILL processed while it remained active. Require today's OK reply, no write-bearing budget retry/killed exit and exactly one visible write; do not change the expectation to UNKILLABLE. Use independently scheduled contexts so the request is processed during execution. |
| Read-only script without mutation | Use pure Lua or reads of unexpired entries and request KILL at a budget gap. Require today's OK followed by the script-killed reply and zero dataset changes. |
| Read-only script after lazy expiry | Delete an expired key through a read, then continue finite long read-only work. Require changed/held.wrote, unlimited same-attempt continuation and zero budget restart, as today. A KILL processed while it remains active answers OK without introducing a new cancellation checkpoint. Retain one deletion; counter parity remains owned by `firn-script-expired-count`. |
| No-effect WRITE and fallback composition | DEL a missing key must leave changed/held.wrote false, allowing today's budget retry and kill-epoch check; a KILL at that gap answers OK and can stop the invocation. Also test real write followed by undeclared-key fallback: restore narrow state, start a fresh detector and use the existing kill-epoch behavior at the retry gap, with zero abandoned metadata publication. No persistent Written verdict is added. |

Cover lazy-expiry deletion, FLUSH, each value family, no-effect/failed commands,
and accepted Lua errors in the mutation-hook audit. Verify the 526/602 HostCall
budget choices as well as the 510/579 Budget branches. Compare these outcomes
with the approved baseline, retaining the documented Redis KILL discrepancy.
The optional KILL work separately owns claim-order, UNKILLABLE, finite-chunk,
one-driver progress and pooled-coordination acceptance cases.

## Follow-ups for filing

**Source-read / judgment.** The responsible session files the conditional
follow-ups when an option needing them is chosen; the expiry defect already
has its board item:

| Follow-up | What to file and when it is needed |
|---|---|
| AOF-health observation, to Whitefoot | The session files the minimal live-read/recovery trace and acceptance cases when selecting an option such as B that keeps AOF-on execution Meta-free past first WRITE. A and D need no new health channel. Preserve the first-WRITE boundary and error payload; request a semantic answer before choosing an API. |
| Late-acquisition fairness, already known to Whitefoot | When choosing D or another late-acquisition option, the session links this study and its witness/CI acceptance to the existing runtime work and asks for the fixing revision/progress guarantee. Reuse that work rather than file a duplicate. D's final expiry acquisition and A/B's late commits need verification against it. |
| Script lazy-expiry statistics, already filed as `firn-script-expired-count` | A script GET can remove an expired key while INFO `expired_keys` fails to rise, unlike plain GET. Plain GET calls `remove_expired` (`firn/commands/strings.wf:214`), which increments the counter (`store/store.wf:190`); script GET deletes/logs locally (`commands/script.wf:483-489`, shared `commands/keys.wf:249-261`) without counting it. Redis 7.2.16's [`db.c:expireIfNeeded` and `deleteExpiredKeyAndPropagate`](https://github.com/redis/redis/blob/7.2.16/src/db.c) count lazy expiry through `server.stat_expiredkeys`, including lookups from scripts. The existing item owns the INFO/oracle confirmation and accepted-attempt delta fix; no duplicate filing or server fix belongs in this document revision. |

## Remaining uncertainty

**Source-read / judgment.** Capture finalization and mutation-token separation
are Firn design work, not proven language gaps. Plain-command publication stays
with `firn-write-meta-free`. The earlier driver-count/KILL needs have current-pin
facilities; their integration evidence remains with the engine/control work.

Still unverified for this study: the remote main head, compiled witness/lock ledger,
pool/canonical Lua-state integration, fairness-fix revision, the reusable
health API/design, bounded dirty-key/capture representation, timed-history
counterexample execution, all numerical predictions and all CI acceptance.
The conditional ordering argument is not a claim of unrestricted serial Redis
reply equivalence. The owner can choose the bounded option with these limits
visible; completing the full Meta-free target still requires resolving them.
Accepted-WRITE publication, enhanced read-only cancellation progress and pooled
KILL coordination remain separate deferred control work; they do not block D.

## Measurement plan for the approved option D, under the owner-wide verdict standard

**Judgment; registered before measurement, owner-approved 2026-10-14 on board
card `firn-q-script-meta-125`; not yet run.** This section replaces the noise
rule N of "Workload cells and comparison" above (the maximum of 5%, the
base-to-twin deviation and the round spread) with the owner-wide standard: a
cell's noise is the half-width of the twin-to-base ratio's 95% confidence
interval, and the candidate-to-base ratio's 95% interval decides. The arms,
counters and semantic falsifiers above stay. The run waits for a compiler pin
that contains Whitefoot#388 (a statement that holds keys and then takes a
shared object, as the head's script does for Meta at its end, can be barged
past only a bounded number of times), which item `firn-pin-wf-main` moves on
main; the 14900K run is then queued by the resource manager. No workflow is
pushed with this plan.

**Arms.** `base` is `origin/main` at the revision the run names (the pooled
source with the full-attempt Meta hold), `head` is the head of Firn-wf#125
that merges, after main carries that pin, and `base-twin` is a second image of `base`
(`redis-bench.yml`, mode `workloads` with `workload_images`, or mode `compare`
with `revisions: base=<main sha> head=<#125 sha> base-twin=<main sha>`). Same
pin, `make firn-lto`, Redis 7.2.16 tools, idle coordinated 14900K, server on
physical P-cores, clients on separate E-cores.

**Cells.** The class is the rate-limiting script, `limiter-script`, random
100,000-key domain, pipeline depth 1, AOF off (option D changes only this
path):

| Cell | Connections | Server cores | Question | Prediction (prior) |
|---|---|---|---|---|
| L1 | 8 | 1 | does D cost one core anything | interval contains 1, or faster |
| L2 | 8 | 2 | primary: do two cores scale | head/base interval entirely above 1.10 |
| L3 | 50 | 2 | same at higher concurrency | head/base interval entirely above 1.10 |
| L4 | 50 | 4 | where the next global ceiling is (exploratory) | none; measured or reported unresolved |
| C1 | 8 | 2 | control: AOF on, no rewrite, D's logged path unchanged | interval contains 1 |
| N1 | 8 | 2 | negative control: one hot key | interval contains 1 |

The 1.10 in the L2 and L3 rows predicts the size of the gain; the verdict
below is taken against 1, as for every cell. C1 and N1 can only falsify: a
head/base interval entirely on one side of 1
there means the logged path changed or another cause moved throughput, and
the cell is explained before any claim. The diagnostic EVALSHA cells of the
plan above (disjoint keys per client, expiry on every write, error after a
write, undeclared-key fallback) are a second round, chosen only if L2 is
inconclusive or a control fails.

**Pairs and noise.** Each cell runs `base`, `base-twin` and `head` in
interleaved rounds, rotating the first arm, 10 seconds per run. Noise is the
half-width of the 95% CI of the twin-to-base throughput ratio over the cell's
pairs. The floor is the platform's noise for the workload class
`limiter-script`, measured once (a `base` against `base-twin` run on L2 with
enough pairs to bound the interval), recorded with the 14900K queue and
reused until the machine, its settings or the toolchain change. No floor for
this class is recorded in the repository yet, so the floor run is the first
batch of pairs. A cell noisier than the floor is rerun or reported
unresolved; a cell adds pairs only until its interval decides, never as a
study of its own.

**Verdict per cell.** The head-to-base ratio's 95% interval decides: entirely
below 1 is a regression of any size (accepting it takes a decision card with
its size and reason), entirely above 1 an improvement, across 1 no detected
change. The proposal is rejected if L2 and L3 both show no detected change or
a regression, because then the full-attempt Meta hold was not what blocked
scaling and the wait moved elsewhere; it is also rejected if L1 or a control
shows a regression. Each cell reports pairs, noise, floor, interval and
verdict; the Meta wait and hold counters above only explain the result.

**Not part of this plan.** The Meta wait-share instrumentation above needs
its own CI validation and is a separate item if L2 is inconclusive.
