# Deployment performance

## The question

The deployment direction asks for throughput, tail latency, memory and core
scaling against Redis under the same workload, semantics, durability
settings and host
([deployment direction](../firn/DESIGN.md#deployment-direction)). Two
questions follow:
- Has firn kept the throughput the earlier redis-benchmark measurements
  showed, since RESP3, transactions, scripts and the command parts changed
  its command path?
- How does firn compare on what redis-benchmark's own tests leave out and
  the selected consumers use ([consumers](../consumers/README.md#what-the-milestone-needs-from-firn)):
  scripts through `EVALSHA`, transactions, and `SET` with an expiry, with
  tail latency and memory beside the rate?

## Throughput after the command-path changes

Run [37570691427](https://github.com/Ming-Research/Firn-wf/actions/runs/37570691427),
`redis-bench.yml` mode scale on the i9-14900K, measured Firn-wf main
936110104 (Whitefoot release wf-0b7f5c5b9854, LLVM 22). It compared firn
with redis-server 7.0.15, Valkey with I/O threads, Dragonfly and Garnet,
each on 1 and 2 server CPUs. The workload was redis-benchmark's tests at
pipeline depth 16: 50 connections, 100,000 random keys, and one
single-threaded client process per client CPU. "firn twin" is the same image
measured again as `firn-base`; its distance from firn shows the noise, up to
5%.

The last column is the scale probe [37456575942](https://github.com/Ming-Research/Firn-wf/actions/runs/37456575942)
of 2026-10-06. It measured Firn-wf 2c774fb9b, before RESP3, transactions,
scripts and the command parts. That firn was built with Whitefoot release
wf-364f86c2fd16 (specification v0.93) under the host's LLVM 18, before the
14900K moved to LLVM 22, on the same host with the same settings for `set`,
`get` and `mset`. It ran on another day and was not interleaved with this run, so it
bounds a change only roughly.

Server CPUs 1, depth 16, thousands of requests a second (median of 2 passes of 5 s):

| test | Redis | firn | firn twin | Valkey io | Dragonfly | Garnet | firn / Redis | firn / Redis, 2026-10-06 probe |
|---|---|---|---|---|---|---|---|---|
| set | 1312 | 1891 | 1887 | 1279 | 1042 | 920 | 1.44 | 1.48 |
| get | 1416 | 1933 | 1980 | 1437 | 1176 | 925 | 1.37 | 1.43 |
| incr | 1404 | 1846 | 1847 | 1398 | 1081 | 888 | 1.31 | not measured |
| mset | 465 | 724 | 761 | 459 | 403 | 358 | 1.56 | 1.67 |
| lpush | 1473 | 1999 | 1979 | 1394 | 1002 | 835 | 1.36 | not measured |
| hset | 1331 | 1699 | 1728 | 1289 | 932 | 932 | 1.28 | not measured |
| sadd | 1440 | 1767 | 1835 | 1399 | 1018 | 956 | 1.23 | not measured |
| zadd | 676 | 745 | 746 | 693 | 655 | 365 | 1.10 | not measured |

Server CPUs 2, depth 16, thousands of requests a second (median of 2 passes of 5 s):

| test | Redis | firn | firn twin | Valkey io | Dragonfly | Garnet | firn / Redis | firn / Redis, 2026-10-06 probe |
|---|---|---|---|---|---|---|---|---|
| set | 1347 | 3348 | 3339 | 2309 | 1607 | 2386 | 2.49 | 2.47 |
| get | 1442 | 3537 | 3411 | 2322 | 1725 | 2572 | 2.45 | 2.52 |
| incr | 1417 | 3232 | 3219 | 2281 | 1608 | 2433 | 2.28 | not measured |
| mset | 471 | 1475 | 1549 | 577 | 367 | 853 | 3.13 | 3.26 |
| lpush | 1478 | 3146 | 3133 | 2067 | 1380 | 1928 | 2.13 | not measured |
| hset | 1349 | 2621 | 2664 | 1900 | 1439 | 2137 | 1.94 | not measured |
| sadd | 1423 | 2840 | 2875 | 2054 | 1476 | 2071 | 2.00 | not measured |
| zadd | 688 | 1164 | 1126 | 927 | 997 | 551 | 1.69 | not measured |

**Result.**
- **Against Redis.** firn answers 1.10 to 1.56 times Redis's rate on one
  CPU and 1.69 to 3.13 times on two. Redis serves commands on one thread,
  so its rate does not grow with the second CPU.
- **Against the earlier probe.** The firn-to-Redis ratios for `set`, `get`
  and `mset` are within a few percent of the probe's. That is inside this
  run's twin spread, apart from `mset` on one CPU, 1.56 against 1.67. With
  this resolution, the command-path changes show no throughput loss beyond
  about 5%.
- **What this does not measure.** Depth 1 is not measured here; scale mode
  runs depth 16 only.

## A regression between the command-path changes and now, on the M5

### The plan, stated before measuring

The question (Firn ledger Q211): did firn's throughput fall between
6239de8c8, before the command-path changes, c60db650d, after them (main
after #23), and 08a60d308, main now? The interleaved comparison on the
i9-14900K was lost twice when the machine went offline, and the machine is
out of service; the owner's M5 Air stands in for timing only.

- **Images.** Built with `make firn-lto` on a GitHub-hosted macOS arm64
  runner (`macos-15`) by a temporary workflow on the branch `claude/m5-timing`,
  each revision with its own pinned compiler (wf-364f86c2fd16 for the first
  two, wf-0b7f5c5b9854 for the last) and the runner's one Apple clang. The
  three therefore differ only in firn's and the compiler's sources, not in
  the LLVM version, unlike the 14900K images, of which the first two used
  LLVM 18 and the last LLVM 22.
- **Client.** `redis-benchmark` of Redis 7.0.15, built from the release
  archive whose SHA-256 `tests/redis-suite/run.sh` pins.
- **Settings.** 50 connections, 100,000 random keys, pipeline depths 1 and
  16, the server with `WF_DRIVERS` 1 and 2. macOS offers no CPU pinning, so
  the server and the client share the machine's cores as the scheduler
  places them.
- **Order.** In every cell (test, depth, drivers) each pass measures old,
  mid, new and new-twin, a second run of new, in a shuffled order; the whole
  run holds the host-wide lock.
- **Size.** The Air slows under sustained load, so a sample of three tests,
  two passes and 2 seconds a run comes first; its twin spread, and whether
  it grows over the run, sets the scale.

The rule: in a cell, a revision is reported slower or faster than new only
when its median over the passes differs from new's by more than the largest
difference between new and new-twin in that cell, in the same direction in
every pass. A loss is claimed only for a test whose cells show it at both
depths.

**Calibration, before the measurement.** The sample (three tests, two
passes, 50,000 requests at depth 1 and 300,000 at depth 16) ran each
measurement for 0.1 to 0.25 seconds, too short to resolve. At depth 1 every
image, Redis 7.0.15 included, reached the same 205,000 to 225,000 requests a
second; with one to four client processes the total stayed between 208,000
and 247,000 for both firn and Redis. Depth 1 on the M5 is therefore bound by
the loopback round trip, not by the server, and cannot show a server's
change; it waits for the 14900K. The measurement is depth 16 only: the ten
tests of the 14900K comparison, `WF_DRIVERS` 1 and 2, three passes, each
test's request count set from a short run of new to about 2.5 seconds. The
rule above then claims a loss only for a test whose cells show it at both
driver counts.

## Scripts, transactions and expiring keys

### The plan, stated before measuring

Each workload is the selected consumers' own command forms, as the
[consumers' profiles](../consumers/README.md#results) record them:

- **Scripted rate limiting.** `EVALSHA` of rate-limiter-flexible's consume
  script, which runs `SET key 0 EX ttl NX`, `INCRBY`, `PTTL` and, for a key
  without an expiry, `EXPIRE`. Each call takes one of 100,000 keys at random.
- **Rate limiting in a transaction.** `MULTI`, `INCRBY key 1`, `PTTL key`,
  `EXEC`: the form rate-limiter-flexible's transactions take in 24 of its 29.
- **Cache `set_many`.** `MULTI`, `MSET` of three keys, `EXPIRE` of each,
  `EXEC`: Django's form in 19 of its 20.
- **Session store.** connect-redis's `SET sess:<id> <200-byte value> EX
  86400` alone, and `GET` of a stored session alone.

The settings are as follows:
- Every workload runs against redis-server 7.0.15 and firn, with the
  append-only file off, and on with fsync every second.
- It runs on 1 and 2 server CPUs, with 50 connections at pipeline depth 1,
  since these clients send one command or one transaction and wait.
- Each line has 3 interleaved passes of 10 seconds after a probe of 2 passes
  of 5 seconds.

The recorded measures are:
- requests or transactions a second;
- the median and p99 latency;
- for the session store, each server's resident memory after one million
  sessions are stored.

This measurement is exploratory and predicts nothing. Its result is each
workload's firn-to-Redis ratio of rate and of p99, with the noise between
passes, and it names every workload where firn's rate falls below Redis's or
its p99 rises above.

**The client.** redis-benchmark sends one command repeatedly and cannot send
a `MULTI` block. memtier_benchmark is not installed on the i9-14900K, and
its runner cannot install packages. The client is therefore a small Rust
program in `research/experiments/redis-bench/`, which redis-bench.sh runs
for every workload of this section, so that one client measures all of
them. It is built with the host's cargo; the 14900K's runner has it in
`~/.cargo/bin`, where Whitefoot's `compute-bench.yml` placement job finds it
to build the compiler there. It is removed when a common tool can send these
workloads.

Run `sh research/experiments/redis-bench/redis-bench.sh workloads` through
`redis-bench.yml` mode `workloads` on the 14900K; `cpus`, `passes`, `seconds`
and `tests` set `WORKLOAD_CPUS`, `WORKLOAD_PASSES`, `WORKLOAD_SECONDS` and
`WORKLOADS` (leave `tests` empty for all five). Probe with 2 passes of 5
seconds before the 3 passes of 10 seconds. The harness tests and builds the
std-only Cargo project in `research/experiments/redis-bench/workload/`
offline in release mode, then starts a fresh server for each workload and
line, reversing the line order on even passes. It uses 50 connections, one
nonblocking worker per assigned client CPU (up to 16), and waits for each
complete command or transaction reply before sending another on that
connection. Request latencies use exact one-microsecond bins, merged across
all workers; elapsed time excludes connection setup and script loading and
includes draining requests in flight at the deadline. `workloads.csv` holds
`line,pass,cpus,workload,connections,requests,seconds,rate,p50_ms,p99_ms`.
Before each `session-get` sample, `--fill 1000000` pipelines exactly
`sess:0` through `sess:999999` with 200-byte values and a one-day expiry;
`workloads-memory.csv` holds `line,pass,cpus,workload,sessions,rss_kib`
from the server's `/proc/<pid>/status` after all fill replies arrive.
Measured requests keep the planned 100,000-key range. The client also takes
`--port`, `--workload`, `--connections`, `--threads`, `--keys`, `--value-size`
and either `--seconds` or a total `--requests`; fill emits no timing row,
and any protocol, I/O or Redis error fails the run.
