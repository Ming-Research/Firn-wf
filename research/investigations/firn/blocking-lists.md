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

Producers that create a list where a client waits, RENAME, RENAMENX and COPY,
and every list write of a promoted connection, run the existing held command
parts, which stage the ready key and serve the waiters before releasing their
hold; every other list write holds only its keys and stages nothing (see
[producer holds](#producer-holds)). Only actual Redis
`dbAdd`-equivalent list creation stages a ready key: new LPUSH/RPUSH lists,
new move destinations and list-valued RENAME/COPY targets. LPUSHX/RPUSHX and
LINSERT do not create a missing list. SET,
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

## Producer holds

The first version ran every top-level list write through the held command
parts under the whole-keyspace, Meta, event-hub and server-state hold, as
`run_list_producer` does. Measured on the 14900K with two server CPUs and the
append-only file off ([the record](blocking-lists-performance.md#a1-cost-on-existing-commands-append-only-file-off)),
it ran LPUSH, RPUSH, LINSERT and LMOVE at 0.31 of the rate of main at pipeline
depth 16 and at 0.83 to 0.88 at depth 1; about half the cycles were in the
runtime's whole-keyspace hold, and the allocation and copy of the staged
effects were each under 1%. The second version held it only for the writes
that can create a list, which returned the pushes and moves onto a live list to
0.94 to 1.00 of main and left a push that creates its list at 0.313 (depth 16)
and 0.801 (depth 1), with 38.8% of its cycles in the hold ([the second
run](blocking-lists-performance.md#a1-again-the-narrowed-hold-7a4e4de)). The
third version took only the entry, the event hub and Meta, in a second
statement, to ask the registry; it ran that push at 0.857 and 0.960, and 0.931
and 0.972 with the append-only file on, with about 12% of the cycles (estimated
from one profile) in the second statement's lock operations ([the third
run](blocking-lists-performance.md#the-third-version-2d91961)). The cost is
the locks, so a write needs them only when it can make a client ready:

A list write that cannot make a client ready does not need that hold:

- A request registers under the whole hold only when its failed lookup finds
  no live list at any key it names, and it is served when a list is created at
  one of them (Redis `dbAdd`). SET, deletion and a replacement by another kind
  neither serve nor remove it. So no key that holds a live list has a
  registered request once a hold is released: the producer that created the
  list served it before releasing.
- A write onto a live list therefore readies nobody. An entry hold excludes the
  whole hold, so a registration and such a write on one key never overlap.
- LINSERT, LPUSHX and RPUSHX never create a list and always run holding their
  key's entry and Meta. LPUSH and RPUSH do so when the entry holds a live list
  (`list_kind` is `entry_wanted`); LMOVE and RPOPLPUSH unless the source holds a
  live list and the destination, another key, holds none (`lmove_creates`). A
  destination holding another kind is refused and creates nothing.
- A creation where no client waits readies nobody either, and a registration
  cannot appear while the creating write holds the entry of its key and Meta,
  so the write only has to learn whether a client waits on that key. Meta keeps
  `waiter_keys_bound`, an upper bound on the number of keys blocked clients wait
  on: every registration (the two places that install a request, both under the
  whole keyspace, Meta and the hub) and every second statement below set it to
  the registry's count (`note_waiters` in `firn/commands/blocking-runtime.wf`),
  and no end of a wait lowers it, so it is never below the count. The creating
  write reads it in its first statement, which holds its entries and Meta: at
  zero no client waits on any key, and it creates the list and appends in
  place, as main does, in that statement. Otherwise it leaves for a second
  statement that takes the event hub as well, in the order of the whole-hold
  path (keyspace, Meta, hub), sets the bound to the count and asks
  `key_awaited` (the registry's count of waiting keys first, then a lookup of
  the key). Without a waiter the second statement creates the list and appends
  in place too, so in both cases its effects and bytes are main's, and it
  stages no ready key: only a command that holds the
  whole keyspace drains a connection's staged keys, so a key staged here would
  stay in the connection's queue, one copy per list created, and every staging
  scans the whole queue for a duplicate (`stage_ready` of `push_body` and
  `lmove_body`, set by the held callers only). With a waiter it returns,
  having answered and changed nothing, and the dispatcher runs
  `run_list_producer`, which holds the whole keyspace and serves the client.
  Taking the hub only in the second statement, which a creating write reaches
  only while the bound is not zero, leaves a push onto a live list, and a
  creation while no client is blocked, with the hold of the first version of
  this section: entry and Meta. While any client is blocked on any key, every
  creating write on every key takes the second statement; the bound does not
  say which keys are waited on.
- The read costs a shared lock when the file is off. Main's compiled statement
  takes Meta only on its logging path, so with the file off a push holds its
  entry alone and two server CPUs share no lock (the decision of
  `firn/command-parts` for statements that read nothing else from Meta);
  reading the bound takes Meta's lock for every push that finds no live list.
  Measured at the design placement, a push that creates its list ran at 0.83 of
  main's rate at depth 16 with the file off and 1.01 with it on, and the lock, not
  the entry read of `list_kind` and not the place of the field in Meta, is the
  cost ([measurement](blocking-lists-performance.md#the-fourth-version-2f2ae10)).
  Information about waiters that costs no lock when the file is off would have to
  come from the key's own entry, which a registration would mark; no version of
  this section does that.
- The test is a live list, not an existing entry. A request stays registered
  on a key that a string with an expiry has taken. Once the expiry has passed,
  and before anything removed the entry, a push to that key deletes the string
  and creates a list, which readies the request; an existence test would run
  it holding the entry only and serve nobody. The same holds for an entry
  whose list has expired.
- RENAME, RENAMENX and COPY always create their target and run
  `run_list_producer`; a promoted connection runs every list write there too
  ([ownership](connection-waits.md#ownership-and-delivery)).

The entry-holding run answers nothing before it decides. When it finds that
the command would create a list on a key where a client waits it returns,
having changed no entry; the only state it leaves is the client's prepared
elements, which the held part prepares again. A run that cannot create a list
is the code main ran for these commands (`run_push`, `run_linsert`,
`run_lmove_named`, `run_lmove` in `firn/commands/lists.wf`) with the test added.

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
to test the no-measurable-regression requirement. A list producer that creates
a list on a key where a client waits, and RENAME, RENAMENX and COPY, pay
whole-map/Meta/hub/server-state exclusion; a push or move that creates a list
where nobody waits pays one event-hub hold and one registry check beside its
entries, and the other list writes nothing ([producer holds](#producer-holds)).
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
paths did. (The writes that cannot create a list later went back to their
entries' own hold, where they open the same sequence.) A network case pushes 200 times onto a 20,032-element list inside
a rewrite window and bounds the rewritten base at 12 MiB; one image per push
would add about 28 MB to its 9 MB dataset.

Served blocking commands had the same cost: the immediate attempt and the
waiter retry both opened the general sequence, so every served pop on a list
already journaled in the window journaled another complete image. Journaling
a statement's own bytes is correct exactly when its logged bytes name only
keys it changes and, at the end of the statement, it has changed one key whose
previous write lies inside the window: that key's journal already reproduces
its state before the statement, and the bytes touch nothing else. The capture
code (`firn/store/rewrite-capture.wf`, `sequence_capture`) chooses byte mode at
the first key the statement captures, from that key's anchor (whether its
previous write lies inside the window), and from then on records no image of
that key. The choice is therefore made before the statement knows whether a
second key will change. A second changed key would otherwise enter the
journal only through bytes whose effect on it was never anchored, and the
scanned image of that key may already hold the change, which replay would then
apply twice.

Commit `7cee86a` answered this statically: the statement opened in one-key
mode only when its arguments could name one key, so a BLPOP or BRPOP over
several distinct keys always paid images. The current rule checks it at run
time instead. A statement opened in one-key mode retains a copy of the name of
the first key it captures, charged to its scratch before allocation, and
whether that key is present after its latest change. Every later capture in
the same statement compares its key with that name before it can return on
byte mode; `single` is read nowhere else (only `sequence_capture` reads it,
and `sequence_publish` reads only the resulting effect flag and images), so
the comparison runs before anything is published. On a different key the
statement drops one-key mode and journals complete images of every key it
changed:

- if the first key was captured as an image (it was not anchored), its image
  is already held and the second key's image joins it, exactly as a
  statement opened for several keys does;
- if the first key took byte mode and the statement removed it, its final
  state is absence, whose image (a removal of that name) depends on the name
  alone, so it is rebuilt from the retained copy and the second key's image
  joins it;
- if the first key took byte mode and is still present, its image would have
  to be copied from an entry the statement no longer holds a reference to, so
  capture fails and the rewrite aborts; the restart state stays correct and
  the cost is the rewrite.

Per command:

- BLPOP and BRPOP over any keys: the attempt visits keys in argument order.
  An absent key is left untouched. An expired key it reaches is removed,
  logged as a removal of that key, and the attempt moves on; a wrong-type key
  ends it with an error and no change. The served key is popped, logged as
  `LPOP K` or `RPOP K` without count, as Redis 7.2.16's
  blockingPopGenericCommand propagates it (an emptied list is removed by that
  pop on replay too), and the attempt returns at once. So the logged bytes
  name only changed keys, every key changed before another one is a removed
  one, and the third case above cannot arise. When no earlier key has
  expired only the served key changes and the statement journals its pop
  bytes; this is the Sidekiq and Resque case, one worker blocked on
  `BRPOP q1 q2 ... 0`. When an earlier anchored key has expired, its removal
  takes byte mode, the served key revokes it, and the statement journals the
  removal image and the served list's image, as before the change.
- BRPOPLPUSH and BLMOVE with source equal to destination: the move rotates K,
  and removal of an expired K is the only other change; the logged
  `RPOPLPUSH K K` or `LMOVE K K from to` replays on K alone.
- BRPOPLPUSH and BLMOVE between two keys: they change source and destination
  and their bytes read the source, so they keep complete images, as LMOVE
  and RPOPLPUSH do.

A waiter's retry re-runs its retained original arguments through the same
attempt, after its own eviction and AOF admission and with a statement
opened after them, so its bytes are exactly the attempt's and the same
choice applies. A network case registers 100 BLPOP waiters on an absent
list, installs a 20,000-element list with RENAME inside a delayed rewrite
window, which serves them, then pops 50 times from each end at once and
bounds the rewritten base at 12 MiB; with an image per pop, about 0.25 MB
each, the base would grow by about 49 MB. A second case does the same with
`BRPOP absent1 absent2 queue 0` and `BLPOP absent1 absent2 queue 0`, the
Sidekiq shape, under the same bound; before the run-time check those pops
opened the general sequence and journaled one image each. A third case writes
`early` keys inside the window with an absolute expiry in the past and then
pops with `BRPOP early queue 0` from a list last written before the window,
so each pop removes an anchored expired key before serving an unanchored one;
restart must hold exactly the list the pops left, no `early` key, and the
rewrite must finish ok, which separates the fallback from both journaling the
bytes (the queue's scanned image, taken after the pops, would be popped again
on replay) and aborting the rewrite.

### Limits of the run-time fallback evidence

The scoped review of the run-time fallback named three limits that remain
open:

- `firn_rewrite_falls_back_to_images_when_a_blocking_pop_removes_an_expired_key`
  can detect only one wrong fallback per run, and which one depends on whether
  the rewrite scan reaches `queue` before or after the pops. A fallback that
  published the pop's bytes fails it only when the scan reaches `queue` after
  the pops; a fallback that dropped `queue`'s image fails it only when the scan
  reaches `queue` before them. `EXISTS early:N` does not discriminate, because
  the replayed key is already expired. No counter reports that the fallback
  path ran.
- The abort branch, where a first key still present meets a second key, has no
  test. The callers of `sequence_new_key` were read and none reaches it; it is
  defensive.
- During a rewrite, every byte-journaled one-key statement now copies its key
  name, where it previously allocated nothing. That cost is not measured.

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

### Incremental AOF SELECT

Run 38480614566 matched every recorded AOF byte of the `aof` and
`expiry-before-ready` scenarios except one line: Redis 7.2.16 resets its
selected database when it opens a new incremental file, so the first command
written there is preceded by `SELECT 0`, and firn, which has one database,
writes no SELECT. The lazy SELECT encoder belongs to database selection
(status board item firn-scope-select-db), which is blocked on a Whitefoot gap.
The recorded bytes stay unchanged. The tests compare them as the suite's other
AOF comparisons do: every byte of both sides must parse as complete canonical
command frames, and only `SELECT 0` frames are then ignored, wherever Redis
placed them (the `expiry-before-ready` recording has one inside the `MULTI`).
Placement of `SELECT 0` is covered by the database-selection item.

## Rules main added that reach the blocking commands

Main's `EVAL_RO` and `EVALSHA_RO` take command kinds 199 and 200, so the four
blocking commands are kinds 201 (`BLPOP`) to 204 (`BLMOVE`); a shared number
would run `EVALSHA_RO` as `BLPOP`. Two list-related rules of main reach them
as well.

Framing. [append-only-file](../../../design/firn/append-only-file.md) frames
an execution unit that propagated several commands in `MULTI`/`EXEC`,
including a command's own lazy-expiry `DEL`s. Redis 7.2.16 runs a served
waiter's command as its own unit (`unblockClientOnKey` wraps
`processCommandAndResetClient` in `enterExecutionUnit` and calls
`afterCommand`, which reaches `propagatePendingCommands`), after the
producer's unit has been propagated, so every blocking attempt and every
served waiter frames its own effects: an expired key removed before the pop
or move gives `MULTI`, `DEL`, `LPOP`/`LMOVE`, `EXEC`; one effect stays bare.
`blocking_commit` applies the rule to the three sites that append blocking
effects (the immediate attempt, a served waiter and the list producers that
create a list and share the held path). This is read from the Redis 7.2.16 source
(`src/blocked.c` `unblockClientOnKey`, `src/server.c`
`propagatePendingCommands`) and rests on main's recorded lazy-expiry groups for
ordinary commands. Redis 7.2.16 recorded the remaining cases in run
39899773861
(`research/experiments/aof-oracle/blocking-expired-keys-recording.json`), each
key made expired by `SET key v PXAT 1` in the same write as the command:

- `RPUSH` and `LPUSH` onto an expired key, `LMOVE` and `RPOPLPUSH` into one,
  and `RENAME` or `COPY` over one give `MULTI`, `DEL`, the command, `EXEC`;
  `RPUSHX` and `LINSERT` on one write the `DEL` bare, because the command
  itself changes nothing.
- An immediately served `BLPOP`, `BRPOPLPUSH` or `BLMOVE` over an expired key
  gives `MULTI`, `DEL`, `LPOP` (or `RPOPLPUSH`, `LMOVE`), `EXEC`: the pop is
  propagated as the command it is rewritten to.
- A waiter blocked on `BLMOVE s d` whose destination `d` expired while it waited
  is served by `RPUSH s w`: the file holds the producer's `RPUSH`, then a
  separate `MULTI`, `DEL d`, `LMOVE`, `EXEC` for the waiter, so each served
  waiter is its own execution unit.

`tests/blocking_lists_oracle.rs` replays all three against firn (scenarios
`aof-expired-top-level-producers`, `aof-expired-immediate-blocking-pops` and
`aof-expired-served-waiter`). The one waiter case records a single
destination key; a waiter served with several expired keys of its own is not
recorded.

List ceiling. Main refuses a list addition beyond 2^32 elements before
changing anything, and `lmove_body` checks a distinct destination before
popping its source. A blocking move reuses `lmove_body`, so a source that has
an element and a destination at the ceiling would otherwise look like an
empty source and leave the waiter registered although no push could serve
it; `blocking_move_try` instead completes the attempt with the same
list-limit error as `LMOVE`. `tests/list_ceiling.wf` runs the real
`blocking_move_try` against an eight-element ceiling: the full destination
case fails if the check is removed (the attempt returns without a reply) and
the one-short case fails if the check refuses too early.
