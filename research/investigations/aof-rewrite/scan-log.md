# Reconciled live scan and after-image log

## Question, evidence and scope

Can firn replace its full private replay keyspace with a bounded live scan,
produce a Redis-compatible base at one end cut, and continue service within
the owner's memory and latency budgets? **Proposal, registered 2026-10-10**
on `claude/rewrite-scanlog`, based on Firn-wf `c1144b6`; the registration
changed no source. Stages 1 and 2 are now implemented on that branch (see
[Stage 2 validation and review](#stage-2-validation-and-review-2026-10-10)).
The [original investigation](README.md) records the replay that main runs.

**Implementation at registration.** `firn/persistence/rewrite.wf:185` creates the
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

Bounded scan and stalled-file cleanup are Whitefoot contract requests, now
board items `firn-gap-scan-bound` and `firn-gap-file-cancel`; neither changes
the witness's recorded pass.

Read-only review: a separate GPT-6 agent inspected the complete documentation
change against `c1144b6`, affected source/design context and checklist A/D/R
and applicable G/DC checks; no findings within scope after clarification of
stamp necessity, aggregate capture work and rotation-buffer pressure. No
suite ran; implementation, design soundness and resource guarantees remain
unverified.


## Stage 1 implementation record (2026-10-10)

The owner has selected option A on `firn-q-rw-cut`, `firn-q-rw-stamp` and
`firn-q-rw-limits` and requested sequencing alone as stage 1. The
[implementation record](sequence-stage-1.md) describes the
stamp/counter support, every shared-helper call site, network cases,
independent review and unverified compiler/layout/behavioral evidence.
This supersedes the investigation-only stopping point above. Scanning,
capture, rotation, emission and resource-limit implementation are later
stages; the other unresolved protocol questions above remain unresolved.

## Stage 2 prerequisite inspection (2026-10-10)

**Stage 2 is blocked before implementation by the bounded-scan contract.**
The requested implementation includes the scan KeySet and transient growth
in R and requires charging before allocation. At the unchanged pin
`wf-f3d081b90a8d`, `map_scan` provides no way to impose these per-call
bounds for general live keyspaces. This is
the existing [board item `firn-gap-scan-bound`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip),
owned by the Whitefoot runtime session, not a new Firn command limitation.
The owner-selected end cut, ordered replacements, inline stamps,
flush-aborts and service-first reserve remain the requested direction.

Inspection was against Firn-wf
`5faa658597d6c786f791c6e3a54db7ba7c891ad0`, the local and remote head of
[draft PR #42, reconciled-scan AOF rewrite](https://github.com/Ming-Research/Firn-wf/pull/42),
and Whitefoot `f3d081b90a8d36e4cc2175dbf2d3734a074336ff`, resolved from the
pinned revision in a read-only local clone. The repository's uninitialized
Whitefoot-kit submodule was read from its separate clone at the recorded
submodule commit, without changing the checkout or its pin.

### Contract and implementation evidence

- [SHARE-1, line 2276 at the pin](https://github.com/Ming-Research/Whitefoot/blob/f3d081b90a8d36e4cc2175dbf2d3734a074336ff/spec/kernel-spec.md#L2276)
  makes the scan extent an execution input; count is only a hint. It
  promises no key-byte, allocation-capacity or visited-work ceiling.
- [The prelude boundary, line 388](https://github.com/Ming-Research/Whitefoot/blob/f3d081b90a8d36e4cc2175dbf2d3734a074336ff/compiler/src/prelude.rs#L388)
  takes only `map`, `cursor`, `count` and `keys`, returning the next cursor.
  Its only postcondition says KeySet length does not decrease. There is no
  byte allowance, bounded destination or budget-exhaustion result.
- [Runtime scan, line 2597](https://github.com/Ming-Research/Whitefoot/blob/f3d081b90a8d36e4cc2175dbf2d3734a074336ff/compiler/src/backend/concurrent_map.c#L2597)
  can inspect up to the table's capacity. It grows its temporary `scanned`
  array before releasing the old allocation, then copies every selected
  key into the KeySet at line 2671. That scratch also belongs in R.
- [KeySet growth, line 1523](https://github.com/Ming-Research/Whitefoot/blob/f3d081b90a8d36e4cc2175dbf2d3734a074336ff/compiler/src/backend/concurrent_map.c#L1523)
  allocates a larger key arena before releasing its predecessor.
  `wf_cmap_key_set_insert` at line 1565 copies the entire key. The
  `capacity: 64` constructor argument is not a key-byte cap, and the
  opaque KeySet exposes no reservation operation to Firn.

### Minimal semantic witness

This reduced function is a semantic example, not a complete runnable
program. It has not been compiled or run. A hypothetical remaining scan
allowance of 65,536 bytes cannot cover even the key copy below, before
KeySet metadata or runtime scratch is counted:

```whitefoot
fn scan_key_bytes() -> result: u64 pure waits {
  let store = shared_map_new::<u64>(capacity: 1_u64);
  let key = box_array_filled::<u8>(count: 65537_u64, value: 0_u8);
  let length = key.inner.len;
  let bytes = &key.inner[0_u64..length];
  atomic slot = &store[bytes] {
    set slot^ = Some<u64>(value: 1_u64);
  }
  let keys = key_set_new(capacity: 64_u64);
  atomic table = &store {
    let next = map_scan::<u64>(map: table, cursor: 0_u64, count: 64_u64, keys: &keys);
  }
  return keys.len;
}
```

At the inspected runtime, a map containing one live key and count 64 scans
through the end and inserts that key. `room_for` must provide at least
65,537 key bytes before returning. Reading its length afterward cannot
prevent this allocation. Choosing a larger default R does not turn count
into a byte or work limit. Precharging a conservative bound for all live
key bytes and runtime scratch would require global bounds and admission
against the entire keyspace, rather than the selected bounded batches;
it would not provide the missing general-scan contract. This is a
resource-interface gap, not a rejected spelling:
there is no observed compiler diagnostic, and none is claimed under the
owner's prohibition on local builds and execution.

The upstream acceptance case must cover a single binary key larger than
the allowance and batches of individually small keys whose total exceeds
it. Enumeration must either return an explicit budget refusal before any
overshoot, or provide bounded progress with an exact continuation; it must
never silently skip or truncate keys. Peak allocation must include KeySet
metadata/capacity, scan scratch and overlapping growth, rather than only
returned byte lengths. A separate visited-work limit must cover sparse
tables and collision runs. Firn must then be able to abort capture on
refusal without refusing the client's mutation. A check only after return
fails this acceptance case even if it immediately discards the batch.

### Disposition and delivery limits

The existing board requirement needs to precede this strict-reserve stage,
with priority at least that of `firn-wf-snap-log`. The board currently lists
it as P2 and the route as P1. Its published UI was read, but this session
has no ArtifactData row-writing capability; neither the dependency nor the
priority nor a progress log was updated. No upstream implementation or
new filing is claimed. The coordinator/runtime session must update that
existing item and supply the bounded interface before this implementation
can meet its stated acceptance condition; adopting a new compiler remains
a separate owner action because this task prohibits moving the pin.

Only this investigation was changed. No S0 admission, scan worker, journal,
S1 cut, rotation, reconciliation, abort path or network test was implemented
in this turn. The requested startup-MULTI refusal and design-tree replacement
also remain unimplemented; the existing private-replay code and its current
design nodes remain together. No fixed unlimited-maxmemory reserve is
selected for an implementation that does not yet exist. No local build,
execution, test, lint, measurement, commit, push or CI dispatch was performed.
The pin, submodules, `docs/todo.md` and `design/log.md` are unchanged. There
is consequently no new gate result or mutation-test evidence to report.
The separate stalled-filesystem cleanup-time contract remains unverified as
described above; it is not needed to establish this earlier scan blocker.

An independent read-only Codex/GPT-6 agent reviewed the complete uncommitted
documentation diff against the Firn revision above, the untracked-file
inventory, relevant design nodes and persistence code, and the cited pinned
Whitefoot sources. Its exact runtime model identifier was not exposed.
Finding F1 narrowed an overbroad impossibility claim to the missing general
bounded-scan interface and added the whole-keyspace precharging alternative
and its disposition; the reviewer inspected that repair and reported no
remaining findings within scope. A1, D1, G2, G3 and DC1/DC2 passed for this
documentation scope; code, test, pin, measurement and tree-edit checks were
not applicable. Requested stage-2 completeness (DC4), compiler acceptance
and executable witness evidence remain unverified. The review used only
Git/source inspection, did not independently verify remote PR or board
state, and establishes no gate, memory measurement or implementation result.

## Stage 2 implementation

The owner's addendum supersedes the stage-2 prerequisite stop above: implement
the end-cut protocol now and let `firn-gap-scan-bound` gate the strict reserve
guarantee alone. The worktree implements that direction without changing the
pin, submodules, manifest syntax or normal command framing.

### Reserve and the scan exception

R is `floor(maxmemory / 20)`, or **16,777,216 bytes (16 MiB)** when maxmemory
is zero. Admission requires counted heap plus R plus `max(R, 1 MiB)` service
headroom to fit maxmemory. Counted heap still includes rewrite storage and
excludes only the ordinary collecting and private pending AOF capacities.
The writer reuses those two buffers at S1; it creates no third sealed buffer.
During sealing and rotation each statement takes a separate LogBudget from
remaining R under the same Meta hold as its sequence. Every allocating AOF
encoder carries it to log_reserve, which charges the entire new backing
capacity before grow (old capacity remains conservatively charged until the
attempt ends). Overflow marks the local abort before allocation, permits the
ordinary command/AOF bytes, and publishes the abort when that same atomic
statement commits. This covers rotation-induced backlog even though normal
maxmemory accounting excludes AOF buffers. Private effects builders stay
unmetered here; their final append to Meta.log is charged. The boundary reader
notices abort between chunks and returns to ordinary draining of the old
file; an already published rotation remains authoritative with all old files.
Filename/manifest copies, growth overlap and serialization are conservatively
precharged at admission from their lengths. The journal has a fixed directory
of at most 8188 pointers, itself at most 64 KiB, and allocated byte chunks of
at most 64 KiB plus their Slots headers. No directory growth overlaps an old
one. Each execution unit retains bounded coalesced key/image scratch and
precharges journal growth while that scratch is still alive.

The stage was first delivered with provisional work ceilings of 1024
payload visits and 64 KiB of measured serialized work per scan statement or
mutating execution unit; they were removed after the completion review (see
[Stage 2 validation and review](#stage-2-validation-and-review-2026-10-10)),
so only R bounds an image. Measurement is an upper bound including command
lookahead, repeated collection-command key names and RESP framing; it is not
a payload-size promise, and it is charged before copying. Repeated writes to
one key replace its scratch image. Every journal unit has an internal
sequence/length frame, bounded by R and stripped before writing the base.

The scan uses **count hint 1**, as authorized. A fresh KeySet's known pinned
baseline, including the runtime's reusable small arena, is precharged. The
returned key lengths and any additional step storage are charged immediately
after `map_scan`, before copying a key for lookup or any payload. The exact
long-key test is ignored with the owner's specified reason and no other new
test is ignored.

**Known limitation, not the approved strict bound:** for a step returning one
key, R can be exceeded by at most that key's newly allocated bytes before the
post-step charge. The intended claim in the addendum was one key per step.
However, the pinned implementation does not establish that premise:
`compiler/src/backend/concurrent_map.c:2622` stops the *home range* after its
hint, then `:2643-2671` collects every live key belonging to those homes,
including collisions. Thus count 1 can return several keys, with KeySet arena
and scan scratch growth. The general exception is **one step's returned
storage**, not provably one key. The implementation accounts conservatively
for that returned storage and aborts immediately on overflow, but cannot
undo the allocation. This remains the same
[firn-gap-scan-bound board item](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip),
owned by the runtime session. Clarification of the narrower addendum claim
was requested; no hard one-key bound or new workaround is claimed. Upstream
acceptance still needs long/binary keys, collisions, capacity overlap and
visited-work bounds, rather than only a count hint.

`firn_aof_rewrite_reserve` reports the most recently admitted allowance.
`firn_aof_rewrite_scan_peak` reports the largest observed PRE-2 heap increase
between immediately before KeySet creation/map_scan and immediately after it.
It uses the runtime meter independently of all reserve charges. The ignored
long-key case loads a single non-expiring key from an AOF and makes no
concurrent writes; even this one step must fit R under a strict guarantee.
Removing post-scan accounting therefore still leaves an observed allocation
larger than R and fails the assertion. This necessary check is not a proof
of the whole rewrite's exact peak: unrelated allocations or frees in other
contexts can affect the process-wide difference. The additional
`firn_aof_rewrite_cut` flag identifies whether the latest
admitted attempt has crossed the atomic S1 cut, including sequence zero.
The rotation-backlog regression waits for it before writing, excluding
scan-journal exhaustion as an alternative cause of err. These fields exist
for those regressions under the reported-facts rule; none measures RSS or
allocator usable size. No timing, peak RSS or overhead result is claimed.
The existing PRE-2 stalled-I/O cleanup limitation remains
unverified: bounded chunk ownership and logical abort do not supply a host
I/O cancellation deadline.

### Protocol and failure handling

S0 checks exhaustion, inherited startup divergence, shutdown, reserve and
headroom under Meta and ServerState, then enables capture with the same
statement's `commit_seq`. Manual callers receive started at that admission.
Every keyed mutation shares the stage-1 token, now retaining complete owned
replacement bytes. Repeated keys in EXEC/scripts coalesce under their one
outer hold; ordinary AOF effects retain their existing framing. FLUSH marks
the token failed in that same statement, so no reset-aware journal or millions
of tombstones are needed.

Each scan observation holds the whole map and Meta together, reads membership,
last_write, complete payload and absolute expiry, and emits owned bytes only
after releasing the hold. It retains physical expired entries. Every step,
including an empty step, executes a nonzero sleep outside all holds.

The worker hands scan completion to the sole writer. After draining any prior
private partial append, the writer atomically reads S1, disables capture,
freezes the journal and swaps the collecting bytes through S1 into its empty
private buffer. A drain that does not swap buffers sends only this sealed
prefix to the old increment. The syntax-only boundary reader shares request
header/inline parsing and MULTI/EXEC classification with loading, and
executes no commands. RESP bulk bodies stream across a 64-KiB window without
retention, so a large overwritten historical value does not become current
image work. Incomplete headers or inline lines use the ordinary parser rules;
any required input/span growth is charged, including overlapping old/new
capacity, before allocation. Partial request and first/latest-MULTI state
survive read boundaries. The old file is synced before the new increment's
manifest is published with every old file still named. Post-S1 bytes then go
to the adopted new handle; before publication failure they drain behind any
remaining sealed suffix to the old handle.

The worker strips internal frames and emits chronological DEL/reconstruction/
PEXPIREAT units. After sync/close, the existing rename and manifest publication
path installs base(S1) plus increment(>S1). Published-versus-durable handling,
history retention, shutdown exclusion and normal MULTI/EXEC bytes are retained.

Reserve or work overflow, flush, sequence exhaustion, shutdown and host I/O
failure all prevent installation. A failed published rotation keeps its new
handle and all old files; a failure before publication keeps the old handle.
A journal is detached in one Meta statement and released at most one chunk
per waiting turn. In-progress is cleared only after the worker joins and its
chunks are gone. The reserve cannot be reused early. Eviction aborts capture
when service headroom is consumed throughout the in-progress lifetime,
including frozen post-S1 reconciliation, and, when already over maxmemory,
waits for cleanup before evicting to sustain retained images. The worker
observes logical abort between chunks and switches to release-only cleanup;
the installer also refuses an aborted attempt. Setting aof_installing under
Meta is the existing installation commitment: pressure that wins that hold
aborts first, while pressure arriving afterward waits for publication and
cleanup without recording a cancellation that can no longer be honored.
Stalled host calls can delay this wait, as the unresolved cleanup contract
already warns.

A startup-retained unfinished MULTI no longer refuses capture (status board
card `firn-q-rw-startup-multi`, implemented on its recommendation A). If the
sealed file still ends inside that block at S1, the boundary reader reports
the block's first MULTI and the writer cuts the file there before syncing and
publishing it, as main's switch-time cut did; base(S1) is the live dataset,
which holds the writes appended inside the block, as Redis's live rewrite
does. A block closed by a later appended EXEC needs no cut, and base(S1) then
omits the never-applied prefix, again as Redis's live rewrite does. A crash
between rotation and installation restarts without the writes appended inside
the block, where Redis refuses to start because its truncated file is not the
last one.

### Network evidence to run in CI

`tests/rewrite_scan.rs` uses ordinary binary-safe Redis reads as a quiescent
dump oracle. It observes complete records appearing in the temporary base and
requires the old manifest still to be selected before starting its concurrent
writer, so the selected keys have already been observed before their writes.
The fixture has 6000 keys across all five types; clients continuously write
bounded values while the scan continues. No server test hook is added.

- `reconciled_scan_restores_live_dump_and_non_idempotent_units`: compares a
  restarted server with a quiescent dump of all keys, complete values and
  absolute expiries. It also requires each counted INCR exactly once,
  including paired keys in EXEC and a script, and separately loads the base
  alone to require equal paired counters at S1. Removal of capture loses
  changes behind the observed cursor; omission of DEL duplicates the scanned
  list or retains cleared TTL/content; loss of a tombstone retains a deleted
  scanned key or RENAME source; pairing base(S1) with the S0 suffix double
  applies counted increments; a split execution-unit cut can separate its
  paired counters. The existing witness remains the failing command-replay
  control. These are intended distinguishing failures, not mutation-run
  results.
- `reconciled_scan_reserve_abort_keeps_writes_and_old_files`: CONFIG maxmemory
  chooses the real five-percent R. Repeated 32-KiB hot-key images fit each
  unit but exhaust retained journal reserve while scanning continues. It
  requires err, the old manifest, accepted subsequent writes, no automatic
  retry despite crossing its startup threshold, and restored hot-key bytes,
  an acknowledged counter and full cardinality. Removing reserve abort makes
  the required err fail; discarding ordinary bytes breaks the restore oracle.
- `reconciled_scan_flush_aborts_even_in_exec_and_script`: FLUSHALL, FLUSHDB,
  EXEC-with-flush and script-with-flush each require err, unchanged manifest
  and only the post-flush key after restart. Removing the shared abort permits
  stale scan images to survive and fails the status/restore requirements.
- `reconciled_scan_crash_uses_old_authoritative_files`: after scan progress,
  it observes the acknowledged INCR in the original increment, requires the
  original manifest and active rewrite, kills the process, and checks every
  fixture key remains plus that counter after restart. It rejects rotation
  at S0 and any premature selection of the temporary fuzzy base. This is a
  process crash, not a power-loss or fsync guarantee.
- `reconciled_scan_exact_reserve_includes_one_long_key_step`: a 17-MiB key
  exceeds the explicit 16-MiB unlimited-memory reserve. The test requires err,
  preserved service value and an independently sampled allocation increase
  no greater than R for even this single step; the
  current post-allocation scan step violates the last assertion. It is the
  only new ignored test, pending `firn-gap-scan-bound`, with the exact reason
  requested in the addendum.

Existing rewrite cases remain wired. Startup-tail cases require Redis's
live-rewrite results after a rewrite and its reload results without one;
failure cases now require started followed by err
because rotation moved after admission; shutdown during a scan keeps the
original manifest. The older concurrent growing-list case retains 100 strict
non-idempotent writes instead of growing without limit past the explicit
capture work budget; the new continuous bounded-value case supplies observed
scan-window interleaving. No ratchet row or existing command expectation is
removed. Restoring the refusal fails the retained-tail cases' rewrite
status and restored `after`/`new` values. The switch-time cut itself matters
only if the process stops between rotation and installation, because the
installed manifest no longer loads the old increment; no case covers that
window.

The repair cases additionally require a 128-KiB overwritten historical SET
and a mixed-case RESP MULTI/EXEC block to rewrite successfully with a small
current dataset, catching a parser that retains whole RESP commands in a
fixed window. A post-S1 pressure case builds retained replacements, observes
the rotation manifest while the old base is still authoritative, lowers the
ordinary Redis maxmemory setting, and requires an err result with both old
and new increments retained and a correct restart. Removing post-S1 pressure
abort makes this strict case complete successfully and fail its err assertion.

`reconciled_scan_post_cut_backlog_charges_rotation_reserve` builds a long
old increment from overwritten 32-KiB values, then waits until INFO confirms the
atomic S1 cut while the old manifest is still selected. Repeated 16-KiB writes keep live
memory bounded while the streaming boundary reader still visits history.
With maxmemory at 64 MiB, the rotation budget must abort before publication;
subsequent commands and a restart must retain the acknowledged value and
counter. Removing LogBudget charging leaves the excluded backlog unbounded
by R and allows a successful rewrite, failing the required err outcome.

### Stage 2 source map

| Protocol step | Source anchor |
| --- | --- |
| Reserve/headroom, startup/exhaustion checks and atomic S0 admission | `aof_rw_admit` in `firn/persistence/rewrite.wf` |
| Per-statement token, key stamps and same-hold publication | `sequence_new`, `sequence_commit`, `sequence_stamp_key` in `firn/store/sequence.wf` |
| Complete after-images/tombstones, coalescing, precharged journal chunks | `sequence_capture`, `rewrite_journal_room`, `sequence_publish` in `firn/store/rewrite-capture.wf` |
| Shared five-type reconstruction and 64-element collection commands | `rewrite_header`, `rewrite_entry`, `rewrite_image` in `firn/store/rewrite-image.wf` |
| Whole-map-plus-Meta count-1 observation, returned KeySet charge, owned output | `aof_rw_dataset` in `firn/persistence/rewrite-dataset.wf` |
| Genuine wait after each step and each released chunk | `aof_rw_pause`, `aof_rw_worker`, `aof_rw_discard` in `firn/persistence/rewrite.wf` |
| Atomic S1 counter/capture/journal/collecting-buffer cut | `aof_rw_cut` in `firn/persistence/rewrite.wf` |
| Drain sealed bytes without swapping, then sync/rotate with old files named | `aof_rw_drain`, `aof_rw_switch`, `aof_rw_cycle` in `firn/persistence/rewrite.wf` |
| Streaming syntax-only validation and shared block classification | `aof_rw_boundary`, `aof_block_command` in `firn/persistence/rewrite-boundary.wf`; `parse_request_stream` in `firn/protocol/stream.wf` |
| Charge rotation backlog before grow, commit overflow without failing clients | `log_reserve`, `log_budget_begin` in `firn/store/store.wf`; `sequence_commit` |
| Ordered reconciliation; sync, rename and manifest publication | `aof_rw_reconcile` in `firn/persistence/rewrite-dataset.wf`; `aof_rw_worker`, `aof_rw_install` |
| Reserve/flush/exhaustion abort and bounded release | `rewrite_abort`, `sequence_publish`; `flush_body` in `firn/commands/server.wf`; `aof_rw_worker`, `aof_rw_discard` |
| Pressure cancellation before installation; eviction waits after commitment | `memory_over`, `evict_before` in `firn/commands/eviction.wf`; `aof_rw_cycle` |
| Switch-time cut before a startup-retained MULTI | `aof_rw_boundary` in `firn/persistence/rewrite-boundary.wf`; `aof_rw_switch`; `firn_rewrite_cuts_a_retained_unfinished_block_before_switching` in `tests/network.rs` |
| Final status and automatic-retry suppression | `aof_rw_complete` in `firn/persistence/rewrite.wf`; `write_log` in `firn/persistence/persistence.wf` |

### Stage 2 validation and review (2026-10-10)

The gate passes on the stage-2 commits: CI run
[38057920387](https://github.com/Ming-Research/Firn-wf/actions/runs/38057920387)
at `302ce4b` builds firn with the pinned compiler, passes 150 network cases
with one ignored (the exact-reserve long-key case above), keeps the Redis
7.0.15 suite ratchet at 603/603 required tests and passes design lint. The
mutation controls described above are reasoned, not executed.

Reaching that run took compiler-driven repairs, none of which changes the
protocol: a binding renamed from the reserved word `copy` (FORM-3); an image
consumed on every path when the journal has no room (LIV-1); bare blocks,
which the grammar lacks, replaced by moving buffers into a consuming function
before each pause or refund (GRAM-5); a copy result read through its fields
because destructuring requires `move`, which a copy value refuses (OWN-1,
GRAM-4); the boundary reader stopping when growth leaves no room to read,
as the loader does; append loops testing their exit before each append; and
the reconcile append's end written as a clamp the prover can use (FN-8). The
second and fifth expose Whitefoot questions now on the status board
(`proof-bl-scoped-release`, `proof-bl-copy-destructure`, and a witness on
`coord-wfbl-08-10` for the two-premise limit of automatic affine derivation).

Two scan cases were corrected to stay within the reference. The large-history
case wrote inline `MULTI`/`EXEC` lines into the AOF, which Redis 7.0.15's
loader refuses (`src/aof.c` accepts only RESP arrays and `#` lines); it now
writes mixed-case RESP commands. The post-cut backlog case's 128-MiB history
crossed the automatic-rewrite threshold, which Redis also applies, so an
automatic rewrite could race its BGREWRITEAOF; it raises the minimum.

A read-only completion review (Claude Opus 5.5) covered `15293c5..302ce4b`
against the project checklist and the design and correspondence checks. Its
findings and their dispositions:

- Automatic-retry suppression applies to every unsuccessful attempt, while
  the capture node described only budget exhaustion. The owner's ruling A on
  `firn-q-rw-limits` excludes automatic retry, so the node now states the
  broader rule and its difference from Redis's `aofRewriteLimited` backoff.
- The per-statement and per-step limits (1024 visits, 64 KiB) made any key
  above 512 elements or about 64 KiB serialized abort every rewrite, where
  main's replay and Redis rewrite such keys. On status board card
  `firn-q-rw-bigkey` the recommended option A is implemented pending the
  owner's ruling: keys are copied whole, limited only by R, at the cost of a
  pause proportional to their size; bounding that pause by splitting a key
  needs resumable HashMap iteration (board item `coord-wfbl-02-01`). The new
  case `reconciled_scan_rewrites_large_keys_scanned_and_written` fails under
  the old ceiling, and the concurrent growing-writes case again overlaps
  the whole rewrite as on main.
- A startup-retained unfinished `MULTI` refused the rewrite, so writes
  appended after it were lost at restart, where Redis's live rewrite keeps
  them. On status board card `firn-q-rw-startup-multi` the recommended
  option A is implemented pending the owner's ruling: the sealed file is cut
  before that MULTI at S1 and base(S1) keeps the live writes.
- Stale status text in this record, the investigation index, the stage-1
  record and the firn README is corrected.
- Transfer and store commands (LMOVE, SMOVE, SUNIONSTORE) now run in the
  restore case's concurrent writer, so a dropped capture at a helper call
  site fails the independent dump comparison.
- `aof_rewrite_commit_seq` is renamed `firn_aof_rewrite_commit_seq`, as
  firn-only fields are.
- Design wording that needed the board to understand, and progress text in
  a node, are rewritten.
- The boundary reader charged its window once even when no increment
  matched, and a growth charge could skip its refund on one break; it now
  charges per selected part and counts growth before that break.

Unverified: restoring the new base into Redis 7.0.15 itself (the registered
oracle uses firn's restart and an independent dump); crashes after S1
(during the sealed drain, mid-reconciliation, between base rename and
manifest publication); whether the post-S1 pressure case can race
installation; capture across all mutating commands rather than the covered
set; and cleanup time under stalled host I/O.
