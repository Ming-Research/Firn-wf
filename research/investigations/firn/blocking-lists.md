# Blocking lists: implementation and remaining acceptance

This is the implementation study for `firn-scope-blocking-lists`, based on
Redis 7.2.16 `src/t_list.c`, `src/blocked.c`, `src/timeout.c`, `src/db.c`,
`src/networking.c`, `src/server.c` and its list/scripting tests. Sources were
read from the pinned reference cache, not executed on the development machine.
The proposed decisions are in [blocking lists](../../../design/firn/blocking-lists.md).
Compilation, reference recordings and runtime acceptance remain CI work.

## Service boundary

BLPOP/BRPOP reuse the existing pop body while preserving original operand
order, including duplicates, and propagate LPOP/RPOP with one selected key.
BRPOPLPUSH/BLMOVE reuse the move body and propagate RPOPLPUSH/LMOVE without
the timeout. A missing move source leaves the destination unexamined; a wrong
destination after a ready source does not consume that source.

A command first tries under one whole-map/Meta/hub/server-state hold. If no
source is available it registers before releasing the same hold. Each
source key has a FIFO of registration ids. A ready pass considers at most
its initial FIFO length; it removes every membership before retrying the
whole original command. An unsuccessful retry retains its original deadline
and generation. Successful moves append newly ready destinations after the
already queued keys. No connection wakes to compete for the value itself.

Potential producers run existing held command parts. Only actual Redis
`dbAdd`-equivalent list creation stages a ready key: new LPUSH/RPUSH lists,
new move destinations and list-valued RENAME/COPY targets. LPUSHX/RPUSHX and
LINSERT enter the producer boundary but do not create a missing list. SET,
DEL and type-changing replacement do not wake list requests. MOVE, RESTORE,
SORT and SWAPDB have no command implementation in this base; future support
needs its own exact dbAdd/dbSetValue analysis, not a blanket write hook.

EXEC and scripts collect readiness until their final effects and response
are complete. A push followed by DEL therefore serves nobody. A declared-key
script requests whole-map retry before executing a producer or blocking-list
command; abandoned attempts discard readiness. An accepted Lua error retains
preceding mutations and services them. In EXEC/scripts these four commands
never register: empty pops use a RESP2 nil array and empty moves use nil bulk;
actual timeout/UNBLOCK TIMEOUT uses nil array for all four. RESP3 uses null.
Recorded expectation provenance is cited in the network fixture; uncovered
transcripts remain pending rather than presenting these deductions as observed
bytes.

The producer's promoted reply and staged publications commit before ready
replies, under the same hold that protects visible dataset changes. Each
resumed request then gets a fresh wall-clock snapshot, refreshed memory
settings, its own sequence and its own AOF records after any producer
MULTI/EXEC wrapper. Its reply commits before another owner can execute a
causally dependent command. A nonpromoted producer retains its existing
private reply buffer; receipt order across different sockets is not promised.

## Ownership

The existing pub/sub hub now owns blocked requests as well. First suspension
promotes only that connection to independent receive, command and output
owners. Its original command owner retains authentication, pipeline and MULTI
state and its original Client. The stored request owns arguments and a small
execution continuation; it does not borrow the parser's input or retain a
second Client. Actual service creates a temporary execution/reply adapter. The owned `blocked` flag causes only
suspended command owners to perform the guarded hub wait. After completion,
buffered pipeline commands resume without another network read, restoring
updated random/peak state before parsing, with fresh time and settings. A
RESP3 subscriber continues receiving pushes while blocked; RESP2 subscriber admission retains Redis's existing command ban.

Registration removal and reply commitment are one hub statement. Timeout,
CLIENT UNBLOCK and ready service compete for that same pending request.
Disconnect removes all keys and finite timer membership before cancellation;
if service already won, later EOF does not undo its mutation. A new wait has
a monotonically allocated u64 generation, never reused within the supported
process bound of fewer than 2^64 waits. Finite timers use an indexed heap;
completion removes the entry rather than retaining an abandoned deadline.

A narrow projection contains only registration ids and counts for guarded
waits; the guard does not observe execution scratch or completion state.
Blocked clients are exempt from configured idle timeout. Pub/sub output
limits still depend on actual subscription membership, so blocking alone
does not activate them. Existing independent output owners isolate slow peers.
INFO counts pending clients, finite timers and distinct keys; the
`total_blocking_keys_on_nokey` deletion-wait index remains zero for list waits.
CLIENT UNBLOCK uses the existing network-client-id index, not a registry scan.
Its promoted requester commits the confirmation before releasing the hub,
so the released target cannot publish ahead of that confirmation. Unlike
PING, this control observes and changes another connection's pending state.

## Request and continuation ownership

The request-only design is proposed on ownership and lifetime grounds, not
as a substitute for Whitefoot's heap support. Redis 7.2.16 keeps its client
in place and reprocesses it (`blocked.c`, `handleClientsBlockedOnKey`,
`unblockClientOnKey` and `processUnblockedClients`); Firn's original command
owner likewise remains the owner of its session and parser. The previous `blocking_snapshot` was
not a complete session copy: it created `client_new(128)` and copied id,
registration id, protocol, random, authentication/lock flags, static settings
and a meter share. Several copied fields were never used while serving.

The following audit covers all 38 Client fields transitively reached by
`blocking_try`, `evict_held`, access tracking, persistence refusal and reply
commit, plus timeout/CLIENT UNBLOCK. R/W describes service use, not default
initialization by `client_new`.

| Client fields | Actual service use | Required owner and lifetime |
|---|---|---|
| `bytes`, `at` | R/W by reply writers, AOF error text and `reply_frame`; commit clears `at`. | Temporary serving reply workspace, copied to the hub's immutable output frame before it is released. There is no reply while pending. |
| `now` | Fresh write; expiry/classification/access/eviction read. | Temporary service time; never the initial wait's time. |
| `state`, `number`, `low`, `high` | R/W pop/move result, count and direction scratch. Move direction is reparsed from original argv. | Temporary serving adapter. |
| `list_ready` | R/W successful move's readiness; drained into producer queue. | Temporary effects staging, merged before the producer hold exits. |
| `random` | R/W eviction selection and LFU increments. | Connection execution continuation: transfer scalar to pending request, update on every retry, return to suspended owner on completion. |
| `access_ms` | Fresh write by `prepare_access`; LRU/LFU read. | Temporary service clock snapshot. |
| `memory_policy`, `maxmemory`, `maxmemory_samples`, `maxmemory_tenacity`, `lfu_log_factor`, `lfu_decay_time` | Fresh writes from held ServerState; eviction/access read. | Temporary current admission configuration, not copied startup configuration or the producer's possibly older per-read snapshot. |
| `meter` | Heap observation and map-reserve release write the handle. | Temporary execution capability shared from producer; it measures the entire process, not this client. |
| `pubsub_id` | Registry/output recipient lookup. | Pending request's connection id; existing hub Record owns lifecycle. This is not public CLIENT ID. |
| `resp` | Reply commitment and timeout/null selection read. | Pending request's reply protocol. HELLO cannot execute on that connection until its blocked command completes. |
| `seen_peak` | R/W finite-memory admission's monotone optimization cache. | Scalar continuation; merge with original owner's cache by maximum on completion. Authoritative peak remains Meta-owned. |
| `unix_us` | Previously written but not read in this chain; output receives local seconds separately. | Original owner; removed the redundant temporary assignment. |
| `id`, `authenticated`, `locked`, `settings` | Previously copied but not read. Static settings are not the mutable memory settings above. | Suspended owner; CLIENT UNBLOCK identity already lives in the existing hub client-id index. |
| `items`, `scores`, `key_metadata`, `key_lookups`, `keys` | Not read or written by this service chain. | Original owner's general scratch. Existing Client-based bodies still require a temporary adapter whose unused scratch is default-initialized. |
| `blocked`, `pre_command_oom`, `sub`, `psub`, `ssub`, `name`, `closing`, `since_us` | Not read or written by this service chain. | Existing owner/session and hub membership. Original `blocked` controls suspension; copied/default subscriber counters or names are not authoritative INFO data. |

Supporting implementations are `commands/blocking-runtime.wf`,
`commands/lists.wf` (pop/move), `commands/eviction.wf`,
`commands/access.wf`, `commands/admission.wf`, `store/store.wf`
(`access_settings`), `protocol/protocol.wf` (reply encoders) and
`connection_events/frames.wf` (`commit`, `reply_frame`). Firn currently
accepts only database zero (`commands/connection.wf`, SELECT), so the
request's database is implicit. Multiple database or ACL support would
reopen execution context requirements.

Pinned Whitefoot PRE-2 and `std::process::meter_share` specify observations
of the same execution process. Every network producer has a share from the
startup meter; replay runs before there can be network waiters. Actual
retry therefore obtains a short-lived share from that producer's Client,
loads the target connection's random/peak scalars, and refreshes time and
memory settings from the same held ServerState. It neither attributes heap
bytes to the producer nor substitutes its random stream or cached settings.
Eviction and persistence admission remain unchanged. The meter is not
retained in the pending map because no service runs during that lifetime.

Failed retries put updated random/peak scalars back in the same generation's
request. Successful/error retries commit the reply and a `BlockResume`
scalar result in the existing live connection Record under the same joint
keys/Meta/hub hold. Timeout/UNBLOCK create a short-lived reply adapter with
the request's protocol, identity and continuation, reuse the shared
`reply_nil_array`/`reply_error` encoders, and commit under the hub; they do
not run eviction or retain a meter. Pressure checks run before storing
completion, so a removed Record is never recreated. EOF/shutdown/removal drops pending and completed
state. The command owner atomically takes and clears its completion before
parsing any pipeline suffix and restores random plus the maximum peak cache.
It cannot register its next generation before that take; no asynchronous
service retains a request after releasing the hub, so a late completion
cannot overwrite a later wait. Pending removal and completion publication
are invisible to other owners until the same statement exits.

The previous snapshot advanced its private random state during LFU or
eviction but dropped it on final completion; the original owner resumed its
pre-wait stream. This violated Firn's per-connection continuation ownership,
although no Redis PRNG parity or externally measured failure is claimed.
The previous scratch also started peak cache at zero. Returning its maximum
preserves the optimization without changing the authoritative Meta peak.

The old suspended adapter retained a 128-byte reply window, 16 Key slots,
16 f64 slots, 16 u64 metadata slots and deep-copied static settings alongside
the actual connection Client. Request-only storage avoids this duplicate
waiting-lifetime workspace. The scalar completion slot adds fixed storage to
every promoted Record, including subscribers that never block; it adds no
field to the ordinary connection Client. Per-attempt adapters still allocate
general scratch on each retry and terminal timeout/UNBLOCK; this trade has
not been timed and can change actual maxmemory observations. Completion
continues using the shared protocol writers, as required by the
[protocol decision](../../../design/firn/protocol-versions.md), rather than
duplicating RESP selection and null/error bytes in the hub. Reusable adapters
or a broad narrow-context API split are separate future choices requiring evidence. Ordinary GET/SET
handlers and shared targets remain untouched; they do not inspect pending
requests or completion slots.

`completion-reuse` in the network oracle fixture pipelines a successful
BLPOP, a second wait canceled by CLIENT UNBLOCK ERROR, and PING, then starts
a third successful wait on the same connection. One and four drivers check
serial completion handoff, reply-before-pipeline order and a fresh wait that
stays pending until its own push. Missing or misrouted completion can close
or prematurely resume the owner, skip a reply, or emit output before the next
push. Exactly-once slot clearing is established by source inspection; the
case alone cannot detect omission of clearing on take because registration
also clears any stale completion. This is coverage for the new handoff
protocol, not a claimed failure on the old implementation's private PRNG
loss; no deterministic network PRNG probe or test-specific execution path is
introduced. Runtime random-continuation evidence remains unverified. Its exact Redis 7.2.16
bytes now come from the remaining recording; any fixed bytes without that
evidence would stay ORACLE-pending.

## Admission cost

Replay must perform eviction and persistence admission again: a producer can
allocate past maxmemory after its own admission. The nonwaiting held adapter
uses the existing eviction sampling/pool, heap checks, DEL propagation and
sequence rules. BLPOP/BRPOP remain WRITE without DENYOOM; moves are DENYOOM.

When maxmemory is finite and a pending waiter exists, a producer/EXEC/whole
script waits for an active AOF rewrite to finish before entering its full
hold. The guarded wait releases all targets, and held Meta then prevents a
new rewrite beginning during service. This avoids waiting for asynchronous
rewrite cleanup while holding the metadata cleanup needs. The alternative of
skipping replay admission changes observable refusals. The selected delay can
last as long as the rewrite and remains a proposed performance commitment.

The whole-map runtime retains selected deleted map nodes and key storage
until the hold exits. Payloads can be freed sooner, and logical map counts
reflect deletion, but actual heap measurements can include that temporary
storage. Long keys with small values can therefore cause extra evictions or
OOM during held replay. This is source-derived, unmeasured risk, not a proven
Whitefoot defect. A CI witness should compare ordinary admission with held
service under finite maxmemory, long keys, small values and a long FIFO.
No estimated subtraction or unlock/relock workaround conceals this cost.

Ordinary connections issuing GET/SET retain receive/execute/send, their
existing handlers and shared target sets. They perform no blocked-key lookup,
readiness staging or new per-command hub access. Setup adds two owned Client
fields and a retained WallClock handle to Keyspace; source inspection cannot
prove these have zero cache/layout cost. Use the existing
[performance experiment](connection-waits.md#performance-experiment-specified-before-implementation)
to test the no-measurable-regression requirement. List producers deliberately
pay whole-map/Meta/hub/server-state exclusion even with zero waiters.
Current per-key FIFOs use the existing growing owned membership arrays; removal
shifts the tail, so serving N clients on one key can require quadratic array
movement. The private ready queue also deduplicates linearly. The FIFO-load
experiment must expose this cost; no scalable-throughput result is claimed.
If it dominates, replace those internal queues with indexed links while
retaining the same atomic allocation boundary, rather than weakening FIFO.

## Numeric counterexample

`firn-req-extended-float` blocks complete timeout conversion at Whitefoot pin
`wf-af7665be672d`. Redis parses `long double`, multiplies by 1000 with extended
rounding, applies `ceill`, checks signed range and adds its time snapshot.
The parser accepts decimal/hex syntax and distinguishes parse overflow,
underflow to zero, nonzero subnormals and explicit infinities. It checks
negative time after ceiling: `-0.0001` seconds becomes zero, the infinite wait.

Exact decimal ceiling is not equivalent. For `1.00000000000000000001`, the
exact difference from 1 is 10^-20, less than half an extended ULP, 2^-64.
Correctly rounded extended parsing gives exactly 1; multiplication gives
1000 and ceiling gives 1000 ms. Exact decimal multiplication/ceiling gives
1001 ms. Conversely `1.000000000000000001` rounds to 1 in binary64, producing
1000 ms, while the sum of extended parse and multiplication error is less
than its 10^-15 ms excess over 1000, producing 1001 ms. These are deductions
under the reference's round-to-nearest extended format, not executed results.

The minimal requested numeric witness and unavailable type are shown in
[the capability audit](connection-waits.md#missing-capability-exact-reference-timeout-numbers).
Whitefoot needs a length-delimited extended parser with range/classification
information, separately rounded extended multiplication, ceiling and checked
integer conversion. A Firn-local software replacement would hide that gap.
`blocking_timeout` therefore accepts only the canonical byte `0`; every other
spelling returns an explicit incomplete-capability error citing the request.
This includes alternate zero spellings and finite timeouts on nonempty keys.
It is not a claim of Redis compatibility for those inputs.

Timer storage preserves absolute millisecond deadlines and expires only at
`deadline < now`. One-shot monotonic waits recheck wall time at wake. A forward
wall-clock jump during otherwise idle sleep cannot be detected promptly at
this pin; the [minimal requested host operation](connection-waits.md#missing-host-capability-for-strict-clock-jump-parity)
remains missing. No periodic polling was added. Full signed-millisecond range
conversion and clock-jump acceptance must be settled with the timeout gap.

## Acceptance instruments

`tests/blocking_lists.rs` is included by the existing network target. Its
named-client scenarios in `tests/blocking_lists_oracle.rs` run on one and four
Whitefoot drivers. Exact covered wire bytes and selected INFO fields now cite
Redis 7.2.16 runs [38453128755](https://github.com/Ming-Research/Firn-wf/actions/runs/38453128755)
and [38453804021](https://github.com/Ming-Research/Firn-wf/actions/runs/38453804021)
by batch and operation/frame or command index. The 144 expectations
that were `ORACLE-pending` now come from a 20-batch recording of the same
Redis 7.2.16 (all batches succeeded) and no placeholder remains; the
preflight assertion stays so that a future pending value stops its case before
a server starts. Numeric timeout cases are explicitly ignored with
`firn-req-extended-float`, as the owner authorized, because the network gate
does not tolerate deliberate failures. Their Redis expectations are retained,
never changed to Firn's capability error. Independent arity/direction cases
remain active rather than being hidden by the numeric deferral.

The extended `oracle/replay.py` on `claude/oracle-replies` now accepts ordered
named-client `operations`, including open/send/pipeline/read/quiet/poll_info,
scalar identity capture, AOF and restart. Remaining recording batches use
that exact format outside the repository. Its earlier single-connection
`commands` format is still supported. A failed batch supplies no expectation;
notably the finite-negative correction batches polled for a blocked count
instead of reading their immediate error, so they have no client-error frame
and need corrected send/read recordings. The successful `-0.001`/`-0.0005`
recordings show infinite waits until a push; they cannot supply an immediate
negative-timeout error or be used as synchronous commands on missing keys. The fixture actions are the maintained multi-client source:
Send parks, WaitBlocked uses an INFO barrier, Read collects the recorded frame,
Quiet checks no output, and Close exercises cleanup. Test-harness observation
polling is a registration barrier; production scheduling never polls.


## Target layout of heap-resident host handles

The owner supplied CI run 38453196432 for revision
`f28e683aa93e48dffa5ea6af9e11f4c55498dd12`: source checking accepts both
server builds, then code generation reports
`TargetLayout(Unrepresentable(RuntimeSizedAllocation))` without a source
location. The pub/sub base `6b3bc20` builds. No compiler ran locally.

The source-derived culprit in that revision was
`hash_map_new::<u64, BlockedList, extent>` in `connection_events/owners.wf`.
In that revision its bucket was `HashMapSlot<u64, BlockedList>`, storing a
second execution Client, including `Option<std::process::MemoryMeter>`.
The pinned compiler represents an opaque host value as 32 bytes aligned to
16 (`compiler/src/target.rs:1730–1734`); enum and struct layout retain the
largest field alignment. The standard HashMap allocates its buckets with
`box_slots_new`, so the bucket's 16-byte alignment reaches
`runtime_capacity_layout`. The x86-64 Linux target declares ordinary heap
alignment of only 8 (`target.rs:99–105`); its check at 1248–1251 produces
exactly the reported failure. The base Client already contained the meter,
but that revision's new registry put the Client in ordinary heap element storage.
The error label names an allocation category; the specific problem here is
alignment, not a value whose size is unknown where fixed storage is required.

This is a legal source form under TYPE-9: MemoryMeter is not placement
restricted and `nocopy` does not forbid dropping or storing it. STOR-6
explicitly separates source acceptance from target qualification and allows
the latter to stop without a language-rule citation. No checker rejection
has been demonstrated missing, and DIAG-1 does not require a source coordinate
for this failure category. A diagnostic naming the allocation, instantiated
element type and alignment mismatch would make it actionable.

The opaque alignment is real in this pin: LLVM emits `{ i128, i128 }`
(`backend/emitter.rs:3603–3604`) and the C host ABI uses
`alignas(16) uint64_t words[4]` (`backend/ordinary_values.h:15`). The ordinary
heap returns malloc/realloc pointers without shifting them
(`backend/heap.c:25–58`). Source inspection establishes the declared 8-byte
minimum, not that the supported platform allocator actually cannot supply
16. Whitefoot must establish the allocator guarantees on each supported ABI
or change representation/allocation consistently; Firn does not substitute a
second representation to conceal the target-support gap. Capacity zero also
uses a static 64-aligned empty header, yet the current qualifier checks the
element alignment independently of capacity.

The full type/allocation audit found no new bare runtime-capacity field,
recursive inline type, Shared inside Frozen, or unsupported by-value held
map. Boxed runtime buffers keep fixed pointer fields. Map ceiling `extent`
is not an inline allocation bound, and the base already uses that ceiling.
Other new ordinary heap elements (keys, spans, ids and deadline records) have
alignment at most 8. Adding WallClock changes Keyspace's inline alignment,
but does not introduce another new ordinary heap element allocation in this
diff. These exclusions and attribution were initially static deductions. The owner
subsequently supplied [witness CI run 38455091065](https://github.com/Ming-Research/Firn-wf/actions/runs/38455091065):
all six witnesses checked successfully; MemoryMeter in Box, nonempty/empty
Slots and HashMap failed code generation with RuntimeSizedAllocation, while
by-value MemoryMeter and u64 Slots compiled. The owner filed
`firn-req-heap-overaligned` at P0; Firn does not claim to fix that legal-form
support gap by choosing different ownership.

The following minimal instruments live under
`research/experiments/blocking-lists/` and were compiled, never executed, by
the temporary `blocking-layout-witness.yml` workflow:

| Witness | Owner-supplied result from run 38455091065 |
|---|---|
| `u64-slots-control.wf` | Source check and code generation succeed. |
| `memory-meter-value-control.wf` | By-value Option<MemoryMeter> succeeds without an ordinary heap allocation. |
| `memory-meter-slots.wf` | Source accepts; capacity-one MemoryMeter window fails target allocator alignment. |
| `memory-meter-empty-slots.wf` | Source accepts; capacity-zero window reaches the same conservative qualification. |
| `memory-meter-map.wf` | Source accepts; HashMap<u64, Option<MemoryMeter>, 8> reproduces the registry composition. |
| `memory-meter-box.wf` | Source accepts; even a fixed-size Box<Option<MemoryMeter>> fails BoxNew's ordinary allocator alignment check. |

The workflow ran `--check` and `-o` separately for every witness, printed and
retained their direct statuses and output even after failures, and uploaded
provenance and logs. It also searched the downloaded language archive for
the diagnostic spelling; local investigation read the exact pinned Git
commit because no downloaded release language directory was present.
Expected failures keep the temporary job red after all attempts; this job is
an instrument for filing and verifying the Whitefoot gap, not compatibility
acceptance. It was retained until the gap was resolved or Firn's independently chosen
ownership design no longer needed the unsupported allocation. The owner
subsequently supplied [CI run 38457609785](https://github.com/Ming-Research/Firn-wf/actions/runs/38457609785)
for request-only revision `33ca7bcccf4e1788992e5cec911dbdf44e550076`: both server
builds compile, so this ownership design no longer needs the unsupported
heap allocation and the temporary workflow is removed. The six witnesses
stay as evidence for the Whitefoot request even if this server no
longer needs heap-resident meter storage. The earlier layout investigation
changed no production representation or pin; the separate proposed ownership
change above is grounded in request/session lifetimes and scalar continuity,
not target qualification. No compiler was run locally.

## Failed-fixture isolation

Owner-supplied run 38457609785 compiled request-only revision `33ca7bc`
and reported 26 candidate list tests. Its `tests.tsv` contains exactly one
passed row for each candidate; all 26 are added to the sorted ratchet, raising
the required named tests from 670 to 696. No required or unstable row is
removed. The run also reported but the existing RPOPLPUSH
quicklist-source/existing-quicklist-target case lost its source `a b`.
The source trace explains exactly those two missing values without a new
pop/move mutation defect: Redis 7.2.16 `tests/unit/type/list.tcl`'s preceding
“PUSH resulting from BRPOPLPUSH affect WATCH” and “BRPOPLPUSH does not affect
WATCH while still blocked” each first register an infinite BRPOPLPUSH on
`srclist{t}` and only then read WATCH on another connection. Firn does not
implement WATCH outside MULTI. `tests/support/redis.tcl` converts its error
reply to a Tcl exception; durable `tests/support/test.tcl` catches the body
exception and continues, skipping both auxiliary clients' close calls.

The later fixture separately pushes `a`, `b`, `c`, and a large value into
that source. Two surviving blocked clients can consume the first two values;
the fixture's explicit moves then return the large value and `c`, leaving
an empty source. Both entry paths still use the same `lmove_body`, and the
removed-source test uses the remaining length from `take_element`. This is
a contamination diagnosis supported by both source and the supplied artifact:
`shard-1/logs/unit/type/list.1.log` reports the two WATCH read errors before
the later missing-source assertion, and `tests.tsv` classifies both as
unknown-command errors. The repaired full-unit run must still show the
retained source. No standalone list-body bug is established by this trace.

The proposed fix gives those two fixtures exception-safe closure, preserving
the body result and the complete Tcl error options and leaving the unsupported
WATCH errors visible. Both closes are followed by a bounded INFO barrier
waiting for the fixture's original blocked-client count before returning or
rethrowing. Original body errors keep their Tcl options and priority even
when cleanup also fails; extra barrier failure is separately reported. It
runs on every target, changes no expected response, adds no Firn-only path
and excludes no test. A native contextual patch check
rejects stale/missing/repeated input. Stubbed Tcl exercises success and failure,
both closures and preservation of the original error; deliberately missing
cleanup must fail this check. These instruments run through the existing suite
runner in CI only. Socket close is local teardown, so the INFO barrier supplies the observable
server-side registration-removal boundary. The subsequent full-unit result
remains required runtime evidence that the abandoned waiters no longer
consume setup values; it does not claim every socket owner has joined.

## Concurrent rewrite restart

Gate run 38463967822 on `40257cb` located the network failure of the
exactly-once rewrite test: the first load of 16,000 SET records connected,
the rewrite finished with status ok, and the restart then did not accept a
connection within the existing 20-second limit. Before that restart the new
base file held 1,057,150,053 bytes, while its dataset is about 9 MB of SET
records, one string of at most 246,052 bytes and one list of at most 148,696
one-byte elements.

The cause is the shared producer path. LPUSH, RPUSH, LPUSHX, RPUSHX and
LINSERT previously opened their statement with the one-key write sequence,
which the AOF rewrite decision relies on: a later write to a key already
journaled in the rewrite window journals only its own append-only bytes. The
blocking-list change routed every list producer through one whole-map hold
that opened the general sequence, so each push during the rewrite window
journaled another complete image of the growing list, and the reconciled
base grew with pushes times list length. Restart then replays that base
before it listens. The fix keeps the whole-map hold and restores the one-key
sequence for those five commands; RENAME, RENAMENX, COPY, LMOVE and
RPOPLPUSH can change two keys and keep the general sequence, as their former
paths did. A network case pushes 200 times onto a 20,032-element list inside
a rewrite window and bounds the rewritten base at 12 MiB; one image per push
would add about 28 MB to its 9 MB dataset.

## Contaminated zero-timeout passes

Two of the 26 rows added from run 38457609785, “BLPOP: with zero timeout
should block indefinitely” and its BRPOP twin, passed there only because of
the leaked WATCH-fixture waiters described above: their log lines precede the
BLPOP block and the waiters lived until the later quicklist case, so the
INFO barrier `wait_for_blocked_client` (blocked_clients non-zero) held
before the test's own command. In Redis 7.2.16 the preceding
“with non-integer timeout” case blocks on `blist1` with a 0.1-second
timeout and its RPUSH serves that waiter, leaving `blist1` absent. Firn
refuses finite timeouts until firn-req-extended-float, so the RPUSH leaves
`foo` in `blist1`; the zero-timeout BLPOP then returns at once and never
blocks. With the leaked waiters cleaned up, run 38463967822 shows the true
outcome, “no blocked clients”. Neither is a failure to block or to count a
blocked client; both follow from the deliberate timeout deferral, and their
ratchet rows were added on contaminated evidence. Both rows are removed from
`passing.tsv`; they return when firn-req-extended-float lets the preceding
non-integer-timeout case serve its waiter.
