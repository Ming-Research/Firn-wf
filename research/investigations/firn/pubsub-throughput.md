# Pub/sub throughput

## The question

Does firn deliver published messages faster than the fastest of Redis, Valkey,
Dragonfly and Garnet, and did adding publish/subscribe (Firn-wf#100) slow its
ordinary commands? `redis-benchmark` has no publish/subscribe test, so the
first needs a client of its own.

## Criteria, stated before measuring

These are the acceptance criteria of the item that owns this record, with the
medians and the twin spelled out as the firn investigation's
[criteria](DESIGN.md#criteria-stated-before-measuring) define them.

- **Workload.** One publisher and 50 subscribers on one channel
  (`SUBSCRIBE`) and, as a second workload, the same subscribers on one pattern
  that matches the channel (`PSUBSCRIBE`), 64-byte payloads, RESP2, over
  loopback. The server has 2 CPUs and the client the rest of the
  [placement](../../../design/firn/measurement-sessions.md): on the 14900K,
  the servers on CPUs 2 and 4 (P-cores) and the clients on CPUs 16-31
  (E-cores), for every line and for the noise floor alike (AGENTS.md). The
  publisher keeps 16 `PUBLISH` commands unanswered (depth 16) or 1 (depth 1).
- **Measure.** The rate is the messages delivered to subscribers per second,
  50 times the published messages over the span from the first publication to
  the last arrival. The latency of a message runs from just before the
  publisher writes it to its arrival at the last subscriber that receives it;
  the p99 is over all messages. A cell is the workload at one depth; its value
  for a line is the median over at least six interleaved passes of the lines
  (the pass count of the noise floor below, so that a cell's twin interval is
  comparable with it), each pass in the reverse order of the one before.
- **Rate.** At depth 16 firn's rate is at least 1.4 times the fastest
  competitor's; at depth 1 at least the fastest competitor's. Where the client
  is the limit, a cell asks only that firn is not behind: the client is the
  limit when the median over passes of its work fraction (below) is above 0.90
  in firn's line or in the fastest competitor's.
- **Latency.** Firn's p99 is no higher than the p99 of the fastest competitor
  of the cell (the one with the highest median rate, the one the rate is
  compared with). In a client-limited cell the p99 is reported and not judged:
  the client's own queueing sets it there.
- **Competitors.** The competitors are the newest stable release of each
  project, none of them installed on the measuring host: Redis 8.10.2
  (`redis-latest`) and Valkey 9.1.2 (`valkey-latest`, and `valkey-io-latest`
  with as many I/O threads as server CPUs) are built from their release
  archives, Dragonfly v2.0.2 (`dragonfly-latest`, as many proactor threads as
  server CPUs) and Garnet 2.2.3 (`garnet-latest`) are extracted from their
  prebuilt Linux archives, all by `latest.sh`. Garnet is a .NET 10 program and
  the host has no .NET, so `latest.sh` keeps Microsoft's 10.0.13 runtime beside
  it and points `DOTNET_ROOT` at it; nothing is installed. Each archive is
  pinned by SHA-256: Redis and Valkey from the projects' hash tables,
  Dragonfly and Garnet from the digest GitHub shows for the release asset, the
  .NET runtime from the SHA-512 of Microsoft's release metadata, which the
  archive matched. The versions are the newest non-prerelease of each
  project's GitHub release list, read 2026-10-21; the host's own packages are
  older or absent (Valkey 9.0.4 at the first measuring run, no Dragonfly or
  Garnet) and are not measured. `reference` stays Redis 7.2.16, the project's
  reference (AGENTS.md). A run's `host.txt` prints the version of each line,
  and a pin is moved when a newer release appears before the measuring run.
  Every line runs with its defaults on the server's 2 CPUs apart from those
  thread counts; Redis's I/O threads stay off in `reference` and
  `redis-latest`, and a line of `redis-latest` with them on is added only if a
  cell where firn is behind `redis-latest` makes the question matter. A
  competitor that answers `SUBSCRIBE` with an unknown-command error is left out
  of the cells of that workload. The fastest competitor of a cell is whichever
  line has the highest median rate in it, `reference` included.
- **Noise.** Firn is measured as two images of one build, `head` and
  `head-twin`; the 95% interval of the mean log of twin/head over the passes
  is the cell's noise, which `summary.py pubsub` prints. No threshold is
  picked. A first run on the 14900K measures the two images alone (6
  interleaved passes, both workloads, both depths, the settings of the
  comparison) and takes the widest half-width of those intervals as the floor
  of this workload class; the floor is recorded in the board's record of the
  14900K. A cell of the comparison whose twin interval is wider than the
  floor does not count: it is repeated with more passes or reported
  unresolved. The floor is measured again when the machine, its settings or the
  toolchain change.
- **Before and after publish/subscribe.** The 19 default tests of
  `redis-benchmark` of Redis 7.0 (20 rows: the `LRANGE` tests' list fill is
  the 20th, see `SUITE_DEFAULT` in `redis-bench.sh`) at depths 16 and 1 on 2
  server CPUs, on images of the main revision before Firn-wf#100 merged
  (`00e2bd1fa230`), the one after (`10c20af7da77`) and the latter again as
  its twin, interleaved by the compare mode. A competitor's rate does not
  change between the two images of one run, so the change in firn's multiple of
  the fastest competitor equals the change in firn's own rate: the criterion
  is that the 95% interval of the mean log of after/before over the passes is
  not wholly on the slower side, and that the twin's interval holds 1. `summary.py
  ratios` computes both.
- **What rejects.** A cell below its threshold stands as measured, with its
  gap and a profile before any cause is named; this item changes no
  publish/subscribe code, so a cell that fails goes to a performance item.

## The client

The client is `firn-workload` (`research/experiments/redis-bench/workload/`)
with the workloads `pubsub-channel` and `pubsub-pattern`, run by `redis-bench.sh
workloads` ([header](../../experiments/redis-bench/redis-bench.sh)).

- One thread publishes. The subscribers are spread over the other client
  threads, each multiplexing its sockets without blocking. Every message
  carries its sequence number and its send time in its first 16 bytes; a
  subscriber checks that its messages arrive in sequence, complete and
  unaltered, so that a lost, duplicated or reordered message fails the run.
- The publisher sends a batch of `depth` commands and waits for all their
  replies, as `redis-benchmark -P` does. It also stops when it is `--window`
  messages (default 1024) ahead of the slowest subscriber, so that no server
  disconnects a slow subscriber for its output limit (Redis and firn both limit
  pub/sub clients to 32 MiB, or 8 MiB for 60 seconds) and the rate is one the
  subscribers keep up with. At depth 16 the window cannot bind for a server
  that delivers as it replies; it bounds the lead of one that delivers behind
  its replies, and so bounds the latency such a server shows.
- A sample is one fresh set of connections: a warm-up run of 5 seconds that is
  discarded, then the measured run of 10 seconds.
- Client work fraction is the share of loop time a client thread is busy. The
  publisher's is the share in scans that moved bytes, as for the other
  workloads. A subscriber thread polls many sockets in one scan, so its busy
  time is the time in reads that returned data and in parsing what they
  returned; counted by scans, a thread polling 50 sockets read 0.98 busy while
  a server delivered 80 thousand messages a second to a client able to take
  25 times that (trial run on a GitHub runner, 2026-10-21). The fraction of a
  sample is the larger of the publisher's and the subscribers'.
- Not covered: RESP3 push frames, sharded channels, many channels and
  subscribers that unsubscribe during the run.

## Results

All on the 14900K (self-hosted runner, Linux 7.0.0-38), measured 2026-10-21,
servers on CPUs 2 and 4 (P-cores), clients on CPUs 16-31 (E-cores), by the
workflow `redis-bench.yml`. The noise floor ran at revision `d5bb820`, the
other two runs at `94419bc`. Between the two revisions only `latest.sh` (the
fetch of the newest competitor releases) and this record changed, and the floor
run measured firn's two images alone and skipped that fetch, so the floor holds
for the later runs. Nothing was installed on the host.

### Noise floor

Run 39557237919 (`workloads`, `head` against `head-twin`, 6 interleaved passes,
both workloads, both depths): the widest half-width of the twin/head interval
is **0.86%**, which is the floor of this workload class.

### Delivery rate and p99

Run 39566561800 (`workloads`, 6 interleaved passes of 10 s after a 5 s
warm-up, 64-byte payloads; `summary.py pubsub --firn image-head --twin
image-head-twin --floor 0.86`). The twin half-widths are 0.55% (channel, depth
1), 0.85% (channel, 16), 0.47% (pattern, 1) and 0.67% (pattern, 16), none above
the floor, so all four cells count. The client work fraction is 0.108-0.111 for
firn and 0.071-0.178 for the fastest competitor of each cell (`redis-latest` at
depth 16, `dragonfly-latest` at depth 1; no sample of any line is above 0.181),
far below 0.90: no cell is client-limited. Rates are medians, in thousands of
messages a second.

| cell | firn | fastest competitor | firn / fastest | required | firn p99 | fastest's p99 |
|---|---|---|---|---|---|---|
| channel, depth 16 | 349.6 | `redis-latest` 6743.6 | 0.052 | at least 1.4 | 150.5 ms | 0.198 ms |
| channel, depth 1 | 343.5 | `dragonfly-latest` 1098.0 | 0.313 | at least 1.0 | 153.4 ms | 0.091 ms |
| pattern, depth 16 | 344.9 | `redis-latest` 6109.1 | 0.056 | at least 1.4 | 153.3 ms | 0.212 ms |
| pattern, depth 1 | 340.0 | `dragonfly-latest` 1086.1 | 0.313 | at least 1.0 | 156.4 ms | 0.092 ms |

At depth 16 the other lines are `reference` 6275 (channel), `valkey-latest` 5303,
`valkey-io-latest` 5029, `dragonfly-latest` 5819 and `garnet-latest` 643. At depth 1
they are `reference` 712, `redis-latest` 713, `valkey-latest` 690,
`valkey-io-latest` 622 and `garnet-latest` 592. Every line delivered every
message exactly once and in order (the client fails the run otherwise).

All eight criteria (four rates, four p99) are not met, by a margin far outside
the noise: firn delivers about 1/19 (depth 16) and 1/3.2 (depth 1) of the
fastest competitor's rate.

Readings, as measured (no cause is concluded):

- Firn's rate hardly changes with depth or subscription kind: 340-350
  thousand messages a second in all four cells. The competitors are 5 to 9
  times faster at depth 16 than at depth 1 (`redis-latest` 713 to 6744);
  Garnet is the exception.
- Firn's p99 is about 150 ms in every cell. The publisher's window is 1024
  messages and 343.5 thousand deliveries a second (channel, depth 1) is 6870
  publications a second, and 1024 / 6870 is 149 ms: the p99 is the publisher waiting at its
  window, queueing time, not the delivery time of one message. The competitors'
  p99 is below 1.3 ms.
- Server profile (`perf`, the first pass of each cell, `head` and `head-twin`,
  eight profiles, no lost samples): `wf_shared_acquire_as` is 29.5-30.2% of
  the cycles in every profile and the first entry; `wf_executor_loop` is 4.5-4.9%;
  the rest is the kernel's TCP send path (each 1-2.5%) and the runtime's
  `wf_server.send_promoted_frame.resume` and `wf_connection_events.*` (about 1%
  each). These profiles have no call stacks, so they show where cycles are
  spent, not who calls; the first step of a performance item is a profile with
  call stacks (`callers` input of the workflow).

### Before and after publish/subscribe

Run 39573904480 (`compare`, 19 tests at depths 16 and 1, 2 server CPUs, 6
interleaved passes of 5 s, no `reference` line, no profile). The images are
the main revision before Firn-wf#100 merged (`00e2bd1fa230`, "before"), the one
after (`10c20af7da77`, "after") and the latter again (`head-twin`, the twin);
they are consecutive first-parent commits of main, built with the same pinned
compiler release, so they differ by Firn-wf#100 alone. After/before is the
median of the per-pass ratios; the interval is the 95% interval of the mean log
ratio over the 6 passes (`summary.py ratios`). `lrange_500` and `lrange_600` at
depth 16 are client-limited in all 6 passes of all three images (client busy
fraction 0.902-0.928; the other 648 samples of the run are at most 0.889), so
the client sets their rates and their cells are reported and not judged.

| test | depth | after/before | 95% interval | verdict | twin/after 95% interval |
|---|---|---|---|---|---|
| get | 1 | 1.0030 | 0.9767-1.0318 | no detected change | 0.9913-1.0262 |
| get | 16 | 0.9535 | 0.9407-0.9800 | slower | 0.9540-1.0119 |
| hset | 1 | 1.0070 | 0.9711-1.0485 | no detected change | 0.9644-1.0244 |
| hset | 16 | 0.9740 | 0.9585-0.9921 | slower | 0.9752-1.0306 |
| incr | 1 | 1.0053 | 0.9873-1.0375 | no detected change | 0.9797-1.0154 |
| incr | 16 | 0.9788 | 0.9684-0.9967 | slower | 0.9572-1.0189 |
| lpop | 1 | 1.0107 | 0.9722-1.0894 | no detected change | 0.9785-1.0036 |
| lpop | 16 | 0.9619 | 0.9513-0.9703 | slower | 0.9809-1.0270 |
| lpush | 1 | 0.9998 | 0.9809-1.0115 | no detected change | 0.9739-1.0199 |
| lpush | 16 | 0.9855 | 0.9754-1.0080 | no detected change | 0.9715-1.0219 |
| lrange_100 | 1 | 0.9910 | 0.9812-0.9993 | slower | 0.9936-1.0250 |
| lrange_100 | 16 | 0.8890 | 0.8720-0.9045 | slower | 0.9953-1.0238 |
| lrange_300 | 1 | 0.9853 | 0.9593-0.9979 | slower | 0.9812-1.0340 |
| lrange_300 | 16 | 0.9832 | 0.9720-0.9933 | slower | 0.9897-1.0132 |
| lrange_500 | 1 | 0.9934 | 0.9490-1.0148 | no detected change | 0.9695-1.0458 |
| lrange_500 | 16 | 0.9929 | 0.9877-1.0012 | no detected change (client-limited) | 0.9912-1.0106 |
| lrange_600 | 1 | 0.9953 | 0.9881-1.0025 | no detected change | 0.9822-1.0037 |
| lrange_600 | 16 | 0.9909 | 0.9882-0.9961 | slower (client-limited) | 0.9865-1.0119 |
| mset | 1 | 1.0119 | 0.9493-1.0767 | no detected change | 0.9849-1.0321 |
| mset | 16 | 1.0015 | 0.9540-1.0291 | no detected change | 0.9676-1.0397 |
| ping_inline | 1 | 0.9952 | 0.9743-1.0341 | no detected change | 0.9848-1.0084 |
| ping_inline | 16 | 0.9558 | 0.9329-0.9877 | slower | 0.9963-1.0064 |
| ping_mbulk | 1 | 1.0022 | 0.9811-1.0326 | no detected change | 0.9699-1.0190 |
| ping_mbulk | 16 | 0.9469 | 0.9245-0.9719 | slower | 0.9814-1.0137 |
| rpop | 1 | 1.0048 | 0.9708-1.0872 | no detected change | 0.9883-1.0155 |
| rpop | 16 | 0.9787 | 0.9625-0.9921 | slower | 0.9778-1.0194 |
| rpush | 1 | 0.9965 | 0.9831-1.0092 | no detected change | 0.9744-1.0185 |
| rpush | 16 | 0.9682 | 0.9576-0.9813 | slower | 0.9860-1.0230 |
| sadd | 1 | 0.9928 | 0.9581-1.0611 | no detected change | 0.9963-1.0200 |
| sadd | 16 | 0.9647 | 0.9469-1.0045 | no detected change | 0.9853-1.0147 |
| set | 1 | 0.9989 | 0.9826-1.0417 | no detected change | 0.9742-1.0171 |
| set | 16 | 0.9614 | 0.9322-0.9764 | slower | 0.9964-1.0425 |
| spop | 1 | 0.9894 | 0.9588-1.0329 | no detected change | 0.9992-1.0255 |
| spop | 16 | 0.9744 | 0.9635-0.9887 | slower | 0.9758-1.0624 |
| zadd | 1 | 1.0051 | 0.9825-1.0407 | no detected change | 0.9734-1.0076 |
| zadd | 16 | 1.0005 | 0.9912-1.0055 | no detected change | 0.9790-1.0185 |
| zpopmin | 1 | 0.9934 | 0.9798-1.0141 | no detected change | 0.9674-1.0194 |
| zpopmin | 16 | 0.9969 | 0.9394-1.0268 | no detected change | 0.9491-1.0532 |

Reading:

- The twin interval holds 1 in all 38 cells, so the run is as noisy as the
  twin shows: half-widths from 0.5% (`ping_inline`, depth 16) to 5.2%
  (`zpopmin`, depth 16; `spop`, depth 16 has 4.3%).
- 15 cells are wholly on the slower side (14 not client-limited): 13 of the 19
  cells at depth 16 and 2 of the 19 at depth 1. Their medians run from 0.947
  to 0.991, and 0.889 (11% slower, interval 0.872-0.905) for `lrange_100` at depth
  16. The other 23 cells show no detected change. The criterion (no cell
  wholly on the slower side) is not met.
- Chance would give about two cells out of 38 wholly on the slower side at
  95%, not 15: they are concentrated at depth 16, where a server does the
  most work between its reads, and it holds across unrelated commands
  (`get`, `set`, `ping`, list, set, hash). That is a deduction from this run;
  the run has no profile, and it does not say which part of Firn-wf#100 costs
  the time. `no detected change` in a cell with a wide twin interval (such as
  `mset`, depth 1) does not show that there was no change.

### What this decides

Both parts of the item's criterion fail: the delivery rate and p99 in all four
cells, and the ordinary commands' rate in 14 of the 36 cells the client does
not limit. By "What rejects" above, this item changes no publish/subscribe
code; the gaps stand as measured and go to a performance item, whose first step
is the profile with call stacks. The `redis-latest` line with I/O threads on
(see Competitors) is not added: firn is 19 times behind `redis-latest` at depth
16, and one more line can only raise a cell's fastest competitor, never lower
it, so it cannot bring a failing cell to its threshold.
