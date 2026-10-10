# Reconciled live scan and after-image log

## Question, evidence and scope

Can firn replace its full private replay keyspace with a bounded live scan,
produce a Redis-compatible base at one end cut, and continue service within
the owner's memory and latency budgets? **Proposal, registered 2026-10-10**
on `claude/rewrite-scanlog`, based on Firn-wf `c1144b6`. Nothing here changes
source, the design tree, a pin or a submodule; no build, execution or
measurement was performed. The [original investigation](README.md) remains
the history and current implementation record.

**Current implementation.** `firn/persistence/rewrite.wf:185` creates the
worker's private keyspace, `:186` replays the old base and closed increments,
and `:189` emits it. Even switching first replays the current increment into
another private keyspace (`:56–70`). Both copies must disappear, rather than
moving their allocations outside admission. The checked-in
[memory-limit record](../memory-limit/README.md#results-eviction-quality-and-at-limit-behaviour)
records rewrite-induced eviction. The later **supplied observations**, also
recorded with run links in Whitefoot's
[snapshot investigation](https://github.com/Ming-Research/Whitefoot/blob/fe5589ec5f4584c6fd235faa17e2e3173a7d651e/research/investigations/consistent-snapshots/README.md#need-evidence-and-a-limit-no-mechanism-removes),
are 208–240 MB against 117 MB maxmemory and about 2% of normal one-CPU
throughput; those exact later figures are absent from this branch's
memory-limit record and were not remeasured here.

**Owner-set constraints.** Board rulings
[`firn-maxmem-scope` B, actual footprint; `firn-q-snap-budget` A, reserve and
abort; `firn-q-snap-first` A, compare three routes](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip)
require service first and no eager second dataset. This investigates one
candidate beside the persistent library and restricted fork, without selecting
the winner. Whitefoot's approved
[frozen-dataset direction](https://github.com/Ming-Research/Whitefoot/blob/fe5589ec5f4584c6fd235faa17e2e3173a7d651e/design/language/data-model/frozen-datasets.md)
and [retention policy](https://github.com/Ming-Research/Whitefoot/blob/fe5589ec5f4584c6fd235faa17e2e3173a7d651e/design/language/data-model/frozen-datasets/retention-budget.md)
keep fork outside the source abstraction and require bounded cleanup.

**Witness evidence, limited.** At `origin/claude/snapshot-witnesses`
(`0958448`), the
[witness record](https://github.com/Ming-Research/Firn-wf/blob/0958448784cb4f42f05e238728dacf00efb7b2b2/research/experiments/snapshot-log-witness/README.md#run-on-the-held-entry-read-fix-2026-10-10)
reports zero correct mismatches in 20 runs, interleaving in one, and control
mismatches in all 20. Its `protocol.wf:51–73` commits entry, stamp and
after-image together. It has a finite integer domain and no durability,
general values, transactions, expiry or resource-bound proof. It ran with
`wf-fe5589ec5f45`; this branch pins `wf-f3d081b90a8d`, so a pass does not
establish execution on the present pin.

## Cut and authoritative files

**Proposal: delay the existing single rotation until S1.** S0 opens capture;
S1 ends it after the last scan observation, before reconciliation. No
rotation at S0 is necessary. Two rotations would retain an extra scan-window
increment without changing the required final boundary.

1. In one Meta statement, check admission, set capture active, and read the
   shared commit counter as **S0**. Existing files remain authoritative and
   the writer continues ordinary appends during scanning.
2. Each scan batch observes membership, complete owned payload and absolute
   expiry, plus the entry's last-write stamp, in one whole-map-and-Meta
   statement. All observations precede S1; no live reference escapes.
3. After scan completion the sole AOF writer finishes any prior private
   partial append. In one Meta statement it reads **S1**, disables capture,
   freezes/transfers the journal, and swaps all collecting AOF bytes through
   S1 into a sealed old-file buffer. Later commits collect in a distinct
   buffer destined for the next increment. Buffer identity, counter and
   journal state are published together; reading S1 then separately swapping
   bytes would lose the correspondence.
4. Drain that sealed buffer to the old increment, validate its block boundary,
   sync it, then open and publish the new increment with the **old base and
   every old increment still named**. Adopt its handle on publication; only
   then drain the post-S1 buffer to it. During host I/O clients keep committing
   into that buffer. If rotation fails before publication, drain those later
   bytes back to the old handle in order and abort the rewrite. Never discard
   a partial append or append post-S1 bytes ahead of the sealed prefix.
5. Reconcile the finite journal `(S0, S1]`, sync/close the temporary base,
   rename it and publish a manifest selecting **base(S1) + increment(>S1)**.
   Only durable publication permits removal of the old files. The current
   published-versus-durable distinction and shutdown ordering remain
   (`firn/persistence/rewrite.wf:290–335`, `:291`).

**Deduction: restore oracle.** For a committed suffix ending at Q,
`restore(base(S1), commands(S1,Q]) = dataset(Q)` under loading semantics.
Each execution unit through S1 is represented by its final state in the
base; each later unit appears once in the increment. A base at S1 paired
with the increment from S0 would double-apply non-idempotent window writes.
Sequences are internal; the manifest and command bytes need no new syntax.
Crash before installation uses the old base plus *all* named increments,
including the new one, never the fuzzy temporary base.

The current switch drains by swapping collecting bytes
(`firn/persistence/rewrite.wf:232–251`), rotates before rebuilding
(`:338–365`), and retains only the newest increment on install (`:305–319`).
Its switch-time private replay must become a **syntax-only streaming boundary
check**, sharing parser/block rules with `firn/persistence/persistence.wf:189–252`
but executing no commands. Preserve first-unclosed-MULTI versus latest-MULTI
semantics in [the original boundary record](README.md#switch-time-block-boundary).
An inherited unfinished startup block can make live and reloadable states
differ: refuse capture while that condition remains, rather than silently
turning those normally reverted writes into a new base. Detect it before S0
and test it; changing that recovery policy needs a separate owner ruling.

## Sequence and mutation coverage

**Proposal:** add `last_write: u64` to `Entry`, next to `expires` and
`access` (`firn/store/module.wfm:34–39`), and one `commit_seq: u64` in Meta.
Inline metadata avoids a second map, duplicate key bytes, lookups and deletion
retention. Cost: **8 logical bytes per live key**, plus any changed cell
stride/alignment and map spare capacity; one million live keys implies
8,000,000 logical bytes, not a measured heap delta. Measure generated layout
and allocated cells. A parallel map would also charge keys, capacity and
atomic targets; recommend against it for this representation.

One sequence belongs to an entire mutating atomic statement, including all
keys in MSET, EXEC or a script attempt. Take it lazily at the first actual
dataset change or AOF append, reuse it through that statement, and stamp
every surviving changed entry. On deletion append a sequenced tombstone
while capturing; retain no permanent tombstone map. Failed/no-effect commands
need no stamp unless they actually remove expired data. Startup can initialize
all loaded entries and the counter to zero: no export survives restart.
Never wrap a sequence; refuse a new capture near exhaustion and require a
quiescent reinitialization protocol before reuse, rather than corrupt ordering.

There is **no existing shared AOF commit sequence**: Meta holds a byte buffer
and file-size counters (`firn/store/module.wfm:71–99`), buffer length resets
on drain, and manifest sequences name files, not commits
(`firn/persistence/module.wfm:26–36`). Command parts return counts for block
wrapping (`firn/commands/transaction.wf:256–286`); one command can change many
keys or append several records. Neither byte length nor that count can be
reused as the witness's monotonic statement sequence.

| Mutation path | Required publication, in its existing atomic statement |
| --- | --- |
| All string/list/set/hash/sorted-set commands, including in-place changes, transfers and store variants | Stamp every changed destination and source; capture final value or deletion, including expiry-only changes. `firn/commands/strings.wf:450` and `firn/commands/keys.wf:1239–1272` show entry-plus-Meta composition. |
| EXEC | One sequence, final after-images of all touched keys, and existing complete AOF block; command errors keep earlier successful effects. Outer statement is `firn/commands/transaction.wf:255–288`. |
| Scripts | Same rule on the outer attempt, including effects before a script error; no sequence/image from a discarded read-only retry. Outer hold and both effect publication paths are `firn/scripting/entry.wf:431–481`. |
| Lazy and active expiry, including removals during reads | Same-sequence tombstone with DEL/statistics; `firn/store/store.wf:139–187`. A stale due-queue record that changes nothing publishes nothing. |
| Eviction | Tombstone with DEL/statistics at actual removal, not victim selection; `firn/commands/eviction.wf:350–375`. |
| FLUSHALL / FLUSHDB, including inside EXEC/scripts | Advance the statement sequence and abort an active scan in the same whole-map-and-Meta statement as clear (`firn/commands/server.wf:166–187`). No enumeration of millions of tombstones. After S1, clear simply belongs to the normal increment. |

Use one mutation-publication contract shared by network, held command parts,
EXEC and scripts; an AOF-hook alone misses unlogged changes and cannot infer
their final values. For an execution unit, bounded dirty-key scratch coalesces
repeated writes to each key into its final after-image before releasing the
hold. Overflow aborts only capture, and all service effects/AOF bytes still
commit. Access/LRU/LFU-only refreshes are outside the persisted dataset and
must not count as last writes. The no-rewrite path maintains the sequence and
stamps but allocates no dirty-key set or journal.

## Retention, emission and progress

**Proposal:** the in-memory window journal retains ordered records
`(statement_seq, binary_key, type, complete_after_value, absolute_expiry)` or
`(statement_seq, binary_key, tombstone)`. Images own serialized bytes, not
borrowed Entries or shallow copies of their Box/Ring/map descendants.
Record framing distinguishes same-sequence keys and execution-unit end.
Do not log INCR/LPUSH/etc. here and do not use normal command bytes as images.
No full exported map or all-scanned-key stamp table is built in RAM.

Define **R** as an explicit total rewrite allocation allowance: retained
journal capacity, keys/framing, dirty-key scratch, scan KeySet, scan/output
buffers and detached buffers awaiting release. Charge capacity and growth
overlap *before* allocation; fixed byte chunks permit bounded release without
recursive destruction of value owners. Freeze the journal at S1; post-S1
traffic no longer grows it. No default spill or hidden replay fallback.
If another complete image/unit cannot fit, atomically mark capture aborted,
stop retaining records, and let the client mutation and ordinary AOF append
complete. Never install a partially recorded unit.

R remains part of the process footprint and **is not an additional maxmemory
exclusion**. Current admission excludes only ordinary allocated AOF buffers
(`firn/commands/eviction.wf:33–56`); report those buffers separately, including
sealed and post-S1 collecting capacity during rotation. Any extra capacity
introduced by rotation also charges R; ordinary maxmemory exclusion does not
make it free. Stalled boundary checking/sync can grow the post-S1 backlog:
exercise that pressure and fallback to the old file in fault cases, with
no promise of unlimited acknowledged writes during stalled durable storage.
Admit only with room for R and
normal service headroom; if service consumes that headroom, abort capture
before evicting keys to sustain it. A rewrite refused at a full limit is an
expected outcome, not a successful memory experiment. Provisional comparison
setting: R at most **5% of maxmemory**, explicitly specified when maxmemory is
zero; the owner must approve the actual byte budget/headroom (decision 3).
Requested heap bytes are not total RSS; report both and the difference.

**Reply semantics:** a start refusal returns BGREWRITEAOF's generic error;
a later reserve abort sets `aof_last_bgrewrite_status:err`. Once a caller has
received `+...started`, it cannot receive a second asynchronous error.
Recommend answering started after S0 admission, rather than waiting for the
S1 rotation; current handoff waits for switching
(`firn/commands/aof-rewrite.wf:13–45`, `firn/persistence/rewrite.wf:360–364`).
The previous base/log remain authoritative on either failure, retaining a
published new increment when applicable.

**Encoding proposal.** Scan images use RESP `SELECT 0`, `SET`, ordered
`RPUSH`, `SADD`, round-trippable `ZADD`, and `HMSET`, followed by absolute
`PEXPIREAT` when present. All five supported types are the variants in
`firn/store/module.wfm:26–31`. Reuse the format/score rules at
`firn/persistence/rewrite-dataset.wf:36–109`: at most **64 elements per
collection command**, binary-safe keys/values; empty collections mean no key.
The current encoder nevertheless builds an entire key's output before I/O
(`:159–162`); 64 elements per command is not a memory or pause bound.

Stream scanned images first. Then append journal images in ascending sequence,
each as **DEL key + complete reconstruction + optional PEXPIREAT**; tombstones
emit DEL alone. DEL clears old type, collection contents and TTL, so neither
RPUSH nor missing PEXPIREAT retains an earlier image. The temporary file may
temporarily roll a key back; it is never authoritative until all records
through S1 are emitted and synced. This is Redis-readable command AOF, though
less compact than Redis's one-image-per-key base.

**Deduction:** an unchanged key is scanned once with its S1 state. For a key
changed in `(S0,S1]`, its final journal sequence is at least every scan stamp
for that key and its image is exactly the S1 state. Thus ordered replacements
end at the same per-key maximum sequence as the witness's comparison, without
retaining a stamp for every scanned key. Deleted/reinserted keys obey the
same rule, including keys created behind the cursor. This relies on scanning
finishing before S1, complete ordered capture and no unrecorded clear.
Replaying *commands* instead of replacement images invalidates the deduction.

Initial **proposed**, unmeasured work limits: scan count hint 64; emission
chunks at most 64 KiB; total capture work at most 1,024 visited payload elements
and 64 KiB serialized bytes per scan statement or mutating execution unit,
not per key within an unbounded transaction. Abort capture when this allowance
ends; the admitted transaction/script still completes normally. Each batch is a separate
statement, followed by waiting file I/O outside all holds; reconciliation,
boundary checking and cleanup also yield between bounded chunks. Empty scan
batches need a genuine waiting operation too, not a zero-byte append (the
current `append_all`, `firn/persistence/persistence.wf:27–36`, skips it).
These waiting points address
[`firn-gap-ctx-starve`, non-waiting driver monopolization](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip);
they are part of the proposal even after its runtime fix.

**Unsettled resource contracts, not implementation claims:** the pinned
[SHARE-1 map_scan contract](https://github.com/Ming-Research/Whitefoot/blob/f3d081b90a8d/spec/kernel-spec.md#13-execution-overlap)
(`spec/kernel-spec.md:2276`) makes count only a hint and bounds neither
returned key bytes nor scan work. Checking the size *after* it allocates is
not reserve enforcement. Request a bounded scan contract from Whitefoot
(decision 4), with a minimal long/binary-key witness and no truncated scan
fallback. Large single values likewise cannot be coherently copied by
unversioned sub-value reads across statements. Until a bounded complete-image
copy fits, abort capture at its explicit work/byte limit; do not refuse the
client's otherwise valid value or claim large-value rewrite completion.
Repeated aborts on a selected consumer's values reject this route.

Abort detaches the journal in constant shared work; release at most one
64 KiB chunk per cleanup turn with a wait between turns. No new rewrite may
reuse its allowance until the prior worker has stopped and borrowed buffers
are released. For the parent contracts B1–B9: unique owned image chunks cover
B1/B2/B4; atomic observations/end cut cover B3; bounds refer to owned bytes
(B5); only the worker owns its temporary output, the writer owns authoritative
files (B6); fork-specific B7 is inapplicable; B8 remains **unverified** for
stalled filesystem calls; B9 belongs to the library comparison.
Pinned PRE-2 (`spec/kernel-spec.md:2558–2559`) gives filesystem operations no
deadline/cancel bound. A stuck append can keep one borrowed chunk and its
handle indefinitely. A logical abort and bounded retained bytes do not prove
bounded-time cleanup or prompt service headroom recovery. Bring that gap to
Whitefoot's owner (decision 5); no kill-thread/free-borrowed-bytes workaround.

## Expiry and multi-key semantics

**Proposal:** the S1 dataset is physical key membership, type, payload and
absolute expiry metadata at the cut, including a physically retained elapsed
expiry. Do not sample a fresh clock per emitted key or omit such a value:
a later recorded PERSIST or mutation may rely on it during loading.
Deletion by lazy/active expiry is a real sequenced mutation. At service time
T, visibility uses expiry `< T` (`firn/store/store.wf:90–96`); at loading
firn suppresses expiry and retains elapsed deadlines until service resumes
(`firn/persistence/persistence.wf:131`). The oracle must distinguish physical
cut equality from logical visibility at one agreed restore time.

**Redis source-derived comparison**, not a new run: Redis 7.0.15
[`src/aof.c:2445–2452`](https://github.com/redis/redis/blob/7.0.15/src/aof.c#L2445)
flushes, opens the next increment, then forks with no command interleaved;
its cut is at start, whereas this proposal reconstructs an end cut.
[`rewriteAppendOnlyFileRio:2285–2322`](https://github.com/redis/redis/blob/7.0.15/src/aof.c#L2285)
retains expiry metadata and writes PEXPIREAT without filtering elapsed keys.
[`db.c:1616–1651`](https://github.com/redis/redis/blob/7.0.15/src/db.c#L1616)
suppresses expiry while loading and expires strictly after the deadline;
script expiry uses script-start time. firn's existing script time policy is
recorded in [its script decision](../../../design/firn/scripts.md), and this
investigation does not alter that approved difference.
Redis also preserves already-past PEXPIREAT deadlines during loading
([`expire.c:486–494`](https://github.com/redis/redis/blob/7.0.15/src/expire.c#L486)),
which matters when an intermediate image precedes a later PERSIST.

S0/S1 share Meta with every modifying statement, so cannot bisect EXEC or a
script; both changing several keys commit wholly before or after a cut.
Journal images of one unit may be emitted over multiple waits, but only the
complete base is published. A post-S1 unit's original MULTI/EXEC framing is
kept intact; a crash with its incomplete tail applies none of that unit.
Neither a failed command in EXEC nor a script error rolls back earlier
effects. Redis's event-loop fork boundary similarly cannot split a running
execution unit; compare its complete command/effect blocks and error cases,
not a synthetic per-key transaction cut.

## Validation registered before measurements

All future checks run in CI, after the required runtime/pin work; none is
authorized by this edit. Run the smallest useful case first, record its
duration/spread, then size the batch. Correctness precedes performance.

| Case and independent oracle | Rejection criterion |
| --- | --- |
| Unit traces: all five types, binary keys/values, list order, scores, expiry/PERSIST, transfers, create/delete/reinsert around cursor, repeated writes and no-effect errors. A serial model of committed logical operations builds the expected state, without scan stamps or reconciliation code. | Any type/member/value/deadline or membership mismatch; negative cases must reach their intended refusal. |
| Network restore: pause mutators and expiry/eviction at S1 using a test-only barrier; read a quiescent canonical dump directly from held entries, independent of map_scan/images. Restore completed files into fresh firn **and** Redis 7.0.15 with RDB preamble off and compare strings, ordered lists, canonical sets/hashes/scores and absolute deadlines. Keep this expensive oracle outside performance runs. | Any physical cut mismatch. For elapsed expiries also compare logical visibility at a controlled common T; portable cross-server checks use sufficiently future deadlines, with exact boundary cases under a controlled clock. |
| Cut/suffix: two-key EXEC and scripts, error after first write, lazy/active expiry and eviction on both sides; capture another quiescent dump at Q after post-S1 non-idempotent writes. Restore base plus its whole suffix to Q. | Partial unit, lost effect, duplicate effect or boundary-dependent mismatch. Require observed commits between scan batches, not just two spawned contexts. |
| Failing control: S0; INCR x from 0 to 1; scan x=1; reconcile with command replay. | Must produce x=2 versus direct oracle x=1. If it passes, the experiment cannot detect double application. |
| Reserve/abort: exact fit and one-byte/record overflow, long keys, large values, all-key replacement, hot-key amplification, stalled exporter, FLUSHALL inside/outside EXEC/scripts; continue writes and restart from authoritative files. | Any overshoot of R (including transient capacity), changed service result due to capture overflow, partial journal publication, new-base installation after abort, or inability to restore. Expected oversized capture refusal is not value rejection or proof of large-value completion. |
| Faults: inject short writes and open/append/sync/rename failures; crash during scan, sealed-buffer drain, rotation, reconciliation, base rename, manifest rename, directory sync and cleanup; include inherited/nested MULTI tails. | A missing named file, loss of synchronized complete units, duplicate apply, premature history deletion, or failure to adopt a published manifest. Unsynced crash loss follows existing fsync policy, not an invented guarantee for every acknowledged byte. |
| Cancellation/cleanup: delay file completion while aborting; measure borrowed-buffer lifetime, chunk release and next-rewrite admission, then complete the operation. | Use-after-free, unbounded retained allocation, allowance reused early, or exceeding the agreed cleanup-time target. An indefinite I/O wait is a failed/unverified B8 contract, never a waived case. |

## Comparison and inactive cost

On the idle **i9-14900K through CI**, build firn with `make firn-lto` and
use the redis-bench workflow/instrument with identical logical traces for
closed-log replay, this route, the library, qualified fork and Redis 7.0.15.
Record exact firn/compiler/Redis revisions, kernel, allocator, affinity,
drivers/workers, AOF/fsync/auto-trigger settings, reserve, headroom, key/value
distribution and offered rates. Start with a timed small probe. Interleave
same-source before/after and a base twin; changing the runtime pin separately
requires its own control so starvation repair is not credited to this route.

Workloads: one and two server CPUs; cache/session and conditional-update
scripts; read-mostly traffic with access stamps; repeated hot-key writes;
uniform replacement, deletion/reinsertion, multi-key units, large mutable
collections/strings, eviction at the limit and stalled output. Repeat the
supplied million-key/117 MB case where feasible, including a separate
deliberately no-headroom refusal case. Use identical budgets and datasets for
each mechanism; shrinking firn's dataset alone is not an improvement.

| Measurement | Pre-registered rejection |
| --- | --- |
| Peak requested heap, counted heap, journal/scratch/capacity, RSS and aggregate physical memory relative to maxmemory; Redis parent+child without double-counting shared pages | R exceeded or capture induces eviction/OOM solely to retain images; resolved memory peak fails to improve on replay. Report ordinary AOF exclusions and allocator/OS overhead; Redis parent used_memory alone is no physical-memory comparison. |
| End-to-end rewrite duration, S0/S1 pause, rotation pause, scan/reconcile/sync/cleanup time and output bytes | Correctly completing candidate fails to improve rewrite time over replay beyond control noise, or cannot complete a selected consumer trace within reserve; a fast abort is not a duration win. |
| Sustained throughput, p99 (also p99.9 and longest service gap), during rewrite versus idle firn and Redis on the same offered workload | Worse throughput or latency than replay beyond noise, or exceeds the owner's absolute pause/tail target. Redis ratios are reported separately; matching Redis performance is a comparison goal, not an asserted result. |
| No rewrite: support absent versus present but unused, reads, misses, writes, EXEC/scripts, AOF on/off | A statistically resolved **over 1%** overhead rejects the route under Whitefoot's comparison rule, in throughput or latency and separately memory footprint. Unresolved variation requires a better-sized comparison, not rounding to a pass. |

Inactive work includes counter serialization, one stamp store per changed key,
extra entry stride/cache traffic and checks at every mutation site, even when
AOF is off. Existing Meta participation helps but establishes no cost bound.
The logical 8 MB per million keys may already breach the 1% memory screen;
that is a real cost to expose, not a reason to waive the rule. Compare an
opt-in representation only if the owner selects it, with its activation/cut
protocol separately proved.

## Decisions awaiting the owner

These are proposals for the board, not approvals or new TODO files. Existing
manifest layout, command encoding, publication/durability separation and
transaction semantics stand; private replay and its pre-switch copy would be
replaced only if this route is selected and the affected
[AOF-rewrite decisions](../../../design/firn/aof-rewrite.md) are approved anew.

1. **End-cut rotation and streaming replacements.** Recommend one rotation
   at S1 and chronological DEL-plus-after-image reconciliation. Alternative:
   rotate at S0 and again at S1, or externally sort/coalesce images for a
   smaller base; each adds I/O/storage machinery. Confidence **4/5** in the
   end-cut deduction, **3/5** in the streaming choice pending output-size/time
   results. This is a candidate protocol, not a choice over the other routes.
2. **Inline stamps and clear behavior.** Recommend u64 entry stamps and a
   statement counter; abort scanning on FLUSHALL/FLUSHDB. Alternatives:
   parallel stamps, or a sequenced database-reset image with reset-aware scan
   reconciliation. Also compare **stamp-free ordered replacements**: the
   deduction above does not need per-entry comparisons; inline stamps support
   witness-style observation/auditing but are not logically required by this
   emitter. Recommend retaining them for that first comparison, then requiring
   evidence of a production need before charging every key. Inline stamps cost
   memory even idle; aborts can prevent completion in clear-heavy workloads.
   Confidence **3/5**; the 1% screen and real clear frequency can reject it.
3. **Numeric resource/service limits and start reply.** Recommend the 5%
   reserve screening setting, counted headroom, immediate admitted-start
   reply with later INFO error, and no silent retry loop. Owner must fix R in
   bytes, service headroom, maximum pause/p99/service gap and cleanup deadline
   before performance acceptance. Alternative: other explicit limits; holding
   BGREWRITEAOF open to final failure changes Redis's asynchronous behavior.
   Confidence **2/5** in sizing, **4/5** in reply semantics.
4. **Bounded enumeration and large values.** Recommend requesting a hard
   key-byte/work scan budget from Whitefoot and a complete-image copy witness;
   treat over-budget images as rewrite aborts. Alternative: versioned
   sub-value export via the library route. An after-the-fact KeySet size check
   or torn nested-value scan is not an acceptable alternative. Confidence
   **5/5** that the current count hint supplies no bound, **2/5** that these
   provisional image limits suit the selected consumers. Repeated large-value
   refusal reopens the mechanism, rather than narrowing supported commands.
5. **Stalled-I/O cleanup contract.** Recommend a Whitefoot-owned bounded
   completion/cancellation contract for file export, with a minimal borrowed
   output-buffer witness and acceptance test. Alternative: retain bounded
   quarantine until the host completes, which proves a byte limit but not
   the owner's cleanup-time guarantee. Confidence **5/5** that PRE-2 lacks
   that guarantee, **3/5** in the required runtime remedy; do not call B8 met
   before it is demonstrated. Service-headroom recovery is part of this test.

Bounded scan and stalled-file cleanup are proposed Whitefoot contract requests,
not filed or resolved gaps; neither changes the witness's recorded pass.

Read-only review: a separate GPT-6 agent inspected the complete documentation
change against `c1144b6`, affected source/design context and checklist A/D/R
and applicable G/DC checks; no findings within scope after clarification of
stamp necessity, aggregate capture work and rotation-buffer pressure. No
suite ran; implementation, design soundness and resource guarantees remain
unverified. Work stops at this uncommitted investigation as requested.
