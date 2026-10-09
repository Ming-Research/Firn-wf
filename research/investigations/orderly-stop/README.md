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
that cell. Report depth 16 as well. The comparison requires head to drop the
per-receive timer when timeout is zero; it must first support the wake
generation described below. The pinned language currently prevents storing
that generation in shared state, so the partial adoption still cannot test
this prediction. No measurement has been made for this comparison.

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

The selected direction, pending option A of
[owner card `firn-adopt-timeout-wake`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip),
is a wake generation. The proposed lifecycle is:

1. Shared server state owns an initially unfired generation source. A client
   reads the timeout, shutdown request and a watch of that generation
   together, retaining the watch across receives until the generation changes.
2. Each change of the idle limit installs a fresh source and fires the old
   generation, then closes the replaced source. Old watches independently
   retain the fired state until their clients close them. The timeout and
   source replacement must be in the same atomic statement, so a client
   cannot pair the new generation with the old limit.
3. A receive has no deadline at timeout zero and otherwise the deadline
   computed from its last activity. Cancellation or expiry re-reads shutdown
   and timeout; a continuing client closes a stale watch and takes the current
   generation's watch. Shutdown records its request and fires that generation
   too. A CONFIG update racing shutdown must not leave a client parked on a
   new unfired generation after missing the request.
4. Every client closes its final watch, and server teardown closes the last
   generation source on every exit path, including startup failures. Keep
   the separate shutdown source for accept, signal and expiry: CONFIG changes
   should not terminate those waits, and they do not need replaceable watches.

This is a proposal, not implemented behavior. The receive polling deadline
remains until the language gap below is resolved; it still protects live
timeout changes. The existing timeout cases retain their expectations.
`firn_reapplies_an_idle_limit_after_disabling_it_on_both_routes` adds a
sequence of timeout changes by the client that will become idle: enable one
second, disable it and PING in the same command batch, then observe 1.2
seconds of silence without EOF. Another client enables one second again;
the idle client must reach EOF within three seconds without sending another
byte. It runs on both host I/O routes and exposes a receive that never
re-reads the limit, including
after earlier wake generations. Its bound allows two seconds beyond the
new limit for loaded CI; it does not measure immediate wake latency or prove
timer removal. The case has not been run, including against a deliberately
broken receive that ignores configuration changes.

### Language gap: cancellation handles in shared state

At pinned Whitefoot `5268f516c3f8a57b34263fe1bba65831dd6aea2d`,
[kernel specification v0.110](https://github.com/Ming-Research/Whitefoot/blob/5268f516c3f8a57b34263fe1bba65831dd6aea2d/spec/kernel-spec.md)
PRE-1 declares `Shared<T: drop>`, `shared_new<T: drop>` and
`shared_share<T: drop>`. PRE-2 declares both `CancelSource` and `CancelWatch`
as `nodrop`. PROV-6 makes a struct, enum or box owning either handle linear,
so adding a source to `ServerState` makes `Shared<ServerState>` inadmissible.
Putting the handle in an `Option` or `Box` preserves that obstruction.

Minimal module fragment, expected to be rejected by the specified capability
bound, not compiled in this read-and-edit-only step:

```whitefoot
alias CancelSource = std::time::CancelSource;

fn generation_state() -> state: Shared<CancelSource> pure {
  let source = std::time::cancel_source();
  return shared_new::<CancelSource>(value: move source);
}
```

The atomic block itself is not the obstacle. SHARE-2 excludes waiting calls
and nested atomic statements from its block, but not non-waiting host calls.
`cancel_fire` is non-waiting and `writes(source)`; `cancel_watch` is
non-waiting and `reads(source)`. Both are permitted in such a block if the
source can legally be held there. Its guard cannot fire, since a guard must
write no path. By SHARE-2's footprint rule, accesses rooted in the atomic
target are removed from the enclosing function's footprint, leaving the
read of the shared handle; a watch written to caller-owned storage would
still contribute its write. Source creation and explicit handle closure are
also non-waiting. Swapping out a source under the atomic statement and
firing it afterwards would therefore not resolve this storage restriction.

The required Whitefoot capability is safe shared storage for a replaceable
cancellation source, with a defined way to close every generation and the
last source. `cancel_share` only returns another linear handle to the same
one-shot state; it cannot publish a fresh generation to existing client
contexts. Removing `nodrop`, adding a general shared linear lifecycle, or
adding a different notification capability is a Whitefoot design decision,
not a Firn spelling change. This blocks the implementation under the owner
card above. No workaround, pin change, generation implementation or claimed
performance result is introduced here.

### Results
