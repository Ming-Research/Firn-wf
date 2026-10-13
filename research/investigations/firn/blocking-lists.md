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
All test byte constants await recording rather than encoding these deductions
as an observed transcript.

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
state. The stored request owns arguments and a private execution/reply Client;
it does not borrow the parser's input. The owned `blocked` flag causes only
suspended command owners to perform the guarded hub wait. After completion,
buffered pipeline commands resume without another network read, with fresh
time and settings. A RESP3 subscriber continues receiving pushes while
blocked; RESP2 subscriber admission retains Redis's existing command ban.

Registration removal and reply commitment are one hub statement. Timeout,
CLIENT UNBLOCK and ready service compete for that same pending request.
Disconnect removes all keys and finite timer membership before cancellation;
if service already won, later EOF does not undo its mutation. A new wait has
a monotonically allocated u64 generation, never reused within the supported
process bound of fewer than 2^64 waits. Finite timers use an indexed heap;
completion removes the entry rather than retaining an abandoned deadline.

A host-free projection contains only registration ids and counts for guarded
waits; the guard never observes the stored Client's MemoryMeter handle.
Blocked clients are exempt from configured idle timeout. Pub/sub output
limits still depend on actual subscription membership, so blocking alone
does not activate them. Existing independent output owners isolate slow peers.
INFO counts pending clients, finite timers and distinct keys; the
`total_blocking_keys_on_nokey` deletion-wait index remains zero for list waits.
CLIENT UNBLOCK uses the existing network-client-id index, not a registry scan.
Its promoted requester commits the confirmation before releasing the hub,
so the released target cannot publish ahead of that confirmation. Unlike
PING, this control observes and changes another connection's pending state.

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
Whitefoot drivers after recordings are supplied. Every fixed response, INFO
value and AOF byte expectation is deliberately `ORACLE-pending`; a preflight
assertion stops each case before starting a server when a recording is absent.
No test is ignored or weakened to accept the current capability error.

The existing oracle recorder on `claude/oracle-replies` opens one connection
and immediately reads after each command. It supports RESP selection, config,
drain mode, AOF capture and restart, but cannot express a parked command,
separate send/read or another client. Single-connection request batches and
precise multi-client recording instructions are handed to the owner outside
the repository. The fixture actions are the maintained multi-client source:
Send parks, WaitBlocked uses an INFO barrier, Read collects the recorded frame,
Quiet checks no output, and Close exercises cleanup. Test-harness observation
polling is a registration barrier; production scheduling never polls.
