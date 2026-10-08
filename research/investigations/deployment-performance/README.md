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
  a different compiler, LLVM major and day, and no interleaving, this
  comparison does not separate a loss of a few percent from those
  differences: the observed ratios are listed, and whether the command-path
  changes cost `mset` on one CPU (1.56 against 1.67) stays open.
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

### Results

Images: run [37711707922](https://github.com/Ming-Research/Firn-wf/actions/runs/37711707922)
(macOS 15.7.9, Apple clang 17.0.0, `make firn-lto`), old 6239de8c8 and mid
c60db650d with wf-364f86c2fd16, new 08a60d308 with wf-0b7f5c5b9854. The M5
run took 746 seconds under the host-wide lock on 2026-10-08, each
measurement 2 to 3 seconds; "twin spread" is the largest difference between
new and new-twin over the three passes; "within" means inside it, "mixed"
outside it but not in the same direction in every pass.

`WF_DRIVERS=1`, depth 16, thousands of requests a second (median of 3 passes):

| test | old | mid | new | new-twin | old / new | mid / new | twin spread | old | mid |
|---|---|---|---|---|---|---|---|---|---|
| set | 2721 | 2710 | 2613 | 2685 | 1.041 | 1.037 | 3.4% | mixed | mixed |
| get | 2777 | 2797 | 2761 | 2777 | 1.006 | 1.013 | 3.9% | within | within |
| incr | 2639 | 2336 | 2664 | 2636 | 0.991 | 0.877 | 3.6% | within | mixed |
| hset | 2555 | 2411 | 2431 | 2326 | 1.051 | 0.992 | 4.3% | mixed | within |
| sadd | 2662 | 2580 | 2568 | 2533 | 1.037 | 1.005 | 2.9% | faster | within |
| zadd | 1313 | 1256 | 1249 | 1296 | 1.051 | 1.005 | 3.7% | mixed | within |
| lpush | 2745 | 2732 | 2730 | 2709 | 1.005 | 1.001 | 1.7% | within | within |
| rpop | 2796 | 2783 | 2780 | 2792 | 1.006 | 1.001 | 2.4% | within | within |
| lrange_100 | 284 | 279 | 275 | 282 | 1.033 | 1.014 | 4.7% | within | within |
| mset | 1273 | 1248 | 1260 | 1304 | 1.010 | 0.991 | 3.5% | within | within |

`WF_DRIVERS=2`, depth 16, thousands of requests a second (median of 3 passes):

| test | old | mid | new | new-twin | old / new | mid / new | twin spread | old | mid |
|---|---|---|---|---|---|---|---|---|---|
| set | 2651 | 2665 | 2640 | 2634 | 1.004 | 1.009 | 2.7% | within | within |
| get | 2753 | 2751 | 2769 | 2771 | 0.994 | 0.994 | 0.3% | mixed | mixed |
| incr | 2583 | 2490 | 2646 | 2522 | 0.976 | 0.941 | 4.7% | within | mixed |
| hset | 2351 | 2100 | 2217 | 2265 | 1.060 | 0.947 | 2.1% | mixed | mixed |
| sadd | 2397 | 2383 | 2334 | 2344 | 1.027 | 1.021 | 0.6% | faster | mixed |
| zadd | 1212 | 1125 | 1206 | 1201 | 1.005 | 0.933 | 0.4% | faster | mixed |
| lpush | 2511 | 2434 | 2511 | 2525 | 1.000 | 0.969 | 1.0% | within | mixed |
| rpop | 2387 | 2578 | 2581 | 2592 | 0.925 | 0.999 | 0.4% | mixed | within |
| lrange_100 | 280 | 280 | 282 | 278 | 0.993 | 0.991 | 7.8% | within | within |
| mset | 1161 | 1200 | 1120 | 1093 | 1.037 | 1.072 | 4.1% | within | mixed |

**A check of what the run can resolve.** On the same settings, `set` at
depth 16 with one benchmark process gave firn new (one driver) 2.91 million
requests a second and Redis 7.0.15 2.05 million, 1.42 times, as on the
14900K (1.44); three benchmark processes gave 2.67 and 1.91 million. The run
therefore separates firn from Redis, but firn does not gain from a second
driver or from more client processes, so it runs near a limit outside the
server on this machine, and a difference of a few percent between firn
revisions may not reach the measurement.

**Result.**
- By the rule, no test shows a loss of mid or new against old at both
  driver counts except `sadd`, where old is faster than new by 3.7% and
  2.7%, beyond twin spreads of 2.9% and 0.6%.
- `mid` against new is within the spread or mixed in every cell; no
  command-path change shows a consistent loss.
- What this does not settle: depth 1, which the M5 cannot resolve, and
  differences smaller than the limit above; the losses of 3 to 6% that the
  14900K's non-interleaved runs suggested for `set` and `mset` are neither
  confirmed nor excluded here (`set` is mixed, `mset` within). The
  interleaved 14900K comparison stays queued for when the machine returns.

## Scripts, transactions and expiring keys

### The plan, stated before measuring

Each workload is the selected consumers' own command forms, as the
[consumers' profiles](../consumers/README.md#results) record them:

- **Scripted rate limiting.** `EVALSHA` of rate-limiter-flexible's consume
  script, which runs `SET key 0 EX ttl NX`, `INCRBY`, `PTTL` and, for a key
  without an expiry, `EXPIRE`. Each call takes one of 100,000 keys at random.
- **Rate limiting in a transaction.** `MULTI`, `INCRBY key 1`, `PTTL key`,
  `EXEC`: the `INCRBY` variant of rate-limiter-flexible's transactions that
  follow `SET`, `GET` or `INCRBY` with `PTTL`, a family that makes 24 of its
  29.
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

### Results

Run [37775047771](https://github.com/Ming-Research/Firn-wf/actions/runs/37775047771)
on the i9-14900K, 2026-10-08: Firn-wf 133119b (whitefoot.pin
`wf-e42e715c664c`, Halo-wf 0def88248) against redis-server 7.0.15, 3
interleaved passes of 5 seconds per line after a probe of 1 pass of 3
seconds ([37774316578](https://github.com/Ming-Research/Firn-wf/actions/runs/37774316578)).
The plan asked for passes of 10 seconds; the passes were shortened to keep
the machine's slot short. The table gives each rate's spread between passes
as the range over the mean; it gives no spread for p99, so the p99
comparisons below rest on medians of three passes alone. One rate
comparison does not clear its spread: the scripted limiter on one CPU with
the append-only file on, 0.88, where firn's own rate varied by 14.6%.

Rate in thousands a second (spread), firn over Redis, and median p99 in ms:

| CPUs | workload | AOF | Redis | firn | firn/Redis | p99 Redis | p99 firn |
|---|---|---|---|---|---|---|---|
| 1 | limiter-script | off | 154 (11.8%) | 120 (6.5%) | 0.78 | 0.70 | 3.42 |
| 1 | limiter-script | on | 130 (5.6%) | 115 (14.6%) | 0.88 | 0.98 | 3.42 |
| 1 | limiter-tx | off | 253 (10.2%) | 312 (5.6%) | 1.23 | 0.48 | 0.24 |
| 1 | limiter-tx | on | 226 (6.7%) | 317 (3.9%) | 1.41 | 0.46 | 0.26 |
| 1 | setmany-tx | off | 150 (2.7%) | 187 (3.2%) | 1.24 | 0.82 | 0.39 |
| 1 | setmany-tx | on | 96 (2.6%) | 104 (0.9%) | 1.08 | 3.74 | 1.76 |
| 1 | session-set | off | 267 (6.7%) | 305 (5.8%) | 1.14 | 0.36 | 0.26 |
| 1 | session-set | on | 203 (1.9%) | 187 (1.9%) | 0.92 | 0.65 | 0.84 |
| 1 | session-get | off | 273 (6.8%) | 314 (6.4%) | 1.15 | 0.31 | 0.24 |
| 1 | session-get | on | 266 (8.4%) | 323 (16.3%) | 1.21 | 0.44 | 0.24 |
| 2 | limiter-script | off | 160 (7.2%) | 109 (3.5%) | 0.68 | 0.60 | 5.12 |
| 2 | limiter-script | on | 135 (9.0%) | 107 (6.7%) | 0.79 | 0.88 | 5.51 |
| 2 | limiter-tx | off | 263 (4.4%) | 598 (4.3%) | 2.28 | 0.34 | 0.15 |
| 2 | limiter-tx | on | 229 (2.0%) | 513 (1.5%) | 2.24 | 0.42 | 0.50 |
| 2 | setmany-tx | off | 150 (10.1%) | 335 (4.6%) | 2.24 | 0.83 | 0.31 |
| 2 | setmany-tx | on | 106 (5.4%) | 208 (6.0%) | 1.95 | 1.23 | 0.97 |
| 2 | session-set | off | 258 (10.6%) | 560 (4.6%) | 2.17 | 0.49 | 0.16 |
| 2 | session-set | on | 210 (1.7%) | 450 (5.2%) | 2.14 | 0.64 | 0.37 |
| 2 | session-get | off | 273 (4.4%) | 626 (4.9%) | 2.29 | 0.32 | 0.15 |
| 2 | session-get | on | 274 (5.6%) | 556 (5.0%) | 2.03 | 0.32 | 0.16 |

Resident memory after one million sessions, KiB, per pass: Redis 355,036
to 359,148 with the append-only file off or on; firn 411,460 to 411,732
off, 437,516 to 439,128 on with one CPU, and 522,792 to 559,168 on with
two.

Where firn falls below Redis:
- **The scripted rate limiter.** At 0.68 to 0.88 of Redis's rate, with a
  p99 of 3.4 to 5.5 ms against 0.6 to 1.0 ms, and worse on two CPUs than on
  one. firn runs every script on one engine that scripts take in turn
  (`firn/script_pool`); waiting for the engine is the hypothesis for the
  second CPU's loss, not measured here, and why one CPU's p99 is five times
  Redis's is not known.
- **Session writes with the append-only file on one CPU,** at 0.92 with a
  p99 of 0.84 ms against 0.65 ms.
- **The transaction rate limiter's p99 with the append-only file on two
  CPUs,** 0.50 ms against 0.42 ms, while its rate is 2.24 times Redis's.
- **Memory,** 16 percent above Redis for the same sessions, 23 percent with
  the append-only file on one CPU and 47 to 58 percent on two.

Every other line's rate is above Redis's: 1.08 to 1.41 times on one CPU and
1.95 to 2.29 times on two, where Redis serves from one thread; their p99 is
at or below Redis's.

## Why the scripted limiter's tail is long

### The question, stated before measuring

On one CPU firn runs the rate limiter's script at 0.78 of Redis's rate with
a p99 of 3.4 ms against Redis's 0.7 ms, while its median is close to
Redis's. firn runs every script on one engine: a script takes it in an
atomic statement whose guard waits while another script holds it
(`take_engine` in `firn/scripting/entry.wf`), and putting it back wakes
every context waiting on that guard, of which one takes it and the rest
wait again, in no order. A long tail would follow if some contexts lose
that race repeatedly.

The comparison: the same build, one CPU, the limiter script with 1, 8 and 50
connections, 2 passes of 5 seconds, firn and Redis, with the append-only
file off. With one connection no script waits for the engine.

- If waiting for the engine makes the tail, firn's p99 at one connection is
  near Redis's and grows with the connections well beyond Redis's growth.
- The hypothesis is rejected if firn's p99 at one connection is already
  several times Redis's: the time is then in the script's own path (the
  per-call copy of the source out of the registry, the cache lookup, the
  engine's run, or the reply), which a profile of that path then locates.

### Result

Run [37781681980](https://github.com/Ming-Research/Firn-wf/actions/runs/37781681980)
on the i9-14900K, Firn-wf 703cdfb, one CPU, 2 passes of 5 seconds. Rate in
thousands a second and p99 in ms, both passes:

| connections | Redis | firn | Redis, AOF | firn, AOF |
|---|---|---|---|---|
| 1 | 49.0, 49.0 / 0.030, 0.030 | 47.3, 45.3 / 0.028, 0.033 | 47.2, 45.8 / 0.031, 0.033 | 46.4, 46.7 / 0.029, 0.028 |
| 8 | 211.7, 212.7 / 0.084, 0.084 | 155.3, 158.5 / 0.087, 0.085 | 190.5, 197.4 / 0.112, 0.082 | 155.2, 161.3 / 0.086, 0.082 |
| 50 | 155.5, 155.0 / 0.554, 0.571 | 121.0, 113.0 / 3.420, 3.433 | 139.1, 138.3 / 0.964, 0.938 | 111.7, 110.3 / 3.420, 3.424 |

With one connection firn matches Redis in rate and p99, so the script's own
path is not the cost: the hypothesis survives its rejection test. The loss
appears when connections compete: at 8 connections firn's rate is 0.75 of
Redis's with an equal p99, and at 50 its p99 is 3.42 ms in every pass and
line, six times Redis's. Whitefoot's runtime wakes every context watching a
unit when a statement writes it (`wf_watch_wake_locked` in its
`completion/bridge.c`), so each release of the engine wakes every waiting
script. A p99 this constant across passes points to a fixed delay rather
than to chance in that race; a profile of firn at 8 and 50 connections is
the next measurement.

A profile of firn under the same workload (run
[37782618040](https://github.com/Ming-Research/Firn-wf/actions/runs/37782618040),
one CPU, `perf record` of one extra unmeasured 5-second run per line, flat
self time) does not support the race as the main cost. In all four profiles
(8 and 50 connections, append-only file off and on) the largest symbols are
Halo's collector check `halo.vm.collect_if_due` at 11.8 to 12.6%, the C
allocator (calloc, malloc, free and consolidation) at 11 to 13% together,
Halo's string interning and table rehash at about 5%, and firn's copy of the
script's source out of the registry on every `EVALSHA` (`text_bytes`) at 2.8
to 3.8%; waiting for the engine and waking (`acquire_whole`, the shared
lock) take about 3%. Halo collects with a whole mark and sweep each time a
collection is due (`collect_if_due` in its `vm/collect.wf`), where Lua 5.1,
which Redis runs, collects incrementally. The working hypothesis is
therefore a periodic collection pause of about 3 ms: with 50 connections
waiting, one pause delays some 50 requests and so sets the p99, while with
one connection it delays one request in thousands and stays below the p99.
The collection pause has not been measured; Halo's session owns that
measurement and the collector. firn's own part is the per-call copy of the
source.

Halo's session asked for the pause to be measured before any change to its
collector. Run
[37792606166](https://github.com/Ming-Research/Firn-wf/actions/runs/37792606166)
used an experiment branch (`exp/script-gc-probe` at 51f9b04, not merged)
that reads Halo's `collection_count` and the monotonic clock just after a
script call takes the engine and just before it returns it, and sorts calls
into those during which a collection completed and the rest. One CPU,
5 seconds per line; the 50-connection figures subtract the 8-connection run
that preceded them on the same server.

| line | connections | calls | with a collection | mean, with | mean, without | calls per collection | client p99, firn / Redis |
|---|---|---|---|---|---|---|---|
| firn | 8 | 786,572 | 241 | 3.24 ms | 2.7 µs | 3,264 | 0.087 / 0.100 ms |
| firn | 50 | 1,440,274 | 430 | 3.70 ms | 3.0 µs | 3,349 | 3.457 / 0.714 ms |
| firn, AOF | 8 | 815,809 | 250 | 3.05 ms | 2.6 µs | 3,263 | 0.085 / 0.090 ms |
| firn, AOF | 50 | 1,283,655 | 383 | 3.78 ms | 3.1 µs | 3,352 | 3.457 / 0.814 ms |

Calls with a collection fall between 1 and 8 ms, most between 2 and 4; the
longest took 6.7 ms. A collection pauses every waiting connection: about one
request in 3,300 runs into one, and each pause delays the other waiting
connections' requests too, about 50 / 3,300 = 1.5% of requests at 50
connections, above the 1% the p99 counts, and 8 / 3,300 = 0.24% at 8,
below it. That accounts for the p99 equal to the pause at 50 connections
and for its absence at 1 and 8. Collections took 0.78 of 5 seconds at 8
connections, 15.6% of the server's time, a large part of the rate's 0.75.
The profile of the same run no longer lists `text_bytes`, the per-call copy
this branch removed. The collector belongs to Halo, whose design records a
whole-heap collection as an owner decision; these figures go to Halo's
session as the evidence for its card.
