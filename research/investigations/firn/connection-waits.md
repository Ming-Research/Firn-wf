# Connection waits, publication, watches and databases

## Status, question and evidence

The original analysis is a read-only design study, not an approved design.
Pub/sub now has a draft implementation and recorded reference evidence in
[the implementation section](#pubsub-implementation-evidence); blocking lists,
WATCH and databases remain proposals.
It asks whether Firn can add Redis 7.2.16 blocking lists, pub/sub, WATCH and
databases 0–15 without losing its concurrent keyed fast path. The database
cost question is answered first below. Every mechanism described as proposed
needs implementation evidence and owner approval before becoming a design
commitment. No compiler, build, test, Redis process or measurement was run for
this study; CI is driven by the owner. The original study changed no pin, submodule or design node; its proposed
mechanisms remain subject to owner approval.

The inspected Firn revision is `c6f6058fb17277d4d44d364a45efad1eae01a22b`.
Its compiler pin, `wf-d75f318d5631`, resolves in the Whitefoot repository to
`d75f318d56318834785322bde3a3656fb93116a6`. Whitefoot citations below use that
commit, not the current main branch. Redis citations use tag `7.2.16`.
The local [reference acquisition script](../../../tests/redis-suite/reference.sh)
pins the release archive used by CI. The local suite directory contains the
runner and ratchet, not a vendored copy of all upstream Tcl tests. In particular,
its [unstable list](../../../tests/redis-suite/unstable.tsv) already identifies a
false WATCH pass: a test with no assertion can pass despite unsupported WATCH.
Neither that pass nor Firn's old output establishes reference behavior.

Existing owners of this work are [the project direction](../../../design/firn.md),
[command parts](../../../design/firn/command-parts.md),
[transactions](../../../design/firn/transactions.md),
[scripts](../../../design/firn/scripts.md),
[enumeration](../../../design/firn/enumeration.md),
[protocol versions](../../../design/firn/protocol-versions.md),
[memory accounting](../../../design/firn/memory-limit.md),
[orderly stop](../../../design/firn/orderly-stop.md), and the
[append-only file](../../../design/firn/append-only-file.md) and
[rewrite tree](../../../design/firn/aof-rewrite.md).
Some nodes retain their original Redis 7.0.15 grounds. New behavior here is
checked against 7.2.16 rather than inferred from those historical citations.

## Database verdict first

**Zero additional database-selection operations on an ordinary AOF-off,
single-database keyed command are possible in the proposed architecture.
Zero total cost for the complete multi-database feature is not established.
Within-noise performance, especially with AOF enabled, remains a measurement
condition, not a conclusion from source inspection.**

Today `Keyspace.keys` is already a retained
`Shared<ConcurrentHashMap<Entry>>` handle, copied by `keyspace_share` when a
connection is accepted. A keyed command such as `run_push` or a string command
addresses `store.keys[key]`; it does not first look up a database. `run_select`
currently accepts only database zero and changes nothing. See
[store interface](../../../firn/store/module.wfm),
[construction and sharing](../../../firn/store/store.wf),
[connection commands](../../../firn/commands/connection.wf), and
[server](../../../firn/server/server.wf).

Propose sixteen permanent map identities, with the selected handle cached in
the connection's existing `Keyspace.keys` field. SELECT changes that handle
and a connection-local database number. Ordinary commands retain the same
map target and key bytes. Do not prefix every key with a database number, index
a shared database directory per command, or validate a global generation on
every lookup. Those alternatives add real work where the cached-handle form
does not. The stable identities are essential: SWAPDB exchanges **contents**,
not the directory's handles, so an already connected client sees the swap
without cache invalidation.

Additional fixed memory for empty databases, a larger connection handle set,
generated-code layout, multi-database eviction, and AOF selection handling can
still move performance. In the logged path, switching database during effect
encoding requires a last-emitted-database comparison under the existing Meta
hold; it is not honest to call that zero cost. Keep db 0 only if the controlled
single-db comparison detects a regression, as the owner ruled. Do not ship
sixteen databases on an unsupported assertion that one extra comparison is
negligible. Also do not claim the WATCH representation change below is part of
this zero-selection-cost argument: it needs its own comparison.

## Current execution boundaries and common proposal

### What must change

`serve` currently owns both socket halves, the input buffer, `Client`, its
reply buffer and a separate transaction queue. It receives, runs all complete
commands, then sends; it also flushes when accumulated replies exceed 16,384
bytes. This cannot receive EOF or an unsolicited message while the same
context is parked inside a blocking command. `execute_client` returns only
whether shutdown was requested, not a suspended-command disposition.

`run_exec` holds the whole map and Meta for its complete queue. Script attempts
hold declared keys plus Meta, with whole-map retry for undeclared access.
An abandoned attempt can restore earlier writes. Ordinary AOF-off writes
that need neither expiry nor other metadata deliberately avoid Meta;
`sequence_inert` gives them zero sequence stamps. Thus neither Meta nor
`Entry.last_write` is a free universal publication or WATCH mechanism.

The module graph has separate protocol, store, commands, scripting,
persistence and server modules. Keep wire framing in protocol, semantic
command parts in commands, database/mutation observations in store, and socket
ownership in server. A proposed connection-events module may own request
identities, registries and event records; it must not depend on commands and
then require commands to depend back on it. Command replay remains a commands
helper over already-held state, as script callbacks already respect their
module dependency boundary. The pub/sub implementation registers this boundary in
[modules.wfg](../../../firn/modules.wfg); the other feature adapters remain
interface proposals.

### Ownership and delivery

Propose an event hub: shared state containing live connection identities,
subscription registries, blocked-command records, ready-key queues and a
committed output queue. It owns data, never TCP handles. A connection record
is addressed by a monotonically allocated u64 id that is never reused during
the process; disconnect removes its record. The supported lifetime contains
fewer than 2^64 allocated identities, so a stale retained envelope can only
reach its old closed inbox. Growing standard hash maps hold the records and
channel indexes, following the owner's implementation ruling in
[registration identities](#registration-identities). No long-lived borrowed `Client` or
input-buffer reference crosses a context boundary.

Keep the current receive/execute/send path for connections that have never
needed unsolicited output or blocking. Before the first successful
subscription or a command actually parks, transfer that connection once to
the following ownership arrangement; flush or transfer its existing reply
prefix first, and retain its unconsumed input and transaction state:

| Owner | Owned resources and responsibility |
|---|---|
| Receiver context | `TcpReceive`, initialized input storage, receive cancellation; reads bytes/EOF and queues input without executing commands. |
| Command context | Parser state, client settings and transaction state; executes one command at a time, parks on its completion/input/stop state, and never runs the pipeline suffix while blocked. |
| Output context | `TcpSend`, current immutable frame and partial-send offset; alone writes the stream and closes the send half. |
| Event hub | Membership, waiter FIFO order, completion decisions and ordered committed output; never waits for socket progress. |
| Delivery context | Moves hub-committed events in order into retained per-connection inboxes and publishes their wake state; does no network send. |
| Timer context | Sleeps to an armed deadline, publishes a timeout request, and is cancellable when the request ends early. |

The one-time transition is a Firn ownership design, not a compiler workaround.
Whether preserving the ordinary path is worth its extra server complexity is
an explicit proposal to measure. An always-split connection is simpler but
adds queue transfers and context scheduling to every GET; compare it as an
alternative, not an assumed free refactor.

The receiver must continue reading while a command waits, as Redis does,
including enough to observe a peer's close. Apply Redis's query-buffer limit
and disconnect on overflow; simply stopping the receiver on a full private
queue can strand EOF behind buffered input. Cancellation and teardown must
wake input, command and output waits, remove registrations, disarm timers,
close each owned TCP half once, then join children. Send failure and receive
failure converge on the same idempotent teardown. A network error never rolls
back a completed dataset mutation. Preserve pending prefix offsets after
partial sends.

### Atomic publication, not a wake race

Registration/membership and its acknowledgment or completion commit together
under the hub. The output queue stores complete frames or complete outer
reply groups. Commit order is explicit: producers cannot release their holds
and then race to append directly to subscribers. A single delivery owner
drains the committed queue in order and moves events into individual inboxes;
later commits cannot overtake earlier ones at a recipient.

This extra delivery stage has a concrete language reason. Whitefoot cannot
discover a mailbox handle through a held registry and introduce a nested
atomic acquisition on it. Nor can `cancel_fire`, a waiting function, run
inside an atomic statement. Hub-owned output makes the semantic transition
one statement; individual mailbox publication and cancellation firing happen
afterward. Immutable `Frozen` payloads let many recipients retain one message
body. Per-recipient framing, reference ownership and pending-byte accounting
still cost space. The dispatcher must be woken by a guarded hub queue, not a
periodic poll. Only it waits on that common queue; subscriber contexts wait
on their own inboxes, avoiding a broadcast wake on every publication.

Do not send any event from an abandoned script attempt. Keep recipient
selection, publication counts, ready-key events and WATCH changes private
until the attempt is accepted. A final Lua error is an accepted execution
with its preceding effects; a retry is not. Hold the publication registry
across an attempt that consults it so a returned PUBLISH count and the
eventual delivery recipients describe the same membership snapshot. Before
first entering a feature requiring a wider hold, abandon through the existing
attempt protocol and restart with the complete hold set; never acquire it
inside an atomic body. This extends the attempt's undo/staging obligations.

## Databases 0–15

### Structures and command semantics

Give each logical database a permanent shared map. Keep global server
settings, Lua source cache/engine state, pub/sub channels, connection ids,
maxmemory and the sole AOF writer global. Tag expiry records, eviction
candidates and rewrite images with database identity. Meta can continue to
own the global log and sixteen database-local expiry/scan metadata records;
unrelated AOF-off writes must not start consulting it.

SELECT validates Redis's integer/range rules, installs a connection-local
selected handle and returns OK; the default is zero. WATCH registrations
retain their original `(database,key)` even after SELECT. Subscription
membership ignores SELECT. RESET restores database zero and clears its
other Redis connection state. A SELECT queued in MULTI changes the database
for subsequent queued commands and persists as the client's selection after
EXEC. `redis.call('SELECT',...)` changes the script's execution database,
initially the caller's; it must not change the network client's selection.

MOVE holds source and destination entries plus necessary metadata in one
statement. Validate the destination and same-database error before lookup,
as Redis does. Missing source or existing unexpired destination returns zero;
success transfers the payload and its absolute TTL, removes the source,
invalidates watches in both databases and announces suitable destination
readiness. Lazy expiry performed during either lookup remains an effect even
when MOVE itself returns zero. Refer to `moveCommand` in
[Redis db.c](https://github.com/redis/redis/blob/7.2.16/src/db.c).

Existing COPY is also a database consumer: `copy_parse` in
[key commands](../../../firn/commands/keys.wf) currently accepts only DB 0
and rejects equal key names without a database distinction. Extend COPY's
DB option in the same change. Hold source and destination maps/entries
together; equal names are valid when their logical databases differ. Copy
the source payload and absolute TTL without removing or modifying the source
value; REPLACE controls replacement of a live destination. Preserve option
validation order and lazy-expiry effects from `copyCommand`. A successful
copy invalidates destination watches and announces destination
readiness, while the source remains available. Script/EXEC computed COPY DB
uses the complete-map slow path, and AOF retains the source selection plus
the destination DB argument. This is necessary integration of an existing
command, not an optional later command family.

SWAPDB holds both whole maps and Meta, invalidates watches against the two
pre-swap datasets, swaps contents and expiry metadata, invalidates stale
eviction search state, then announces ready keys in their **logical** database.
Blocked clients and watched registrations stay with their database number.
Redis explicitly leaves `blocking_keys`, `ready_keys` and `watched_keys` in
place in `dbSwapDatabases`; exchanging connection handles would violate this.
With co-located WATCH records proposed below, detach the observation metadata
for watched names, swap data maps, and restore each database's observations
before releasing either hold. This adds work proportional to watched names
on SWAPDB, not on ordinary key commands. Built-in map swap is supported by
Whitefoot TYPE-9/OP-11; the observation-preserving implementation still needs
an ownership/alias witness in CI.

FLUSHDB empties only the selected logical dataset, clears its expiry records
and eviction eligibility, and applies Redis's watch invalidation conditions.
It must preserve observer-only cells until registrations are removed. FLUSHALL
continues to mean every database. Respect SYNC/ASYNC parsing and logical
completion; a synchronous implementation can match reply/state but cannot
claim Redis's background-freeing latency or memory trace. DBSIZE counts
physical dataset keys, including keys not yet physically expired, rather than
forcing an expiry sweep. It must exclude observation-only cells. SCAN/KEYS
and RANDOMKEY operate on the selected map only; SCAN's cursor is scoped to
that database, not a global cursor split by key prefixes. Expiry removes and
logs from the database named in its queued record. A stale expiry with the
same key and time must never delete a key in the wrong database after a swap.

### Transactions, scripts, memory and persistence

Cross-database EXEC and arbitrary script SELECT/MOVE/SWAPDB access cannot be
implemented by releasing the old map hold and acquiring a new one halfway
through execution. Proposed initial slow path: take all sixteen permanent
map targets plus Meta for executions whose database access is not a fixed
single-map set. For scripts, retain the existing declared-key path until
such a command requires a retry with the complete hold set. A transaction's
queued commands can be inspected in advance, but initial correctness can
use all maps for cross-database execution. The fixed set is known before the
statement; no dynamic nested target acquisition is assumed. Compilation of
the helper interfaces over multiple held maps is unverified, especially the
alias-proof issue in the capability section.

Eviction uses a server-wide limit and considers candidates from all databases,
including when the allocating command is on db 0. Preserve database identity
in the shared candidate pool. Reserve reclamation and eligibility exhaustion
must cover all databases before declaring OOM. Unused empty databases add
fixed allocation overhead, which counts toward heap use; do not initialize
sixteen full keyspace-sized reserves without measuring their memory cost.
Disconnect releases cached handles and watches, not the selected database.

The AOF encoder keeps `last_emitted_db`, initially invalid for each new
increment, and emits SELECT before a propagated effect when its database
differs. SELECT by itself is not a durable dataset mutation. Transaction and
script effect streams need database-tagged records before final framing;
one opaque buffer without selection context is insufficient. Selection changes
within a MULTI/EXEC effect group must replay in their original order. MOVE
logs its destination argument while retaining its source database context;
SWAPDB remains a global command effect. Replay tracks its own selection,
including SELECT inside transaction blocks. Rewrite visits each database,
emits selection for each reconstructed dataset, and keys its journal's
coalescing by `(database,key)`, never by key alone. S0/S1 capture, rotation,
after-images and later effects still share one global sequence and cut.
See `feedAppendOnlyFile`, `rewriteAppendOnlyFileRio` and rotation's selection
reset in [Redis aof.c](https://github.com/redis/redis/blob/7.2.16/src/aof.c),
and Firn's [rewrite emitter](../../../firn/store/rewrite-image.wf), which now
only emits database zero.

Bulk transformations need an explicit capture policy. Today's `flush_body`
in [server commands](../../../firn/commands/server.wf) aborts an active rewrite
through the Flush reason. Propose preserving that bounded-capture policy for
FLUSHDB/FLUSHALL and aborting the active capture before SWAPDB changes maps,
while the authoritative incremental AOF records the accepted command normally.
A later rewrite starts from the new dataset. This avoids treating a map swap
as a handful of ordinary key images. It is a proposed design choice with
observable rewrite-status consequences; compare those against Redis and
report the divergence rather than claiming rewrite-job parity. Supporting a
successful in-flight rewrite across SWAPDB would instead require a proved
database-remapping journal protocol and is not established by this study.

### Alternatives and unused-path cost

Reject per-command directory locking, key-prefix namespaces and a per-db
server process: respectively they add a global lookup, hashing/copying work,
or break atomic MOVE/SWAPDB and shared server state. Reject swapping cached
map handles because it leaves existing connections on old contents. A global
command sequencer would simplify cross-db commands but erase the concurrency
experiment. Stable map identities preserve the existing ordinary lookup;
only exceptional cross-db commands broaden exclusion. Verify generated code
and the AOF-on/off comparison below before enabling the feature. If a
multi-map natural helper is rejected, stop that implementation and submit
the witness; no pointer trick, command-by-command lock release or alternate
spelling is proposed as a repair.

## WATCH and UNWATCH

### Reference behavior to preserve

[Redis multi.c](https://github.com/redis/redis/blob/7.2.16/src/multi.c), especially
`watchForKey`, `touchWatchedKey`, `isWatchedKeyExpired`,
`touchAllWatchedKeysInDb` and `execCommand`, defines more than value comparison:

- Watch identity is `(database,key)`, duplicates are idempotent, and a key
  missing at WATCH can still be invalidated by creation followed by deletion.
  A same-value SET is a modification for WATCH. A failed NX condition, a
  failed operation or a lookup/access-stamp refresh is not automatically one.
  Audit each Redis `signalModifiedKey` call, including TTL changes and no-op
  special cases; WRITE flags and AOF record counts are not the criterion.
- The watching connection's own changes invalidate its watch too. Physical
  eviction and lazy/active expiry participate. EXEC checks expiry even if
  no background expiry task has removed the key yet.
- Redis records whether a watched key was **already expired** at WATCH.
  Its later physical removal alone is not a new logical change. Bulk
  flush/swap invalidation distinguishes missing, expired and live values.
  Copy those branches rather than invalidating every watch on every flush.
- WATCH inside MULTI is refused. UNWATCH outside clears observations and
  the CAS-dirty state; inside MULTI it is queued and cannot rescue an EXEC
  already invalidated before execution. EXEC tests watches, removes them
  before running members, and clears them on success or discard/abort.
  DISCARD, RESET and disconnect also release watches. An EXEC outside
  MULTI returns its normal error; do not infer that every EXEC-shaped input
  necessarily clears a prior watch without checking its admission path.
- Dirty queue errors take precedence over a CAS null abort. CAS abort is
  RESP2 null array or RESP3 null; it executes no queued command and appends
  no transaction effects. Scripts cannot invoke WATCH/UNWATCH, but accepted
  mutations made by scripts invalidate another client's watches.

### Proposed representation and ownership

Use one map cell per key containing a logical `Option<Entry>` payload and
optional key-local watch records. This is a proposed replacement of the
map's value representation, not an extra map lookup. Each record contains a
registration identity, owner connection identity, dirty state and Redis's
already-expired state. A connection owns a reverse list of its
`(db,key,registration)` handles. Keep a payload-less cell while it has
observers; remove it after its final observation goes away. A deleted then
recreated value therefore cannot erase an observation. A slab registration
handle avoids wrapping a mutation counter and accepting an ancient watch.

Under the key's existing hold, a successful operation marks its affected
watch records. It need not acquire a shared client object: EXEC scans its
own registrations while holding their maps, and decides CAS validity in the
same statement that will execute the queue. Marking a dirty record and
later testing it avoids the forbidden nested acquisition of a client flag.
Cleanup uses the reverse list. The initial proposal retains dirty records
until cleanup; Redis often unlinks all of a dirty client's records eagerly.
This retention difference is real memory overhead and must be bounded by
connection lifetime and included in pressure experiments. If it is material,
an eager cleanup design needs a separate ownership review; it is not a free
optimization of the key-only write path.

For logical key counts, Meta keeps the number of observation-only cells in
each database. Under a whole-map plus Meta hold, DBSIZE can subtract that
number from `map_count`. WATCH on absence, last-watch removal and watched
payload creation/deletion update the count; ordinary unwatched changes do
not. All enumerators, eviction samplers, emptiness checks, rewrite scans and
expiry paths must distinguish a dataset key from an observation-only cell.
The memory limit counts both. Snapshot/rewrite emits only logical payloads;
watches and their dirty state are never AOF data.

WATCH registration and payload inspection are one atomic operation. Mutation
and invalidation are one operation. EXEC checks and consumes the registrations
before executing the queue without releasing its holds. That covers the
register/write race, the check/EXEC race, missing-key ABA, and same-client
writes. For scripts, save/restore the watch state along with tentative data,
or stage invalidations in the attempt and apply them only on acceptance while
the held maps still protect the observations. Prefer staging: retrying a
script must not cause a false CAS abort in another connection. Expiry effects
before a final Lua error remain accepted effects.

### Cost and alternatives

**No measurable slowdown is an acceptance requirement, not proved here.**
The normal keyed write still has one map target and no extra shared object,
but it must test whether the cell has observations; the cell is larger and
the logical payload introduces a representation access. These can affect
cache density and missing-key behavior even with zero watchers. Measure
unwatched keys both when the server has no watches and while other keys have
many watches. Measure missing-key create/delete separately from overwrites.

Reject `last_write` reuse: AOF-off writes stamp zero, deletion loses the stamp,
and rewrite sequencing does not encode Redis's WATCH invalidation rules.
Reject value snapshots: SET of the same value and create/delete ABA would
escape. A separate observer map is semantically straightforward but adds
another keyed target/lookup on writes. A global watch-present counter or a
connection's cached watch-present bit is not a safe free bypass: WATCH may
be registered concurrently, and Whitefoot shared observation itself needs an
atomic access. Do not invent relaxed access or poll to close that race.

The co-located proposal is the best candidate found for retaining a single
keyed hold, but if its measured cost breaches the owner's requirement, stop
and bring that result as a design question. This study cannot promise that a
runtime-level observation facility would be faster without its own witness
and experiment. The existing rejection of WATCH as unnecessary for selected
consumers is reopened by this task's explicitly expanded feature scope.

## Blocking list commands

### Reference behavior

Use [Redis t_list.c](https://github.com/redis/redis/blob/7.2.16/src/t_list.c),
[blocked.c](https://github.com/redis/redis/blob/7.2.16/src/blocked.c),
[timeout.c](https://github.com/redis/redis/blob/7.2.16/src/timeout.c),
and [list tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/type/list.tcl).

An immediate BLPOP/BRPOP scans operands in order, returning from the first
usable list or reporting WRONGTYPE where that scan encounters it. Repeated
names create only one waiter membership per key. Blocking registration is
FIFO on each key; ready keys are deduplicated and drained in their readiness
order after the outer command, EXEC or script completes. A ready pass rotates
each candidate and considers at most its initial queue length. It reprocesses
the **whole original command**, not just a pop from the triggering key. Thus
an earlier operand can win or cause WRONGTYPE at wake-up.

A source replaced with a non-list does not alone wake a list waiter.
BRPOPLPUSH/BLMOVE inspect the destination after source availability; a wrong
destination must leave the source unchanged and let the next eligible waiter
proceed. Successful moves can make further keys ready; process the existing
ready batch before the newly generated batch. RENAME, COPY, MOVE, SWAPDB, SORT STORE
and every operation introducing an appropriate value need the same readiness
hooks as list pushes. Design the event kind to distinguish ready-value from
deletion notifications, since later stream group waits use deletion events.
Later sorted-set and stream commands reuse scheduling and lifecycle, not
the list-specific readiness predicate or reply builder.
In particular BZPOPMIN/BZPOPMAX and BZMPOP need ordered destructive selection
and their own count/reply shapes, while XREAD BLOCK may deliver the same
stream append to several readers without consuming it. The common interface
must let the command adapter say whether readiness remains and which waiters
may proceed; it must not encode "one event consumes one value for one client"
as a universal rule. Stream ids, XREAD multi-key priority and stream-specific
deletion/group behavior need their own later source study.

Scripts and EXEC allow these commands but set a no-blocking execution mode;
they are not blanket NOSCRIPT commands. They consume available data normally.
Empty BLPOP/BRPOP returns null array; empty BRPOPLPUSH/BLMOVE returns null bulk
in RESP2. In contrast, an **actual blocked timeout returns null array for all
four** in 7.2.16. RESP3 uses its single null form. The script cases are present
in [upstream scripting tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/scripting.tcl),
and the flags are established by
[eval.c](https://github.com/redis/redis/blob/7.2.16/src/eval.c) and `execCommand`.

Timeout seconds are parsed as x86-64 `long double`, multiplied by 1000,
rounded with `ceill`, checked for range/negativity, and added to the command's
calendar snapshot. Checking negativity after ceiling allows a tiny negative
fraction to round to zero. Zero means no deadline; expiry is strict
`deadline < now`. Reprocessing must retain the original absolute deadline,
not grant a fresh interval. This numeric path has a confirmed missing native
capability at the pin, documented below; do not substitute a binary64 parser.
Firn currently reads calendar time once per receive batch. Registration and
resumed execution need the appropriate outer-command snapshot rather than a
timestamp frozen across a long blocked interval or inherited by the resumed
pipeline suffix. Keep one frozen snapshot within EXEC or a script as Redis
does. Source inspection alone does not establish that the existing per-read
clock boundary is sufficient for these new waits; refresh and measure the
changed boundary explicitly instead of charging the full timeout again.

### Waiter lifecycle and ready execution

The hub's waiter record owns the original arguments, selected database,
command identity/directions, absolute deadline, connection/request generation,
and links into FIFO lists for its distinct operand keys. States are pending,
completed or disconnected. A completion holds an immutable response and its
ordering position. Checking all keys and installing every registration must
occur within the same protected transition; checking first and registering
later loses pushes.

Propose a correctness-first **ready phase within the producer's protected
execution**. A readiness-producing operation takes the whole affected map
plus the hub and its required Meta hold. Finish its ordinary semantics;
after the outer EXEC/script commits, process ready keys and replay eligible
waiters before releasing the protected map. Ordinary nonblocking pops then
cannot steal a value between its production and service of an older waiter.
Each successful resumed command has its own effects, WATCH invalidation,
reply and AOF boundary after the producer's boundary. It is not a nested
atomic statement and its records must not be accidentally wrapped inside
the producer's MULTI/EXEC. The ready phase invokes nonwaiting helpers over
held state; socket I/O follows release.

This is broader exclusion than the current list-write path, deliberately
visible as a performance proposal. Releasing the map and merely waking FIFO
tasks is insufficient: runtime scheduling and a competing LPOP can reverse
the reference's service order. Dynamically collecting a waiter's other keys
after taking a narrower hold cannot extend that same atomic statement.
Whole-map ownership solves that issue without pretending the scheduler is
FIFO. If the proposed broader hold is too expensive, reopen the architecture;
do not add a polling guard or let ready clients race for values.

Reprocessing also repeats applicable live admission in Redis `processCommand`,
including OOM/eviction and persistence refusal; it is not only `pop_body`.
An eviction can reach any database. The multi-db/maxmemory ready slow path
therefore needs all database maps plus Meta and hub from the start, or a
separately proved admission boundary. Propose the complete hold set for that
path, with nonwaiting versions of the relevant admission helpers. A
configuration snapshot must not authorize a later unheld database access.
Where live configuration must be observed at replay, include that state in
the held transition as well. This is significant interface work and must be
compiled as a small ownership witness before implementation. Preserve
command-table distinctions: BLPOP/BRPOP are WRITE without DENYOOM, while
BLMOVE is WRITE and DENYOOM.

A timer posts a timeout request; EOF/cancellation posts a disconnect request.
The same hub ownership that awards a ready result decides these requests,
removes **all** key memberships and makes at most one completion visible.
An event for an older generation is discarded. A disconnected waiter cannot
consume a later value. Once successful mutation has won, a subsequent peer
close does not undo it. Queued commands after a block resume in connection
order when completion is published, even if no further input arrives.
Blocked connections are exempt from Redis's ordinary configured idle-client
timeout; the current `serve` idle policy must gain that exemption.

Use cancellable one-shot timers and the stored absolute deadline. A timer
wake must recheck calendar expiry, since Whitefoot sleep is monotonic while
Redis's deadline comparison is calendar based. A backward clock step can
rearm the same absolute deadline; a forward step cannot be observed promptly
by monotonic sleep alone. Exact behavior under host clock changes needs a
calendar-deadline notification facility or an owner-approved timing boundary,
not a hidden periodic poll. Normal host scheduling also means neither Redis
nor Firn promises completion at an exact real-time instant. The capability
section distinguishes this clock-domain limitation from supported state/time
composition.

### Persistence, memory and unused paths

Log successful operations as their nonblocking effects: LPOP/RPOP,
RPOPLPUSH or LMOVE, as the source command rewrite requires. Neither blocking
registration, timeout nor disconnect adds AOF data or changes WATCH.
Ready events from a script attempt remain private until acceptance; EXEC
and scripts do not expose intermediate list contents to waiters. A push
followed by deletion in one outer execution must not pop a vanished element.
Expiry and eviction invalidate watches as usual but are not synthetic pushes.

Count request copies, FIFO nodes, timers, input backlog, completion buffers and
all retained payloads in process memory. They are not AOF-excluded memory.
Admission, client input limits and teardown must prevent silent drops or
unbounded abandoned timer/registration retention. Output-buffer overflow
disconnects the affected connection under the chosen Redis-compatible policy;
it must not block a producer waiting for a peer to read.

GET and unrelated keyed writes acquire no hub solely for list waits. List
producers and value-transfer commands pay broader holds and ready checks even
when there are no waiters in this initial proposal. Scripts/EXEC may pay wider
holds and event staging. Timers and extra I/O contexts are charged only when
a connection uses the feature. Measure idle thousands-of-waiters, active
waiters on unrelated keys, move chains and high-contention FIFO service as
well as the feature-unused list-write path. Directly parking on map guards
was considered but rejected as the ordering mechanism: it gives no FIFO and
the pin wakes map-wide observers on unrelated writes.

## Pub/sub, including shard channels

### Registries, replies and restrictions

Use [Redis pubsub.c](https://github.com/redis/redis/blob/7.2.16/src/pubsub.c),
[networking.c](https://github.com/redis/redis/blob/7.2.16/src/networking.c),
[pubsub tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsub.tcl)
and [shard tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsubshard.tcl).
Scope is standalone Redis; shard pub/sub commands do not imply implementing
Cluster routing or its cross-slot errors.

The hub owns three server-wide indexes: exact channels to subscribers,
patterns to subscribers, and shard channels to subscribers, plus each
connection's reverse memberships. Classic and shard namespaces are separate
and ignore the selected database. Duplicate subscription emits another
acknowledgment but adds no duplicate membership. Unsubscribing an absent name
still acknowledges. No-argument unsubscribe emits one acknowledgment per
membership, or the null-name acknowledgment when that class is empty.
Binary channel, pattern and message bytes must survive unchanged; patterns
use Redis's binary glob matching rules.

Classic subscription acknowledgment counts are channels plus patterns;
shard acknowledgments count shard subscriptions separately. Subscriber mode
ends only when all classes are empty. PUBLISH first delivers exact-channel
messages and then matching-pattern messages; it returns the number of
delivery paths, not distinct connections. One channel and two matching
patterns on one connection yield three messages and count three. SPUBLISH
uses only the shard registry and emits `smessage`. NUMPAT counts distinct
patterns globally, not subscriptions. NUMSUB/SHARDNUMSUB exclude patterns and
preserve the channel operands' order and duplicates. CHANNELS/SHARDCHANNELS
enumerate active exact memberships with optional pattern filtering.

RESP2 subscription acknowledgments/messages are arrays. RESP3 uses `>` push
frames with the exact Redis element counts and bulk payloads; add a shared
push-header writer rather than translating an already flattened RESP2 reply.
The RESP2 restricted command set is PING, QUIT, RESET and all six
subscribe/unsubscribe commands. RESP3 subscribers can execute ordinary
commands. RESP2 subscriber PING has the pub/sub pong-array form; RESP3 keeps
the ordinary PING reply. Check the restriction at command admission, with
Redis's error precedence, rather than letting arbitrary commands fall through
to unknown-command handling.

### Publication order and execution units

Membership change and its acknowledgment commit together before a later
publication can target that membership. Hub serialization gives a definite
publication order and per-recipient message order; delivery to one slow
connection cannot stall other output contexts. Share immutable body bytes
among recipients, retaining per-recipient offsets and headers separately.
Do not rely on runtime wake order to order messages.

PUBLISH/SPUBLISH are allowed in scripts and MULTI. They are neither WRITE nor
DENYOOM and force replication-only propagation, not AOF propagation. Their
absence from the AOF does not make their external effects safe to repeat:
script retries discard staged deliveries and accepted attempts publish once,
including publications preceding a Lua error. If a script budget retry has
already staged publications, its termination/retry rule must treat those as
observable effects to avoid endless restarts that never publish anything.
Recipient-count reads require a stable membership snapshot through acceptance.

For a subscriber publishing to itself under RESP3, the complete EXEC array
or script reply precedes its pushed messages. Stage those self-pushes after
the entire outer reply, never between its array elements. Subscription
acknowledgments have historical exceptions: multiple SUBSCRIBE acknowledgments
inside EXEC can make irregular framing. Preserve 7.2.16's transcript rather
than silently regularizing it; the recorded batches below establish it. Subscription
commands are NOSCRIPT. SSUBSCRIBE additionally refuses a no-blocking client
inside EXEC, unlike classic SUBSCRIBE/PSUBSCRIBE's historical exception.
SUBSCRIBE, PSUBSCRIBE and SSUBSCRIBE also carry DENYOOM; removal commands
must remain available to release memberships under pressure. Command-table
flags and script-specific admission remain separate checks.

Neither membership nor messages belong in AOF or rewrite images. Replay has
no subscribers. A PUBLISH in a transaction must not increase the number of
dataset propagation records used to choose AOF transaction framing. Selected
database changes, FLUSHDB and SWAPDB do not change pub/sub membership.

### Memory, disconnect and cost

Subscription indexes, matching work, per-recipient frame envelopes and bytes
waiting in hub, dispatcher and socket-owner buffers all count as live memory.
Do not stop accounting when a frame leaves the hub. Charge retained shared
payload allocation once to process heap, while the Redis-style output limit
needs an explicit per-client pending-output accounting policy. Two consumers
sharing one allocation does not make either one's backlog zero.

Implement Redis's hard/soft client-output-buffer limits and their duration
logic with asynchronous closure. Slow readers must not cause publisher
backpressure or silent message loss. Disconnect removes all memberships and
queued output; a generation protects an in-flight delivery snapshot from
reaching a newly accepted connection. Pub/sub clients are exempt from the
ordinary idle timeout even when silent. Memory pressure can still evict
dataset keys and invalidate WATCH; publication itself is not DENYOOM.
The reference defaults are 32 MiB hard and 8 MiB continuously for 60 seconds
soft for pub/sub, with normal-client output limits disabled, and a 1 GiB
query-buffer limit; see [Redis configuration](https://github.com/redis/redis/blob/7.2.16/redis.conf).
The numeric limits do not imply matching allocator-dependent closure at an
identical message number. Preserve CONFIG behavior for the limit settings
exposed by this feature, and test falling below/re-entering the soft limit.

Ordinary non-subscriber connections need no hub lookup or queued output
solely for pub/sub. The dispatch table gains command identities; subscriber
admission gains a connection-local mode check. RESP3 subscriber ordinary
commands use the split/output-queue path and therefore do pay its cost.
PUBLISH costs exact-channel lookup plus pattern matching and O(deliveries)
envelopes; do not claim shared bodies make fanout constant-time. Scripts that
can publish need wider holds/staging. Measure zero subscriptions, many idle
subscriptions, pattern misses, and one-to-many delivery with a slow reader.

Reject direct network sends from publishers because they serialize on slow
recipients and cannot run under command atomicity. Reject a global socket
writer because a blocked send would delay unrelated clients. Reject an
unsynchronized recipient snapshot because subscription acknowledgment,
unsubscribe and publication counts can disagree. A trie/pattern index may be
a future performance choice; the first implementation should reproduce
Redis glob semantics and measure the simple registry before changing it.

## Whitefoot capability audit at the pin

The [writing guide](../../../whitefoot-kit/docs/writing.md), especially its
whole concurrency part, agrees with the following pinned interfaces and
rules. Snippets here are **uncompiled semantic witnesses**, not supplied
complete programs or claimed accepted compiler runs.

| Need | Available evidence and qualification |
|---|---|
| Park and resume another context | [Specification WAIT-2/WAIT-3 and SHARE-1/2/3](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/spec/kernel-spec.md#L2268): spawn with owned values; a false guarded atomic suspends and retries on observed-state change. Progress is conditional, not FIFO. |
| Wait for state or cancellation | [Time interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/time/module.wfm): `cancel_state` returns a retained read-only shared state, usable with the inbox in one guard. |
| Timer and disarm | Same interface: monotonic `sleep_until`, `cancel_source`, `cancel_watch`, `cancel_fire`; use a separate disarm watch and fire it before a child join. No polling is needed for ordinary state-or-timer waits. |
| Queued output and EOF | [Network interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/net/module.wfm): separate owned receive/send halves, cancellable host calls, explicit partial transfer endpoints, EOF and consuming closes. |
| Many recipients | SHARE-1 `Frozen<T>` and `frozen_share` retain immutable data without a lock; ordinary shared queue state protects envelopes. A Frozen body contains no live socket/shared-state handle. |
| Queues, ordered timers, connection ids | [Library inventory](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/modules.wfg), deque, priority queue and growing hash map interfaces support the selected representation. Slab retires exhausted generations but has no capacity-growth API; its separately filed request does not block the [nonreused-id dictionary design](#registration-identities). |
| Multiple database maps | SHARE-2 admits multiple whole-map targets, and TYPE-9/OP-11 admits swapping two held maps. The maps' permanent handles can be known before the atomic header. General multi-map helper alias proofs remain a witness obligation below. |
| Memory evidence | [Process interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/process/module.wfm): actual drivers, `heap_in_use` and resident bytes; heap requested bytes are not Redis allocator-usable bytes. |

There is no channel or select module in the inventory, but the required
semantics are already composed by guarded shared state and timer contexts.
For example, this fragment waits without polling:

```whitefoot
fn wait_for_reply(inbox: Shared<Bool>, stop: std::time::CancelWatch) -> result: Bool pure waits {
  let cancelled = std::time::cancel_state(watch: &stop);
  let ready = False();
  atomic item = &inbox, ended = &cancelled when bor(item^, ended^.fired) {
    set ready = item^;
  }
  return ready;
}
```

A separately retained producer handle writes the inbox; a timer can fire
`stop`. Production needs a result enum/generation and the hub's winner
decision, not just this Bool. Host outcome versus cancellation/deadline races
follow PRE-2: an already produced outcome can win. An implicit child join is
not cancellation; disarm and publish stop before any edge that joins children.

The [guard runtime](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/compiler/src/backend/completion/bridge.c)
registers watches, handles notification-before-park, and wakes contexts for
rechecking. It does not supply Redis FIFO. The
[map runtime](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/compiler/src/backend/keyed_table.c)
checks the map watch count after writes and wakes map-wide guards. Therefore
one natural direct key wait can be re-evaluated after another key is written:

```whitefoot
fn present(value: &Option<u64>) -> result: Bool reads(value) {
  match value^ {
    Some(value: stored) => {
      return True();
    }
    None() => {
      return False();
    }
  }
}

fn wait_key(keys: Shared<ConcurrentHashMap<u64>>, key: Box<Slots<u8>>) -> result: unit pure waits {
  let bytes = &key.inner[0_u64..key.inner.len];
  atomic value = &keys[bytes] when present(value: value) {
  }
  return unit;
}
```

This is an inspected **performance limitation**, not an observed compiler
rejection or missing wake semantics. Explicit FIFO registries are needed
independently for Redis ordering, so dedicated reply inboxes are a justified
design rather than a spelling change to hide a compiler error. If direct
per-key waits become the selected interface, request finer-grained wake
registration with a no-lost-wake test and an unrelated-key no-recheck test.

Shared state must be droppable. `TcpSend`/`TcpReceive` are `nodrop`, so trying
`Shared<TcpSend>` is not an available ownership model. The proposed socket
owners use their ordinary linear ownership instead. SHARE-2 also forbids
nested atomic statements, waiting calls in a guard/body, and target-handle
paths dependent on the statement's own bindings. Conditional acquisition is
a compiler policy by state-type group: extra map targets acquire early, and
later groups may pull earlier groups in. Use the lock ledger in CI to verify
that an allegedly unused hub or Meta is actually untaken; source `if` alone
is no evidence.

### Registration identities

The owner selected growing standard hash maps keyed by monotonically
allocated u64 registration ids, with no reuse during the process. This is the
natural dictionary representation of Redis's channel/client ownership, not a
substitute spelling of the slab. Its lifetime bound is fewer than 2^64
identity allocations; the implementation documents that bound rather than
introducing a new subscriber ceiling. The proposed
[pub/sub registry node](../../../design/firn/pubsub-registries.md) records this
choice and rejects the slab for this representation.

The pinned slab's inability to reserve more cells was filed by the primary
agent as Whitefoot request `firn-req-slab-reserve`, P2. The capacity witness
used `slab_new::<u64, 3, 1>(capacity: 2_u64)`, retained two successful
insertions and attempted a third. The interface promises unchanged capacity;
the implementation and maintained slab program return the third owner. No
compiler or runtime result was recorded here. Explicit growth preserving
indices, generations, occupancy and free-list sentinel meaning remains that
request's acceptance, separately from pub/sub delivery. The standalone
capacity-only witness has been removed after filing; no pub/sub dependency
on its resolution remains.

### Missing capability: exact reference timeout numbers

Redis `getTimeoutFromObjectOrReply` and
[`string2ld`](https://github.com/redis/redis/blob/7.2.16/src/util.c) use the host's
extended floating conversion and arithmetic. Whitefoot's pinned GRAM-3,
TYPE-1 and operation table expose only f32/f64; its
[text interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/text/module.wfm)
contains no extended numeric parser. The natural arithmetic requirement is:

```whitefoot
fn timeout_ms(seconds: f80) -> result: Result<i64, NarrowError> pure {
  let scaled = fmul.strict(seconds, 1000.0_f80);
  let rounded = fceil(scaled);
  return cvt.checked::<f80, i64>(rounded);
}
```

Here **f80 and its literal suffix are requested, unsupported syntax**, not
existing declarations. The witness omits Redis admission checks so it isolates
the missing precision. The companion requirement is text conversion with
Redis's accepted syntax, underflow/overflow and rounding behavior, followed
by Redis's exact error precedence. No compiler diagnostic has been generated.
An integer-based software implementation is possible in principle; that
does not justify hiding the capability question inside Firn. Stop the exact
timeout-parser implementation pending a Whitefoot decision and CI evidence.
Neither f64 multiply/ceil nor exact decimal ceiling is generally the same as
the reference's rounded extended computation. The numeric ORACLE batch below
is a proposed witness, not a measured result.

### Missing host capability for strict clock-jump parity

The pin has monotonic sleep and a calendar read, but no calendar-deadline wait
or wall-clock-change notification. A natural requested operation is:

```whitefoot
fn wait_calendar(clock: std::time::WallClock, deadline_ms: i64, cancel: std::time::CancelWatch) -> result: unit pure waits {
  let outcome = std::time::sleep_until_unix_ms(clock: &clock, deadline: deadline_ms, cancel: &cancel);
  return unit;
}
```

`sleep_until_unix_ms` is an **invented requested interface name**, absent at
the pin. Its required contract would reconsider the deadline after either
direction of host clock change and remain cancellable. This does not request
a hard real-time guarantee. Redis rechecks calendar time during event-loop
processing; a far-future monotonic one-shot cannot see a forward jump during
otherwise idle service. If controlled clock-jump parity is required, this
part waits for the host facility/contract decision. A periodic wake to poll
the wall clock is not proposed as a workaround. Ordinary stable-clock tests
can proceed independently; they do not settle the stronger contract.

### Unverified capability: natural cross-map helper boundaries

The specification does not infer distinct shared object identities merely
from different source handles or database integers. The natural MOVE helper
needs disjoint source and destination entry references. Minimal intended form:

```whitefoot
fn transfer(source: &Option<u64>, destination: &Option<u64>) -> result: unit writes(source), writes(destination) {
  swap(first: source, second: destination);
  return unit;
}

fn move_between(left: Shared<ConcurrentHashMap<u64>>, right: Shared<ConcurrentHashMap<u64>>, key: &[u8]) -> result: unit reads(key) waits {
  atomic source = &left[key], destination = &right[key] {
    transfer(source: source, destination: destination);
  }
  return unit;
}
```

This deliberately isolates the boundary, not full MOVE behavior. Built-in
swap permits exact aliasing, while an ordinary two-write helper call needs
EFF-5 disjointness; caller knowledge that database numbers differ may not
establish handle identity. Compile the natural full witness before selecting
an interface. This is a **suspected proof/interface gap**, not a demonstrated
diagnostic. If rejected, supply the diagnostic and the distinct-database
construction to Whitefoot; do not silently inline the helper or add a runtime
alias check to bypass it. The same concern applies to helpers receiving many
whole-map views for EXEC and eviction.

These are request-ready findings, not filed board requests. No wf-board tools
are exposed in this session, so this study does not claim a board update or
invent request keys.

## What byte-for-byte acceptance can and cannot mean

The deterministic wire contract remains byte-exact: command errors and their
precedence, RESP type markers, lengths, binary payloads, subscription counts,
transaction outcomes and the explicitly ordered results of controlled
histories. Compare raw bytes from the pinned reference, never from an older
Firn build. Do not normalize a wrong null type, duplicate delivery, error or
FIFO winner away.

The following observations cannot promise the same transcript across these
different implementations or even across unconstrained Redis runs:

| Observation | Why exact bytes/order cannot be promised | Acceptance instead |
|---|---|---|
| SCAN and later SSCAN/HSCAN/ZSCAN cursor values, batch splits, traversal order | Redis dictionary layout, hash seeds, resizing and compact encodings differ from Whitefoot's maps; Firn does not yet implement all collection cursors. | Cursor is opaque; zero terminates; required stable elements are covered; returned elements satisfy type/match rules and duplicate allowances. Test cursors only against their own server, never transfer them between servers. |
| KEYS, PUBSUB CHANNELS/SHARDCHANNELS order; no-argument unsubscribe order; order among matching pattern deliveries | Hash traversal order is not a cross-server ordering contract. | Compare exact element/message bytes as a multiset, preserve multiplicity and each acknowledgment's count progression, require direct-channel delivery before pattern deliveries. |
| Competing connections with no happens-before edge; equal-deadline ordering and timeout latency | OS/event-loop scheduling, clocks and runtime wake order differ. | Establish barriers for FIFO registration and producer order; check a valid reference-compatible linearization, exactly one completion and no early timeout. Race tests check allowed outcomes and final state. |
| Memory values, victim identity, exact publication count at slow-client closure | Requested versus allocator-usable bytes, map layout, sampling/randomness and socket buffers differ. | Check global accounting inclusion, configured limit behavior and bounded cleanup; force a unique eligible victim where a WATCH result depends on eviction. Report allocation-related differences explicitly. |
| Whole AOF file bytes after rewrite | Dataset traversal order, batching, allowed base format and file lifecycle differ; Firn already writes command bases rather than Redis's default RDB preamble. | Check supported command records/SELECT context and transaction ordering byte-exact in controlled incremental traces; replay each supported AOF on both servers and compare logical datasets/absolute TTLs. Rewrite acceptance is equivalent replay state, not equal file hashes. |
| Absolute deadlines and wall-clock jump response | Calendar observations and host notification availability differ. | Frozen/controlled numeric conversion cases and barrier-separated readiness versus expiry; clock-jump parity remains the explicit missing-host-capability decision above. |

This is not permission to relax deterministic behavior. It also does not
claim that the proposed feature set closes Firn's other existing differences
(for example its transaction queue ceiling, current memory-accounting basis
or unsupported persistence modes). Keep those separate from new failures.

## Pub/sub implementation evidence

The draft implementation of steps 4 and 5 follows the proposed
[connection ownership](../../../design/firn/connection-ownership.md),
[pub/sub registries](../../../design/firn/pubsub-registries.md) and
[script external effects](../../../design/firn/script-attempt-external-effects.md)
nodes. The network cases live in [tests/pubsub.rs](../../../tests/pubsub.rs).
Reference recordings establish Redis's replies; they do not establish that
Firn compiles, passes those cases or meets the performance criteria below.

### Pub/sub connection ownership

A first successful subscription creates a connection record and commits the
existing reply prefix and subscription acknowledgment under the same hub
hold. The original server loops then end and transfer both TCP halves,
client/parser buffers and transaction state once. The receiver owns input
and EOF observation, the command owner runs one command at a time, and the
output owner alone sends complete frames, retaining its partial-send offset.
An independent hub dispatcher publishes committed actions to retained
inboxes in order and performs no network I/O. Publication bodies use frozen
shared storage; recipient headers remain separate. Disconnect, send failure,
query overflow and pressure cleanup remove all memberships and cancel every
owner. An ordered finish marker allows QUIT and protocol errors to drain
preceding replies before cleanup.

Query bytes include both queued chunks and the command owner's retained
incomplete input until consumption is reported; a receiver closes above one
GiB rather than waiting for private queue space and stranding EOF. Active
subscriptions exempt the connection from the idle timeout. Membership,
configuration and completed I/O activity changes wake receives directly. Final
idle expiry checks authoritative hub membership and latest activity together
with registration removal; a delayed dispatcher notification cannot expire
an already subscribed client. Each parsed promoted command also checks its
live registration and cancellation together before execution, so queued
pressure cancellation cannot admit a later command on an invalidated id.
An admitted command may finish if removal happens after that boundary.
A guarded forwarding context combines those cancellation generations with
connection shutdown into the single watch accepted by the host receive;
its owner disarms it before joining after every receive.

A never-subscribed GET/SET still receives, executes and sends in the original
context. Its dataset atomic statements retain their existing key and optional
Meta targets. Added work is local command classification/admission, a local
promotion check, a larger client record and one retained hub handle created
at connection setup; there is no hub target or queue transfer on these
commands. RESP3 subscribers' ordinary commands use committed queued output
and pay the promoted-path cost. The review fix described below adds no new
ordinary target, runtime ownership predicate or wrapper call: the original
owner instantiates the existing coordinator with compile-time mode zero.
No unused-path timing result or emitted-code comparison is claimed.

Channel and pattern registries are server-wide and separate from shard
channels. Duplicate membership acknowledges without adding another path.
NUMSUB/SHARDNUMSUB preserve operand order and duplicates; NUMPAT counts unique
patterns. RESP2 subscriber admission permits the six subscription commands,
PING, QUIT and RESET, after the reference's earlier admission checks. RESP3
allows ordinary commands. Shared protocol writers own framing, including
push headers; the event module owns membership and delivery, not sockets.

The shared glob matcher now follows Redis 7.2.16's recursive algorithm,
including signed-char range handling, malformed classes, empty-name behavior,
shared suffix-search pruning and refusal above depth 1000. Its existing
public interface remains unchanged. This also corrects the former greedy
matcher's missing reference depth refusal for other glob consumers. The
source's BSD notice is retained beside the port. Stack-ledger evidence is
still required for the maximum 1002 active calls including a rejected call.

### Promoted command visibility and reply ordering

The previous boundary committed a promoted data reply after its command
released the data hold. That admits this incompatible causal history: A
sets `k` to `v`; B reads `v` with GET and publishes `seen`; A receives the
message before its SET reply. Redis 7.2.16's
[SET handler](https://github.com/redis/redis/blob/7.2.16/src/t_string.c),
[EXEC handler](https://github.com/redis/redis/blob/7.2.16/src/multi.c) and
[reply construction](https://github.com/redis/redis/blob/7.2.16/src/networking.c)
queue the complete command reply before another client's command executes.
That ordering applies equally to data-only EXEC and declared-key scripts.

The proposed correction executes an admitted promoted dataset command
through the existing held command parts, holding the whole keyspace, Meta
and event hub until the complete reply is committed. Direct commands retain
ordinary AOF records without an added MULTI/EXEC wrapper. Promoted EXEC
uses the existing event-aware transaction body even for a data-only queue.
Promoted EVAL/EVALSHA starts with the existing whole-map/hub attempt; it
retains Lua KEYS and all acceptance, error and budget-abandonment semantics.
The broader hold reuses the existing transaction/script execution interface;
its effect on promoted-command contention and latency remains unmeasured.

The direct adapter excludes PING, ECHO, TIME, SELECT, COMMAND and OBJECT
HELP from the wider hold. Each uses its ordinary handler; the promoted
command owner then commits the complete reply under the hub as it does for
other standalone control replies. The exclusion is based on command
semantics, not on an empty key-argument list: DBSIZE, KEYS, SCAN and
RANDOMKEY observe the dataset despite having no fixed key operand, and
FLUSHDB/FLUSHALL change it. These remain protected, as do implemented OBJECT
key queries. Semantic validation inside a dataset command may still run
under the wider hold, including unrecognized subcommands of a dataset
family; names without held command parts and command-table arity or
authentication refusals retain ordinary dispatch.

| Standalone control | State it observes or changes | Publication-order obligation |
|---|---|---|
| PING | Optional request payload and the same connection's RESP/subscription mode; its existing AOF-health admission check remains. | No dataset observation orders its reply with another client's data-triggered publication. RESP2's pong array and RESP3's ordinary reply keep their existing handlers. |
| ECHO | Request payload only. | Echoing bytes does not observe the dataset, even if the application chose those bytes after a data read. The application/network barriers and existing output order remain effective without a dataset hold. |
| TIME | The receive-time calendar snapshot already stored on the client. | It neither observes nor changes a dataset version or registry publication; wall-clock values do not create a dataset visibility dependency. |
| SELECT | Validates the requested database index and changes only this connection's database selection in Redis; Firn currently supports zero only. | It observes no key contents. Subsequent commands use the same command owner, so selection-before-command order needs no dataset hold around SELECT itself. |
| COMMAND | Command catalogue/argument metadata; Firn's implemented forms retain their existing empty catalogue replies and unsupported-subcommand refusals. | Command metadata does not observe keyspace state or publication recipients. This scope change adds no new COMMAND support or reply expectation. |
| OBJECT HELP | Static help text. | It has no key lookup. OBJECT IDLETIME/FREQ inspect keys and remain in the data path; excluding the whole OBJECT family would be incorrect. |

The reference command bodies are Redis 7.2.16's
[controls in server.c](https://github.com/redis/redis/blob/7.2.16/src/server.c),
[SELECT in db.c](https://github.com/redis/redis/blob/7.2.16/src/db.c) and
[OBJECT in object.c](https://github.com/redis/redis/blob/7.2.16/src/object.c).
None of the six standalone exclusions requires the wider dataset/reply
boundary. Their replies still commit once, and each connection's next
command starts only after that commitment. A concurrent publication with
no causal dependency may be ordered before or after such a reply; removing
the data hold does not allow it to split a complete reply. Pub/sub commands
retain their own registry/hub boundary. EXEC and EVAL/EVALSHA keep their
existing enclosing transaction/attempt holds, including controls nested in
those units; this direct-dispatch exclusion does not split an execution
unit or change its atomicity.

The promoted direct adapter rechecks AOF health under Meta before the held
handler parses semantic options. Ordinary SET parses those options before
its mutation-time recheck. If admission first observes healthy AOF and the
writer then reports failure concurrently, an invalid SET may therefore
return MISCONF on the promoted path where the ordinary path returns syntax.
Stable healthy and failed states retain the existing precedence; the
overlapping failure and command can be ordered with failure first, consistent
with Redis's pre-execution health refusal. No exact parity of the internal
racing-health observation point is claimed.

There are two necessary ordering directions. If A changes data first, a
conflicting reader cannot observe that change until A releases its data
hold, which happens after A's reply enters the hub order; its later
publication therefore follows the reply. If B executes an atomic
SET/PUBLISH transaction first, its publication enters that same hub order
before it releases the data hold; A cannot read B's value and commit its
read reply ahead of that publication. Reserving A's reply position before
acquiring the data hold would violate the second history: A reserves a GET
reply, B commits SET/PUBLISH, A finally reads B's value but its reserved
reply precedes B's message. No placeholder reservation or deferred busy-client
publication is used.

The original receive/execute/send owner instantiates `execute_client::<0>`;
the promoted owner instantiates `execute_client::<1>`. The coordinator is
the original function body with compile-time dispatch selection, without an
adapter call layer. Ordinary EVAL uses the existing evaluator instantiated
with initial whole mode zero, while promoted EVAL uses one. The pinned
language's FN-2 monomorphization fixes these const arguments; they are not
runtime connection-state predicates. Ordinary GET/SET still call their
original handlers with their existing key and optional Meta targets and no
event-hub target, frame commitment or queue transfer. This source argument
does not establish identical generated instructions or measured timings;
CI emitted-code inspection can check that stronger claim.

This boundary protects admitted dataset execution. Authentication,
pre-command eviction, parser failures and other control replies retain
their existing admission phases; eviction still removes victims in separate
statements. It is not a claim that Firn globally serializes every admission
phase as Redis's event loop does. Once dataset execution starts, reply
commitment precedes releasing that hold. Socket sends never occur under it;
the dispatcher and independent output owners retain committed order even
when a socket makes no progress.

The multi-driver network witness uses four drivers and 200 iterations for
each of direct SET, data-only EXEC containing SET, and declared-key EVAL
returning SET's status. A is subscribed with RESP3 and does not consume its
reply until B's GET observes that iteration's unique value and B finishes
PUBLISH. A must then receive its entire reply followed by the message. The
old interleaving fails that exact assertion, but its failure frequency is
unknown: no old/new run was performed in this round. The race may be too
short to fail reliably in 600 iterations. A passing stress run therefore
supplements, rather than replaces, the ordering argument above.

The transaction-abort witness now gives a real RESP2 subscriber a valid
queued PUBLISH before DISCARD, and another before a wrong-arity command
makes EXEC abort. A subsequent distinct publication is the barrier in each
history: the subscriber's next frame must be that barrier, with no leaked
queued message ahead of it. Expected status, count, EXECABORT and message
bytes follow the pinned Redis command bodies; no new oracle recording or
ORACLE placeholder is needed for these histories.

### Membership confirmations and the Redis suite barrier

The reported regression in
[gate run 38429510997](https://github.com/Ming-Research/Firn-wf/actions/runs/38429510997)
returned one recipient for `PUBLISH chan1.hi hello` after the suite called
argument-free `punsubscribe`. The assumption that this helper had read all
three confirmations is false. Redis 7.2.16's
[util.tcl](https://github.com/redis/redis/blob/7.2.16/tests/support/util.tcl)
passes its default empty name list to `consume_subscribe_messages`, whose
loop runs exactly once per supplied name. It therefore sends PUNSUBSCRIBE
and reads zero replies. A publication on another connection can run first,
legitimately counting the still-subscribed pattern. This missing barrier
already existed in the previously passing versions; neither the promoted
data-boundary change nor the subsequent control narrowing changed registry
mutation or pub/sub reply commitment. Timing can expose the test race, but
the failed run supplies no command-execution trace identifying its exact
interleaving.

Firn's existing subscription handlers add or remove forward indexes and
reverse names before constructing each confirmation. Argument-free removal
consumes every name and updates its index before appending that name's
confirmation; explicit removal uses the same index operation. Direct
commands retain one hub hold across the handler and complete reply
commitment. The dispatcher must acquire that same hub before taking the
committed Send action, and publishers consult those authoritative indexes
under the hub. Thus a confirmation received on the wire implies the
preceding membership change is visible to a later publisher. RESP2 versus
RESP3 changes framing, not this boundary. The receiver's asynchronous
subscription-class notification is not the publisher's source of truth.
This also holds for subscription commands nested in the existing EXEC
boundary. No server-ordering repair is supported by this source audit.

The suite runner applies one
[contextual patch](../../../tests/redis-suite/pubsub-barriers.patch) to
all targets, preserving every original argument-free wire command and
recipient/count assertion. It explicitly reads the three known names and
requires confirmation counts 2, 1, 0 in the ordinary, pattern and shard
removal cases. Two argument-free NUMPAT cleanup calls similarly drain their
two known patterns and require 1, 0. The original helper still checks each
message kind and each unordered name exactly once. A general loop until
zero is unsuitable: ordinary channels and patterns share a combined count,
so removing one family need not reach zero. Run metadata names the patch;
a native forward-only, zero-fuzz application fails on stale context. A
[CI input check](../../../tests/redis-suite/check-pubsub-barriers.py) uses
fresh copies of the pinned files to require successful application and
rejection of repeated application, a missing target file and a changed test
heading. The patch and its input check are removed when the pinned upstream
suite supplies these barriers. Compilation, patch application and network
execution of this correction remain CI evidence to obtain; no local run is
claimed.

The four-driver network witness independently repeats the three-pattern
sequence and both subscription directions. For each of RESP2 and RESP3,
ordinary/pattern/shard families and explicit/argument-free removal, it drains
all three confirmations, then immediately requires publication counts one
after subscription and zero after removal on the other socket. All three
channels are checked and each live delivery is read. A final PING catches
leaked removed deliveries. Each of the twelve combinations runs 100 rounds;
there is no sleep, polling-to-pass or intervening registry query hiding the
confirmation boundary. The case is a concurrency witness, not a proven
detector: it rejects a server whose registry is still unchanged when the
other socket's publication runs, but a server that queued the confirmation
early and finished the registry change before that publication arrived
would still pass, and no CI run of a deliberately reordered server is
recorded. The ordering itself rests on the registry change preceding the
confirmation in the same held section. Nor is this a claim that the previous
server violates that boundary or that the missing-barrier suite fails at a
known frequency.

RESET already removes all three indexes under the hub before its reply is
formed. Its four-driver witness repeats 100 rounds per starting protocol,
combines three memberships from each family, reads +RESET, then immediately
requires zero from ordinary and shard publication before observing all
registry counts at zero. PING detects leaked output, and an empty
UNSUBSCRIBE checks RESET's restoration of RESP2.

Client-local socket close has no membership confirmation: completion of a
local close does not mean the server has observed EOF. A different client's
immediate NUMSUB/PUBLISH therefore has no removal barrier from that local
event. Cleanup remains eventual once EOF is observed. Firn removes indexes
under the hub before cancelling owners; pressure removal does likewise.
The output owner also forgets the registration before closing its send half.
QUIT's +OK is an ordered finish response, not an unsubscribe confirmation;
its queued output drains before cleanup. No new immediate-removal assertion
is attached to local close or to QUIT's status reply.

### Pub/sub script attempts

A declared-entry attempt encountering PUBLISH, SPUBLISH or PUBSUB introspection
abandons through the existing uncatchable whole-map retry before observing
membership. Its existing journal restores preceding writes and access stamps.
Every whole-map attempt holds the hub from its start together with keys and
Meta, including nonpublishing whole-map scripts, because those attempts have
no general rollback boundary for a later wider hold. Promoted connections
start with that broad hold even when every access is declared, so their
accepted result commits before data visibility as described above. A
never-promoted declared-only script retains its original declared-entry
path. Whole-map and promoted script hub contention remains unmeasured.

Publication counts and recipients use the held membership snapshot. Frames
remain in an attempt-local stage until the final Lua result or error reply
is formed; accepted errors retain preceding publications. Actual staged
recipient deliveries prevent interpreter-budget abandonment; a zero-recipient
publication has no external delivery to duplicate and can retry. Publications
add no AOF dataset records and do not set script WRITE_DIRTY. The complete
outer script or EXEC reply commits before self-publication frames. Script
callbacks retain the real subscriber protocol in the hub while using Lua's
selected response protocol, and ordinary callbacks temporarily clear the
network client's membership counts to model Redis's unsubscribed script
client for PING.

The existing busy-script limitation remains: once an attempt must continue
without releasing its holds, SCRIPT KILL cannot observe it through the pool
between attempts. The implementation does not introduce an in-attempt publication
channel to solve the already-recorded Whitefoot requirement. Its expanded
external-effect path needs that limitation included in acceptance reporting.
A minimal three-session example is A: `SUBSCRIBE news`; B: `EVAL
"redis.call('PUBLISH','news','once'); while true do end" 0`; C: `SCRIPT KILL`.
Redis's dataset-unwritten publisher remains killable after its busy boundary;
Firn's selected delivery prevents budget abandonment, so it cannot reach its
between-attempt kill check. This part remains dependent on the existing
Whitefoot request `firn-req-attempt-publish`, not claimed as reference-complete.
A zero-recipient publication remains retryable. No new compiler diagnosis was
produced on this machine.

### Pub/sub output accounting

Active pub/sub connections enforce the reference numeric defaults: hard at
at least 33,554,432 charged bytes and soft at at least 8,388,608 charged bytes
continuously for more than 60 observed Unix seconds. Falling below the soft
threshold, or leaving subscriber mode, clears the interval. A deadline queue
and cancellable timer generation drive closure without periodic polling.
Each frame charges its prefix and body reserved byte capacities until the
whole frame owner is released, including hub, dispatcher, inbox and partially
sent ownership. A shared body is charged in full to each recipient's limit;
physical process heap accounting retains one allocation while references
share it.

This accounting excludes allocator rounding, reference-control and queue
storage overhead, whereas Redis includes dynamic reply-allocation and
block/list overhead. Exact allocation values and the publication number at
which a slow client closes therefore differ; no allocator-equivalence claim
is made. The progress case still only checks that the other subscriber
receives. A separate hard-limit witness publishes one 33 MiB body to an
unread subscriber's exclusive channel, requires NUMSUB to reach zero and
then EOF or connection reset, and checks a second subscriber on a small
heartbeat channel remains live. Disabled limits cannot pass that zero-count
and closure requirement. Redis charges the large reply allocation and checks
its hard limit while constructing the command reply, before socket flushing
([output limit check](https://github.com/redis/redis/blob/7.2.16/src/networking.c),
[defaults](https://github.com/redis/redis/blob/7.2.16/src/config.c)); the single
33 MiB body exceeds the 32 MiB threshold without relying on kernel buffering.
CI compilation and execution remain required. The 60-second soft threshold
and interval reset are still unverified. A controlled accounting witness
must hold a backlog above the soft threshold without reaching the hard
threshold, then exercise a drop
below and re-entry. Compare the reference and Firn without assuming identical
kernel socket buffering.

The timer reads Unix time and arms a monotonic sleep for the next observed
threshold. Strict response to a forward calendar jump without another hub
wake retains the research note's missing host-notification limitation; no
periodic poll was introduced. Cancellation/partial-send races, old-id retained
envelopes after cleanup, query overflow and idle exemption also need runtime
evidence. The existing host calendar-jump notification and script
attempt-publication requirements remain open; this design does not close them.

CLIENT INFO uses local sub/psub/ssub counts. The hub exports a nonwaiting
`client_counts` observation keyed by the network client id; CLIENT LIST can
hold its existing table and the hub together and obtain consistent counts
and subscriber classification. This avoids delayed client-table count
publication and adds no table/hub target to ordinary GET/SET. A combined client table must use that observation for its formatting and
subscriber classification; hardcoded zero counts do not compose with this
interface.

### Recorded pub/sub transcripts

[Redis 7.2.16 recording run 38399914253](https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253)
recorded all 12 [input batches](../../experiments/pubsub/oracle-batches.json).
Each command's reply is every byte received within 0.3 seconds, decoded as
Latin-1: ordinary replies, pushes and acknowledgments are concatenated in
wire order. The [test fixture](../../../tests/pubsub_oracle.rs) preserves all
89 command drains and 3,410 reply bytes. Its batch names retain the original
ORACLE prefix as identifiers; their expectations are now recorded bytes.

The ordinary promotion, RESP2 restrictions, self-publication after command,
EXEC and Lua replies, accepted script error, direct-plus-pattern fanout and
shard namespace results agree with the source-derived expectations within
the recorded command contexts. A two-client fanout count is not compared as
if it had the recorder's single client. RESP2 deliveries on a separate socket
are not claimed byte-identical to RESP3 self-pushes. Other network cases still
use the reference source and require their own execution evidence.

The raw replay case consumes known transcript lengths instead of interpreting
an EXEC header as a complete framing boundary. All fields and frame markers
are compared byte for byte; only dictionary iteration order in no-argument
UNSUBSCRIBE is allowed to permute its two member names. The fixture retains
the actual observed order. Count progression, channel identity, aggregate
header and following PING bytes are unchanged. This is the same unordered
enumeration rule used by the original membership tests and registry node.

## Acceptance cases and ORACLE recordings

The original study proposed the following independent acceptance cases;
pub/sub now has registered cases and recorded replies as linked above. The
remaining proposals use upstream
[multi tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/multi.tcl),
[list tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/type/list.tcl),
[pub/sub tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsub.tcl),
[shard tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsubshard.tcl)
and scripting tests as independent oracles. Implement only cases contributing
observations missing from existing coverage; every regression case must fail
for its intended defect before acceptance.

| Proposed case | Failure it must detect |
|---|---|
| SELECT isolation, pipelined SELECT, cross-db EXEC, script-local SELECT | A stale selected map or applying a command to the previous database. |
| MOVE with live/expired/missing destination and absolute TTL | Nonatomic transfer, lost TTL, wrong error precedence, omitted expiry effects. |
| COPY of the same key name into another DB, with/without REPLACE and through EXEC/script/AOF replay | Retaining the DB 0 restriction, false same-key errors, source loss, lost TTL, missing destination watch/readiness hooks or replay in the wrong database. |
| SWAPDB with pre-existing connections, watches, waiters and queued expiries | Swapped handles, moved registrations, missed ready notification or deleting from the wrong database. |
| FLUSHDB/DBSIZE/SCAN with expired and observation-only cells | Cross-db clearing, ghost keys becoming visible, or incorrect physical-size semantics. |
| Mixed-db AOF restart/rewrite with EXEC, scripts, MOVE and swap | Missing SELECT, coalescing same-named keys across databases, replaying an effect twice. |
| WATCH missing→create→delete, same-value SET, failed NX, own write, duplicate WATCH | Lost ABA, value-equality approximation or false invalidation. |
| WATCH live→expired without active removal; already-expired WATCH then physical removal; flush/swap combinations | Missing deadline check or confusing physical cleanup with a new logical change. |
| WATCH eviction with one eligible victim, EXEC/DISCARD/UNWATCH/reset/disconnect cleanup | Omitted internal mutation hook, lingering observation or wrong CAS/error precedence. |
| Block registration race and FIFO, duplicate keys, two ready keys, move chains | Lost wake, duplicate service, scheduler-order substitution or incorrect ready-batch ordering. |
| Source wrong type while blocked; earlier-key wrong type at retry; destination wrong type with another waiter | Waking for the wrong event, replaying only the triggering key, or removing a value before destination validation. |
| Producer push/delete inside EXEC/script; all four commands inside scripts/EXEC | Premature wake or blocking inside a no-blocking execution unit. |
| Fractional timeout, original deadline after retry, reply/timeout/EOF races, queued suffix | New timeout granted on retry, duplicate reply, consumed value after disconnect or a stranded pipeline. |
| Ordinary/shard namespace separation, duplicate memberships, NUMPAT and direct+pattern fanout | Wrong counts, pattern delivery on shard publish or lost multiplicity. |
| RESP2 restrictions/PING; RESP3 self-publication under EXEC/script | Wrong framing, command admission or pushes inserted inside an outer reply. |
| Pub/sub from an abandoned script attempt and from a final script error | Duplicate publication on retry or dropping accepted pre-error effects. |
| Slow subscriber, disconnect during fanout, reused connection slot, idle exemption | Publisher stalls, stale delivery, leaked queued bytes or closing a legitimate blocked/subscribed client for idleness. |

The pub/sub transcripts below are recorded. The later blocking-timeout and
WATCH transcripts still have expected output **ORACLE** and need a Redis
7.2.16 recording in owner-driven CI before their bytes enter tests. Commands below are command batches, not shell invocations. Use
fresh connections/databases, raw RESP capture and explicit barriers.

**O1 — recorded subscription acknowledgments within EXEC.** Each batch uses
a fresh connection; RESP3 negotiation precedes the batch. These bytes come
from the linked Redis 7.2.16 run, with literal CR/LF escaped below. The complete
command/reply sequences are in the test fixture.

`ORACLE-step4-multichannel-subscription-exec-resp2` (RESP2):

```text
MULTI
SUBSCRIBE first second
PING tail
EXEC
UNSUBSCRIBE
```

Recorded EXEC drain:

```text
"*2\r\n*3\r\n$9\r\nsubscribe\r\n$5\r\nfirst\r\n:1\r\n*3\r\n$9\r\nsubscribe\r\n$6\r\nsecond\r\n:2\r\n*2\r\n$4\r\npong\r\n$4\r\ntail\r\n"
```

`ORACLE-step4-multichannel-subscription-exec-resp3` (RESP3):

```text
MULTI
SUBSCRIBE first second
PING tail
EXEC
UNSUBSCRIBE
```

Recorded EXEC drain:

```text
"*2\r\n>3\r\n$9\r\nsubscribe\r\n$5\r\nfirst\r\n:1\r\n>3\r\n$9\r\nsubscribe\r\n$6\r\nsecond\r\n:2\r\n$4\r\ntail\r\n"
```

`ORACLE-step4-unsubscribe-all-exec-resp3` (RESP3):

```text
SUBSCRIBE first second
MULTI
UNSUBSCRIBE
PING tail
EXEC
```

Recorded EXEC drain:

```text
"*2\r\n>3\r\n$11\r\nunsubscribe\r\n$5\r\nfirst\r\n:1\r\n>3\r\n$11\r\nunsubscribe\r\n$6\r\nsecond\r\n:0\r\n$4\r\ntail\r\n"
```

The additional MULTI; SSUBSCRIBE news; EXEC batch records:

```text
"*1\r\n-ERR SSUBSCRIBE isn't allowed for a DENY BLOCKING client\r\n"
```

These aggregates retain Redis's historical multiple-acknowledgment framing.
No-argument UNSUBSCRIBE traversal differs across recorded connections; its
wire types, member set and integer countdown remain fixed.

**O2 — timeout syntax and range.** Start with an empty database. Queueing
inside MULTI prevents a rounded-to-zero timeout from hanging:

```text
MULTI
BLPOP missing -0.0001
BLPOP missing -0.001
BLPOP missing 0.0000000000000000001
BLPOP missing 0x1p-10
BLPOP missing 1e-5000
BLPOP missing 9223372036854775.807
BLPOP missing 0.00100000000000000001
EXEC
```

Expected raw transcript: **ORACLE**. For the last decimal and adjacent
representable boundaries, also record the timeout-conversion result using a
source-derived CI instrument against the reference conversion routine;
network elapsed time cannot reliably distinguish one from two milliseconds.
The analysis suggests a binary64/extended distinction here but records no
measured result. Include NaN, infinities, leading/trailing whitespace and
embedded NUL as separately encoded bulk arguments in the numeric conformance
work; their exact transcripts remain **ORACLE** until read/recorded there.
The explicit additional argument batch is: `nan`, `inf`, `-inf`, `" 1"`,
`"1 "`, and the three bytes `31 00 30` (hex), each as the timeout of
`BLPOP missing <argument>` in its own MULTI/EXEC. Quotes denote argument
boundaries, not literal quote bytes.

**O3 — already-expired watch and physical cleanup.** Use two connections and
the upstream test harness's expiry-control facility; first verify that the
fixture can leave an expired key physically present. A creates `k` with a
short TTL; after expiry but before physical removal, A sends `WATCH k`, B
sends `GET k` to force cleanup, then A sends `MULTI`, `PING`, `EXEC`.
Repeat with B replacing the expired key with a live value, and with
`FLUSHDB` and `SWAPDB 0 1` into live/expired/missing counterparts. Expected raw
transcripts: **ORACLE**. The command order and the physical-expiry fixture are
part of this batch; ordinary sleep alone does not establish its precondition.

## Performance experiment specified before implementation

Question: does each feature add measurable cost to commands that do not use
it, particularly the single-db workload and unwatched single-key writes?
The competing hypotheses are that cached handles/conditional paths preserve
the existing work, versus representation, lock or queue changes impose cost.
A source argument proves neither elapsed time nor absence of cache effects.

The owner runs all builds and experiments in CI. Use `make firn-lto` and the
existing [redis-bench instrument](../../experiments/redis-bench/redis-bench.sh)
on the 14900K under the project's CPU-placement/session-cleanup rules. Record
exact source revisions, compiler pin, driver count, CPU placement, client
count, pipeline depth, key distribution, payload size, AOF settings,
maxmemory policy, dataset size and live-feature population.

For each step, use same-source variants differing only in the mechanism
being attributed: baseline A, changed B, and an independent build/run twin
A′ of baseline A. Keep toolchain and flags identical. Interleave and alternate
order across blocks, for example A/B/A′ then A′/B/A. Reuse a fixed workload
seed and restore the same initial dataset/connection state per run. Include
the total change against the original baseline as well as stepwise comparisons;
several individually small costs must not escape by accumulation.

First run the smallest useful short sample on one configuration, record its
duration and spread, then choose repetitions and duration. Do not start with
hours of runs. Before the confirmatory run, freeze the twin's noise interval
and the required precision; the pilot is exploratory. Report paired B/A and
A′/A ratios, a paired uncertainty interval, and raw runs rather than only a
pooled best throughput. A proposed decision rule is a reproducible B/A
throughput loss whose paired 95% interval lies below both 1 and the twin's
lower noise bound, or a latency increase outside the equivalent twin bound.
If the interval overlaps the bound, report the largest loss still compatible
with it; do not equate lack of significance with zero cost. A wide interval
requires more evidence or an inconclusive verdict, not a pass. This
statistical rule is a proposed implementation acceptance rule for owner review.

| Comparison | Workloads that distinguish the cost |
|---|---|
| Databases alone | db-0-only GET hit/miss, SET overwrite/create, INCR and small list operations, without issuing SELECT; AOF off and everysec on; one, two and available higher driver counts; pipeline 1 and 16. Also compare one SELECT at connection setup to repeated SELECT, separately. |
| WATCH representation | Same single-key writes and read misses, zero watches; unrelated active watches; hot watched keys separately. Inspect cell size, map occupancy, missing-key claim behavior and lock ledger. |
| Blocking infrastructure | Ordinary GET/SET, list pushes with no waiters, idle waiters on other keys, and active same-key FIFO/move-chain service; measure CPU and context wakes as well as throughput/latency. |
| Pub/sub infrastructure | Never-subscribed GET/SET, RESP3 subscriber ordinary commands, PUBLISH with zero/one/many recipients, pattern misses, and one deliberately slow reader; report bytes retained and healthy-recipient latency. |
| Cross-feature load | Expiring keys plus WATCH; blocked moves with eviction; cross-db transactions/scripts; mixed publications and AOF rewrite. These test contention and correctness, not an attribution of one mechanism. |

Falsifiers: a lock ledger that adds hub/Meta to the supposedly untouched
keyed path rejects that cost argument before timing; a B-only slowdown outside
twin noise rejects the no-measurable-cost claim; a direct-map-wait wake count
growing with unrelated blocked clients rejects its scalability argument; a
subscriber's stalled socket delaying another recipient rejects output
independence. For databases, detected single-db cost means retain db 0 only
and report the result to the owner. For WATCH, do not ship a measurable
single-key regression as satisfying this task. No performance acceptance is
marked passed by this document.

## Independently reviewable implementation order

These are proposed mergeable units, not a new maintained TODO list. The
status board remains the work tracker. No step claims readiness while its
acceptance is missing; owner-driven CI and review precede delivery.

1. **Resolve capability witnesses and database cost direction.** Compile
   natural multi-map and ownership witnesses in CI; obtain the Whitefoot
   decision for exact timeout conversion and, if required, calendar-jump
   parity. Produce a minimal cached-db-handle comparison with its twin before
   committing to sixteen databases. Accept only a compiled natural interface
   and a bounded cost result; an unresolved gap stops its dependent part.
2. **Database support as one coherent feature.** Implement stable maps,
   selection, MOVE/SWAPDB/FLUSHDB/DBSIZE and COPY DB, enumeration/expiry, global eviction,
   transaction/script database context, replay and rewrite together. This
   can merge independently of connection waits once isolation, persistence
   and the no-measurable-single-db-cost condition pass. If that condition
   fails, retain db 0 and use explicit db identity zero in later interfaces.
   Do not expose SELECT 1 while persistence still silently targets db 0.
3. **WATCH representation and lifecycle.** Migrate logical cells and all
   consumers, implement registration/check/cleanup and every mutation hook.
   Accept the upstream and discriminating ABA/expiry cases plus unwatched
   single-key cost evidence. This does not depend on socket fanout and can
   merge separately. Stop on a natural representation/proof rejection.
4. **Shared connection lifecycle with a real consumer.** Introduce promotion,
   socket owners, committed output, generation-safe cleanup, queued input,
   push writer and cancellation. Land with ordinary SUBSCRIBE/UNSUBSCRIBE,
   PUBLISH and channel/count introspection so the infrastructure is exercised,
   not unused scaffolding. Include MULTI/script staging and RESP2/RESP3
   admission in that vertical slice. Accept framing, self-publication,
   disconnect, partial sends, pressure behavior and unused-path comparison.
5. **Pattern and shard pub/sub.** Add their indexes, glob behavior, complete
   introspection and acknowledgment/count distinctions atop the same output
   path. Accept direct-plus-pattern multiplicity and shard isolation without
   changing ordinary channels. This is independent of list parking.
6. **Blocking pop lifecycle.** Once timeout capability is settled, add
   BLPOP/BRPOP with atomic registration, whole-command replay, FIFO ready
   phases, timer/EOF arbitration, no-blocking contexts and effect-only AOF.
   Ready hooks must cover every existing value-introducing path, not just
   pushes. Accept race/fairness/type/timeout/pressure cases and list-path cost.
7. **Blocking moves.** Add BRPOPLPUSH/BLMOVE using the same registration and
   reply lifecycle, with destination validation, source preservation,
   chained readiness and independent resumed-effect boundaries. Accept the
   upstream move-chain, wrong-destination and WATCH cases. Later sorted-set
   and stream waits get their own semantic adapters and acceptance rather
   than expanding this step silently.

Each implementation PR must propose the corresponding live design changes:
database identity and scope; observation cells and WATCH publication;
connection ownership/output ordering; blocked-command service ordering and
timeout semantics; script attempt external effects; AOF database context;
memory/output-limit accounting. The command-parts, transactions, scripts,
enumeration, protocol, memory, shutdown and AOF trees named above need
correspondence review. The original study changed no design nodes. The pub/sub implementation now
proposes the three nodes linked above; the remaining alternatives are still
research proposals.

## Remaining uncertainty and disposition

The major unresolved outcomes are compiler acceptance of natural cross-map
interfaces, timeout numeric/clock facilities, single-db and WATCH measured
cost, and the raw transcripts marked ORACLE. The broader list-producer hold
is correct in intent but its full admission/eviction helper composition and
performance are unverified. The hub's one-to-many throughput and connection
promotion complexity are also unmeasured. The pub/sub draft implementation
and its reference-recording boundary are described separately above; the
other feature families remain unimplemented proposals.

The design examination found three concrete integration obligations beyond
new command names: preserve the AOF-off no-Meta fast path; include externally
observable publication/WATCH/readiness in script attempt acceptance; and
separate logical dataset keys from observation storage in every enumerator
and persistence path. They are included in the proposals above. Extended
numeric support and strict clock-jump notification are missing pinned
facilities; multi-map alias proof is a suspected gap requiring a compiler
witness, and map-wide wake amplification is an inspected performance
limitation. No workaround, external request filing, pin movement or claimed
validation run accompanies this study.

### Review evidence

A separate GPT-6.1-sol read-only review covered this research change against
the stated Firn revision, relevant design ancestors and nodes, affected source
and available fixed-version reference evidence. It found and rechecked the
missing COPY DB integration, now included with its watch/readiness, TTL,
transaction/script and replay obligations. A final limited review checked
the command-clock clarification against `serve`. No findings remain within
that review's scope. G2/G3, DC1/DC2, A1, R1 and D1 passed as a study;
G1/DC3 and code/test/pin checklist groups were not applicable. DC4 remains
unverified for implementation, compiler acceptance and measured behavior.
The reviewer did not independently retrieve every Redis timeout/command-flag
source; those claims rely on the source investigation, not an additional
independent check. No suite was run or claimed green.
