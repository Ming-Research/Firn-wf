# Orderly stop

## The question

Redis 7.0.15's `SHUTDOWN`, and its handling of SIGTERM and SIGINT, append and
sync the append-only file, close every client and end the server at once.
firn ends only when its entry returns, and Whitefoot's spawns are structured:
the entry joins every context it started, so every client's context, the
accept loop and the writers must reach their end first
([kernel specification v0.94, WAIT-3's join rules](https://github.com/Ming-Research/Whitefoot/blob/0b7f5c5b98547dd27deb9691aed36281e350576b/spec/kernel-spec.md)).
A context waiting in `tcp_accept`, `receive_next` or `send_once` with no
deadline waits until the host produces an outcome, and v0.94 has no way to
cancel such a wait from another context. A stop request can therefore reach
every context in one of two ways:

- **Polling.** Every wait carries a deadline of at most a second, and each
  context reads the keyspace's `stopping` when its wait ends. `serve`
  (`firn/server/server.wf`) already does this while an idle limit is set.
- **A Whitefoot capability.** The language or its library lets a stop end
  the waits of other contexts, or lets the program end without joining them
  once its files are synced.

Polling needs no Whitefoot change. Its cost is a monotonic clock read and a
timer armed and removed on every receive that parks, which in an unpipelined
request and reply is every request (`wf_context_arm_deadline` in Whitefoot's
`compiler/src/backend/completion/bridge.c`). The question is whether that
cost is measurable on firn's throughput.

## The comparison, stated before measuring

- Workload: `redis-bench.yml` mode compare on the i9-14900K. Revisions:
  - `base`, Firn-wf main 936110104;
  - `head`, the same with a receive deadline of at most a second armed
    whether or not an idle limit is set;
  - `head-twin`, head's image again, as the noise control.
- Tests `set` and `get` at pipeline depths 1 and 16, on 1 and 2 server CPUs.
  A probe of 2 passes of 5 seconds sizes the run; the decision rests on the
  sized run.
- **Rejection rule.** Polling is rejected if head's median rate at depth 1,
  for either test on either CPU count, is lower than base's by more than 2%
  and by more than the difference between head and head-twin there. The
  alternative is then to ask Whitefoot for the capability above. Depth 16
  is reported, not decided on, since a deep pipeline parks once per batch.

## Results

**Probe.** Run [37570894247](https://github.com/Ming-Research/Firn-wf/actions/runs/37570894247)
compared base 936110104, head 54268619b (branch `claude/stop-poll-probe`)
and head-twin. It ran 2 passes of 5 seconds. Each cell is the median rate,
and the last column is the difference between head and head-twin:

| depth | CPUs | test | base | head | head / base | head and twin |
|---|---|---|---|---|---|---|
| 1 | 1 | set | 147k | 152k | +3.3% | 1.0% |
| 1 | 1 | get | 155k | 152k | -2.3% | 2.9% |
| 1 | 2 | set | 264k | 263k | -0.4% | 2.5% |
| 1 | 2 | get | 272k | 278k | +2.2% | 3.1% |
| 16 | 1 | set | 1751k | 1804k | +3.0% | 3.7% |
| 16 | 1 | get | 1931k | 1892k | -2.0% | 1.8% |
| 16 | 2 | set | 3137k | 3181k | +1.4% | 2.4% |
| 16 | 2 | get | 3361k | 3469k | +3.2% | 5.7% |

At this resolution, about 3%, the probe shows no cost. That is too coarse
for the 2% rule. The sized run takes depth 1 alone, 6 passes of 10 seconds
on 1 and 2 CPUs.

**Sized run.** Run [37573701622](https://github.com/Ming-Research/Firn-wf/actions/runs/37573701622)
compared the same images at depth 1, 6 passes of 10 seconds each. head's
only change from base was the receive deadline that the implementation on
this branch also makes: in `serve`, the deadline of at most a second is
computed whether or not an idle limit is set. Each cell is the median rate;
the next columns give the server's processor time per request:

| CPUs | test | base | head | head-twin | head / base | head and twin | base µs | head µs |
|---|---|---|---|---|---|---|---|---|
| 1 | set | 152.1k | 150.2k | 151.0k | -1.25% | 0.51% | 6.56 | 6.65 |
| 1 | get | 154.0k | 153.3k | 154.3k | -0.45% | 0.63% | 6.49 | 6.52 |
| 2 | set | 259.7k | 265.9k | 269.1k | +2.36% | 1.21% | 7.67 | 7.51 |
| 2 | get | 268.9k | 267.3k | 271.9k | -0.59% | 1.71% | 7.43 | 7.47 |

**Decision under the rule.** No test on either CPU count loses more than
2%, so polling is not rejected.

**What remains uncertain.** On one CPU, `set` loses 1.25%, more than the
0.51% between head and its twin, and its processor time per request rises
by 1.2%. A cost of about 1% on unpipelined writes on one CPU is therefore
not excluded; it lies below the rule's threshold. The rule weighs nothing at
depth 16, where a batch parks once.

## Replacing polling with cancellation

### Question and comparison, stated before measuring

Do Whitefoot v0.119's shared cancellation states end firn's shutdown waits
promptly and recover the cost of arming a timer for each parked receive,
while preserving replies, live idle limits and the final append-only drain?
This is the work of [board item `firn-cancel-adopt`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip).

Run `redis-bench.yml` in `compare` mode on the i9-14900K, building both
revisions with `make firn-lto` and their pinned Whitefoot release:

- `base=11a3fb546f6516373b158e2a0af7764d8fc3d48f`, main when this work began;
- `base-twin=11a3fb546f6516373b158e2a0af7764d8fc3d48f`, the base image again;
- `head=claude/cancel-adopt`, resolved to the revision tested;
- `head-twin=claude/cancel-adopt`, the head image again.

Use interleaved `set` and `get` runs at pipeline depths 1 and 16 on 1 and
2 server CPUs, with the same settings for every image. Probe with 2 passes
of 5 seconds; inspect the spread before choosing any longer run. The twin
of each image controls for noise without another source change.

**Prediction.** With receive timers removed, head's median throughput at
depth 1 is at least base's, since parked receives no longer arm a timer.
**Rejection rule.** Reject that prediction if head's depth-1 median for
either command on either CPU count is below base by more than 1% and by
more than the absolute relative difference between head and head-twin in
that cell. Report depth 16 as well. The comparison requires head to drop the
per-receive timer when timeout is zero; it must first support the wake
generation described below. That implementation is now written against the
pinned v0.119 interface; validation and measurement remain pending. No
measurement has been made for this comparison.

The network cases can observe shutdown latency with an established client
left connected and idle, timeout zero, and no append-only writer: start the
clock before a fresh PING, read its PONG, send SHUTDOWN on another established
connection and require successful process exit within 500 ms while both
peers remain open. Starting before PING prevents a test-side scheduling
delay from consuming the old one-second poll before the clock starts.
Run on both host I/O routes. This is a regression bound with scheduling
margin, not a precise cancellation-time measurement; CI still must establish
that the old revision fails it and the new revision passes under load.

### Pending client replies

Redis 7.0.15's
[`prepareForShutdown`](https://github.com/redis/redis/blob/7.0.15/src/server.c#L4083)
and [`finishShutdown`](https://github.com/redis/redis/blob/7.0.15/src/server.c#L4165)
wait for replicas when required, flush and sync the AOF, and make a
best-effort replica-output flush before exit. They do not drain ordinary
clients' pending replies. In
[`networking.c`](https://github.com/redis/redis/blob/7.0.15/src/networking.c#L1957),
`writeToClient` leaves unsent bytes for a later writable event;
[`flushSlavesOutputBuffers`](https://github.com/redis/redis/blob/7.0.15/src/networking.c#L3846)
visits replicas alone. Exiting need not deliver a complete ordinary reply.

Keep `send_all`'s existing `cancel_never()` watch and deadline rule: retry
one-second deadlines during normal service, but abandon the remaining bytes
when a deadline passes after the shutdown request. This preserves firn's
existing bounded opportunity to send a reply, including a peer that resumes
reading during shutdown. Firing the shutdown watch must not additionally
truncate it. Redis's source supports allowing incomplete replies at exit;
it does not specify firn's one-second grace interval or require a full flush.
Successful SHUTDOWN still discards its own connection's unsent replies,
including earlier commands from the same read, and sends no success reply.

### Ownership and drain

Main creates the shared keyspace, whose server state owns an initially
unfired receive wake source, and a separate shutdown source before spawning.
It shares shutdown sources with clients and the signal context, gives expiry
a shutdown watch, and owns the accept watch. Keeping that source separate
means a CONFIG update cannot end accept, signal or expiry waits; those
contexts need no generation refresh.

A successful SHUTDOWN records the request and retains the current receive
source in the same atomic statement, guarded against manifest installation,
then fires that source outside the statement. Command dispatch closes its
client and returns true only for that accepted request. The client's context
then fires its separate shutdown source without another shared-state read.
Authentication and option refusals return false. Standalone dispatch used by
replay also records and fires the receive generation; no network contexts
are serving during replay, and it needs no separate shutdown source.

The signal context closes the listener before recording and firing the
request, preserving the second-signal escape. Main retains the existing
client-limit meaning: stop accepting after the limit, let those clients
finish, then record shutdown with Meta.stopping, retain and fire the current
receive source, and fire the separate shutdown source. All fires occur
outside atomic statements: v0.119 declares cancel_fire as waiting.

Sources and watches now drop their shared handles. Replaced sources drop
after firing, stale watches drop on replacement, and the final receive source
drops with shared server state, including startup failures. Explicit watch
closure remains at connection and signal-listener cleanup to mark that
release point. Other contexts' watches retain the cancellation state
independently.

The AOF writer and rewrite cycle retain their never-firing watches, periodic
drains, installation exclusion and structured joins. They must keep appending
and syncing until main observes no clients; the request alone cannot end
them. The drain case holds a client inside a large send with a write already
in its input batch, then lets it continue after shutdown and verifies that
write on replay. This replaces the old reliance on an idle client accepting
a new request during its one-second shutdown poll. Its bounded retries
cover TCP splitting the small pipelines or the blocked send reaching its
deadline first; none counts as a passing drain observation.

The second-signal case uses an unfinished script to keep a client alive,
and another client's EOF after the first signal proves the request was
recorded after the listener closed. It no longer relies on staggered receive timers
to prolong the drain. Script waits, including the guarded atomic statement
that takes the engine and the retry sleep, remain unchanged: guard
observation of cancellation was added by
[Whitefoot PR #304](https://github.com/Ming-Research/Whitefoot/pull/304), but
adopting it for script waits is outside this receive-generation change.

### Live idle-limit updates

The existing compatibility contract and
`firn_applies_an_idle_limit_to_a_previously_unlimited_receive` require this:

1. Client A sends PING with `timeout` zero and then parks in a receive.
2. Client B sends `CONFIG SET timeout 1` and receives OK.
3. Client A closes under the new limit without sending any more bytes.

Redis 7.0.15's
[`clientsCronHandleTimeout`](https://github.com/redis/redis/blob/7.0.15/src/timeout.c#L57)
compares each ordinary client's last interaction with the current
`server.maxidletime`, which its
[`timeout` configuration](https://github.com/redis/redis/blob/7.0.15/src/config.c#L3078)
updates. The comparison does not require another client request.

A receive with no deadline and only the unfired shutdown watch has no input
that can wake it at step 2. Firing that watch would permanently cancel the
server's other waits, even though no shutdown was requested. A limit reduced
from five seconds to one also has to reach an already parked receive; the
existing idle-limit case covers that boundary.

The owner selected option A, wake generations, on
[card `firn-adopt-timeout-wake`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip).
Whitefoot v0.119's droppable CancelSource and CancelWatch structs, adopted
through release `wf-f3d081b90a8d`, permit the source in shared server state
([Whitefoot PR #319](https://github.com/Ming-Research/Whitefoot/pull/319)).
The implementation has this lifecycle:

1. Shared server state owns an initially unfired source. A client reads the
   timeout, shutdown request and a watch of that source together before its
   first receive, then retains the watch across receives.
2. Each actual idle-limit change creates a fresh source, swaps it for the
   current source in the same atomic statement that changes the limit, and
   takes the old source out. The caller fires the old source outside the
   statement before replying OK, then lets it drop. Refused CONFIG changes
   and setting the same value neither replace nor fire a generation.
3. A receive has no deadline at timeout zero and otherwise has the remaining
   idle interval, counted from the existing last-activity time. A deadline
   or cancellation re-reads shutdown and timeout together with the cached
   watch's fired state. If shutdown is false, a fired watch must belong to a
   replaced generation: only a replacement or shutdown fires a source.
   Only then does the client take the current source's watch, dropping the
   old one. It closes if stopping or expired under the new limit; otherwise
   it waits again without resetting last activity. Active clients retain
   their existing at-most-once-a-second state reads between request batches.
4. If a client reads the new limit before CONFIG fires the old source, it
   temporarily retains its old unfired watch. That source remains owned by
   the CONFIG caller, whose ensuing fire causes another state read. Reading
   the fired bit and server state in one atomic statement makes shutdown
   and refresh coherent, without a numeric generation counter or wraparound.
5. Shutdown records its request and retains the current source together,
   then fires it outside the statement. If a concurrent CONFIG replaces it
   first, shutdown captures the replacement; if CONFIG replaces it later,
   every client reading that replacement also sees shutdown and exits.
   Watches of older generations still receive their CONFIG callers' fires.
   The same reasoning covers several overlapping CONFIG changes and a
   client changing its own limit several times in one command batch.
6. Every client releases its final watch before closing the connection.
   Shared server teardown drops the final source; outstanding watches and
   temporary source handles retain each state until their last release.
   Accept, signal and expiry retain their separate shutdown cancellation.

The existing timeout cases retain their expectations.
`firn_reapplies_an_idle_limit_after_disabling_it_on_both_routes` changes
timeout on the client that will become idle: enable one second, disable it
and PING in the same command batch, then observe 1.2 seconds of silence
without EOF. Another client enables one second again; the idle client must
reach EOF within three seconds without sending another byte. It runs on
both host I/O routes and covers refreshing after earlier generations. The
bound allows two seconds beyond the new limit for loaded CI; it does not
measure immediate wake latency or prove timer removal.

No additional network case asserts the absence of unlimited receive wakes.
A polling receive and a parked receive emit the same bytes and remain
connected, so socket observations cannot distinguish them. Process CPU or
scheduling counts also include expiry and other contexts and are not a
cheap deterministic oracle. The receive's None deadline is inspectable in
the implementation; the comparison above will measure its cost without
adding a test-only server path.

### Results

**Correctness.** CI `check` on 43b58dd passed all 113 network tests,
including the repeated timeout changes and the 500 ms shutdown bound, both of
which run on both host I/O routes, and the Redis 7.0.15 suite ratchet
(591/591). That idle clients with no limit no longer arm a receive timer
follows from the code (the receive's deadline is `None` at a zero limit), not
from these throughput numbers.

**Failing control.** The same 500 ms shutdown-bound case on main's polling
shutdown, built with the same release (branch `exp/stop-control`,
[run 38020404152](https://github.com/Ming-Research/Firn-wf/actions/runs/38020404152)),
fails: stopping took 1.103 s, the one-second receive poll, while every other
test passed. The bound therefore separates cancellation from polling.

**Probe.** [Run 38019220128](https://github.com/Ming-Research/Firn-wf/actions/runs/38019220128)
(i9-14900K, 2026-10-10 03:03-03:13 UTC). Revisions:
- `base`: main's source on the same release (`exp/stop-base`, 21c2595,
  `wf-f3d081b90a8d`);
- `head`: 43b58dd.

Each was paired with a `-twin` measuring its image again. The run used two
interleaved passes of 5 seconds. Median throughput relative to `base`, with
the twin's ratio in parentheses:

| CPUs | test | depth 1: head (head-twin) | base-twin | depth 16: head (head-twin) |
|---|---|---|---|---|
| 1 | get | 1.008 (1.018) | 1.016 | 1.037 (1.022) |
| 1 | set | 1.010 (1.030) | 0.999 | 1.046 (1.073) |
| 2 | get | 0.985 (1.009) | 0.996 | 0.992 (1.006) |
| 2 | set | 1.032 (1.005) | 1.012 | 1.001 (0.976) |

By the rule stated before measuring, the prediction is not rejected. The only
depth-1 cell below base, `get` on two CPUs, is 1.5% below it. That is under the
2.4% between head and head-twin there.

The prediction's stronger half, that head is at least base, is not resolved:
the twins differ by up to 3%, so a gain from dropping the per-receive timer
(at most 1.25% at depth 1 in the polling measurement above) is below this
probe's noise. The result is reported as no measured loss, not as a gain.

## Accept errors keep serving

### Question and acceptance criteria

Before the accept-recovery fix, every operational `tcp_accept` error broke
firn's accept loop, closed its listener and, once clients left, returned
status 0. Board item `firn-accept-error-keeps-serving` requires recovery
without changing orderly stop or hiding a Whitefoot limitation. After
Linux EMFILE and the release of four admitted connections, a fresh client
must receive `+PONG` within ten seconds on each I/O route. The old
unconditional break must fail the regression for an accept-loop failure,
with the point of failure reported rather than presumed to be recovery.

The pacing extension additionally requires an exhausted listener with no
pending peer to remain quiet: over at least one second, its process CPU
must be at most 25% of one core, rounded up to one accounting tick, and its
new diagnostic count must be at most the whole elapsed seconds plus two
boundary/drain lines. A retained client must be able to request successful
orderly exit within one second while exhaustion persists. These are coarse
CI regression bounds with scheduling margin, not precise CPU or
cancellation-latency measurements. They were stated before the first
pacing extension's CI run.

### Redis reference

Redis 7.2.16's
[`connSocketAcceptHandler`](https://github.com/redis/redis/blob/7.2.16/src/socket.c#L311-L330)
tries at most
[1,000 accepts per callback](https://github.com/redis/redis/blob/7.2.16/src/connection.h#L43).
On EMFILE it logs `Accepting client connection: %s` and returns without
closing the listener or removing its readable registration.
[`anetGenericAccept`](https://github.com/redis/redis/blob/7.2.16/src/anet.c#L521-L548)
retries EINTR internally and formats an `accept:` description; the
[retryable network-error list](https://github.com/redis/redis/blob/7.2.16/src/anet.c#L715-L730)
excludes EMFILE. The
[epoll registration](https://github.com/redis/redis/blob/7.2.16/src/ae_epoll.c#L74-L89)
is level-triggered. With a pending peer, a persistent EMFILE can make the
listener readable on later loop turns. Returning on the first EMFILE and
capping each callback permit other events to run; they provide no timed
backoff or low-CPU promise in that state. With a valid listener and no
pending readable event, Redis does not invoke this callback merely to try
an idle accept. This does not mean every malfunctioning listener becomes
unreadable: a still-registered readable failure can refire on later loop
turns. On Linux, closing the last reference to a watched descriptor invokes
[eventpoll release](https://github.com/torvalds/linux/blob/v6.8/fs/file_table.c),
whose [removal path](https://github.com/torvalds/linux/blob/v6.8/fs/eventpoll.c)
removes its epoll entries; other listener failures do not universally imply
that removal. Firn's direct accept retry is not gated on a new readiness
callback, so defensive pacing is needed beyond the resource-error case.

Firn retains its listener after operational errors. The pinned Whitefoot
release `wf-af7665be672d` provides a
[distinct Cancelled result and a supplied watch](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/lib/std/net/module.wfm#L44),
so only orderly-stop cancellation, recorded shutdown or the existing
accepted-client limit ends the loop. A completed operation can win a race
with cancellation; shutdown checks at the loop head and after each outcome
preserve the stop policy. Retaining the listener matches Redis; the pacing
and diagnostic limits below are deliberate differences required by firn's
blocking-accept structure.

### Retry errors and pacing

A blocking accept need not wait for a connection before returning EMFILE.
Linux 6.8
[`__sys_accept4_file`](https://github.com/torvalds/linux/blob/v6.8/net/socket.c#L1841-L1863)
reserves a descriptor before calling `do_accept`; the
[protocol accept](https://github.com/torvalds/linux/blob/v6.8/net/socket.c#L1819)
happens later. Its
[descriptor allocator](https://github.com/torvalds/linux/blob/v6.8/fs/file.c#L465-L527)
returns EMFILE when no permitted slot remains. The
[io_uring accept path](https://github.com/torvalds/linux/blob/v6.8/io_uring/net.c#L1231-L1298)
also reserves the descriptor before `do_accept`. These source paths
establish the mechanism; they do not identify the kernel version of a
particular CI run.

The pinned Whitefoot
[adapter path](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/file_posix.c#L410-L432)
issues `accept4` directly, and its
[native path](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/linux_io_uring.c#L609-L621)
submits `IORING_OP_ACCEPT`. A
[watched accept disables the separate readiness route](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/bridge.c#L5576-L5618)
and dispatches through the native ring or helpers. Firn therefore issues
its next accept after the last successful admission fills the table, even
without another queued peer. Immediate retry can busy loop in this idle
state, unlike Redis's readiness-triggered callback.

The runtime
[yields after 64 immediately answered waits](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/bridge.c#L1958-L1988)
when another context is ready on its driver; its
[host-call lowering](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/emitter/frames.rs#L368-L428)
includes synchronous completions. This ensures a scheduler handoff to ready
clients, not low CPU when no other context is ready. It is not a substitute
for retry pacing.

Firn now waits 100 ms after every surfaced accept error except
`ConnectionAborted` and orderly-stop cancellation, on the same shutdown
watch as accept and diagnostics. Cancellation ends acceptance before the
classifier is called; an aborted connection retries immediately. A
cancelled sleep also ends acceptance, while an elapsed sleep returns to the
normal shutdown checks. Persistent invalid, forbidden or otherwise broken
accepts receive the same defensive pacing as resource exhaustion. The
100 ms delay limits repeated paced failures to at most ten retries per
second, apart from the initial attempt; logging or scheduling can reduce
that rate. Recovery after a paced error can wait for the remaining interval,
apart from bounded logging and scheduling.

The pinned
[POSIX error mapping](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L507-L573)
establishes these source-level classifications:

| Native error | Whitefoot class | Firn retry |
|---|---|---|
| ECONNABORTED | ConnectionAborted | Immediate |
| EMFILE, ENFILE, ENOMEM, ENOBUFS | ResourceExhausted | 100 ms pause |
| EINVAL | InvalidInput | 100 ms pause |
| EPERM, EACCES | PermissionDenied | 100 ms pause |
| EBADF, ENOTSOCK, a surfaced EINTR | Other | 100 ms pause |

[Factory-credit exhaustion](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L594-L601)
also uses ResourceExhausted. Every other non-cancellation class defaults
to the same pause. This table is source inspection, not fault injection:
ENFILE, ENOMEM and ENOBUFS have not been independently provoked, and the
new non-resource pause paths and immediate ECONNABORTED path have no
injection results. Actual EMFILE exercises the existing shared sleep path
on both routes; it does not prove each listed failure can be induced.

The pinned
[IoError interface](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/lib/std/io/module.wfm#L13-L44)
has no Interrupted class. The
[adapter absorbs ordinary EINTR](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/file_posix.c#L612-L634)
for accept unless cancellation ended the operation. On the native route,
[the transfer-kind predicate excludes accept](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/linux_io_uring.c#L539-L543),
so the ordinary
[EINTR resubmission branch](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/linux_io_uring.c#L1007-L1029)
does not apply to it; a surfaced uncancelled interruption can therefore
reach the mapping as Other. Native
[deadline/cancellation completion](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/completion/linux_io_uring.c#L991-L1004)
is separate and retains the distinct cancellation outcome.

Exempting all Other errors to accelerate that possible interruption would
also exempt persistent EBADF and ENOTSOCK failures. Firn instead accepts a
bounded 100 ms delay for a surfaced native EINTR without checking platform
errno numbers in application code. Some Linux network failures that Redis's
retry predicate handles immediately can likewise be paced by this
conservative class policy. The cost is a bounded retry delay, not an exit
or loss of the listener. No real uncancelled native accept interruption has
been observed here; whether that producer warrants aligning the native and
adapter interruption handling is a possible future consistency question,
not a confirmed Whitefoot runtime bug or a required language change.

A client-close-only notification was not selected: global descriptor or
memory resources can recover without a firn client closing. A cancellation-
aware timer handles those cases without adding a timer context or a new
shared notification protocol. This is an application policy using the
pinned time interface, not a workaround for a missing Whitefoot capability.
The former immediate-retry proposal conflated scheduler fairness with
quiet idle behavior. Pacing only ResourceExhausted was then too narrow:
persistent failures in the other classes can also repeat without a new
readiness event. The current policy preserves immediate retry only for the
specific aborted-connection class and ends orderly cancellation normally.

### Bounded accept-error diagnostics

Before this change, firn had no operational logger; main discarded stderr
and `DEBUG LOG` was a no-op. Accept errors now use a bounded stderr sink,
Redis's phrase, the host error-description API already used for AOF errors,
and LF. The first Linux EMFILE diagnostic remains
`Accepting client connection: Too many open files\n`. Firn does not
reproduce Redis's process/time prefix or its `accept:` wrapper.

All accept-error classes share main's one limiter. The first error can
attempt a diagnostic immediately. Further attempts occur at least one
second after the preceding bounded attempt finishes, including across
successful accepts. Ineligible errors increment a saturating u64 count;
the next eligible error appends ` (suppressed N accept errors)` when that
count is nonzero and resets the count after the attempt. The suffix counts
preceding omitted failures, not only failures of the current error class.
Success does not reset the limiter or flush the count. If no later error
occurs, that count remains unreported. A failed diagnostic attempt still
consumes the interval and clears its preceding count, consistent with a
best-effort sink rather than a reliable event log.

A fixed 512-byte buffer reserves room for the largest 20-digit suppression
count, its framing and LF; unavailable or oversized host descriptions use
a generic fallback. Partial writes share one absolute 100 ms deadline and
the orderly-stop watch. Error or zero progress abandons the remaining
bytes. A completed write can win over expiry; finite message length and
strictly advancing endpoints also bound retry count. Diagnostics cannot
change the listener-retention policy, and `DEBUG LOG` continues to return
OK while discarding its message.

### Regression assumptions

Separate adapter and native-route cases use the existing child, port and
output harness, one worker and one driver, no AOF, unlimited clients and
no idle timeout. They rely on Linux `/proc`, util-linux `prlimit` and
permission to lower the child's soft limit through `prlimit64`.

A tiny limit set only before exec can exhaust factory credits before native
EMFILE. The cases instead start with soft and hard limits of 256, verify
that limit and the child descriptor set after readiness PING, then lower
only the soft limit to the highest open descriptor plus one plus eight.
All startup descriptors stay below that limit, including inherited high
ones. Available slots are the new soft limit minus the startup descriptor
count; holes below the highest descriptor are included. Sequential
PONG-admitted peers must each add exactly one descriptor and preserve the
startup set. Before the failing attempt, every descriptor below the new
soft limit must be occupied.

The pinned runtime reads the initial soft limit once,
[clips it at 1,048,576 and subtracts a reserve of 64](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1947-L1973),
so 256 initially supplies 192 credits. Its
[shared acquisition counter](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L594-L617)
counts outstanding acquisitions, not descriptor numbers or the live kernel
limit. Lowering RLIMIT_NOFILE constrains new native allocation without
shrinking this counter. The
[TCP listener](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1641),
[stop listener](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1848)
and [each accepted connection](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1723)
each consume one credit. A
[failed accept returns its credit](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1749);
closing both connection halves
[returns one credit on the final close](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d/compiler/src/backend/ordinary_values.c#L1478-L1482).

The conservative requirement is two fixed listeners, one readiness client,
every additional admitted client and one failing in-flight accept:
`additional_clients + 4 <= 192`. It ignores credits returned by closing
the two startup directory handles; these can only add room. With no AOF
and the checked admission behavior, this permits native exhaustion rather
than an earlier factory refusal. A very sparse inherited descriptor range
can still fail this precondition. For highest descriptor H and startup
count C, the requirement is `H + 13 - C <= 192`. With ten startup
descriptors, H=189 fits exactly and H=190 fails; H=183 still fits. There is
no universal highest-descriptor threshold independent of the count. This
residual limitation is retained and diagnosed rather than changing the
initial limit or removing inherited descriptors speculatively.

Admission has a 15-second deadline. Once all slots are occupied, each case
waits at most five seconds for the first exact EMFILE diagnostic while
retaining every admitted peer and creating no new pending peer. It checks
that the full descriptor set stays unchanged. This observes the idle
accept failure before any recovery step can free resources.

The quiet phase retains that state for 1.1 seconds and reads process
`utime + stime` from `/proc/<pid>/stat` before and after. `getconf CLK_TCK`
supplies the CI host's accounting frequency. CPU ticks are compared with
25% of one core over the actual elapsed wall time, rounded up to one tick;
new diagnostic prefixes are compared with whole elapsed seconds plus two
boundary/drain lines. After process exit, each case also compares the full
captured diagnostic count with whole elapsed seconds plus two, measured
from just before the last admission through exit. This includes errors
around queued-peer recovery and successful accepts, which must not reset
the limiter. Loaded CI can mask CPU spin by starving the child,
so this coarse CPU bound is not a precise idle-cost attribution. The log
bound independently checks output growth, not scheduler behavior.

The two recovery cases then queue a peer whose PING must stay unserved for
500 ms, close it and four admitted peers, and require a fresh PONG within
ten seconds. Other clients remain open through a five-second successful
SHUTDOWN NOSAVE deadline. The two saturated-stop cases instead free no
slots or queued peer: a retained client requests SHUTDOWN NOSAVE and the
process must exit successfully within one second. The case names are
`firn_shutdown_completes_during_emfile_on_adapter` and
`firn_shutdown_completes_during_emfile_on_native_route`: they bound overall
stop during exhaustion, not isolated cancellation of the 100 ms sleep. A
one-second deadline could also pass if that shorter pause merely elapsed.
Cancellation is established separately by inspecting the call's
`sleep_until(..., cancel: &cancel)` shutdown watch and the cancellation
result's exit path. Socket operations use remaining absolute deadlines,
and stderr must retain the EMFILE evidence.

### Results and remaining evidence

The initial
[fixed-server CI run](https://github.com/Ming-Research/Firn-wf/actions/runs/38417574158)
on `1d12de7beb1097bfc7304b14e23437018ed2d1eb` and
[control](https://github.com/Ming-Research/Firn-wf/actions/runs/38417578518)
on `1204bec9b794a50dc77a29e37b5e25440bf44e77` failed the former `soft <= 64`
precondition before testing recovery. Both observed ten descriptors
`{0, 1, 2, 3, 4, 5, 6, 7, 142, 145}`, giving soft limit 154, 144 additional
clients and a conservative need for 148 of the initial 192 credits.

After replacing that precondition with the credit calculation,
[the full gate and AOF sync-fault job passed](https://github.com/Ming-Research/Firn-wf/actions/runs/38420646489)
on `6229b9b1510258def3485aedbd94c7789dff738a`. Its combined recovery case
ran both I/O routes. The
[old-accept-loop control](https://github.com/Ming-Research/Firn-wf/actions/runs/38420652184)
on `1d08c2ee5f36528030a002f9a76b360a29a3ef95` failed on the adapter route
at the last admission's descriptor-preservation check, not at the later
fresh-client PONG check. Descriptor 153 was admitted and replied, but the
next accept returned idle EMFILE and closed listener descriptor 6: the
before set was 0 through 152, and the after set added 153 while losing 6.
No additional peer had been queued. The control's combined case stopped
there, so native-route failure before the fix remains unverified. Splitting
the routes gives each future control an independent result.

The quiet CPU/log checks are predicted to fail on `6229b9b` by source
reasoning: after the descriptor table fills with no peer pending, Linux's
allocation-before-wait order returns EMFILE immediately, and that revision
immediately retries and attempts a diagnostic for every failure. Its
fairness handoff is not a pause. Given enough scheduled CPU, this produces
busy retry CPU above the quiet bound and diagnostic growth above the
one-per-second allowance. A heavily loaded runner can mask the CPU result
by starving the child; unbounded diagnostic growth provides an independent
output check. This is a prediction from the inspected paths, not a measured
CPU/log control result on that revision or a universal timing guarantee.

The passing run recorded ten startup descriptors on the adapter and
thirteen on the native route, both with highest descriptor 145 and soft
limit 154. Descriptor 142 was a nonblocking read pipe and 145 a write pipe
with a different inode; neither had close-on-exec. Both matched the parent
Rust test process by descriptor number, device, inode and flags, confirming
inherited pipes rather than Whitefoot ring descriptors. The native route
separately recorded its ring, eventpoll, eventfd and signalfd as descriptors
5 through 8. No jobserver authentication tokens were present in the
recorded Cargo/make environment. The earlier jobserver hypothesis is not
established by these observations; the upstream producer of the two
inherited pipes remains unknown. The diagnostics retain targets, fdinfo,
parent matches and authentication tokens for subsequent runs.

The subsequent
[full gate and AOF sync-fault job passed](https://github.com/Ming-Research/Firn-wf/actions/runs/38424744118)
on `e7fa268b3a2e1950d0443e0651d233f145f00cda`, which paced only
ResourceExhausted. Its four independent EMFILE cases cover quiet CPU and
bounded diagnostic growth, recovery and successful stop during saturation,
on the adapter and native route. The one-second stop result does not
isolate sleep cancellation. This run provides no fault-injection evidence
for ENFILE, ENOMEM, ENOBUFS or the newly paced non-resource errors.

No local compiler, build, test or measurement was run. The current broader
pause classifier and the two stop-case name corrections are uncommitted
and have no CI result. The green resource-only gate does not validate this
new error-class policy. Its non-resource paths and immediate
ConnectionAborted exception have source-inspection evidence only; the
existing EMFILE cases and the full gate must still pass on the revised
code. No required Whitefoot gap, pin movement or submodule change was found.
