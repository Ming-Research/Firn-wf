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

Does Whitefoot v0.110's shared cancellation state end firn's shutdown waits
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
that cell. Report depth 16 as well. Removing receive timers is blocked by
the live-idle-limit question below; a measurement of the partial adoption
that retains them cannot test this prediction.

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

Main creates one cancellation source before spawning. It shares sources
with clients and the signal context, gives expiry a watch, and owns the
accept watch. A successful SHUTDOWN records the request and closes its
client in command dispatch, which returns true only for that accepted
request. The client's context then fires its source without another wait
or shared-state read. Authentication and option refusals return false;
standalone dispatch used by replay keeps its existing state-only behavior.
The source remains owned by the network context instead of adding it to
every keyspace handle and replay invocation. The signal context closes the listener before recording and firing,
preserving the second-signal escape. Main retains the existing client-limit
meaning: stop accepting after the limit, let those clients finish, then
record shutdown with Meta.stopping and fire. Closing a source releases only
that handle, so other contexts' watches remain valid until their cleanup.

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
observation of cancellation belongs to
[Whitefoot PR #304](https://github.com/Ming-Research/Whitefoot/pull/304).

### Live idle-limit updates

The existing compatibility contract and
`firn_applies_an_idle_limit_to_a_previously_unlimited_receive` require this:

1. Client A sends PING with `timeout` zero and then parks in a receive.
2. Client B sends `CONFIG SET timeout 1` and receives OK.
3. Client A closes under the new limit without sending any more bytes.

A receive with no deadline and only the unfired shutdown watch has no input
that can wake it at step 2. Firing that watch would permanently cancel the
server's other waits, even though no shutdown was requested. A limit reduced
from five seconds to one also has to reach an already parked receive; the
existing idle-limit case covers that boundary.

Pending an owner decision, retain the receive deadline of at most one
second, now also cancellable immediately on shutdown. It observes live
timeout changes, including zero to nonzero; it is no longer needed to
propagate shutdown. Do not weaken either idle-limit test or claim the timer
cost has been removed. The open design question is how one receive can
observe configuration changes independently of permanent shutdown while
keeping one shared shutdown state. v0.110 provides one watch per host wait
and no combination or reset operation; a different notification or source
generation design needs examination before calling this a language gap.

### Results
