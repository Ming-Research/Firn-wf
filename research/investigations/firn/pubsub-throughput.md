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
  loopback. The server has 2 CPUs, the client the CPUs
  [placement](../../../design/firn/measurement-sessions.md) leaves it. The
  publisher keeps 16 `PUBLISH` commands unanswered (depth 16) or 1 (depth 1).
- **Measure.** The rate is the messages delivered to subscribers per second,
  50 times the published messages over the span from the first publication to
  the last arrival. The latency of a message runs from just before the
  publisher writes it to its arrival at the last subscriber that receives it;
  the p99 is over all messages. A cell is the workload at one depth; its value
  for a line is the median over at least three interleaved passes of the lines,
  each pass in the reverse order of the one before.
- **Rate.** At depth 16 firn's rate is at least 1.4 times the fastest
  competitor's; at depth 1 at least the fastest competitor's. Where the client
  is the limit, a cell asks only that firn is not behind: the client is the
  limit when the median over passes of its work fraction (below) is above 0.90
  in firn's line or in the fastest competitor's.
- **Latency.** Firn's p99 is no higher than the lowest competitor's p99 in the
  cell.
- **Competitors.** The `reference` line is Redis 7.2.16, the project's
  reference (AGENTS.md), `valkey`, `valkey-io`, `dragonfly` and `garnet` are
  the executables the host has, their versions printed in the run's
  `host.txt`; a competitor that answers `SUBSCRIBE` with an unknown-command
  error is left out of the cells of that workload.
- **Noise.** Firn is measured as two images of one build, `head` and
  `head-twin`; the twin's gap to `head` is the run's noise. A cell where the
  two differ by more than the 5% that earlier runs on this host showed (see
  [deployment performance](../deployment-performance/README.md)) is repeated
  with more passes before it is read.
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
- Client work fraction is the share of the client threads' loop time spent in
  scans that moved bytes, as for the other workloads. It is the larger of the
  publisher's and the subscribers'.
- Not covered: RESP3 push frames, sharded channels, many channels and
  subscribers that unsubscribe during the run.

## Results

Not measured yet.
