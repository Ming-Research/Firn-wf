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
