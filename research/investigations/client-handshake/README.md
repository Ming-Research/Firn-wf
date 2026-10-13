# Client handshake and client enumeration

## The question, stated before measuring

Does firn's connection-handshake rate and `CLIENT LIST` rate lead the
fastest of Redis 7.2.16, Valkey, Dragonfly and Garnet? This measurement
serves [Firn-wf PR #74](https://github.com/Ming-Research/Firn-wf/pull/74),
the `CLIENT SETINFO`, `CLIENT LIST` and `CLIENT NO-TOUCH` work on
`claude/client-handshake`. It measures the first two command paths;
`CLIENT NO-TOUCH` is not exercised by either workload.

The instrument is the std-only Rust `firn-workload` client in
[`research/experiments/redis-bench/workload/`](../../experiments/redis-bench/workload/),
run by [`redis-bench.sh workloads`](../../experiments/redis-bench/redis-bench.sh).
The existing client supplies the worker assignment, complete-reply counting,
exact one-microsecond latency bins and CSV format. This record contains no
measurements yet.

## Workloads

- **`handshake`.** Each iteration opens a **new TCP connection** and sends,
  in order, `HELLO 3`, `CLIENT SETINFO LIB-NAME firn-workload`,
  `CLIENT SETINFO LIB-VER 1.0` and `PING`. The commands may be pipelined in
  one write. The client reads all four replies using RESP3 framing and
  requires a map for `HELLO`, then `+OK`, `+OK` and `+PONG`. It closes the
  connection after the final reply. One completed iteration is one
  operation; latency starts before connecting and ends at the final reply.
- **`client-list`.** Open `WORKLOAD_CONNECTIONS` long-lived TCP
  connections, default 50. Each first sends
  `CLIENT SETINFO LIB-NAME firn-workload` and requires `+OK`.
  After every connection is ready, each loops `CLIENT LIST` at depth 1.
  Each reply must be a non-null bulk string containing one nonempty line
  per connected client, at least the number of the workload's own
  connections. All workload connections remain open until the sample
  ends. One complete, checked reply is one operation; latency covers the
  request through its reply, excluding the initial connection setup.

Any I/O error, reply error or type/content mismatch fails the run loudly;
an invalid operation never enters the count or latency histogram. A missing
competitor executable is reported and skipped, as in scale mode. A present
server that does not support the commands fails the run, rather than
silently becoming an omitted competitor. An incomplete competitor set
cannot establish the four-server comparison.

## Comparison and settings

- **Machine and placement:** the owner's i9-14900K through CI only, with
  the script's default topology placement for two server CPUs, which puts
  every line, competitors and firn alike, on the same server and client
  CPUs; the run's host record names them. No other work should load the
  host during the measurement.
- **Lines:** the pinned Redis 7.2.16 reference, Valkey without I/O threads
  and with two I/O threads, Dragonfly with two proactor threads, Garnet,
  firn and a twin of the exact same firn executable. Competitors use the
  same executables, startup flags and CPU placement as scale/suite mode.
  Append-only files are off on every line. The workflow records installed
  competitor versions and the firn revision and compiler pin.
- **Client:** 50 concurrent connection slots, distributed across one worker
  thread per assigned client CPU, as in the existing workloads. Handshake
  slots replace their stream on every iteration; `client-list` slots retain
  their streams. The std-only TCP connect call is synchronous within a
  worker; reply I/O is nonblocking and multiplexed across its slots.
- **Samples:** six interleaved passes of ten measured seconds for each
  workload and line. Each workload gets a fresh server. Line order reverses
  on even passes. A separate five-second warmup is excluded from the
  measured histogram and rate. The sample stops issuing at its deadline
  and drains outstanding operations, including them in both elapsed time
  and the operation count. Every completed operation supplies a latency;
  the shared `--sample-ms` option is accepted but does not enable an
  auxiliary sampler for either connection workload.
- **Depth:** one operation in flight per connection slot. Depth 16 does
  not apply to `handshake`: a connection exists for exactly one handshake,
  and its four commands together count as one operation. For `client-list`,
  **depth 1 is the measured depth**: replies grow with the connected-client
  count, so pipelining sixteen lists would multiply buffered response bytes
  and measure a different queueing and backpressure workload.

Before the six-pass measurement, dispatch a probe with two passes of five
seconds. Use its runtime and rate/p99 spread, together with the firn twin,
to check whether the planned sample can resolve the comparison; do not
claim a lead from unresolved noise.

## Prior acceptance criterion and rejection

For **each workload separately**, choose the competitor line with the
highest median operation rate over the six passes. Both Valkey configurations
participate in that selection. Firn's median rate must be **at least** that
fastest competitor's median rate (ratio **>= 1.0**), and firn's median of
the per-pass p99 latencies must be **no greater than** that same competitor
line's median p99. Report all lines' rates and p99 values, with their
between-pass spread.

The original board acceptance also asked for **1.4 times the fastest
competitor's rate at depth 16**. That depth does not apply to a
per-connection handshake, and this investigation measures `client-list`
only at depth 1 for the reason above. Whether to replace or separately
evaluate the original depth-16 requirement remains the owner's decision;
the instrument does not treat this experiment as satisfying it.

The proposal is rejected if firn falls below the fastest competitor's
median rate on **either workload**, or exceeds that competitor's median
p99 on either workload. Missing competitors or unresolved noise leave the
full comparison unverified, rather than relaxing the criterion.

The **firn twin** is a second line of the same image in the same
interleaved passes, a noise control rather than a competitor. Report its
rate and p99 alongside firn and their paired differences on every pass;
do not average the twin into firn's acceptance result.

## Dispatch

Dispatch the instrument branch while building the PR's server branch and
its twin. The image names below yield `image-head` and `image-head-twin`
in the existing `line` column of `workloads.csv`. Use `head`, rather than
`firn`, as the image name: the script builds this checkout's server at
`$OUT/firn`, so naming a supplied image `firn` would overwrite it.

```sh
gh workflow run redis-bench.yml --ref claude/bench-handshake \
  -f runner=14900k -f mode=workloads \
  -f revisions='head=origin/claude/client-handshake head-twin=origin/claude/client-handshake' \
  -f workload_images=true \
  -f workload_servers='reference firn valkey valkey-io dragonfly garnet' \
  -f tests='handshake client-list' \
  -f cpus=2 \
  -f connections=50 -f passes=6 -f seconds=10 -f profile=false
```

For the probe, use the same inputs with `-f passes=2 -f seconds=5`.
The `depths` input belongs to compare mode and does not change these
depth-one workloads. Keep the host/version record, revision record and
`workloads.csv`, and copy the results and their spread here after the
measurement; workflow logs and artifacts expire.

## First full run, kept for reference (2026-10-13)

[Run 38396589987](https://github.com/Ming-Research/Firn-wf/actions/runs/38396589987)
on the i9-14900K: firn at
[Firn-wf#74](https://github.com/Ming-Research/Firn-wf/pull/74)'s de6d7b5 and its
twin, Redis 7.2.16, Valkey and Valkey with io-threads; 2 server CPUs (2,4),
50 connections, 6 interleaved passes of 10 s. Dragonfly and Garnet were not
installed on the host and are absent. This run does not decide the criterion
above: the instrument predated explicit placement, so its automatic hybrid
placement put the 10 client threads on P-cores 6-15
(`placement,server=2,4,client=6,...,15,threads=10,source=hybrid`) instead of the
16 E-cores the owner ruled for performance comparisons, and it recorded no
client CPU load.

Medians over 6 passes (operations/s; p99 in ms):

| workload | firn | firn twin | Redis 7.2.16 | Valkey | Valkey io-threads |
|---|---|---|---|---|---|
| client-list | 132779 (0.58) | 133090 (0.54) | 34603 (2.52) | 26800 (3.25) | 28325 (1.79) |
| handshake | 21664 (5.47) | 21717 (5.47) | 21704 (5.46) | 21618 (5.48) | 21899 (5.51) |

CLIENT LIST ran 3.84 times the fastest competitor. Every server completed
about 21.7 thousand handshakes per second with the same p99, so that workload
was bounded outside the server, by connection setup or the client; the rerun
records client CPU load to tell which.

## Rerun with the owner's placement (2026-10-13)

[Run 38400671487](https://github.com/Ming-Research/Firn-wf/actions/runs/38400671487):
the same images and settings as the first full run, with servers on P-cores
2,4 and the client on E-cores 16-31
(`placement,server=2,4,client=16,...,31,threads=16,source=override,server_class=2:P,4:P,client_class=16:E,...,31:E`).
Dragonfly and Garnet are still absent from the host.

Medians over 6 passes (operations/s; p99 in ms):

| workload | firn | firn twin | Redis 7.2.16 | Valkey | Valkey io-threads |
|---|---|---|---|---|---|
| client-list | 132251 (0.59) | 131618 (0.70) | 34435 (2.47) | 26710 (3.19) | 28368 (1.78) |
| handshake | 23133 (5.44) | 23062 (5.34) | 23138 (5.28) | 23036 (5.37) | 19222 (17.9) |

CLIENT LIST ran 3.84 times the fastest competitor, with a lower p99. The
handshake rate was the same for firn, Redis and Valkey within 0.5%; firn's p99
was 1.03 times Redis's, while the two firn images differed by 1.02 times. One
Valkey io-threads pass fell to 1669 handshakes/s, which pulls its median down.

The recorded client CPU load does not separate client-limited cells: every
cell, including Redis's 34 thousand CLIENT LIST calls per second, shows a
client mean busy fraction of 0.998 or more. The workload client polls
non-blocking sockets and yields in a loop (`workload/src/main.rs`), so its CPUs
are busy whether or not it is the bottleneck. The client therefore reports
its own `client_work_fraction` (loop time in scans that made progress, over
all loop time), and the 0.90 rule applies to that.
