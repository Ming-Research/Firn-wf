# Performance baseline against Redis, Valkey, Dragonfly and Garnet

## The question, stated before measuring

On the owner's i9-14900K, with firn and every competitor given the same
server CPUs, the same client and the same durability, does firn lead the
fastest competitor in every cell, and by how much? Board item
`firn-perf-rebaseline` fixes the scope; the owner's flagship aim is that firn
lead its competitors widely, about 1.7 times the fastest, without losing any
lead an earlier baseline showed. This record fixes the rules before any
measurement. Nothing below has been measured yet.

This is the first baseline on the native host (reinstalled 2026-10-10) with
the owner's placement (servers on performance cores, clients on efficiency
cores). Earlier numbers, from the Hyper-V guest or from other placements
([deployment performance](../deployment-performance/README.md)), are not
compared with it. It becomes the reference later runs are judged against.

## Lines

| line | server | how it runs on n server CPUs |
|---|---|---|
| `reference` | Redis 7.2.16 | single-threaded, default settings |
| `reference-io` | Redis 7.2.16 | `io-threads n`, `io-threads-do-reads yes` (n ≥ 2 only) |
| `valkey` | Valkey | single-threaded, default settings |
| `valkey-io` | Valkey | `io-threads n` (n ≥ 2 only) |
| `dragonfly` | Dragonfly | `--proactor_threads=n` |
| `garnet` | Garnet | default thread pool, sized from the CPU affinity (below) |
| `firn` | firn, `make firn-lto` | `WF_DRIVERS=n` |
| `firn-twin` | the same firn executable | `WF_DRIVERS=n`; noise control, never a competitor |

n counts logical CPUs: 1, 2, 4, 8, and 16 for the cell of 8 cores with both
hyperthreads. Every line runs under `taskset -c <server list>`, so a
single-threaded server is offered the same CPUs as the threaded ones.

### Rule 1: single-threaded servers and the fastest competitor

- Redis and Valkey run twice in every cell with n ≥ 2: as shipped
  (single-threaded) and with I/O threads equal to n. Both projects document
  `io-threads 1` as "just use the main thread", so n counts the main
  thread and the threaded line uses exactly the n CPUs it is given, as
  firn's drivers and Dragonfly's proactors do
  ([Redis 7.2.16 redis.conf](https://raw.githubusercontent.com/redis/redis/7.2.16/redis.conf),
  [Valkey 9.1.2 valkey.conf](https://raw.githubusercontent.com/valkey-io/valkey/9.1.2/valkey.conf)).
  At n = 1 the threaded lines are not run: they equal the plain ones.
- In Valkey 9.1.2 `io-threads-do-reads` "is deprecated and has no effect";
  Redis 7.2.16 needs it set to `yes` to thread reads and parsing.
- Reason: firn is compared against each competitor's best configuration
  on the same CPUs, not its default. Other I/O-thread counts (the
  configuration files suggest leaving a core spare) are not searched; a cell
  where that choice could decide the verdict is labelled so.
- The **fastest competitor** of a cell is the competitor line, of all
  configurations above, with the highest median throughput over the passes,
  among the lines that passed verification in that cell and, in an AOF cell,
  have equal persistence (rule 2). firn's p99 is compared with that same
  line's p99; the lowest competitor p99 is reported beside it.

## Machine and placement

- The i9-14900K through CI only (`redis-bench.yml`, runner `14900k`),
  nothing else running; the host-wide lock held for the whole run.
- Performance cores are CPUs 0-15, core k's hyperthreads being 2k and 2k+1;
  efficiency cores are CPUs 16-31.
- Server lists (`server_cpus`): 1 → `2`; 2 → `2,4`; 4 → `2,4,6,8`;
  8 → `0,2,4,6,8,10,12,14`; 8 cores with 16 threads → `0-15`. The smaller
  cells leave core 0 to the OS; the two largest need every performance core.
- Client list (`client_cpus`): always `16-31`, 16 client threads or
  processes, so the client is identical in every cell.
- A cell is **client-limited** when the client measure exceeds 0.90: for
  redis-benchmark cells the mean OS busy fraction over the client CPUs, for
  workload cells the workload client's own `client_work_fraction`
  ([deployment performance, CPU load](../deployment-performance/README.md#scripts-transactions-and-expiring-keys)).
  A client-limited cell cannot show a lead: it only requires firn not to
  fall behind (rule 6).

## Workloads and settings

**redis-benchmark cells.** Redis 7.0's 19 default tests (`SUITE_DEFAULT`:
ping_inline, ping_mbulk, set, get, incr, lpush, rpush, lpop, rpop, sadd,
hset, spop, zadd, zpopmin, lrange_100, lrange_300, lrange_500, lrange_600,
mset) at pipeline depths 1 and 16, 50 connections, 100,000 random keys,
3-byte values, the redis-benchmark that `tests/redis-suite/reference.sh`
builds from Redis 7.2.16. Redis 7.2.16's default suite adds XADD; it joins
when firn's minimal streams land (`firn-scope-min-streams`). No
persistence on any line: Redis and Valkey `--save "" --appendonly no`,
Dragonfly `--dbfilename=` (no snapshot), Garnet without `--aof` (its
default), firn without an append-only file.

**Application cells.** The workload client of
[`research/experiments/redis-bench/workload/`](../../experiments/redis-bench/workload/),
50 connections at depth 1, append-only file on with fsync once a second:
- Django cache and sessions: `setmany-tx` (`MULTI`, `MSET` of three keys,
  `EXPIRE` each, `EXEC`);
- connect-redis sessions: `session-set` and `session-get` (200-byte values,
  one-day expiry);
- rate-limiter-flexible: `limiter-script` (`EVALSHA` of its consume script).

**Metrics.** Throughput (requests or transactions a second, over the
client's wall time); p99 latency (the client's, merged over all its
processes or threads); memory: the server's resident set (`VmRSS`, summed
over its processes) after `session-get`'s fill of 1,000,000 sessions, and
`VmHWM` at the end of each redis-benchmark line, recorded.

**Order.** Every pass starts each line in turn on a fresh server and runs
all of that cell's tests; the line order reverses on even passes, so firn
and its twin, and every competitor, see the same drift.

### Rule 2: persistence parity in the AOF cells

| competitor | setting in AOF cells | source |
|---|---|---|
| Redis 7.2.16 | `--appendonly yes --appendfsync everysec`, automatic rewrite off (`auto-aof-rewrite-percentage 0`) | [redis.conf](https://raw.githubusercontent.com/redis/redis/7.2.16/redis.conf) |
| Valkey (both lines) | the same as Redis | [valkey.conf](https://raw.githubusercontent.com/valkey-io/valkey/9.1.2/valkey.conf): `appendfsync everysec` is the default |
| Dragonfly | **excluded**: no AOF lines | ["Currently, Dragonfly does not support AOF."](https://www.dragonflydb.io/docs/managing-dragonfly/aof) It persists only by snapshots (`--dbfilename`, `--snapshot_cron`), [backups](https://www.dragonflydb.io/docs/managing-dragonfly/backups) |
| Garnet | `--aof --aof-commit-freq 1000`, `--aof-commit-wait` off; **provisional** (below) | [configuration](https://microsoft.github.io/garnet/docs/getting-started/configuration), [defaults.conf](https://raw.githubusercontent.com/microsoft/garnet/main/libs/host/defaults.conf) |
| firn | `firn.aof`, its only policy being fsync every second | [deployment performance](../deployment-performance/README.md#scripts-transactions-and-expiring-keys) |

- Dragonfly has no log a write enters before its reply: a snapshot loses
  every write since the last one. Measuring it without persistence beside
  firn with its append-only file would compare unequal work, so Dragonfly
  is not run in AOF cells and the fastest competitor there is chosen
  without it. Its no-persistence results stay in the redis-benchmark cells.
- Garnet's AOF commit frequency is in milliseconds: `0` commits after every
  operation (its default), `-1` leaves commits to `COMMITAOF`;
  `--aof-commit-wait` holds each reply until the commit is flushed, and is
  off as `everysec` holds no reply. Garnet's documentation names no fsync
  option, and whether a commit reaches the disk with `fsync` or `fdatasync`
  is **unverified**. Before the full run, Garnet's source at the pinned
  release is read and its commit path cited here; until then, and if a
  commit does not force the file to disk, Garnet's AOF results are reported
  as "durability not equal" and excluded from the fastest competitor, as
  Dragonfly's are.
- Every AOF line is checked after it starts: Redis, Valkey and firn report
  `aof_enabled:1` in `INFO persistence`; Garnet's equivalent check (its
  log file growing in its directory) is written with the instrument change.

### Rule 3: threads of Dragonfly and Garnet

- **Dragonfly:** `--proactor_threads=n`, one per logical server CPU. Its
  default, 0, takes "as many as there are CPU cores"
  ([flags](https://www.dragonflydb.io/docs/managing-dragonfly/flags)), so
  the count is given explicitly. `--proactor_affinity_mode` defaults to
  `on`; whether it pins threads only within the inherited `taskset` mask is
  **unverified**. The probe reads `Cpus_allowed_list` of every Dragonfly
  thread under `/proc/<pid>/task/`; if any thread may run outside the
  server list, the baseline runs Dragonfly with `--proactor_affinity_mode=off`.
- **Garnet:** no thread option. Its thread-pool minimum and maximum
  (`--minthreads`, `--maxthreads`, `--miniothreads`, `--maxiothreads`)
  default to 0, "the system default"
  ([configuration](https://microsoft.github.io/garnet/docs/getting-started/configuration)),
  which .NET sizes from `Environment.ProcessorCount`; on Linux that is the
  number of CPUs the process is affinitised to
  ([ProcessorCount](https://learn.microsoft.com/en-us/dotnet/api/system.environment.processorcount)),
  so `taskset` gives Garnet n. Garnet needs `--lua` for scripts (`EnableLua`
  is false by default).
- A competitor that fails a workload's verification (for example
  Dragonfly's rule that scripts touch only declared keys, **unverified** for
  rate-limiter-flexible's script) is reported as failed in that cell, never
  silently left out, and the cell's fastest competitor is chosen among the
  rest.

### Rule 4: versions

- Each competitor is its latest formal release, read from its releases page
  on the day the install workflow runs and pinned there by archive SHA-256.
  On 2026-10-13 these were Valkey 9.1.2
  ([releases](https://github.com/valkey-io/valkey/releases); 9.2.0-rc1 is a
  release candidate), Dragonfly v2.0.2
  ([releases](https://github.com/dragonflydb/dragonfly/releases)) and Garnet
  v2.2.1 ([releases](https://github.com/microsoft/garnet/releases)). Redis
  is the reference 7.2.16, built from the archive `reference.sh` pins.
- `host.txt` records each server's `--version`, the redis-benchmark
  version, the firn revision and `whitefoot.pin`, the placement lines and
  the settings. The full baseline fails unless every version line names
  the pinned release; the probe only records them.
- Dragonfly and Garnet are not installed on the 14900K; their install
  workflow waits on an owner card. The probe may run without them (each
  absent line printed as `skip,<line>,no executable`). The full baseline
  may not: a competitor set short of all four leaves the comparison
  unverified, and no verdict is drawn from it.

### Rule 5: run sizing

The probe: server CPUs 2 and 8, tests `set`, `get` and `mset`, depths 1
and 16, 2 passes of 5 seconds, every installed line and the twin. From it:

- the **twin spread** of a cell is the largest difference, over the passes,
  between firn and firn-twin, as a fraction of firn's rate; and the **line
  spread** is each line's range over its mean across the passes.
- If every probe cell has twin spread ≤ 3% and every line spread ≤ 5%: the
  full run uses 3 passes of 5 seconds.
- If the largest twin spread is above 3% but ≤ 6%, or a line spread above
  5% but ≤ 10%: 5 passes of 10 seconds.
- Above that: no full run; the noise is investigated first (load on the
  host, frequency, placement), and this rule is revisited with what it
  found.
- The full run is dispatched one server-CPU count at a time, so that each
  dispatch holds the machine for a bounded slot (at 3 passes of 5 seconds,
  19 tests × 2 depths × 8 lines × 15 s ≈ 76 minutes before overhead), and
  the probe's own duration is used to check that estimate before the first
  dispatch.

### Rule 6: verdicts and what rejects "firn leads"

Per cell (server CPUs, test or workload, depth), with ratio r = firn's
median throughput over the fastest competitor's, and s the cell's twin
spread:

- **leads** when r > 1 + s and firn's rate exceeds that competitor's in
  every pass; **leads widely** when, in addition, r ≥ 1.7;
- **level** when 1 − s ≤ r ≤ 1 + s, or the passes disagree;
- **behind** when r < 1 − s.
- **p99:** firn's median p99 above the same competitor's by more than the
  p99 twin spread (the same definition on p99) is a p99 loss, reported
  whatever the throughput verdict.
- **Client-limited cells** (client measure above 0.90 in any pass of firn
  or the fastest competitor) can only be level or behind: they pass if not
  behind.
- **Memory:** firn's resident set above the leanest competitor's by more
  than the twin's difference is a memory loss, reported per CPU count.

The claim "firn leads every competitor" is rejected by any cell that is
behind or has a p99 loss, and is unproven in any cell that is level and not
client-limited. The report lists every cell's verdict and r, and counts the
cells that lead widely against the owner's 1.7 aim.

**Later runs.** A later run regresses a lead when a cell that leads here
no longer leads, or its r falls below this baseline's by more than the sum
of the two runs' twin spreads in that cell.

## Gaps in the instrument against these rules

`research/experiments/redis-bench/redis-bench.sh` and `redis-bench.yml` at
Firn-wf f12587f do not yet run this baseline:

1. No mode measures competitors under the comparison's conditions.
   `compare` mode has the twin, the reversed order, p99 and the
   client-limited record, but measures only firn images and Redis.
   `scale` mode measures `reference valkey-io dragonfly-n garnet-n firn-n
   firn-base-n` in a fixed order every pass, with no plain `valkey`, no Redis
   I/O-thread line, a twin only when `FIRN_BASELINE` names the same
   executable, ten tests (`SCALE_TESTS`) by default, no client-busy record,
   and no p99 for tests its multi-process client runs.
2. `compare` mode's multi-process client opens 3 connections per client CPU
   (48 with 16 client CPUs, not 50) and keeps only the first process's p50
   and p99.
3. `workloads` mode runs AOF only for `reference-aof` and `firn-aof-n`;
   lines selected by `WORKLOAD_SERVERS` always run without AOF, and there
   is no AOF start for Valkey or Garnet.
4. `garnet-*` starts with `--port` and `--bind` only: no `--lua`, so
   `limiter-script` cannot run on it, and no AOF options.
5. `dragonfly-*` leaves `--proactor_affinity_mode` at its default; whether
   its threads stay inside the server list is unchecked.
6. `valkey-io` passes `--io-threads-do-reads yes`, a no-op in Valkey 9.1.2,
   and at one CPU equals `valkey`; there is no `reference-io` line.
7. `host.txt` records competitor versions without checking them ("none of
   them stops the run"), and no competitor archive hash.
8. The AOF start check (`INFO persistence`, `CONFIG SET
   auto-aof-rewrite-percentage`) is Redis-specific and runs only in
   `compare` mode; `workloads` mode checks the Redis version only for its
   memory and connection workloads, not for the application workloads.
9. Memory is recorded only after `session-get`'s fill; redis-benchmark
   lines record none.
