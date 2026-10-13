# Script execution without a full-attempt Meta hold

Board item: `firn-script-meta-free`, step 2 of the owner's two-step study.
This is research, not an implementation or an approved replacement design.

**Source-read.** Baseline: the worktree's `origin/main` and HEAD both resolve to
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

Compare the same engine-pool source with its existing early Meta use against a
candidate whose Lua path takes no Meta until a semantically necessary read or
final publication. Use the same compiler, Halo, workload, settings and machine,
plus an independently built twin of the baseline. Distinguish AOF off, AOF on,
and AOF rewrite; an AOF-off gain does not answer the AOF-on question.

Prediction: with disjoint keys on two physical cores, the pooled baseline
spends at least 20% of summed in-server script residence time waiting for Meta;
a fully commit-only candidate halves that fraction, improves throughput
by at least 10% beyond the twin/spread threshold defined below, and regresses
neither one-core throughput nor p99 by more than that threshold. These are
proposed decision thresholds, not estimates derived from existing measurements.

Reject the performance premise if the Meta fraction is below 20%, if a
repeatable throughput gain fails the 10% threshold outside noise, or if the
queue is simply replaced by engine, key, allocator or commit waiting without
the required end-to-end gain. A noisy result is inconclusive. Reject the
unqualified correctness proposal on any reply, accepted-effect, recovery or
capture counterexample, or if it requires weakening an existing contract.
The source study already rejects the claim that moving two setup calls alone
makes every script's execution Meta-free: first-WRITE disk admission remains.

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
and [writing guide](../../../whitefoot-kit/docs/writing.md#how-do-i-lock-an-object-only-on-paths-that-use-it)
support the runtime team's reported answer. One keys-plus-Meta statement is
expressible. Key-set targets acquire early; the later Meta type group can
acquire at its first use and remains held to statement end. Helper footprints,
a later type group or a target-free guard can force acquisition early. Header
order is not lock order. Nested atomic statements and waiting calls inside
the block are forbidden. The actual refactor needs the CI lock ledger;
source acceptance alone does not prove acquisition timing.

**Source-read.** More specifically, the pinned compiler's
[`expression_bindings` and `take_units_for`](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L947-L970)
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
| First accepted WRITE admission `commands/script.wf:2069-2084`; `commands/admission.wf:35-44`; `store/store.wf:553` onward | Reads `aof_append_error`, `aof_sync_error`, `aof_append_partial`, including error selection. Later accepted WRITEs bypass it via local WRITE_DIRTY. | **(b)** when AOF is enabled. It changes a call exception/pcall result before Lua continues. AOF off has no writer and can use the known healthy condition without Meta; this is an explicit configuration fact, not a stale snapshot. |
| OOM command check `commands/script.wf:2076-2084`; `entry.wf:439,492` | Local `pre_command_oom` and WRITE_DIRTY only. | **(c)** preserve the original captured OOM state across retries; no per-redis.call memory measurement or eviction. |
| All mutation helpers `store/sequence.wf:28-99,126-190`; `store/rewrite-capture.wf:59-167` | Entry stamps and local WriteSequence fields/images; their initial values currently came from Meta. | **(a)** track actual mutations independently of the eventual sequence. Assign final stamps/capture at commit. Do not use an inert token as the mutation detector. |
| Reply conversion `entry.wf:349-376,539,548-550,615,624-626`; `scripting/replies.wf` | Engine/Env/client bytes and sequence.taken; no Meta. | **(c)** conversion needs none; keep replies private until publication. Replace sequence.taken's role in `held.wrote` if sequence allocation is deferred. |
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
| ZADD, ZINCRBY, ZREM, ZPOPMIN/MAX, ZCARD, ZSCORE, ZMSCORE, ZRANK/ZREVRANK, ZRANGE and reverse/score/lex variants, ZCOUNT, ZLEXCOUNT, ZREMRANGEBYRANK/BYSCORE/BYLEX | `3674-4018` | **(a)** effects/stamps; scores, members and reply errors are entry/local calculations. No direct Meta field. |
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
| SCRIPT FLUSH/KILL/LOAD/EXISTS: `scripting/entry.wf:695-813`, take/return/ends/kill helpers above | Pool.running/generation/registry/kills/out, not Meta. | **(c)** for Meta inventory. FLUSH waits across all attempts; KILL's accepted-WRITE verdict needs independent in-attempt publication, not a commit-time flag. |
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
3. Local bookkeeping tracks actual mutations (including deletions), ordered
   propagated effects, expiry insertions and necessary reset actions. Mutation
   detection remains separate from sequence allocation and WRITE_DIRTY.
4. Done **and Lua Error** accept preceding effects. Only internal abandonment
   rolls back, restoring entries, access and last-write stamps, client reply
   position/random state and engine-visible state before releasing keys.
5. While keys are still held, one Meta acquisition assigns one sequence to an
   actual mutating statement, stamps surviving changed entries, captures final
   images/tombstones if required, appends its entire effects block, applies due
   changes and publishes counters. No sequence/capture is published for aborts.
6. Plain writes, EXEC, expiry/eviction and writer cuts preserve that same
   publication invariant. No write releases keys and later queues its AOF.
7. Replies are buffered until the accepted statement completes; reads cannot
   create a causal dependency on an uncommitted mutation. The file writer
   preserves Meta append order and existing fsync/failure/truncation rules.
8. Time, memory-policy/OOM inputs and shared Lua library/random state obey
   their explicitly approved contracts. Engine pooling must settle its shared
   Lua state; separate engines alone do not make scripts independent.
9. AOF health is checked at the first accepted WRITE under an approved live
   observation contract. Today this premise acquires Meta then and keeps it;
   full commit-only acquisition cannot simply assume it away.

**Judgment, from premises 1-9 and SHARE-3.** Order accepted mutating statements
by Meta publication. Conflicting key operations cannot pass an earlier holder;
independent dataset operations commute. Read-only statements can be inserted
at their atomic points. A client that reads A's write then invokes B cannot
put B's record before A's, because A's publication preceded release/observation.
This covers cross-key causality through reads, not merely per-key log ordering.
Each complete execution-unit prefix replays a permitted state in that order;
MULTI/EXEC framing preserves multi-effect units at truncation. This proves no
new durability guarantee beyond the existing AOF policy.

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

**Judgment.** A script need not inspect capture state while interpreting Lua.
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
Shared command parts must expose mutation/effect information without depending
on an already assigned sequence; ordinary callers can retain their existing
token wrapper. Do not convert all callers to inert tokens or defer plain AOF
publication past key release. Coordinate any changed token/capture contract
with the plain-command owner; EXEC uses `held_run` and must remain correct too.
No new plain-command benchmark or duplicate redesign belongs here.

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
request candidate, not a confirmed new missing API or a filed request.

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
No board connector is available in this session, so no new request/card has
been filed; the proposed request and fairness follow-up remain dispositions
for `firn-script-meta-free`, not falsely reported board actions.

## Owner options

**Judgment.** The decision is whether to pursue fully commit-only Meta access
for AOF-enabled scripts now, or first establish the smaller improvement with
the present live-check boundary. No choice is made by this document.

| Option | Deliverable / what it unblocks | Cost and risk |
|---|---|---|
| **A — Stage metadata, retain necessary live checks; recommended** | Refactor expiry/effects/sequence/capture publication after engine pooling, retaining Meta from first WRITE where required. AOF-off finite scripts can avoid the execution hold; AOF-on scripts can at best avoid it before first WRITE. Obtain the attribution measurements and pursue the health request openly. | Moderate Firn/interface/design work and capture validation. Early-WRITE scripts may gain little; this is explicitly **not completion of the full Meta-free target**. No snapshot workaround or omitted failure behavior. |
| B — Resolve the full observation contract first | Whitefoot/Firn joint design for live persistence status, plus an explicit ruling on the timed-history claim and accepted memory exception, before implementing commit-only AOF-on execution. | Larger, uncertain language/semantic scope; strongest concurrency target, but no existing API/diagnostic proves its final shape. May uncover a fundamental shared dependency rather than an optimization. |
| C — Keep full-attempt Meta and measure only after pooling | A clean measured baseline with no token/capture refactor yet. | Lowest immediate change cost; retains long-script serialization and delays the scaling experiment. Appropriate if the baseline falsifies the 20% queueing prediction. |

**Judgment; confidence 4/5 in A as the next bounded step, not in its speedup.**
The source identifies removable bookkeeping and a necessary first-write
decision, while a same-statement commit needs no new nesting capability.
Low Meta queueing, dominant first-WRITE suffix cost, capture overhead or an
unresolved semantic witness would favor C or B. Approval of A must retain the
full target as unfinished and record the live-status need, not close the item
on an AOF-off-only result.

## Measurement and correctness plan — do not run locally

**Judgment; preregistered plan.** The owner runs CI. Begin only after the pool
lands and its shared-library state/control semantics are known. Record exact
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

Use three arms: pooled baseline B; independently rebuilt same-source twin T;
candidate C, all with the same pin. If a runtime fix is required, first compare
same-source B/T on that runtime; do not attribute the compiler change to Firn.
Interleave B,T,C and C,T,B per round, rotating the first arm between rounds.

Primary workload: the existing `limiter-script` application script, random
100,000-key domain, 8 and 50 connections, pipeline depth 1, 1 and 2 physical
cores. Preserve its script source and business logic. Add diagnostic ordinary
EVALSHA workloads with preloaded sources: per-client disjoint keys; one hot key
as a negative control; identical finite Lua computation before versus after
first WRITE; expiry on every write; error after a write; and an undeclared-key
fallback. These are workload inputs, never server-selected code paths.

Run AOF off and on as separate cells; AOF on must use the normal recorded
fsync policy and a real file. Existing workload-image mode must be extended
and verified if it cannot enable AOF; never label its AOF-off result AOF-on.
Add rewrite-active cells with the same dataset and script, documenting scan
progress and complete/aborted capture. Use a fixed finite memory limit well
above the primary workload's use; add near-limit correctness cells separately.
After the 1/2-core question is resolved, 4/8-core cells can locate the remaining
global commit ceiling, without changing the primary acceptance criterion.

Start with the smallest useful hosted correctness witness, recording its
duration. On the 14900K, first sample one 5-second run per arm in the primary
two-core cell, then a reversed round to expose spread. Choose only inconclusive
cells for longer runs, provisionally three interleaved 10-second rounds. Do not
start a long matrix before inspecting duration/spread. For each cell define
noise N as the maximum of 5%, the absolute B/T median ratio deviation and the
largest arm's `(maximum - minimum) / median` across interleaved rounds.
The required throughput gain is greater than max(10%, N), Meta queue share
falls by at least half from a baseline of at least 20%, and one-core throughput
loss/p99 increase are at most N. If the partial AOF-on option retains most of
the suffix hold, report rejection for that cell, not a Meta-free success.

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
8. Pool generation/FLUSH across retry gaps and accepted-WRITE KILL arbitration:
   no stale-source execution, no lost running invocation and no false kill of
   a written script. Verify the available DecisionCell integration separately.
9. Late-lock fairness stress: zero stranded waiters after a finite flood;
   report maximum overtakes and the runtime contract tested. Throughput gains
   cannot excuse starvation or an unbounded key-hold regression.

## Dispositions and remaining uncertainty

**Source-read / judgment.** This research leaves all implementation unchanged.
It identifies one proposed Whitefoot semantic request (live reusable AOF-health
observation), one reported runtime fairness follow-up, and current-pin facilities
for the older driver-count/KILL needs. None was newly filed from this session.
Capture finalization and mutation-token separation are Firn design work, not
proven language gaps. Plain-command publication stays with `firn-write-meta-free`.

The missing script lazy-expiry counter increment is a source-inferred defect:
confirm with INFO expired_keys before/after a script that lazily removes an
expired key, compared with Redis, with active expiry controlled by the test
schedule. Defer its fix to the command/accounting owner; do not silently label
it an implemented staged counter. No server fix belongs in this research PR.

Still unverified: the remote main head, compiled witness/lock ledger,
pool/canonical Lua-state integration, fairness-fix revision, the reusable
health API/design, bounded dirty-key/capture representation, timed-history
counterexample execution, all numerical predictions and all CI acceptance.
The conditional ordering argument is not a claim of unrestricted serial Redis
reply equivalence. The owner can choose the bounded option with these limits
visible; completing the full Meta-free target still requires resolving them.
