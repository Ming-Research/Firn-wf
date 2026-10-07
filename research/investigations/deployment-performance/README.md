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

## Scripts, transactions and expiring keys

Not measured yet.
