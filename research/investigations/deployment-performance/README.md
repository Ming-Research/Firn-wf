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

**Compare mode and AOF.** `redis-bench.yml` mode `compare` measures the
images `revisions` names, interleaved, with a `-twin` as the noise control.
Its `aof` input (`COMPARE_AOF=true`, default false) starts every image with
the append-only file on, as firn's third positional argument, `firn.aof`,
turns it on, with appendfsync everysec (firn's only policy). Each start
sets `auto-aof-rewrite-percentage` to 0 with `CONFIG SET`, read back by
`CONFIG GET`, so that no rewrite runs and the cost measured is the
steady-state append path. Each server starts in a fresh directory that is
removed when it stops. The run prints `aof,<line>,aof_enabled=<n>` from
INFO persistence at every start and fails unless it reads 1; it records
`setting,aof=true` (`false` when off) in its output and in `host.txt`.
Its `compare_reference` input (`COMPARE_REFERENCE=true`, default false)
adds Redis 7.2.16 as one more line named `reference`, measured after the
images under the same settings: with `aof` set it runs with the append-only
file on (appendfsync everysec, a fresh directory removed when it stops) and
the same checks; the script fails unless `redis-server --version` reads
7.2.16. It records `setting,reference=true` (`false` when off) in its output
and in `host.txt`.

**Placement.** By default the script places servers and clients itself, one
thread per physical core, leaving core 0 to the OS. `server_cpus` and
`client_cpus` (`SERVER_CPU_LIST`, `CLIENT_CPU_LIST`) replace either list with
explicit CPU numbers or ranges such as `0-15`; empty keeps the automatic
choice. The server list holds exactly as many CPUs as the count in `cpus`
(so give one count) and may name both threads of a core, so the count is
logical CPUs; the client list sets the number of client threads or
processes, one per CPU. A CPU may appear once in a list, must be online, and
no client CPU may share a physical core with a server CPU; otherwise the run
fails before measuring. The owner's shape is the server on 1, 2, 4 or 8
performance cores, or on 8 with all 16 threads, and clients on the 16
efficiency cores; the dispatcher keeps it by passing the lists, since the
script does not enforce core classes (hosts without a hybrid topology must
run too). Each placement is printed once and appended to `host.txt` as
`placement,server=<cpus>,client=<cpus>,threads=<n>,source=<kind>,`
`server_class=<cpu:class,...>,client_class=<cpu:class,...>`, where the class
is `P` (performance core), `E` (efficiency core) or `-` (no hybrid
topology), read from `/sys/devices/cpu_core/cpus` and `cpu_atom/cpus` or
else from `cpu_capacity`. `placement-selftest` checks the rules against a
fake topology.

**CPU load.** Where `/proc/stat` exists, `compare-cpu.csv` has one row per
measured sample and per server and client CPU, with 10 columns:
`line,pass,cpus,test,depth,cpu,role,busy,user,system`. `busy` is the share
of that CPU's ticks, over the client's run, that were neither idle nor
iowait; `user` is the user and nice share and `system` the system, irq and
softirq share (a thread spinning on `sched_yield` shows as system time).
The workload client spins (it yields between scans and never sleeps, so that
its throughput and latency add no wake-up cost), so the OS busy fraction says
nothing about whether it is limited: run 38400671487 showed every cell at 0.998
or above. `workloads` mode therefore judges the client by its own accounting
(below); `compare` mode keeps the `busy` rule, and its client is a different
program.
A cell is client-limited when the mean `busy` over its client CPUs is above
0.90: a client that busy cannot be shown to be slower than the server, so
such a cell only requires firn not to fall behind. `compare-limited.csv`
marks each sample with `line,pass,cpus,test,depth,client_busy_mean,client_limited`
(`client_limited` is 1 above 0.90, else 0; exactly 0.90 is not limited).
`workloads` mode records the same for its handshake, client-list and
consumer workloads: `workload-cpu.csv` has the same columns with the workload
name in `test` and the connection count in `depth`, and `workload-limited.csv`
has `line,pass,cpus,workload,connections,client_busy_mean,client_work_fraction,client_limited`.
`client_work_fraction` is the client's own accounting: each worker times every
scan over its connections, a scan counts as work when any connection wrote
bytes, read bytes, connected or completed a request, and the fraction is the
work-scan time over the whole loop time, summed over workers, for the measured
run (warm-up excluded). `client_limited` is 1 when it is above 0.90 (exactly
0.90 is not limited). `client_busy_mean` is the `/proc/stat` mean over the
client CPUs for the whole client run, including its connection setup and
warm-up; it is recorded but does not decide `client_limited`. The memory
workloads are not recorded.

**Other inputs.**

- `maxmemory`: sets `maxmemory` in bytes with `CONFIG SET` on every measured
  server after it starts, read back with `CONFIG GET`; empty sets none.
- `aof`: described above.
- `compare_reference`: described above.
- The correctness pass of scale and suite modes runs Redis 7.0's
  redis-benchmark default suite, its 19 tests named explicitly
  (`SUITE_DEFAULT`), on every line. Redis 7.2.16's default suite adds XADD;
  it is left out only because firn has no streams yet, and goes back in when
  firn's minimal streams (board item `firn-scope-min-streams`) land.
- `drivers`: sets the Whitefoot driver count of each image, apart from the
  number of server CPUs; empty gives one driver per server CPU.
- `c2c`: with `profile`, also records `perf c2c` on each profiled test,
  showing the cache lines several cores contend for.
- `sched_report`: sets `WF_SCHED_REPORT=2` on firn images, so each pass's
  kept server log ends with the runtime's per-driver scheduling counts.

The workflow has no concurrency group, so a new dispatch cancels no running
one. On the 14900K the single runner takes one job at a time, so dispatches
queue and run in order; hosted-runner dispatches run side by side.

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
  second CPU's loss, not measured here; why one CPU's p99 is five times
  Redis's is answered [below](#why-the-scripted-limiters-tail-is-long).
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
appears when connections compete: at 8 connections firn's rate is 0.73 to
0.75 of Redis's without the append-only file and 0.81 to 0.82 with it, with
an equal p99, and at 50 its p99 is 3.42 to 3.43 ms in every pass and line,
6.0 to 6.2 times Redis's without the append-only file and 3.5 to 3.7 times
with it. Whitefoot's runtime wakes every context watching a
unit when a statement writes it (`wf_watch_wake_locked` in its
`completion/bridge.c`), so each release of the engine wakes every waiting
script. A p99 this constant across passes points to a fixed delay rather
than to chance in that race; a profile of firn at 8 and 50 connections is
the next measurement.

A profile of firn under the same workload (run
[37782618040](https://github.com/Ming-Research/Firn-wf/actions/runs/37782618040),
Firn-wf 7b96ba5, one CPU, one pass, `perf record` of one extra unmeasured
5-second run per line, flat self time) shows where firn's CPU goes. A flat
on-CPU profile cannot show time a context spends waiting off the CPU, so it
bounds only the CPU cost of the race, not its delay; the probe below
measures the time a script holds the engine. In all four profiles
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
one pass, 5 seconds per line. The counters run from the server's start and
were read after each measured run, before the unmeasured profiled rerun, so
the reading after the 50-connection run also holds the 8-connection
measured run and its profiled rerun, which cannot be separated: only the
8-connection rows are clean, and the table gives the 50-connection lines
their client p99 alone.

| line | connections | calls | with a collection | mean, with | mean, without | calls per collection | client p99, firn / Redis |
|---|---|---|---|---|---|---|---|
| firn | 8 | 786,572 | 241 | 3.24 ms | 2.7 µs | 3,264 | 0.087 / 0.100 ms |
| firn | 50 | not separable | | | | | 3.457 / 0.714 ms |
| firn, AOF | 8 | 815,809 | 250 | 3.05 ms | 2.6 µs | 3,263 | 0.085 / 0.090 ms |
| firn, AOF | 50 | not separable | | | | | 3.457 / 0.814 ms |

In the 8-connection runs, calls with a collection fall between 1 and 8 ms,
most between 2 and 4; the longest took 6.2 ms. A collection pauses every waiting connection: about one
request in 3,300 runs into one, and each pause delays the other waiting
connections' requests too, about 50 / 3,300 = 1.5% of requests at 50
connections, above the 1% the p99 counts, and 8 / 3,300 = 0.24% at 8,
below it. That accounts for the p99 equal to the pause at 50 connections
and for its absence at 1 and 8. Collections took 0.78 of 5 seconds at 8
connections, 15.6% of the server's time, a large part of the rate's 0.73 to
0.75 without the append-only file.
The profile of the same run no longer lists `text_bytes`, the per-call copy
this branch removed. The collector belongs to Halo, whose design records a
whole-heap collection as an owner decision; these figures go to Halo's
session as the evidence for its card.

## Scripts on two cores, on the native host

The i9-14900K runner was reinstalled as native Ubuntu (kernel 7.0) on
2026-10-10, replacing the Hyper-V guest of every earlier section; numbers
from before and after are not compared. The benchmark placement was also
corrected (PR #43): until then a "two-CPU" server ran on one physical
core's two hyperthreads. The runs below place the server on CPU 2 (one CPU)
or CPUs 2 and 4 (two CPUs), each on its own performance core, and the
client on the remaining performance cores' threads (4-15 or 6-15).

### Baseline

Run [38052762047](https://github.com/Ming-Research/Firn-wf/actions/runs/38052762047),
Firn-wf 0cfcf57 (whitefoot.pin `wf-78223721f77d`), 2026-10-10
12:39-12:56 UTC, 2 passes of 10 seconds. Rate in thousands a second as
firn / Redis (ratio), p99 in ms as firn / Redis, append-only file off. The
Redis measured is the reinstalled host's packaged Redis 8.0.5, not firn's
reference 7.0.15 (the run's host log; status board item
`firn-ops-redis-ref`), so its column compares firn with 8.0.5:

| workload | CPUs | connections | rate | p99 |
|---|---|---|---|---|
| limiter-script | 1 | 1 | 121 / 136 (0.89) | 0.01 / 0.02 |
| limiter-script | 1 | 8 | 216 / 231 (0.94) | 0.08 / 0.06 |
| limiter-script | 1 | 50 | 219 / 231 (0.95) | 0.27 / 0.41 |
| limiter-script | 2 | 8 | 159 / 233 (0.68) | 0.73-0.84 / 0.06 |
| limiter-script | 2 | 50 | 211 / 231 (0.91) | 1.45-1.46 / 0.41 |
| limiter-tx | 1 | 50 | 435 / 371 (1.17) | 0.12 / 0.25-0.26 |
| limiter-tx | 2 | 8 | 676 / 356 (1.90) | 0.01 / 0.04 |
| limiter-tx | 2 | 50 | 767 / 375 (2.05) | 0.08-0.09 / 0.25 |

With the append-only file on, the ratios are within 0.15 of these. On one CPU
the script path is within 11% of Redis with an equal or lower tail, the
collection pause of the previous section being gone at Halo's 64 KiB
collection floor. On two CPUs transactions scale (1.9 to 2.05 times
Redis) and scripts do not: their rate falls below the one-CPU rate at 8
connections and their p99 is several times Redis's.

### Against the reference Redis 7.0.15

Run [38066137541](https://github.com/Ming-Research/Firn-wf/actions/runs/38066137541),
Firn-wf 541349d (main 8744d72 with PR #46's workflow step, which builds
Redis 7.0.15 from its release tarball; whitefoot.pin `wf-78223721f77d`);
the host log shows `Redis server v=7.0.15` and `redis-benchmark 7.0.15`.
2026-10-10 16:04-16:11 UTC, the placement of the section's introduction, 2
passes of 5 seconds. Rate in thousands a second as firn / Redis (ratio),
each the mean of the two passes, p99 in ms as firn / Redis with both passes
where they differ, append-only file off:

| workload | CPUs | connections | rate | p99 |
|---|---|---|---|---|
| limiter-script | 1 | 8 | 213 / 210 (1.01) | 0.079-0.080 / 0.066 |
| limiter-script | 1 | 50 | 219 / 210 (1.05) | 0.26 / 0.45 |
| limiter-script | 2 | 8 | 158 / 208 (0.76) | 0.71-0.73 / 0.064-0.065 |
| limiter-script | 2 | 50 | 209 / 210 (1.00) | 1.51-1.57 / 0.45 |
| limiter-tx | 1 | 8 | 409 / 347 (1.18) | 0.021 / 0.036-0.037 |
| limiter-tx | 1 | 50 | 432 / 364 (1.19) | 0.12 / 0.26 |
| limiter-tx | 2 | 8 | 672 / 352 (1.91) | 0.014-0.015 / 0.035 |
| limiter-tx | 2 | 50 | 769 / 366 (2.10) | 0.069-0.093 / 0.26 |

With the append-only file on, firn / Redis is 0.96-1.10 for the scripted
limiter and 1.11-1.16 for transactions on one CPU, and 0.81-1.03 and
1.85-1.95 on two. Across runs, Redis 7.0.15 ran the scripted limiter about
9% slower than the host's 8.0.5 did above (210 against 231 thousand a
second, 5- against 10-second passes), so on one CPU firn's scripts match or
exceed the reference's rate, with a lower p99 at 50 connections and a
higher one at 8.
The two-CPU picture is unchanged: transactions scale to about twice Redis,
scripts do not, and their p99 is several times Redis's.

### Where scripts wait

Run [38050875132](https://github.com/Ming-Research/Firn-wf/actions/runs/38050875132),
experiment branch `exp/engine-probe` (not merged), which counts in the
engine pool every checkout's wait (from the first attempt to taking the
engine) and the engine's hold, reset before each measured run and read
after it, same placement, 2 passes of 10 seconds:

| CPUs | connections | waits under 10 us | 10-100 us | 0.1-1 ms | 1-10 ms | longest wait | longest hold |
|---|---|---|---|---|---|---|---|
| 1 | 50 | 100% | 0 | 0 | 0 | 1.9 us | 0.16 ms |
| 2 | 8 | 82.5% | 7.5% | 9.9% | 0.09% | 3.4 ms | 0.07 ms |
| 2 | 50 | 54.0% | 7.7% | 31.9% | 6.5% | 8.8 ms | 0.08 ms |

On two CPUs with 50 connections the waits sum to 446 seconds over a
10-second run, 89% of the 50 connections' time, while the engine is held for
8.1 seconds in total and never longer than 75 us at a time. Two causes
compound: one engine serves every script, and each script holds the whole
keyspace while it runs, so two scripts never run at once; and the waiters on
the engine's guard are all woken at each release and race, in no order, so
some lose many times in a row. The second is a Whitefoot runtime question,
recorded on the status board as `firn-gap-guard-fairness` with a standalone
reproducer. The first is firn's design; the direction (an engine per driver
with scripts holding only their declared keys) awaits the owner on card
`firn-q-script-engine`.

### A fair guard, measured on the script engine

The Whitefoot session that owns the runtime built one remedy for the racing
waiters: a guard release gives each woken waiter one turn before a newcomer
may take the object ("turns", experiment release `wf-exp-5f6dca744b8f`,
Whitefoot commit 5f6dca744 on branch `claude/guard-fairness`, based on main
fe5589ec5). On its own probe this cut the longest wait from 510 ms to 9-17
ms while lengthening the run 2.5-2.9 times, and it asked for firn's real
engine before the owner decides between fairness and throughput (status
board card `gran-guard-fairness`).

Run [38065358945](https://github.com/Ming-Research/Firn-wf/actions/runs/38065358945),
2026-10-10 15:52-15:57 UTC, workloads mode with `workload_images`, an
exploratory comparison with no criterion registered beforehand: the same
firn tree (main 8744d72) built with the matching control release
`wf-fe5589ec5f45` (branch `exp/guard-control`, 22283eb) and with the turns
release (branch `exp/guard-turns`, 4b35075), plus a twin of the control as a
noise control, interleaved with the host's Redis 8.0.5 (not the reference
7.0.15), the order reversed on the second pass. limiter-script at depth 1, server on CPUs 2 and 4 (two
drivers), client on CPUs 6-15 (10 threads), append-only file off, 2 passes
of 5 seconds. Rate in requests a second and p50/p99 in ms, pass 1 / pass 2:

| line | connections | rate | p50 | p99 |
|---|---|---|---|---|
| Redis 8.0.5 | 8 | 228770 / 228835 | 0.029 / 0.029 | 0.059 / 0.058 |
| Redis 8.0.5 | 50 | 231140 / 231900 | 0.202 / 0.201 | 0.410 / 0.408 |
| control | 8 | 159068 / 159481 | 0.018 / 0.018 | 0.793 / 0.742 |
| control | 50 | 211012 / 209346 | 0.087 / 0.071 | 1.500 / 1.628 |
| control twin | 8 | 159278 / 159617 | 0.018 / 0.018 | 0.724 / 0.699 |
| control twin | 50 | 208578 / 205304 | 0.062 / 0.086 | 1.679 / 1.587 |
| turns | 8 | 21281 / 21292 | 0.346 / 0.346 | 0.873 / 0.874 |
| turns | 50 | 4411 / 4394 | 7.959 / 7.879 | 51.211 / 52.503 |

The control and its twin agree within about 2% on rate; their p50 and p99
differ by up to about 40% and 12%. Turns cuts the scripted rate 7.5 times at
8 connections and 48 times at 50, and its p99 at 50 connections is 32-34
times the control's. The likely cause, not yet
profiled: every script checks out the one engine under a guard, so each
acquisition now pays a context switch, and more connections queue more
turns. This tested one workload
on one machine; the workload client reports p50 and p99, not the longest
wait. It is evidence against turns as built for this engine, and the
reason firn asks that any bounded-overtaking build be measured the same way.

## Handshake and CLIENT LIST rate against four rivals

Firn-wf#74 adds client handshake commands and `CLIENT LIST`. Run
[38485636396](https://github.com/Ming-Research/Firn-wf/actions/runs/38485636396)
of the `redis-bench` workflow on the i9-14900K, dispatched on branch
`claude/perf-reproduce` at commit 9946abf, measured two workloads of the
workload tool: new connections doing the handshake (`HELLO`,
`CLIENT SETNAME`) and `CLIENT LIST`. Rivals: Redis 7.2.16, Valkey 9.1.2,
Dragonfly v2.0.2, Garnet v2.2.1.

Conditions, read from the run's log: firn was built from commit de6d7b5, the
head of the pull request before it merged `main`, with `whitefoot.pin`
`wf-d75f318d5631`; the servers ran on CPUs 2 and 4 and the load clients on
CPUs 16-31; 50 connections; 6 passes, each running every server for 10
seconds per workload, the order of the servers changing between passes.
The pull request's head now differs from that commit (it has merged `main`
and `CLIENT LIST` reads other connections' subscription counts from the
event hub), so these figures describe the earlier build.

CLIENT LIST, firn about 133k requests a second:

| rival | firn / rival |
| --- | --- |
| Redis 7.2.16 | 3.85x |
| Valkey 9.1.2 | 4.70x |
| Dragonfly v2.0.2 | 5.57x |
| Garnet v2.2.1 | 0.69x (Garnet is faster) |

Handshake: all six servers land at about 20-23k new connections a second.
The workload client busy-polls, so every client CPU shows a busy rate above
0.998 and no cell can be told apart as client-limited from the data; the
equal rates may be the client's limit, not the servers'.

Verdict: not judged against the standard. These figures are the run's rates.
The noise-gate and 95% confidence-interval standard needs interleaved
base/candidate pairs beside the twin, the twin ratio's interval half-width as
noise, the platform floor, and the candidate/base interval; this record has no
pairs, noise, floor or interval for this run, so it claims no regression,
improvement or "no detected change". The ratios are descriptive only.

## Final-head check of the CLIENT handshake change (Firn-wf#74)

Written before the measurement. The owner ruled (card firn-client-merge-perf,
option A) that Firn-wf#74 merges when the commands firn already supported are
not slower than before, the handshake meets the depth-1 rule against the
fastest competitor, and CLIENT LIST is reported against all four competitors,
its gap to Garnet followed by firn-client-list-garnet.

Questions and the results that would answer them:

1. Is any command firn supported before this change slower with it? The change
   touches paths all commands share: every `EXEC` takes the client table with
   Meta (design/firn/transactions.md), every connection takes the table when
   it starts and when it ends, and every key access tests the connection's
   NO-TOUCH flag. Rejected if, for any cell below, the 95% interval of
   head/base lies wholly under 1 while the twin's interval contains 1; the
   size does not matter. A cell whose twin interval excludes 1, or whose
   head/base interval contains 1 yet is wider than the twin's, is undecided:
   passes are added up to 12, and a cell still undecided is reported
   unresolved, not as no change.
2. Does the handshake meet the owner's depth-1 rule? Met if firn's median rate
   is not below the fastest competitor's and its p99 is not above that
   competitor's p99. A cell is client-limited when the median client work
   fraction in `workload-limited.csv` of firn or of that competitor is above
   0.90; it then asks only that firn is not behind, and its p99 is reported
   and not judged (as for the pub/sub workloads). A shortfall smaller than the
   twin's half-width is undecided and gets more passes, up to 12.
3. How does CLIENT LIST compare? Reported for Redis 7.2.16, Valkey, Dragonfly
   and Garnet: multiple of rate and p99 against each. The rule of 2 is judged
   against the fastest competitor other than Garnet; the Garnet cell is
   reported and not judged.

Sides, measured with `summary.py` over the run's CSV files (`ratios` for A,
`workloads` for B):

- Run A, `redis-bench.yml` mode `compare`: images `base` (`origin/main` at the
  dispatch), `head` (this branch at the dispatch) and `head-twin` (head
  again); Redis 7.0's default `redis-benchmark` suite, the 19 tests named by
  `SUITE_DEFAULT` in `redis-bench.sh`, at depth 1; server CPUs 2 and 4, client
  CPUs 16-31; 6 passes of 5 seconds, profiling off. The twin is head measured
  again, so its ratio to head is the run's noise, as the tool prints it.
- Run B, mode `workloads` with `workload_images`: the same three images and
  Redis 7.2.16, Valkey, Dragonfly and Garnet; tests `limiter-tx` (`MULTI`,
  `INCRBY`, `PTTL`, `EXEC`), `setmany-tx` (`MULTI`, one `MSET` of three keys,
  three `EXPIRE`, `EXEC`), `handshake` and `client-list`; 50 connections;
  the same CPUs; 6 passes of 10 seconds. The two transaction workloads cover
  `EXEC`, which `redis-benchmark` does not send.

Before either run, a probe of Run A with 2 passes of 5 seconds checks that the
inputs work and times a pass; it is not reported as a result. No noise floor
is recorded yet for these workload classes on this machine, so each run's
largest twin half-width per class is reported as that class's floor, for the
architect to record, and no cell is dropped by it in these runs.

### Changes to the plan after the first dispatch

All runs below measured Firn-wf#74 at commit 9b514d5 (compiler release
wf-af7665be672d) against `main` at 7f8b4f8, which that commit had merged, on
the i9-14900K: server CPUs 2 and 4, load clients on CPUs 16-31, depth 1.

- The probe of Run A (run 39898075275, 2 passes of 5 seconds) took 13.5
  minutes, 2 of them building, and is not reported.
- Run B was split in two. The probe of Run B (run 39910304830) stopped at the
  handshake workload on the `base` image: `main`'s firn has neither
  `CLIENT SETINFO` nor `CLIENT LIST`, so these two workloads have no "before"
  to pair with. This was an error in the plan above, not a finding. Run B2
  measures the handshake and `CLIENT LIST` with images `head` and `head-twin`
  and the four rivals (Redis 7.2.16, Valkey 9.1.2, Dragonfly v2.0.2, Garnet
  2.2.3); Run B1 measures the two transaction workloads with the three images
  and no rivals.
- Every dispatch builds its images again, so the binary of one source differs
  in its SHA-256 between dispatches. A cell the rule left undecided after 6
  passes got a second dispatch of the same inputs, and the two dispatches'
  passes are pooled as passes 1-12. Within a dispatch `head` and `head-twin`
  are one binary; `base` and `head` are two.
- The rule was not changed after its results were seen.

### Results

Run A, `redis-benchmark` default suite (19 tests), 6 passes of 5 seconds in
each of runs
[39899821670](https://github.com/Ming-Research/Firn-wf/actions/runs/39899821670)
and
[39903665355](https://github.com/Ming-Research/Firn-wf/actions/runs/39903665355),
12 passes pooled, `summary.py ratios`. The verdict column applies the rule
above: rejected needs the head/base interval wholly under 1 with the twin's
interval containing 1.

| test | head/base median | head/base 95% interval | twin/head 95% interval | twin half-width | verdict |
| --- | --- | --- | --- | --- | --- |
| get | 1.0042 | 0.9949-1.0257 | 0.9874-1.0019 | 0.73% | undecided (wider than twin) |
| hset | 0.9880 | 0.9723-1.0299 | 0.9811-1.0105 | 1.47% | undecided (wider than twin) |
| incr | 0.9969 | 0.9801-1.0129 | 0.9927-1.0182 | 1.28% | undecided (wider than twin) |
| lpop | 1.0014 | 0.9948-1.0183 | 0.9971-1.0118 | 0.74% | undecided (wider than twin) |
| lpush | 0.9902 | 0.9759-1.0068 | 0.9961-1.0173 | 1.06% | undecided (wider than twin) |
| lrange_100 | 1.0022 | 0.9879-1.0303 | 0.9923-1.0099 | 0.88% | undecided (wider than twin) |
| lrange_300 | 1.0033 | 0.9965-1.0106 | 0.9552-1.0222 | 3.35% | no detected change |
| lrange_500 | 1.0064 | 1.0000-1.0170 | 0.9964-1.0040 | 0.38% | undecided (wider than twin) |
| lrange_600 | 1.0051 | 1.0009-1.0102 | 0.9918-1.0062 | 0.72% | faster |
| mset | 1.0058 | 0.9956-1.0189 | 0.9929-1.0083 | 0.77% | undecided (wider than twin) |
| ping_inline | 0.9911 | 0.9850-1.0136 | 0.9921-1.0075 | 0.77% | undecided (wider than twin) |
| ping_mbulk | 0.9948 | 0.9885-1.0066 | 0.9941-1.0150 | 1.05% | no detected change |
| rpop | 0.9918 | 0.9843-1.0033 | 0.9839-1.0115 | 1.38% | no detected change |
| rpush | 0.9892 | 0.9753-0.9977 | 1.0020-1.0176 | 0.78% | undecided (twin interval excludes 1) |
| sadd | 0.9831 | 0.9706-1.0248 | 0.9877-1.0145 | 1.34% | undecided (wider than twin) |
| set | 1.0084 | 0.9859-1.0531 | 0.9877-1.0098 | 1.10% | undecided (wider than twin) |
| spop | 0.9881 | 0.9786-1.0356 | 0.9867-1.0107 | 1.20% | undecided (wider than twin) |
| zadd | 1.0032 | 0.9875-1.0347 | 0.9766-0.9951 | 0.92% | undecided (twin interval excludes 1) |
| zpopmin | 0.9894 | 0.9725-1.0240 | 0.9821-1.0107 | 1.43% | undecided (wider than twin) |

No client was limited (`compare-limited.csv`: no sample above 0.90). Run B1,
transaction workloads, 50 connections, 6 passes of 10 seconds in each of runs
[39933326848](https://github.com/Ming-Research/Firn-wf/actions/runs/39933326848)
and
[39935877361](https://github.com/Ming-Research/Firn-wf/actions/runs/39935877361),
12 passes pooled, `summary.py workloads`:

| workload | rate base / head / twin | head/base median | head/base 95% interval | twin/head 95% interval | twin half-width | verdict |
| --- | --- | --- | --- | --- | --- | --- |
| limiter-tx | 612.2k / 611.3k / 608.6k per second | 0.9972 | 0.9824-1.0041 | 0.9915-1.0032 | 0.59% | undecided (wider than twin) |
| setmany-tx | 340.9k / 338.5k / 336.6k per second | 0.9927 | 0.9761-1.0038 | 0.9668-1.0132 | 2.32% | no detected change |

Run B2, the handshake and `CLIENT LIST` against the rivals, 50 connections, 6
passes of 10 seconds in each of runs
[39922503246](https://github.com/Ming-Research/Firn-wf/actions/runs/39922503246)
and
[39927545727](https://github.com/Ming-Research/Firn-wf/actions/runs/39927545727),
12 passes pooled, `summary.py workloads`. Medians over the 12 passes:

| handshake | rate per second | p99 (ms) | client work fraction |
| --- | --- | --- | --- |
| firn (head) | 23.29k | 5.329 | 0.981 |
| firn (head-twin) | 23.30k | 5.316 | 0.984 |
| Redis 7.2.16 | 23.38k | 5.128 | 0.981 |
| Valkey 9.1.2 | 23.25k | 5.274 | 0.978 |
| Dragonfly v2.0.2 | 21.90k | 5.412 | 0.962 |
| Garnet 2.2.3 | 20.90k | 5.216 | 0.966 |

The client work fraction is read from `workload-limited.csv` of each run's
artifact; the artifacts expire, so its range is copied here: over the 12 passes
every handshake line stayed between 0.943 and 0.990, every `CLIENT LIST` line
between 0.021 and 0.156. Both ranges come from all 72 lines of each workload
(six lines, 12 passes) of runs 39922503246 and 39927545727.

| CLIENT LIST | rate per second | firn / rival | p99 (ms) | client work fraction |
| --- | --- | --- | --- | --- |
| firn (head) | 127.1k | | 0.635 | 0.085 |
| firn (head-twin) | 127.5k | 0.997 | 0.677 | 0.085 |
| Redis 7.2.16 | 34.5k | 3.679 | 2.458 | 0.036 |
| Valkey 9.1.2 | 28.2k | 4.508 | 3.015 | 0.034 |
| Dragonfly v2.0.2 | 23.9k | 5.311 | 4.521 | 0.021 |
| Garnet 2.2.3 | 218.5k | 0.582 | 0.625 | 0.152 |

Twin half-widths over 12 passes, offered as the first floor measurements of
these classes on this machine: `redis-benchmark` default suite, 19 cells,
median 1.05%, from 0.38% (lrange_500) to 3.35% (lrange_300); `limiter-tx`
0.59%; `setmany-tx` 2.32%; handshake 1.04%; `CLIENT LIST` 0.54%.

### Verdicts

1. Commands firn supported before: no cell is rejected. Of the 21 cells, 16
   are undecided, 4 show no detected change and 1 (lrange_600) is faster.
   Fourteen of the undecided cells are undecided by the second clause only (the
   head/base interval contains 1 but is wider than the twin's); none of them is
   detected slower, since each interval contains 1. The clause is unmeetable as
   written, because `base` and `head` are two binaries whose build-to-build
   difference adds variance the twin, one binary measured twice, does not have;
   the first dispatch of Run A made hset look wholly slower (0.9835,
   0.9765-0.9970) and the second made it look unchanged (0.9914, 0.9502-1.0840).
   Two cells have a twin interval that excludes 1 (rpush: twin 1.1% faster than
   head, the same binary; zadd: twin 0.7% slower). rpush is the only cell whose
   head/base interval lies wholly under 1 (0.9753-0.9977); the twin as the
   candidate against `base` gives 0.9957 (0.9866-1.0056), no detected change,
   which does not support attributing the rpush difference to the code.
   Exploratory, not part of the rule: the geometric mean over the 19 tests of
   each pass's ratio, with the interval taken over the passes as for a cell, is
   1.0025 (0.9927-1.0123) for head/base in requests per second, 0.9981
   (0.9886-1.0078) for head/base in server CPU per request, and 0.9993
   (0.9949-1.0038) for twin/head in requests per second. Unresolved by the
   rule: these 16 cells.
2. Handshake: unresolved. The cell is client-limited (work fraction above 0.90
   for every server), so only "not behind" is asked and p99 is reported, not
   judged. Firn's median is 0.996 of the fastest rival (Redis), 0.4% short;
   the twin half-width is 1.04%, so the shortfall is smaller than the noise
   and, after 12 passes, the rule leaves it undecided. The paired per-pass
   ratio firn/Redis has median 0.9943 and 95% interval 0.9867-1.0000, not part
   of the rule. Firn is level with Redis and Valkey (1.002 of Valkey) and ahead
   of Dragonfly (1.063) and Garnet (1.114); the three leaders sit at about
   23.3k a second, which suggests the workload client, not the servers, sets
   the ceiling.
3. `CLIENT LIST`: against the fastest rival other than Garnet (Redis) firn is
   3.679 times as fast with p99 0.635 ms against 2.458 ms, so the rule is met.
   Against Garnet the figures are reported and not judged, as the owner ruled
   (card firn-client-merge-perf): firn is 0.582 times as fast, p99 0.635 ms
   against 0.625 ms; the gap is followed by item firn-client-list-garnet.
   Against the earlier run of this section (firn de6d7b5, Garnet 2.2.1) firn's
   `CLIENT LIST` rate is about 4% lower (127k against 133k a second) and the
   multiples over Redis, Valkey and Dragonfly are about 4% lower (3.68, 4.51,
   5.31 against 3.85, 4.70, 5.57); the commit differs by the merge of `main`
   and by `CLIENT LIST` reading the other connections' subscription counts from
   the event hub, which this measurement does not separate.

### Second measurement, on the merged head

Written before the measurement. Everything above measured Firn-wf#74 at
9b514d5 against `main` at 7f8b4f8. The commit that merges is 5a0a1b9
(5a0a1b942aeb2ba2343282a77495b4cb44fc4e89), which has since merged `main`
three more times: the `maxclients` check and count in one atomic statement of
`accept` with the new startup options, and the stream commands, which add a
stream check to command dispatch and copy two more connection fields once per
request read. The handshake workload crosses the changed accept path and every
cell crosses dispatch, so the first measurement does not stand for the merged
head. The three groups are measured again on it; the first results stay above,
marked as made on 9b514d5. `main` is not merged into the branch again unless
the host reports a conflict, so these revisions stay the merged head. The host
reported one after Run B1, in `design/log.md` only, and `main` was merged at
30de5d8; that change was the release packaging (nine files outside `firn/`,
`tests/` and the Makefile), so `firn/`, `tests/`, the Makefile and the pins at
30de5d8 equal those of 5a0a1b9, and the runs after it (B2, C1, C2) name the
same image commits.

Revisions: `head` and `head-twin` are 5a0a1b9; `base` is
05f7e89c62a7b9d10733a43b9a08aaea53fa2090, the `main` that 5a0a1b9 merged, so
base and head differ only by this change (14 `firn/` files, 920 insertions and
82 deletions). The machine, CPUs, depth, connection count and rivals are those
of the first measurement. Each group is one dispatch, so one binary per image
for all its passes (the first measurement pooled two dispatches of six, whose
binaries differed).

- Run A, mode `compare`: the 19 tests of the first measurement and `xadd`,
  which `main`'s `SUITE_DEFAULT` now holds and which `main` supports, so it is
  a command supported before this change; 5 seconds, 18 passes.
- Run B1, mode `workloads` with `workload_images`, firn only: `limiter-tx` and
  `setmany-tx`; 10 seconds, 18 passes.
- Run B2, mode `workloads` with `workload_images`, images `head` and
  `head-twin` and Redis 7.2.16, Valkey, Dragonfly and Garnet (the newest stable
  release of each at the dispatch): `handshake` and `client-list`; 10 seconds,
  12 passes.

The rule is that of the questions above with these changes:

1. Commands supported before (Run A, B1). The condition is the first sentence
   only: no cell has a head/base 95% interval wholly under 1. A cell whose
   head/base interval contains 1 yet is wider than the twin's is reported
   undecided and does not block: base and head are two binaries, whose
   build-to-build difference the twin, one binary measured twice, does not
   carry, so that clause cannot be met and gets no more passes. A cell whose
   twin interval excludes 1 has too much noise in this run to count: it is
   judged on the first 12 passes, and while its twin interval still excludes 1
   the later passes of the run are added one at a time, up to all 18, then it
   is judged by the first sentence; one still excluding 1 at 18 is reported as
   noise that did not settle, with its size. rpush and zadd were such cells in
   the first measurement, which is why Run A and B1 have 18 passes. A cell
   rejected (interval wholly under 1, twin interval containing 1) blocks the
   merge until it is fixed or accepted on an owner's card with its size and
   reason.
2. Handshake against the fastest rival. The cell is client-limited when the
   median client work fraction of firn or of that rival is above 0.90; it asks
   only that firn is not behind: the 95% interval of the per-pass ratio of
   firn to the rival is not wholly under 1. A shortfall smaller than the twin's
   half-width is reported undecided. p99 is judged where the cell is not
   client-limited.
3. `CLIENT LIST` as in the questions above: judged against the fastest rival
   other than Garnet, reported against Garnet.

The twin is still `head` measured again, not `base` (the engineering standards
ask for a twin of the base); this keeps the results comparable with the first
measurement and means the twin does not carry the build-to-build difference.
No noise floor is recorded for these classes on this machine; each class's
largest twin half-width is reported again.

What each result shows: a rejected cell says the change slows that command;
no rejected cell, with the undecided ones listed, meets the owner's "not
slower than before" in the sense of the first sentence; a handshake interval
wholly under 1 with a work fraction above 0.90 says firn is behind Redis on
the handshake; `CLIENT LIST` against Redis, Valkey and Dragonfly is expected
near the first measurement's 3.7, 4.5 and 5.3 times, and a figure clearly
below that would say the merged accept or dispatch path costs it.

Expected time on the 14900K, from the first measurement's runs: about 99
minutes for Run A, 25 for Run B1 and 45 for Run B2.

#### Run A on the merged head

Run [40033786239](https://github.com/Ming-Research/Firn-wf/actions/runs/40033786239),
`summary.py ratios` over its `compare.csv`: `base` is 05f7e89, `head` and
`head-twin` are 5a0a1b9, 18 passes of 5 seconds, depth 1, server CPUs 2 and 4,
client CPUs 16-31. `base` is the image with SHA-256 540d71af8a4f, `head` and
`head-twin` one image with SHA-256 4850a10247e4 (first 12 hexadecimal digits).
No client was limited: the largest mean client busy fraction was 0.892 and no
sample was marked limited. On the first 12 passes the twin interval contains 1
in all 20 cells, so the rule judges every cell on those 12 passes; the last
column gives the interval over all 18 for comparison.

| test | head/base median | head/base 95% interval, 12 passes | twin/head 95% interval | twin half-width | verdict | head/base 95% interval, 18 passes |
| --- | --- | --- | --- | --- | --- | --- |
| get | 0.9945 | 0.9886-1.0046 | 0.9879-1.0069 | 0.95% | no detected change | 0.9886-1.0025 |
| hset | 1.0035 | 0.9967-1.0071 | 0.9931-1.0128 | 0.98% | no detected change | 0.9930-1.0056 |
| incr | 0.9965 | 0.9871-1.0067 | 0.9957-1.0122 | 0.82% | undecided (wider than twin) | 0.9940-1.0101 |
| lpop | 1.0075 | 0.9966-1.0223 | 0.9756-1.0035 | 1.40% | no detected change | 0.9932-1.0147 |
| lpush | 0.9973 | 0.9856-1.0280 | 0.9763-1.0092 | 1.65% | undecided (wider than twin) | 0.9805-1.0153 |
| lrange_100 | 0.9981 | 0.9944-1.0070 | 0.9890-1.0078 | 0.94% | no detected change | 0.9897-1.0064 |
| lrange_300 | 0.9989 | 0.9956-1.0038 | 0.9912-1.0119 | 1.04% | no detected change | 0.9947-1.0026 |
| lrange_500 | 1.0015 | 0.9977-1.0063 | 0.9915-1.0057 | 0.71% | no detected change | 0.9942-1.0030 |
| lrange_600 | 1.0040 | 1.0015-1.0072 | 0.9880-1.0032 | 0.76% | faster | 1.0018-1.0070 |
| mset | 0.9906 | 0.9819-0.9947 | 0.9901-1.0083 | 0.91% | rejected (wholly slower) | 0.9857-0.9958 |
| ping_inline | 0.9988 | 0.9924-1.0105 | 0.9826-1.0094 | 1.34% | no detected change | 0.9940-1.0072 |
| ping_mbulk | 1.0038 | 0.9927-1.0181 | 0.9953-1.0061 | 0.54% | undecided (wider than twin) | 0.9935-1.0115 |
| rpop | 0.9983 | 0.9883-1.0086 | 0.9850-1.0089 | 1.19% | no detected change | 0.9901-1.0071 |
| rpush | 0.9981 | 0.9879-1.0090 | 0.9967-1.0106 | 0.69% | undecided (wider than twin) | 0.9898-1.0042 |
| sadd | 1.0011 | 0.9918-1.0113 | 0.9899-1.0123 | 1.12% | no detected change | 0.9913-1.0055 |
| set | 0.9996 | 0.9849-1.0070 | 0.9810-1.0057 | 1.24% | no detected change | 0.9812-1.0029 |
| spop | 0.9986 | 0.9875-1.0126 | 0.9789-1.0089 | 1.50% | no detected change | 0.9846-1.0103 |
| xadd | 0.9998 | 0.9811-1.0124 | 0.9860-1.0151 | 1.45% | undecided (wider than twin) | 0.9887-1.0102 |
| zadd | 1.0030 | 0.9928-1.0192 | 0.9849-1.0045 | 0.98% | undecided (wider than twin) | 0.9929-1.0124 |
| zpopmin | 1.0053 | 0.9935-1.0313 | 0.9889-1.0125 | 1.18% | undecided (wider than twin) | 0.9930-1.0189 |

Of the 20 cells, 1 is rejected (mset), 1 is faster (lrange_600), 7 are
undecided by the second clause only and 11 show no detected change. No cell has
a twin interval that excludes 1, so no cell needed more than 12 passes, unlike
rpush and zadd in the first measurement.

The rejected cell, mset (ten keys per command): head was slower than base in 17
of the 18 passes (head/base per pass 0.959 to 1.012, the exception being pass
16). Median rate over the 18 passes: base 607.1k, head 602.5k and head-twin
599.9k a second; median server CPU per request: 3.280, 3.307 and 3.295
microseconds. The twin, the same binary as head, sits as far below base as head
does, so the difference is between the two binaries, not within a run. It is
not attributed to a change: the diff of `firn/` between 05f7e89 and 5a0a1b9
touches nothing that only MSET runs; its per-request additions (a flag test
after each request, the NO-TOUCH save and restore around a promoted command, a
larger connection struct and keyspace struct) apply to every cell, and no other
cell is slower. Read from the diff and the table, not tested. If each cell's
interval were exact and the cells independent, the chance that at least one of
20 falls wholly on the slow side with no real difference is about 40%
(1 - 0.975^20, deduced). A difference in code layout between the two binaries
is a possibility not examined.

#### Attribution of the mset cell

Question: does the 5a0a1b9 binary do more work per `MSET` than the 05f7e89
binary, and is that work what made Run A's mset cell slower? Both fall out of
the comparisons below; no new rate verdict is drawn from this dispatch.

Written before the dispatch (in the item's log) and copied here: (1) per-request
user-mode instructions, head over base for mset, with set as the control and
the twin as the noise: more instructions in head than base, and more than the
twin differs, would say the code does more; equal instructions with more
cycles, L1 instruction-cache misses or branch misses would say layout or
front end; no difference in any would say this set of counters cannot tell.
(2) The perf profile of mset against set: whether `update_access`,
`mset_body`, `refresh_access` or the `MSET` write loop got heavier. (3) This
dispatch's mset rate is recorded and neither enters the verdict nor overturns
Run A's rejection.

Run [40057908485](https://github.com/Ming-Research/Firn-wf/actions/runs/40057908485),
`redis-bench.yml` mode `compare` on the 14900K at the head of the branch
(74ca257, which differs from 5a0a1b9 only in measuring tooling): `base` 05f7e89,
`head` and `head-twin` 5a0a1b9, tests `mset` and `set`, depth 1, server CPUs 2
and 4, client CPUs 16-31, 18 passes of 5 seconds for the rates, then a profile
of each image and 6 passes of `perf stat` (user-mode instructions, cycles,
L1-icache-load-misses, branch-misses of the server, divided by the requests of
the pass, the order of the images alternating). The images are those of Run A's
commits built again: `base` 4e20c1e56df5 (Run A: 540d71af8a4f) and `head` and
`head-twin` 212f34302ace (Run A: 4850a10247e4), the same release
`wf-af7665be672d` in both dispatches. Two builds of one commit differ in
bytes; the section "Two builds of each commit" below examines where.

Per request, head/base and twin/base medians over the 6 passes, the range of
the 6 in brackets (`compare-counters.csv`; the twin is the same binary as head):

| test | counter | base median | head/base | twin/base |
| --- | --- | --- | --- | --- |
| mset | instructions | 14,820 | 1.0020 (1.0008-1.0025) | 1.0015 (1.0006-1.0018) |
| mset | cycles | 6,906 | 1.0021 (0.9995-1.0343) | 1.0002 (0.9811-1.0125) |
| mset | L1i misses | 538.6 | 1.0015 (0.9858-1.0236) | 0.9980 (0.9818-1.0044) |
| mset | branch misses | 19.9 | 1.0038 (0.9875-1.0675) | 1.0048 (0.9657-1.0404) |
| set | instructions | 4,703 | 1.0010 (1.0005-1.0017) | 1.0006 (0.9996-1.0028) |
| set | cycles | 2,963 | 1.0059 (0.9916-1.0245) | 1.0048 (0.9645-1.0155) |
| set | L1i misses | 443.6 | 1.0171 (1.0086-1.0258) | 1.0138 (0.9711-1.0299) |
| set | branch misses | 1.5 | 1.0298 (1.0090-1.0498) | 1.0078 (0.9828-1.0745) |

- Instructions: the 5a0a1b9 binary executes about 29 more user-mode
  instructions per `MSET` (ten keys) and about 5 more per `SET` (one key), in
  all 6 passes, and the twin agrees with head to 0.04%. That is about 3
  instructions per key, which is what the added `if client^.no_touch` in
  `update_access` costs (a load, a test and a branch; deduced from the diff,
  the instruction count was not split by function).
- Cycles, L1i misses and branch misses: the medians of head and of twin over
  base lie inside the spread of the 6 passes of either (mset cycles: 1.0021
  and 1.0002 against spreads of 0.98 to 1.03), in both tests; the cycles median
  (1.0021) follows the instruction median (1.0020). These counters cannot tell
  a layout difference at this size.
- Profile (one perf record per image, about 50 G cycles each): the share of
  `wf_commands.update_access` in mset is 0.18% (base), 0.28% (head) and 0.24%
  (twin), in set 0.04%, 0.05% and 0.06%. No other firn symbol of `mset`
  moves more between base and head than between head and twin (the largest,
  `wf_server.serve.resume` +0.155 G cycles head over base, `wf_cmap_hold_take`
  +0.152 G twin over head); `mset_body` is 2.87%, 2.79% and 3.02%.
- Rates of this dispatch (not entering a verdict): mset head/base median 1.0005,
  95% interval 0.9836-1.0174 (summary.py ratios over 18 passes), head slower
  than base in 9 of 18 passes and the twin in 10; median server CPU per request
  base 3.279, head 3.294 and twin 3.289 microseconds. set head/base 1.0025,
  interval 0.9953-1.0098. The mset interval holds both 1 and Run A's 0.9906,
  so this dispatch did not detect a difference and neither confirms nor
  contradicts Run A.

Reading: about 0.2% of Run A's 0.9% mset deficit is the added NO-TOUCH test
(instruction count +0.2% and cycles following it; the rate effect itself is
deduced, not separated). The rest (0.7%) is not attributed: no counter or
symbol separates the two binaries beyond what the twin shows, and a second pair
of builds of the same two commits gave a ratio of 1.0005 with an interval that
holds both 1 and 0.9906. That the two rate results differ because the builds
differ in code layout is not supported: the next section shows that two builds
of one commit have the same code.

#### Two builds of each commit

Question: do two builds of one commit differ in code, so that a rate
difference between dispatches could come from the build? The comparison was
asked for in the item's log before it was made: how many bytes differ, in which
section, and whether `.text` changed. The answer is "yes" if the `.text` size,
a symbol address or an opcode differs, and "no" if the section headers and the
symbol table are equal and the instruction streams differ only in operands that
name data.

Run [40060308360](https://github.com/Ming-Research/Firn-wf/actions/runs/40060308360),
`redis-bench.yml` mode `build` on a GitHub-hosted runner at c125af8, `base` and
`base2` both 05f7e89, `head` and `head2` both 5a0a1b9, each name built in a
worktree of its own with the release `wf-af7665be672d`; the images were
compared with `cmp -l`, a small ELF reader over the section headers and
`objdump -d`. The builds were on a hosted runner, not the 14900K, whose builds
were not kept, so this says nothing about differences that only that machine's
builds would show.

| pair | bytes | bytes differing | `.rodata` | `.text` | build id | `.rela.dyn` |
| --- | --- | --- | --- | --- | --- | --- |
| base, base2 | 1,985,984 | 903 | 857 | 24 | 20 | 2 |
| head, head2 | 2,023,440 | 4,007 | 3,583 | 379 | 20 | 25 |

For both pairs the section headers and the symbol table are equal, so every
function has the same address and size; the disassembly has the same number of
lines, the same address on each, and the same mnemonic on each. The only
instructions whose text differs are `leaq` with an address relative to `%rip`:
22 in the base pair and 324 in the head pair, each loading the address of a
string in `.rodata` that is a few tens of bytes elsewhere in the other build.
Those strings are listed in another order in `.rodata`; one of them is the
compiler's temporary directory, named with its process number
(`/tmp/whitefootc-10706-0/concurrent_map.c` in one head build,
`/tmp/whitefootc-12836-0/concurrent_map.c` in the other). The `.text` bytes
that differ are the displacements of those `leaq` instructions.

Reading: two builds of one commit have the same code, at the same addresses, so
a rate difference between builds of one commit does not come from code layout;
the only difference is where read-only strings sit, and whether any of them is
read on the MSET path was not checked. `base` and `head` are different sources
and differ in code layout (`.text` 1,712,029 and 1,747,101 bytes); this
comparison cannot say whether that layout accounts for any part of Run A's mset
cell.

#### Run B1 on the merged head

Run [40060408858](https://github.com/Ming-Research/Firn-wf/actions/runs/40060408858),
mode `workloads` with `workload_images`, firn only: `base` 05f7e89, `head` and
`head-twin` 5a0a1b9, tests `limiter-tx` and `setmany-tx`, 50 connections,
server CPUs 2 and 4, client CPUs 16-31, 18 passes of 10 seconds, at c125af8,
`summary.py workloads`. No sample was marked client-limited: the median client
work fraction was 0.26 in `limiter-tx` and 0.16 in `setmany-tx`, with the
largest 0.267, for every line.

| workload | rate base / head / twin | head/base median | head/base 95% interval | twin/head 95% interval | twin half-width | verdict |
| --- | --- | --- | --- | --- | --- | --- |
| limiter-tx | 609.9k / 606.5k / 611.1k per second | 0.9927 | 0.9882-1.0121 | 0.9961-1.0143 | 0.91% | no detected change |
| setmany-tx | 341.9k / 337.2k / 336.4k per second | 0.9877 | 0.9746-0.9923 | 0.9828-1.0152 | 1.62% | rejected (wholly slower) |

`setmany-tx` (`MULTI`, one `MSET` of three keys, three `EXPIRE`, `EXEC`): head
was slower than base in 16 of the 18 passes (head/base per pass 0.943 to
1.004), the twin in 13; median p99 0.211, 0.222 and 0.223 ms for base, head and
twin. The twin, the same binary as head, sits as far below base as head does, as
in Run A's mset cell. `limiter-tx` (`MULTI`, `INCRBY`, `PTTL`, `EXEC`) has the
same `EXEC` change and shows no detected change (head slower in 11 passes, the
twin in 7). A fixed cost per `EXEC` would weigh more on `limiter-tx`, whose
transaction is shorter (4 commands at 609.9k a second against 6 at 341.9k), so
the `EXEC` change alone does not explain `setmany-tx` (deduced; `limiter-tx`'s
interval does not exclude a 1% slowdown). Per transaction `limiter-tx` looks up
two keys and `setmany-tx` six. The first measurement's `setmany-tx` at 9b514d5
was 0.9927 (0.9761-1.0038, 12 pooled passes of two dispatches), no detected
change.

Taken with Run A, two cells are rejected, mset and `setmany-tx`, and both run
`MSET`. Among the 22 cells of Run A and B1 the chance that at least two fall
wholly on the slow side with no real difference is about 10% (deduced from
`1 - 0.975^22 - 22 * 0.025 * 0.975^21`, each cell independent and its interval
exact). The size of each rejected cell and the cause, 0.2% of which is the
NO-TOUCH test, go to the owner on card firn-q-mset-regression.

#### Run B2 on the merged head

Run [40072228392](https://github.com/Ming-Research/Firn-wf/actions/runs/40072228392),
mode `workloads` with `workload_images`, at ee2dcf4: `head` and `head-twin`
5a0a1b9, Redis 7.2.16, Valkey 9.1.2, Dragonfly v2.0.2 and Garnet 2.2.3 (the
newest stable release of each when the run was made), tests `handshake` and
`client-list`, 50 connections, depth 1, server CPUs 2 and 4, client CPUs 16-31,
12 passes of 10 seconds, `summary.py workloads` (its per-pass interval against
the fastest rival is new in this change, so the table's intervals were first
computed with a short script that gave the same figures). Medians over the 12
passes:

| handshake | rate per second | p99 (ms) | client work fraction |
| --- | --- | --- | --- |
| firn (head) | 23.26k | 5.311 | 0.985 |
| firn (head-twin) | 23.31k | 5.335 | 0.984 |
| Redis 7.2.16 | 23.42k | 5.260 | 0.979 |
| Valkey 9.1.2 | 23.25k | 5.278 | 0.979 |
| Dragonfly v2.0.2 | 21.97k | 5.361 | 0.962 |
| Garnet 2.2.3 | 20.87k | 5.258 | 0.971 |

| CLIENT LIST | rate per second | firn / rival | p99 (ms) | client work fraction |
| --- | --- | --- | --- | --- |
| firn (head) | 127.6k | | 0.704 | 0.085 |
| firn (head-twin) | 127.5k | 1.001 | 0.732 | 0.085 |
| Redis 7.2.16 | 34.6k | 3.693 | 2.455 | 0.036 |
| Valkey 9.1.2 | 28.3k | 4.522 | 3.008 | 0.034 |
| Dragonfly v2.0.2 | 23.9k | 5.339 | 4.556 | 0.021 |
| Garnet 2.2.3 | 219.8k | 0.579 | 0.621 | 0.152 |

Rule 2, the handshake against the fastest rival, which is Redis 7.2.16. The
cell is client-limited, the median work fraction being 0.985 for firn and 0.979
for Redis, both above 0.90 (every line stayed between 0.951 and 0.992 over the
passes), so it asks only that firn is not behind. The head/Redis median is
0.9945 (firn slower in 9 of the 12 passes) and its 95% interval 0.9849-1.0048
contains 1, so the interval is not wholly under 1 and the condition holds. The
shortfall of 0.55% is smaller than the twin's half-width of 1.45% (twin/head
0.9837-1.0125), so it is reported undecided. Against Valkey the ratio is 1.0010
(0.9996-1.0054), against Dragonfly 1.0608 (1.0503-1.0711) and against Garnet
1.1143 (1.1040-1.1301). The p99 is not judged in a client-limited cell: 5.311
ms for firn against 5.260 ms for Redis.

Rule 3, `CLIENT LIST`. Against the fastest rival other than Garnet, Redis, firn
is 3.693 times faster (95% interval 3.669-3.720), with a p99 of 0.704 ms
against 2.455 ms; against Valkey 4.522 (4.484-4.548) with 3.008 ms, against
Dragonfly 5.339 (5.304-5.377) with 4.556 ms. These are the first measurement's
3.7, 4.5 and 5.3, so the merged accept and dispatch paths do not cost
`CLIENT LIST`. Against Garnet firn is 0.579 times as fast (0.573-0.588), its
p99 0.704 ms against 0.621 ms, reported and not judged, as owner card
firn-client-merge-perf ruled; the gap is the work of firn-client-list-garnet.

Twin half-widths of this run: handshake 1.45% (1.04% in the first measurement),
`CLIENT LIST` 0.54% (the same). `summary.py` prints "3 criteria not met": the
`CLIENT LIST` rate and p99 against Garnet, and the handshake median 0.993
(its own median-only check, which rule 2 replaces by the interval above).

#### Runs C1 and C2: is the shortfall stable, and is the NO-TOUCH test its cause?

Written before the runs. Run A rejected mset and Run B1 rejected `setmany-tx`,
each in a dispatch of its own; the attribution dispatch did not detect mset
slower. Two questions: does each shortfall show again in a dispatch built for
it, and if it does, does removing the NO-TOUCH test from `update_access` give
it back? The instruction counts say the test costs about 0.2% of an MSET
(29 more instructions of 14,820); a rate effect larger than that would mean the
test costs more than its instructions.

Images, built in one dispatch each: `base` 05f7e89, `head` and `head-twin`
5a0a1b9, and `variant`, commit 0f45bbf on branch
`claude/client-handshake-notouch-variant`, which is 5a0a1b9 with the three
lines of `update_access` in `firn/commands/access.wf` that return early under
CLIENT NO-TOUCH removed. The variant is for measuring only; it answers wrongly
for a connection in NO-TOUCH mode, which no measured workload uses, and its
branch was deleted after the runs.

- Run C1, `redis-bench.yml` mode `compare` on the 14900K: test `mset`, depth 1,
  server CPUs 2 and 4, client CPUs 16-31, 36 passes of 5 seconds, profile off;
  `summary.py ratios` for `base`/`head` with the twin, `head`/`variant` and
  `base`/`variant`. About 20 minutes.
- Run C2, mode `workloads` with `workload_images`, firn only: tests
  `setmany-tx` and `limiter-tx`, 50 connections, the same CPUs, 36 passes of 10
  seconds; `summary.py workloads` for the same three pairs. `limiter-tx`, a
  transaction of `INCRBY` and `PTTL` that Run B1 did not reject, is the control:
  reported and not read. About 55 minutes.

Per cell of `mset` and `setmany-tx`, with H the 95% interval of head/base, V of
variant/head, W of variant/base and T of twin/head (the mean log ratio over the
passes, Student's t, as in the rules above), read once:

0. T excludes 1: this run's noise did not settle for the cell; it is not read.
1. H contains 1: the shortfall is not detected in this dispatch; it is at most
   1 minus the lower end of H in this dispatch (not shown absent). V and W are
   reported.
2. H is wholly under 1, V wholly above 1 and W contains 1: the NO-TOUCH test
   accounts for the shortfall, of the size of V.
3. H is wholly under 1, V wholly above 1 and W wholly under 1: the test accounts
   for the part of the size of V, and the shortfall left is the interval W.
4. H is wholly under 1 and V wholly under 1: the variant is slower than head
   though it does less work, so a change of three instructions moves the rate by
   about the size of V and the shortfall is not the test; code placement is
   the candidate and is not tested.
5. H is wholly under 1 and V contains 1: the test cannot be told from nothing;
   it accounts for at most the upper end of V minus 1, and the cause is
   undecided.

What would reject the proposal that the test causes the shortfall: readings 4
and 5 with H wholly under 1, or reading 1. No further passes are added whatever
the result. The half-widths to expect, from the 18-pass half-widths of Run A
and B1 scaled by the square root of 18/36 (deduced): H about 0.35% for mset and
0.62% for `setmany-tx`, and T of `setmany-tx` about 1.15% (T was 1.62% in B1).
So unless the shortfall or the test's share is about that size, reading 5 is
the likely one for V, and it is reported as such.

##### Run C1 on `mset`

Run [40089657162](https://github.com/Ming-Research/Firn-wf/actions/runs/40089657162),
mode `compare` on the 14900K, run at 30de5d8 (the dispatching checkout, which
differs from 5a0a1b9 only by `main`'s release-packaging files and the record),
images `base` 05f7e89, `head` and `head-twin` 5a0a1b9, `variant` 0f45bbf; test
`mset`, depth 1, server CPUs 2 and 4, client CPUs 16-31, 36 passes of 5
seconds, profile off, `summary.py ratios` once per pair. No sample was
client-limited (client busy fraction at most 0.33).

| image | rate per second (median) | p99 (ms, median) | slower than `base` in |
| --- | --- | --- | --- |
| base | 606.7k | 0.099 | |
| head | 604.1k | 0.103 | 26 of 36 passes |
| head-twin | 604.9k | 0.103 | 19 of 36 passes |
| variant | 605.3k | 0.099 | 21 of 36 passes |

| pair | median | 95% interval | verdict |
| --- | --- | --- | --- |
| H, head/base | 0.9950 | 0.9902-0.9978 | wholly under 1 |
| T, twin/head | 1.0018 | 0.9939-1.0057 | contains 1 |
| V, variant/head | 1.0044 | 0.9970-1.0069 | contains 1 |
| W, variant/base | 0.9967 | 0.9911-1.0008 | contains 1 |

Reading. T contains 1 (reading 0 does not apply), H does not, V contains 1:
reading 5. The `mset` shortfall shows again in a dispatch built for it, 0.50%
at the median and between 0.22% and 0.98% by the interval, which overlaps
Run A's 0.9906 (0.9819-0.9947 on 12 passes). Removing the NO-TOUCH test moves
the rate by +0.44% at the median, with an interval from -0.30% to +0.69%: the
data cannot tell the test from nothing, and they cannot exclude that it is the
whole shortfall (the upper end of V, 0.69%, is above the shortfall of 0.50%).
By the criterion written before the run, reading 5 with H wholly under 1 is one
of the two results that reject the proposal that the test causes the
shortfall; what the data support is only that removing the test changes the
rate by at most 0.69 percentage points, more than the whole shortfall of
0.50%, so the cause is undecided. The half-widths were as predicted for H
(0.38% against about 0.35%) and wider for T (0.59%). The instruction counts
of the attribution run (29 more of 14,820 per `MSET`, 0.2%) lie inside V.

##### Run C2 on `setmany-tx`

Run [40092094599](https://github.com/Ming-Research/Firn-wf/actions/runs/40092094599),
mode `workloads` with `workload_images`, firn only, run at 44129dc (the
dispatching checkout; the images are the same four commits as in C1): tests
`setmany-tx` and `limiter-tx`, 50 connections, server CPUs 2 and 4, client CPUs
16-31, 36 passes of 10 seconds, `summary.py workloads` once per pair. No sample
was client-limited (median client work fraction 0.16 in `setmany-tx`, 0.26 in
`limiter-tx`, at most 0.27).

| image | `setmany-tx` rate per second | p99 (ms) | slower than `base` in | `limiter-tx` rate per second | p99 (ms) | slower than `base` in |
| --- | --- | --- | --- | --- | --- | --- |
| base | 339.9k | 0.222 | | 612.0k | 0.099 | |
| head | 337.8k | 0.222 | 25 of 36 passes | 607.4k | 0.104 | 22 of 36 passes |
| head-twin | 336.2k | 0.223 | 22 of 36 passes | 606.5k | 0.108 | 23 of 36 passes |
| variant | 336.1k | 0.224 | 24 of 36 passes | 607.8k | 0.107 | 24 of 36 passes |

| pair | `setmany-tx` median | 95% interval | `limiter-tx` median | 95% interval |
| --- | --- | --- | --- | --- |
| H, head/base | 0.9949 | 0.9807-1.0075, contains 1 | 0.9939 | 0.9809-1.0029, contains 1 |
| T, twin/head | 0.9991 | 0.9891-1.0158, contains 1 | 0.9984 | 0.9904-1.0096, contains 1 |
| V, variant/head | 1.0008 | 0.9890-1.0116, contains 1 | 1.0006 | 0.9811-1.0068, contains 1 |
| W, variant/base | 0.9930 | 0.9838-1.0049, contains 1 | 0.9910 | 0.9711-1.0007, contains 1 |

Reading of `setmany-tx`. T contains 1, so the cell counts (reading 0 does not
apply). H contains 1: reading 1. The shortfall of Run B1 (0.9877,
0.9746-0.9923) is not detected in this dispatch; it is at most 1.93% (1 minus
the lower end of H), and the median, 0.51% slower, is below the cell's noise.
This does not show it absent. V and W contain 1 as well and are reported
only. The half-widths were larger than predicted before the run: H 1.34%
against about 0.62%, T 1.34% against about 1.15%; this dispatch's `setmany-tx`
is noisier than a scaling of B1's 18 passes gives.

`limiter-tx`, the control (a transaction of `INCRBY` and `PTTL`, not read):
H 0.9939 (0.9809-1.0029), as in B1 (0.9927, 0.9882-1.0121). It does not detect
a change either, and its median is also 0.6% below 1.

Taken with C1, `mset` has a shortfall of about 0.5% that shows again in a
dispatch built for it, whose cause the variant does not isolate; `setmany-tx`
has one rejection (B1) that the dispatch built for it does not reproduce.
