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

**redis-benchmark cells.** Redis 7.2.16's 20 default tests (`SUITE_DEFAULT`:
ping_inline, ping_mbulk, set, get, incr, lpush, rpush, lpop, rpop, sadd,
hset, spop, zadd, zpopmin, lrange_100, lrange_300, lrange_500, lrange_600,
mset, xadd) at pipeline depths 1 and 16, 50 connections, 100,000 random
keys, 3-byte values, the redis-benchmark that `tests/redis-suite/reference.sh`
builds from Redis 7.2.16. The lrange tests read the first 100, 300, 450 and
600 elements, as redis-benchmark's own tests do; a measurement of lrange_500
that read 500 elements is not comparable with one that reads 450. No
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
client's wall time); p99 latency, exact over all the client's requests:
the multi-process redis-benchmark client that throughput uses can only bound
the merged p99 (the largest of its processes' p99), and a bound cannot decide
a p99 verdict, so every redis-benchmark cell also runs one pass with the
single-process threaded client, whose histogram is merged, and the p99
verdict uses that pass; the workload client merges its own histograms;
memory: the server's resident set (`VmRSS`, summed
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

- The baseline fixes each competitor's formal release and archive SHA-256
  in the install workflow and instrument; repeated installs use those pins,
  and adopting a newer release requires updating both. On 2026-10-13 the
  selected releases were Valkey 9.1.2
  ([releases](https://github.com/valkey-io/valkey/releases); 9.2.0-rc1 is a
  release candidate), Dragonfly v2.0.2
  ([releases](https://github.com/dragonflydb/dragonfly/releases)) and Garnet
  v2.2.1 ([releases](https://github.com/microsoft/garnet/releases)). Redis
  is the reference 7.2.16, built from the archive `reference.sh` pins.
- `host.txt` records each server's `--version`, the redis-benchmark
  version, the firn revision and `whitefoot.pin`, the placement lines and
  the settings. The full baseline fails unless every version line names
  the pinned release, possibly followed by the pinned tag's commit as
  `-` or `+` and at least 7 hex digits of it; the probe only records them.
  A checked source-archive
  hash alone does not establish which executable PATH selects: the instrument
  also requires the selected Redis server's real path to match the server
  in the build directory returned by `reference.sh bin` before recording
  verified reference provenance. For Valkey, Dragonfly and Garnet, the
  installer writes a receipt with version, archive URL and SHA-256,
  executable real path and SHA-256. The instrument checks the receipt's
  version/URL/archive hash against its own fixed pins and checks the selected
  executable against that receipt before recording verified provenance.
  A Garnet .NET wrapper also binds its payload and runtime host.
  [Installation evidence and remaining pin checks](../deployment-performance/README.md#compare-baseline-instrument)
  state how the pins were obtained and what remains unverified.
- Installation on the 14900K is performed only by the dispatch-only install
  workflow; its current receipts and executable checks establish availability,
  not an assumed machine state. The probe may run without Dragonfly or Garnet (each
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
  20 tests × 2 depths × 8 lines × 15 s ≈ 80 minutes before overhead), and
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
  whatever the throughput verdict. Only exact merged p99s decide it (see
  Metrics); an upper bound is reported but decides nothing.
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

At Firn-wf f12587f the instrument did not run this baseline. Each of the
nine gaps found then is listed below with what closes it now, in
`research/experiments/redis-bench/` (the instrument), or where it still
holds. The instrument is not the verdict: it does not calculate rule 6.

1. *No mode measured competitors under the comparison's conditions.*
   Closed in `compare` mode: `COMPARE_LINES` (the `workload_servers`
   input) selects `reference`, `reference-io`, `valkey`, `valkey-io`,
   `dragonfly` and `garnet` beside the firn images and their twin, with the
   images' server CPU list, reversed pass order and client-busy record
   (`competitors_selftest`, `compare-client.py selftest`).
2. *48 connections, and only the first process's p50 and p99.* Closed: the
   client opens exactly 50 connections (`compare-client.py` allocation), the
   rows of several processes are labelled upper bounds, and one extra
   single-process threaded pass per line gives the exact merged p99
   (`compare_rows_selftest`).
3. *`workloads` mode ran AOF only for two lines.* Closed for the baseline,
   which uses `compare` mode with `aof=true` (Valkey and Garnet start with
   AOF, Dragonfly is refused). Still open in `workloads` mode, which runs
   the selected lines without AOF.
4. *Garnet started without `--lua` and AOF options.* Closed in `compare`
   mode (`competitor_args`). Garnet's disk synchronization stays
   unverified, so its AOF results carry `durability=not_equal_unverified_fsync`.
5. *Dragonfly's affinity was unchecked.* Closed in `compare` mode
   (`compare_affinity_check`; a restart with affinity off when startup
   escapes the server list).
6. *`valkey-io` passed a no-op flag and there was no `reference-io` line.*
   Closed in `compare` mode (`competitor_args`, with a `reference-io` line
   from two CPUs up).
7. *Versions and archive hashes were recorded and not checked.* Closed: a
   full (non-probe) run fails on a missing executable, a version that is
   not the pinned release or a missing or mismatched installer receipt
   (`versions_selftest`, `compare_versions_selftest`). The release may
   carry one suffix, the pinned tag's commit: Dragonfly 2.0.2 and Garnet
   2.2.1 print it in full (see the first trial run below), and the matcher
   accepts a suffix only as `-` or `+` and at least 7 lowercase hex digits
   that begin that commit, with terminal colour codes removed.
8. *The AOF start check was Redis-specific and ran only in `compare` mode.*
   Closed for the baseline: `compare` mode checks firn, Redis and Valkey
   through `INFO persistence`, and Garnet through its log
   (`compare_garnet_aof_check`). The application `workloads` mode still
   does not enforce versions or AOF.
9. *Memory was recorded only after `session-get`'s fill.* Closed for
   redis-benchmark lines in `compare` mode (`compare-memory.csv`, written by
   `compare_memory`); the session-fill record stays in `workloads` mode.

## Trial runs of the instrument

The 14900K, 2026-11-25, Firn-wf head 292eae7 (the instrument files of the
work that closed the gaps above):

- Install run 45651522021 (`install-competitors`) installed Valkey 9.1.2,
  Dragonfly 2.0.2 and Garnet 2.2.1 under the runner's `~/.local/opt` with
  their receipts.
- Compare trial 45652263824 (probe; two server CPUs on CPUs 2 and 4; lines
  `reference reference-io valkey valkey-io dragonfly garnet` beside `firn`
  and `firn-twin`; depth 16, one pass of 3 seconds; tests `set get mset
  lrange_500 xadd xadd_maxlen xrange_10 sismember srem hget hdel`). Every
  line started as the rules say, with Dragonfly's threads checked inside the
  server list, and the pilots of `set`, `get`, `mset`, `lrange_500` and
  `xadd` gave a throughput and a p99 for every line but Garnet's `xadd`:
  Garnet 2.2.1 has no streams
  ([API compatibility](https://github.com/microsoft/garnet/blob/v2.2.1/website/docs/commands/api-compatibility.md)),
  and the run stopped there with `compare client process failed`.
  `host.txt` records for all four competitors `archive_provenance=verified`
  and `receipt_check=verified` (Redis: `not-applicable`, its build is checked
  against `reference.sh`), a matched version for Redis and Valkey, and for the
  other two the real `--version` output:

  ```
  dragonfly <ESC>[0;32mv2.0.2-8ce4bbb106586057fbd85f71ed353c60d0d5bf87<ESC>[m
  build time: <ESC>[0;33m2026-10-07 20:47:39<ESC>[m
  GarnetServer 2.2.1+b0255a31de95b1e903ee187feb59ceb04819a639
  ```

  Both suffixes are the full commit that the pinned tag points to
  (`git ls-remote` of `v2.0.2` and `v2.2.1`), which is what the version
  rule now accepts.
