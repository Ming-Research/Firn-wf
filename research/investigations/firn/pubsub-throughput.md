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
  prebuilt Linux archives, all by `latest.sh`, which pins each archive by
  SHA-256 (Redis and Valkey from the projects' hash tables, the other two from
  the digest GitHub shows for the release asset). The versions are the newest
  non-prerelease of each project's GitHub release list, read 2026-10-21; the
  host's own packages are older or absent (Valkey 9.0.4 at the first measuring
  run, no Dragonfly or Garnet) and are not measured. `reference` stays Redis
  7.2.16, the project's reference (AGENTS.md). A run's `host.txt` prints the
  version of each line, and a pin is moved when a newer release appears before
  the measuring run. Every line runs with its defaults on the server's 2 CPUs
  apart from those thread counts; Redis's I/O threads stay off in `reference`
  and `redis-latest`, and a line of `redis-latest` with them on is added only
  if a cell where firn is behind `redis-latest` makes the question matter. A
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

Not measured yet.
