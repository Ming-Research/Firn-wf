# HSET and LPUSH after the move to wf-63b7cbb03820

Item #145. The move of firn from the Whitefoot release `wf-af7665be672d` to
`wf-63b7cbb03820` (#138, [record](whitefoot-v0165-pin.md)) left two cells
slower on the 14900K: `hset` and `lpush` at pipeline depth 16, 2.5% to 4.0%
and 1.7% to 2.7% slower over 24 passes, with 4.1% and 2.6% more server CPU
per request ([confirmation run](whitefoot-v0165-pin.md#confirmation-run)).
This record finds which of the compiler with its standard library, the Halo
sources or firn's own source edits causes each, and holds the evidence.

The old source does not build with the new compiler, but the new source
builds with the old one once the two `Inputs` fields the old release lacks are
left out of `main`'s destructuring: revision `1dddd17` (firn and Halo as on
this branch, pin `wf-af7665be672d`) built with `whitefootc wf-af7665be672d`
on the 14900K on 2026-11-25 (fast workflow run
[45610502247](https://github.com/Ming-Research/Firn-wf/actions/runs/45610502247),
the `firn` target). Its image, the **cross** image below, differs from the
head image only by the compiler with its standard library and runtime, and
from the base image only by the source edits of #138 (firn's and Halo's), so
measuring it beside both splits the regression between the two.

## What the sources of the two builds say

Read on 2026-11-25 in `git diff 7cf12df b08522a` (firn and its Halo commit)
and in Whitefoot between the two release commits `af7665be672d` and
`63b7cbb03820`.

- **Both tests write one key.** `redis-benchmark` 7.2.16 sends
  `HSET myhash element:__rand_int__ xxx` and `LPUSH mylist xxx`
  (`src/redis-benchmark.c`), so with two server CPUs both drivers take turns
  on that one key's atomic statement, and a longer statement shows as lower
  throughput and as more time spent waiting to acquire it. With `-r 100000`
  the hash holds at most 100,000 fields of 20 bytes, so after the first
  fraction of a run nearly every `HSET` replaces a field; the list grows by
  every request.
- **firn's edits leave both paths as they were.** Of the 42 files of `firn/`
  the move changed, the functions `HSET` and `LPUSH` run (`run_hset`,
  `hset_body`, `set_fields`, `put_fields`, `run_push`, `push_body`,
  `push_items`, `push_all`, `push_element`, `list_room`, `prepare_elements`,
  `push_item`, `clear_items`) differ only by the rename of `held`. The
  conditions deleted near them are in functions these commands do not run
  (`random_field`, `insert_item`, `lpos_finish`) or run only with the
  append-only file or a script's access journal, both off in the
  measurement (`log_reserve`, `record_access`).
- **Halo is not on either path.** firn's command code calls Halo only in
  `eviction_limit` (`halo::number::pow`), which runs under a memory limit, and
  the measurement sets none; Halo-wf#71 itself renames and deletes decided
  conditions. Halo's code is linked into the same image, so its size can still
  move where the hot code lies.
- **The list's front operation is unchanged in the library and its emitter.**
  `std::collections::deque` is the same file at both commits, and the emitter
  functions that write `place_front` and `place_back`
  (`boundary_slot` and `move_run_boundary` in
  `compiler/src/backend/emitter/runs.rs`) are the same at both. `LPUSH` and
  `RPUSH` run the same firn code but for `deque_push_front` against
  `deque_push_back`.
- **The hash map changed (Whitefoot#487).** A key's home bucket was
  `hash % count`; it is now the top bits of `hash * 11400714819323198485`,
  over a bucket count that is zero or a power of two of at least two
  (`hash_map_home`, with a bounds branch the checker requires), and the map
  gained a `shift` word. Every probe loop (`hash_map_find`,
  `hash_map_lookup`, `hash_map_edit`, `hash_map_try_put`, the rebuild)
  computes the home this way. A hash of 100,000 fields has 262,144 buckets in
  both builds: the old rule grows a map made for 4 pairs from 4 buckets and the
  new one from 8, each doubling once three quarters are used. firn's keys
  carry an FNV-1a hash (`window_hash`), so the old home took its low 18 bits
  and the new home takes the top 18 bits of its product.
- **The keyspace is not this map.** `store^.keys` is the compiler's shared
  concurrent map (`ConcurrentHashMap<Entry>`), the same for every command; the
  standard library's map holds a hash's fields and a set's members.

**The new home does not lengthen the probes of these fields.** The 100,000
fields `element:000000000000` to `element:000000099999` that `-r 100000`
draws from, hashed with FNV-1a as `window_hash` does, placed by linear
probing in 262,144 buckets (a successful search's mean cost does not depend on
the order of insertion), computed on 2026-11-25:

| home | distinct homes | mean buckets per hit | 99th percentile | longest |
| --- | ---: | ---: | ---: | ---: |
| `hash % 262144` (old) | 81,376 | 1.351 | 5 | 26 |
| top 18 bits of `hash * 11400714819323198485` (new) | 83,591 | 1.288 | 5 | 15 |
| a uniformly random home (seed 1), for reference | 83,246 | 1.308 | 5 | 17 |

So under H1 below the extra cost would come from the code of the map's
operations, not from longer walks. The computation, in Python:

```python
M = (1 << 64) - 1
def fnv1a(data):
    h = 14695981039346656037
    for b in data:
        h = ((h ^ b) * 1099511628211) & M
    return h
B = 262144
hashes = [fnv1a(b"element:%012d" % i) for i in range(100000)]
old = lambda h: h % B
new = lambda h: ((h * 11400714819323198485) & M) >> 46
def mean_probes(home_of):
    table, homes = [None] * B, []
    for i, h in enumerate(hashes):
        j = home = home_of(h)
        homes.append(home)
        while table[j] is not None:
            j = (j + 1) % B
        table[j] = i
    where = {i: j for j, i in enumerate(table) if i is not None}
    return sum((where[i] - homes[i]) % B + 1 for i in range(len(hashes))) / len(hashes)
```

**The map's change alone made no operation slower in Whitefoot's own
measurement.** Whitefoot#487's performance acceptance compared the map before
and after it under one compiler, the candidate's, on the 14900K
(`hash-map-bench.yml`, run 39716174131, recorded in Whitefoot's
`research/investigations/hash-scan/DESIGN.md`, "Third comparison"): of the 21
time cells 19 faster, one unchanged, one unresolved and none slower, lookup of
a present key at 1,000,000 pairs 0.994 [0.983, 0.9995]. Its keys and values are `u64` and it has no cell
that replaces a present key's value, which is what nearly every `HSET` here
does, so it does not cover firn's `HashMap<Key, Bytes>` with byte keys; it
does say that the library's new probe code, run by one compiler, is not
slower than the old.

**What else the compiler and runtime changed between the two releases** (80
merges on Whitefoot's `main`, `git log --first-parent af7665be672d..63b7cbb03820`),
read for what can reach these two paths, none of it measured apart:

- every generated function carries `uwtable` (Whitefoot#461), so the base image
  has no unwind tables and a DWARF-caller profile of it does not walk through
  Whitefoot frames; callers can be compared only between the cross and head
  images;
- enum variant payloads are read and written through body place suffixes
  (Whitefoot#467), which reaches `HashMapSlot`, `Bytes` and `Key`;
- a statement holding map entries that misses a shared object queues a FIFO
  claim (Whitefoot#388, `2c92dc5fa`), a change to the runtime's contended take;
- heap blocks are taken at a recorded 16-byte alignment (Whitefoot#471);
- a `set` evaluates its right-hand side before its target (Whitefoot#515).

## What the earlier runs already show

Server CPU per request in the first run (run
[40120506904](https://github.com/Ming-Research/Firn-wf/actions/runs/40120506904),
8 passes, depth 16, the `server_cpu_us_per_request` column of its
`compare.csv`): the median over passes of each pass's ratio, and in how many
passes the head was above the base.

| test | head/base | passes head above | base-twin/base | what the request does |
| --- | ---: | ---: | ---: | --- |
| hset | 1.034 | 7 of 8 | 1.005 | replaces a field of a hash of 100,000 |
| hget | 1.029 | 6 of 8 | 1.004 | reads a field of that hash |
| sadd | 1.017 | 4 of 8, 1 equal | 0.998 | re-adds a member of a set of 100,000 |
| sismember | 0.995 | 4 of 8 | 0.993 | reads a member of that set |
| hdel | 0.980 | 2 of 8 | 0.990 | mostly finds the drained hash empty |
| srem | 0.997 | 4 of 8 | 1.000 | mostly finds the drained set empty |
| lpush | 1.016 | 7 of 8 | 0.998 | pushes at the head of one list |
| rpush | 0.995 | 2 of 8 | 0.997 | pushes at its tail |
| lpop | 0.971 | 0 of 8 | 0.993 | pops its head |
| rpop | 0.988 | 1 of 8 | 0.993 | pops its tail |

The confirmation run (16 passes, run
[40851400945](https://github.com/Ming-Research/Firn-wf/actions/runs/40851400945))
has the head above the base in all 16 passes of both cells, `hset` 1.037 and
`lpush` 1.026 by this measure. `hget` leans the way `hset` does but was not
measured again, and 8 passes do not decide it.

## Hypotheses, and what a profile of each predicts

A profile gives each symbol's share of the server's cycles. Multiplied by the
profile's event count and divided by its requests it gives cycles per request
for that symbol, which can be set beside the other image's.

- **H1, the hash map's change.** The extra cycles of `HSET` lie in the
  standard library's hash-map functions (their symbols name
  `hash_map`), or in the firn function they are inlined into, while `RPUSH`
  shows none. `HGET`, which probes the same table, shows the same if it is
  slower.
- **H2, the compiler's code for firn's own command code.** The extra cycles
  lie in firn's command functions of the path (`wf_commands.push_body`,
  `wf_commands.run_push`, `wf_commands.put_fields`, ...) and not in the hash
  map or the runtime.
- **H3, the runtime's shared keyspace.** The extra cycles lie in the
  acquire and release functions (`wf_shared_acquire_as`, `acquire_entry`,
  `wf__shared_take`, `wf_shared_unlock_as`, `wf__table_lock_entry`), with no
  rise in the functions inside the statement. `RPUSH` and `SISMEMBER` take
  the same lock and are not slower, which already argues against it.
- Waiting to acquire grows whenever the statement grows, since the other
  driver waits longer. That growth follows whichever of H1 and H2 holds, and
  names no cause of its own.
- **H4, where the code lies.** The head runs the same instructions per
  request in the functions whose cycles grew, at fewer instructions per
  cycle, as Whitefoot's item on code placement records of Halo on the 14900K
  (two byte-identical hot functions at different page offsets, one workload
  4.6% slower at the same instruction count). A flat profile of cycles alone
  cannot tell this from H1 or H2.

H1 to H4 all lie in the compiler with its library and runtime, or in firn's
and Halo's source edits through what the compiler makes of them; the cross
image decides between these two before any profile is read.

## First profile run, stated before measuring

**Question.** In which functions does the head spend the extra server cycles
per `HSET` and per `LPUSH` request?

**Run.** `redis-bench.yml`, mode `compare`, on the 14900K: `base` and
`base-twin` are `7cf12df` (pin `wf-af7665be672d`), `head` is `b08522a` (pin
`wf-63b7cbb03820`), the images of the earlier runs. 2 server CPUs on the
P-cores 2 and 4, clients on CPUs 16-31; tests `hset lpush rpush hget
sismember sadd` at depth 16, 8 interleaved passes of 5 seconds, then one
profiled run of each test for each image (`perf record`, 4999 Hz, the
`profile_test` list), each image's flat profile by symbol.

**Measure.** For each image and test, cycles per request of each symbol, and
of these groups: the hash map (symbols naming `hash_map`), the list's push
(`push_body`, `push_items`, `push_all`, `push_element`, `list_room`,
`deque`), firn's other code (`wf_commands`, `wf_protocol`, `wf_store`,
`wf_server` and the rest of the image), the keyspace's lock (the five
runtime symbols named under H3 and the other `wf_shared`, `wf__shared`,
`wf__table` and `wf_guard` symbols), the allocator (`libc` allocation
symbols) and the kernel. `base-twin` against `base` is the measure's noise
for each symbol and group.

**Rule.** A group accounts for a test's regression if its head-minus-base
cycles per request is more than twice its twin-minus-base, in absolute value,
and at least half the head-minus-base of the whole profile. H1 holds for
`HSET` if the hash-map group accounts for it, with the lock group allowed to
grow beside it; H2 if the push group or firn's other code does. If no group
accounts for it, as when the two compilers inline the path differently and the
symbols do not correspond, the profile is inconclusive and the next step is a
profile with DWARF callers or the annotated instructions of the symbols that
moved. This profile is exploratory: it attributes, and its rule picks the
next step, rather than accepting or rejecting a change.

The passes give another 8 passes of throughput and CPU per request for the
six cells, judged with the twin as in the
[pin record](whitefoot-v0165-pin.md#performance-comparison-stated-before-measuring).

### Results: disturbed and stopped, no profile

Dispatched on 2026-11-25 as run
[45607701747](https://github.com/Ming-Research/Firn-wf/actions/runs/45607701747)
(from `main`, with the inputs above and `revisions` naming `7cf12df` and
`b08522a` in full). It measured from 11:51:42 to 12:07:58 UTC and failed in
pass 7: the measurement's session received TERM (exit 143), whose sender the
log does not name, before its profiles, which follow the eight passes; the
rows below are read from the job's log.

Since 2026-11-24 every agent's tests run on the same machine (`fast.yml` and
the fast workflows of the other projects on the `14900k` runners), and three
Firn fast jobs ran there during the measurement (11:53:29-11:55:34,
12:05:16-12:08:35 and 12:07:52-12:10:35; runs 45608586520, 45610400266 and
45610502247). The twin shows it: `base-twin` over `base` in `hset` rate is
0.540, 3.598, 3.820, 0.977, 0.973 and 1.740 in passes 1 to 6, and pass 6 of
`base`, from 12:05:26, ran at 11% (`sadd`) to 68% (`rpush`) of its pass-5
rate. Passes 4 and 5 are the
only ones whose twin ratios lie within 3% of 1 in all six tests; two passes
judge nothing, and they are shown only as a lead:

| test | head/base rate, passes 4 and 5 | twin/base rate | head/base CPU per request | twin/base CPU per request |
| --- | --- | --- | --- | --- |
| hset | 0.952, 0.934 | 0.977, 0.973 | 1.053, 1.072 | 1.024, 1.027 |
| hget | 0.949, 0.944 | 0.983, 0.991 | 1.052, 1.064 | 1.019, 1.012 |
| lpush | 0.984, 0.987 | 1.004, 1.016 | 1.017, 1.013 | 0.998, 0.983 |
| rpush | 1.007, 1.014 | 0.986, 1.004 | 1.004, 0.990 | 1.030, 1.000 |
| sismember | 0.979, 1.012 | 0.986, 1.019 | 1.022, 0.988 | 1.016, 0.982 |
| sadd | 0.991, 1.018 | 0.988, 1.016 | 1.006, 0.981 | 1.009, 0.983 |

`hget` leans as `hset` does, as in the first run of the pin comparison
(0.9711, 0.9456-1.0055, 8 passes), while `sismember`, a lookup in the same
library map with `unit` values, does not; the next run measures `hget` as a
cell of its own.

## The machine's other jobs, stated before the sample run

Since 2026-11-24 every project's agents run their tests on the 14900K, so a
measurement shares it with other jobs, as the
[disturbed run](#results-disturbed-and-stopped-no-profile) showed (of its six
finished passes only passes 4 and 5 had twin ratios within 3% of 1). The owner chose that the measurement script
protects itself (decision `benchmark-voids-passes-run-beside-other-jobs`),
without a change to a workflow. The rules below are fixed before any result of
the sample run or the four-image run is read, and decide only from the CPUs
the measurement does not use, never from rates.

- **The CPUs.** `PERF_CPU` is the lowest online CPU of `/proc/stat` outside the
  server CPUs and the client CPUs, and every `perf` command of the
  comparison runs on it. The other CPUs are every online CPU outside the
  server, the client and the profiler's CPU. With servers 2,4 and clients
  16-31 on the 14900K's 32 CPUs: the profiler on CPU 0, the other CPUs 1,3,5-15
  (13 CPUs, the sibling threads 3 and 5 of the server cores among them).
- **The reading.** The mean over the other CPUs of the share of ticks that are
  neither idle nor iowait (the `cpu_busy` reading, role `other`), kept for
  every client run in `compare-runs.csv` with the run's start and end in epoch
  seconds. A reading is busy when it is above the threshold T, never when it
  is at it.
- **The threshold T is 0.05**: five percentage points of the other CPUs'
  mean, 0.65 of one CPU among 13. A run of two busy loops on two of the 13
  reads 2/13 = 0.154. The sample run reads the idle level of the machine in
  its control; if that idle reading is above 0.02, T becomes that reading plus
  0.03, and the record says so before the four-image run is dispatched.
- **The control.** At the start of the session, before any image starts, the
  other CPUs are read for 2 s idle, then two busy loops are started, each on one
  of the first two other CPUs, and they are read for 2 s again; the loops are
  stopped by their pids. The run fails unless the loaded reading is above T and
  above the idle one by at least half of loops over other CPUs (0.077 for two
  loops on 13 CPUs), and when there is no other CPU. The second condition keeps
  a machine that is busy anyway from passing the control without the loops
  being seen. `compare-control.csv` has both readings.
- **Passes.** Attempts at passes go on until 16 are clean. An attempt is void
  when any of its runs read busy: it stays in `compare.csv` (whose pass column
  numbers the attempts), is listed in `compare-void.csv`, and `summary.py
  ratios` leaves it out and says how many. An attempt's image order follows
  the number of the clean pass it would become (odd forward, even reversed), so
  a replacement runs in the order of the attempt it replaces. No new attempt
  starts 115 minutes after the session began.
- **Waiting.** Before each attempt, and before the control, the other CPUs are
  read for 2 s; while busy the run waits 10 s and reads again, one wait at most
  600 s and all the waits of the session at most 2700 s; then it goes on and the
  void rule decides.
- **Profiled runs.** Each waits as above for at most 120 s (not counted in the
  2700 s). One that reads busy is run once more; one that reads busy again, or
  has no time to be repeated, is listed in `profile-void.csv` and the profile
  rule does not use it. From 140 minutes after the session began a profiled run
  neither waits nor is repeated. The Measure step ends at 150 minutes.
- **Clean passes short of 16.** With 8 to 15 clean passes the split rule
  judges with them, and the record states the count (the pin comparison's
  first run judged with 8). With fewer than 8 the same images are dispatched
  again and the clean passes of both runs are pooled, as the confirmation run
  pooled.
- **The check against GitHub's records.** After the four-image run, every
  14900K job of the five projects' repositories that overlapped it is read
  (`gh run list` and each job's steps, with their start and end). A heavy step
  is one that is not set-up, checkout or download. Each client run of
  `compare-runs.csv` is compared with the heavy steps' spans.
  T' is the highest reading among the client runs that overlap no heavy step
  and are not void, plus 0.02; those runs read at most T, so T' is at most T +
  0.02.
  - A run that overlaps a heavy step and reads at most min(T, T') is not void
    and does not show T too high: a step that only waits on the network loads
    the machine no more than quiet.
  - A run that overlaps a heavy step and reads above T' and at most T shows
    T was too high. Then the threshold becomes T', and voiding is decided again
    from the recorded readings alone, without looking at rates. No run that
    overlaps no heavy step is voided by this, since they all read at most T'.
    If clean passes then fall short, the rule above applies.
  - A void run that overlaps no job is load from outside GitHub, and the
    record says so.
  - If no client run overlaps any job, the record says that the four-image
    run gives no evidence for the void path, and that the evidence for it is
    the fixtures of `placement-selftest` and `summary.py selftest`.

The reading cannot see a job that runs on a server or client CPU, which is
the one case the rule leaves to the twin: `base-twin` over `base`, a clean
pass's noise, shows it.

## Sample run, stated before measuring

**Question.** Does the measurement script with the rules above run to its end
on the 14900K with the four images, how long do its parts take, and where does
the other CPUs' idle reading lie against T?

**Run.** [45620993537](https://github.com/Ming-Research/Firn-wf/actions/runs/45620993537),
dispatched on 2026-11-25 at 13:11 UTC from the engine's ref
`wf-try/work-145-26e0fade/call/37`, commit `51c7be4`; later commits of this
branch only add two log lines (the CPUs the control's loops and the profiler
are pinned to) and a header comment to `redis-bench.sh`. Mode `compare`, servers
2,4 and clients 16-31; revisions `base` and `base-twin` `7cf12df`, `head`
`b08522a`, `cross` `1dddd17`; test `hset` at depth 16, 2 passes of 5 seconds,
profile `hset 16`. It has the four images of the four-image run so that it
builds all three of them with `make firn-lto` and times a pass of four images.

**Read.** The control's idle and loaded readings (`compare-control.csv`), the
readings of every run (`compare-runs.csv`), the step times of the job and the
`time,` lines of the log (a pass, a profiled run, the builds, the steps before
them), `revisions.csv` (the `cross` row names `1dddd17` and pin
`wf-af7665be672d`), and whether each `profile-*.txt` holds a table of cycles
and one of instructions. The run decides no comparison, and its rates are not
read.

### Sample run, results

The run succeeded: queued at 13:11:06, started 13:13:46 and ended 13:26:08 UTC
on 2026-11-25 (job steps read with `gh run view`). The four images were built
and `revisions.csv` has `cross` as `1dddd17` with `release = wf-af7665be672d`.
Its rates were not used; the log prints the profiled runs' rates, which were
seen while checking the profile files and are not used here either.

- **The control held.** After waiting 331 s from the session's start for the
  other CPUs to fall to T (another project's job; the 2 s readings fell from
  0.98 to 0.09, 28 of them), the idle reading was 0.0069 and the loaded 0.1585
  against the 2/13 = 0.154 the loops should make. The idle reading is at most
  0.02, so **T stays 0.05**.
- **The void path ran.** Three attempts, two clean passes: attempt 1 read 0.0144,
  0.2511, 0.2550 and 0.2989 in its four runs and is void (`compare-void.csv`),
  its replacement attempts 2 and 3 are clean passes 1 and 2 (their runs read
  0.0016 to 0.0096) in the specified order (attempts 1 and 2 forward, 3
  reversed). The four profiled runs read 0.0007 to 0.0026 and none is in
  `profile-void.csv`. No run read between 0.0144 and 0.2511, so T lies in an
  empty gap of this run. The attempt-1 load was spread over the 13 other
  CPUs (0.05 to 0.56 each in the three runs it voided; at most 0.04 on any CPU
  in the clean attempts).
- **The profile files hold both tables.** Each `profile-*-2-hset.txt` has a
  table of `cpu_core/cycles/` and one of `cpu_core/instructions/`, each with
  its event count and an IPC column. perf names the events `cpu_core/...`.
  Symbol names carry an instance hash (`hash_map_try_put$instance$<hex>`), so
  the symbols of two images are matched with that suffix removed. This commit
  predates the log lines that print the CPUs perf and the control's loops are
  pinned to; the four-image run carries them.
- **Times.** The steps before the build took 44 s, the three builds 241 s, and
  the Measure step 452 s, of which 331 s waiting for the control. A pass of
  four images at one test took 19 to 25 s (about 4.7 s a run), a profiled run
  about 9 s (35 s for the four), the size pilot and the four correctness
  checks 5 s. For the four-image run (five tests, sixteen passes, three
  profiled tests) a pass is about 100 s and the profiles about 2 minutes, so
  the Measure step is about 35 minutes without waits; 44 s + 241 s + 150 minutes
  is 155 minutes, below the 175 of the rule, so no `images_run` is needed.
- **The oracle on this run** (a scratch script reading `gh api`: the workflow
  runs of each of the five repositories created in the 3 hours before the
  window, then the jobs of those that overlap it, keeping the jobs labelled
  `14900k`; a step is light when its name starts with `Set up job`, `Run
  actions/`, `Post Run`, `Complete job`, `Check out` or `Download`; 17 calls).
  One heavy step overlaps all 16 client runs: the `Test` step of a Firn-wf fast
  job, 13:16:07 to 13:32:06, 16 minutes. It does not separate the void runs from
  the clean ones, since the whole of the session lies inside it; the void
  attempt overlaps a job, so its load is not from outside GitHub.

**A case the oracle rules did not cover, decided before the four-image run.**
When every client run overlaps a heavy step, no run gives T', which is then not
defined and min(T, T') is T: T stays, a run above T is void, and the record says
that the check could not tighten T. T' is computed only when at least one
non-void run overlaps no heavy step.

## Four-image run, stated before measuring

**Question.** Is each of `hset`, `lpush` and `hget` at depth 16 slower
because of the compiler with its library and runtime, or because of the
source edits of #138, and in which functions does the slower side spend its
extra cycles and instructions per request?

**Images.** `base` and `base-twin`, `7cf12df` (old source, old compiler);
`cross`, `1dddd17` (new source, old compiler); `head`, `b08522a` (new
source, new compiler). `cross` and `head` have the same firn and Halo
sources, so their symbols correspond as far as the two compilers inline
alike, and `cross` and `base` have the same compiler.

**Run.** `redis-bench.yml`, mode `compare`, on the 14900K with the placement
of the pin comparison (2 server CPUs on P-cores 2 and 4, clients on CPUs
16-31); tests `hset lpush rpush hget sismember` at depth 16, 16 clean
interleaved passes of 5 seconds; then one profiled run of `hset`, `lpush` and
`hget` for each image (`profile_test` `hset 16;lpush 16;hget 16`), recording
cycles and retired instructions, each table in the image's `profile-*.txt`.
The passes and profiled runs beside other jobs on the machine follow the rules
of [the section above](#the-machines-other-jobs-stated-before-the-sample-run).

**Dispatched** on 2026-11-25 at 14:14 UTC as run
[45631892786](https://github.com/Ming-Research/Firn-wf/actions/runs/45631892786),
from the branch ref `work-145-26e0fade` at `bc0c257` (`headSha` read from the
run), after the rules above and the sample run's reading of T (unchanged,
0.05) were committed. Inputs: `server_cpus` 2,4, `client_cpus` 16-31, `cpus` 2,
`tests` `hset lpush rpush hget sismember`, `depths` 16, `passes` 16, `seconds`
5, `profile` true, `profile_test` `hset 16;lpush 16;hget 16`, `revisions`
`base=7cf12df4cba634c0b50a6da070b386332e7005de`,
`head=b08522afd76216dbd7289473834e4a4a7f523a18`,
`cross=1dddd17c1408b9f95555f9b4374db8db6f25320c` and
`base-twin=7cf12df4cba634c0b50a6da070b386332e7005de`; no `images_run`. The
control's failing run (loops on the server CPUs) is dispatched after this run
ends, never beside it, since its loops would load the CPUs this run reads.

**Split rule, for each of `hset`, `lpush` and `hget`.** The intervals are
those of the pin comparison: the 95% interval of the mean log ratio of the
per-pass rates (Student's t), as `summary.py ratios` prints it, run once for
each pair without `--twin` (`--base base --head head`, `--base base --head
cross`, `--base cross --head head` and `--base base --head base-twin`), since
that option divides the twin by the head. A cell counts only if its
`base-twin`/`base` interval holds 1. With `head`/`base` wholly below 1, the compiler's share is `head`/`cross`
and the source edits' share is `cross`/`base`:

- **the compiler** if `head`/`cross` is wholly below 1 and `cross`/`base` is
  not;
- **the source edits** if `cross`/`base` is wholly below 1 and `head`/`cross`
  is not;
- **both** if both are wholly below 1;
- **unresolved** if neither is, and then 16 more passes of the same images
  are pooled with these once; still neither, and the record says so with both
  intervals.

If `head`/`base` is not wholly below 1 for a cell, the regression did not
repeat in this run, and that cell is reported, not split. `rpush` and
`sismember` are the controls of `lpush` and `hget`: the same command code and
the same library map, which the pin comparison did not find slower.

**Profile rule.** For each image and test, cycles and instructions per
request of each symbol and of the groups of the first profile run. A group
accounts for the compiler's share if its `head`-minus-`cross` cycles per
request exceed twice its `base-twin`-minus-`base` in absolute value and are at
least half of the whole profile's `head`-minus-`cross`; for the source edits'
share the same with `cross`-minus-`base`. In a group that accounts for a
share, more instructions per request in the same proportion as the cycles is
H1 or H2 (more work); the same instructions at more cycles is H3's waiting or
H4. If no group accounts for it, the next step is the annotated instructions
of the symbols that moved most, in a second profiled run of `cross` and
`head` (`annotate`; DWARF callers only between those two, since `base` has no
unwind tables). The profile attributes and picks the next step; it accepts or
rejects nothing.

### Four-image run, results

Run [45631892786](https://github.com/Ming-Research/Firn-wf/actions/runs/45631892786)
succeeded: the job ran 14:15:05 to 15:05:51 UTC (50 min 46 s, below its 180),
the steps before the build 24 s, the three builds 4 min 39 s, and the Measure
step 14:20:08 to 15:05:46 (45 min 38 s, below its 150). `revisions.csv` has
`base` `7cf12df` and `cross` `1dddd17` with `release = wf-af7665be672d`, and
`head` `b08522a` with `release = wf-63b7cbb03820`.

**Passes.** 18 attempts, 16 clean. The control waited the whole 604 s for the
other CPUs to fall to T, which they did not (another job's; 0.12 at the end),
so it ran beside that job: idle 0.1157, loaded 0.2686, a rise of 0.153 against
the 2/13 = 0.154 two loops make, which satisfies its rule (`compare-control.csv`);
this run's control therefore does not give the idle level, which T takes from the
sample run. Attempt 1 began after that wait gave up and 19 of its 20 runs read
above T (highest 0.9837): void. After a further 266 s of waiting, attempt 2 had
one run at 0.0570 (`head`, `rpush`) and 19 at or below T: void. Attempts 3 to
18 are the 16 clean passes: no run read above 0.0161, and 15 of the 16 attempts
read at most 0.0014. The 12 profiled runs read at most 0.0008 and none is in
`profile-void.csv`. The waits came to about 870 s of the 2700 s budget. The
profiler ran on CPU 0 (`perf pid ... current affinity list: 0` for each of the
12 recorders), the control's loops on CPUs 1 and 3, and no cell is client-limited
(client CPUs 0.13 to 0.15 busy on average).

**Throughput.** Intervals of the mean log ratio of the per-pass rates over the
16 clean passes (Student's t), each from one `summary.py ratios` call without
`--twin` (`compare.csv` with `compare-void.csv` beside it, which the tool
reports as 2 void attempts left out):

| test | head/base | cross/base | head/cross | base-twin/base |
| --- | --- | --- | --- | --- |
| hset | 0.9545 [0.9464, 0.9771] | 1.0100 [0.9983, 1.0382] | 0.9478 [0.9363, 0.9529] | 1.0018 [0.9916, 1.0314] |
| hget | 0.9439 [0.9339, 0.9617] | 0.9983 [0.9922, 1.0071] | 0.9435 [0.9340, 0.9624] | 0.9984 [0.9923, 1.0073] |
| lpush | 0.9747 [0.9687, 0.9946] | 0.9793 [0.9730, 0.9997] | 0.9972 [0.9901, 1.0004] | 0.9989 [0.9903, 1.0177] |
| rpush | 1.0015 [0.9967, 1.0118] | 0.9943 [0.9842, 1.0032] | 1.0088 [1.0056, 1.0156] | 0.9977 [0.9934, 1.0106] |
| sismember | 0.9683 [0.9588, 0.9848] | 0.9719 [0.9606, 0.9811] | 0.9942 [0.9862, 1.0158] | 0.9946 [0.9848, 1.0024] |

The value before each interval is the median of the per-pass ratios. Server CPU
per request agrees (median of the per-pass ratios, and in how many of the 16
passes the numerator is above): `hset` head/cross 1.0558 (16), head/base 1.0414
(15), cross/base 0.9914 (3); `hget` head/cross 1.0618 (15), head/base 1.0580
(15), cross/base 1.0000 (7); `lpush` cross/base 1.0192 (14), head/cross 1.0042
(11), head/base 1.0247 (15); `sismember` cross/base 1.0275 (14), head/cross
1.0108 (11); `rpush` head/base 0.9990 (5).

**Verdicts under the split rule** (every `base-twin`/`base` interval holds 1):

- **`hset`: the compiler**, with its library and runtime. `head`/`base` and
  `head`/`cross` are wholly below 1, `cross`/`base` is not.
- **`hget`: the compiler**, the same way.
- **`lpush`: the source edits.** `head`/`base` and `cross`/`base` are wholly
  below 1 and `head`/`cross` is not; the upper end of `cross`/`base` is 0.9997.
  Whether Halo's or firn's edits is not yet decided.
- Controls: `rpush` shows no change against `base`. **`sismember`, which the pin
  comparison did not find slower, is 3.2% slower here, from the source edits**
  (`cross`/`base` wholly below 1, `head`/`cross` not).

So the old compiler with the new source is as fast as the old source for `hset`
and `hget`, and slower by about 2% for `lpush` and 3% for `sismember`; the new
compiler then costs `hset` and `hget` about 5% and `lpush` and `sismember`
nothing more.

**Profiles** (one profiled run of each test and image, `cpu_core/cycles/` and
`cpu_core/instructions/` per request, symbol names matched with their `$instance$`
suffix removed, group names as in the first profile run). Whole profile, per
request:

| test | | base | base-twin | cross | head |
| --- | --- | ---: | ---: | ---: | ---: |
| hset | cycles | 2645 | 2629 | 2581 | 2688 |
| hset | instructions | 4950 | 4950 | 4948 | 4951 |
| lpush | cycles | 2401 | 2377 | 2419 | 2434 |
| lpush | instructions | 4784 | 4782 | 4782 | 4806 |
| hget | cycles | 2710 | 2636 | 2698 | 2790 |
| hget | instructions | 5182 | 5168 | 5174 | 5174 |

In every profile the four images retire the same number of instructions per
request (to 0.5%), so the compiler does not make any image do more work in
total; `head` runs the same instructions at more cycles for `hset` (IPC 1.842
against `cross`'s 1.917) and `hget`. One profiled run is noisy: the twin's
whole-profile cycles differ from `base`'s by 16, 24 and 74 per request (0.6%,
1.0%, 2.7%). Groups, cycles per request, `head` minus `cross` / `cross` minus
`base` / `base-twin` minus `base`, and the same for instructions:

| test | group | cycles | instructions |
| --- | --- | --- | --- |
| hset | keyspace lock | +84 / -47 / -17 | +30 / -6 / -5 |
| hset | hash map | +21 / -4 / -6 | +14 / -4 / -7 |
| hset | firn and runtime, other | +7 / -18 / 0 | -52 / +2 / +6 |
| hset | kernel | -7 / +3 / +3 | +12 / +8 / +9 |
| hset | whole | +107 / -64 / -16 | +3 / -2 / -1 |
| hget | keyspace lock | +77 / +13 / -38 | +10 / -11 / -19 |
| hget | hash map | +31 / 0 / -9 | +38 / -3 / -5 |
| hget | firn and runtime, other | -18 / -23 / -20 | -22 / -1 / +27 |
| hget | kernel | +5 / -3 / -10 | -14 / +7 / -9 |
| hget | whole | +92 / -12 / -74 | +1 / -9 / -15 |
| lpush | keyspace lock | +11 / -17 / -18 | +29 / -8 / -2 |
| lpush | list push | 0 / +8 / +3 | -6 / -8 / -13 |
| lpush | firn and runtime, other | -4 / +19 / -4 | +11 / -1 / -5 |
| lpush | kernel | +7 / +2 / -8 | -21 / +23 / +9 |
| lpush | whole | +14 / +19 / -24 | +24 / -2 / -1 |

By the profile rule: for `hset` and `hget` the keyspace lock group accounts for
the compiler's share (84 against twice 17 and half of 107; 77 against twice 38
and half of 92), and the hash-map group does not by itself (21 and 31 are below
half of the whole). `try_entry`, the runtime's take of a key's lock, is 17% to
23% of every test's cycles and holds most of the lock group: both drivers write
or read the one key, so a longer statement shows as more waiting there. The
library map's functions are the largest firn-side change in `hget`:
`hash_map_lookup` costs 188 instructions and 159 cycles per request in `head`
against 150 and 127 in `cross` (+25% in both), and `hash_map_try_put` in `hset`
340 instructions and 233 cycles against 326 and 212 (+4% and +10%); the other
functions' instructions fall by about as much, so the total is unchanged (the
second profile run below finds the sign of the instruction difference again but
not its size, and `head`'s listing has fewer instructions in both functions). For
`lpush` `head` and `cross` differ by 14 cycles in 2434; no group accounts for
the compiler's share. For the source edits' share the mechanical rule names
firn's other code (+19 against twice 4, and half of 19), spread over many
symbols (`record_memory_peak` +10, `push_item` +9, `subscriber_refused` +6
cycles) at -1 instructions, but the whole profile's
`cross` minus `base` (+19) lies within the twin's (-24), so one profile does
not resolve the 2%. The profiles say where, not why: they cannot tell the
library map's extra instructions from the same instructions at more cycles, nor
the lock's waiting from its work; the second profile run annotates the
instructions.

**The oracle against GitHub's records.** The scratch script of the sample run
(124 calls) read the 14900K jobs of the five repositories around the session.
Of the 372 client runs, 352 overlap no step of another job, 19 overlap a heavy
step (all in attempt 1, readings 0.0391 to 0.9837, the `Test` and result steps of
two Firn-wf fast jobs) and 1 overlaps only light steps (attempt 1, 0.7885, the
`Complete job` and result steps of those jobs, seconds after their `Test` step).
The runs above T that overlap no heavy step are that one, in void attempt 1, and
`head` `rpush` of attempt 2 (0.0570), which overlaps no step of any of the five
repositories' 14900K jobs: its load is not from them and its source is not
named. T′ is the highest reading among the runs of clean attempts and profiles,
which overlap no heavy step (332 runs): 0.0161, so T′ = 0.0361. One run that
overlaps a heavy step reads in (T′, T]: `base` of attempt 1 (0.0391), so the
rule applies and the threshold for this run's decision is T′ = 0.0361. Deciding
again from the recorded readings alone: attempts 1 and 2 stay void (attempt 2's
run at 0.0570 is above T′ too), attempts 3 to 18 (highest 0.0161) and the 12
profiled runs (highest 0.0008) stay clean, so the 16 passes and every interval
above are unchanged. Every run that overlaps a heavy step and reads above T′ is
in a void attempt. The script's default T stays 0.05.

### Scratch run: the control fails when its loops are not seen

Run [45641919866](https://github.com/Ming-Research/Firn-wf/actions/runs/45641919866),
dispatched on 2026-11-25 at 15:09 UTC from the branch ref at `f446580`, the
scratch commit that pins the control's two busy loops to the server CPUs (2
and 4), which lie outside the other set; images `base` and `base-twin`, one
test, no profile. It failed in the Measure step: the control waited 604 s for
quiet (the other CPUs read 0.57 at the end), the log shows the loops' affinity
lists as `2` and `4`, and `compare-control.csv` has `2,0.9981,0.9985,0.05,2,13,failed`
with the message `the control failed: 2 busy loops on 2 4 of 13 other CPUs read
0.9985 against 0.9981 idle`. Both readings were above T, because another job had
the other CPUs busy; a control that asked only for a loaded reading above T would
have held here, and the rise the loops make (0.0004 against at least 0.077)
is what failed it. `compare-runs.csv` is empty: no image ran. The commit is
reverted by `01b91e6`, which leaves `redis-bench.sh` as it was in `bc0c257`.

## Second profile run, stated before measuring

**Question.** (a) In `hash_map_lookup` and `hash_map_try_put`, which
instructions does `head` have that `cross` does not, and where do the extra
cycles sit? (b) For `lpush` and `sismember`, whose `cross`/`base` is below 1:
are the hot functions of their path the same instructions in `base` and in
`cross`, differing only in where they lie, or does an edit change one?

**Run.** `redis-bench.yml`, mode `compare`, the placement and void rules above,
from the engine's ref `wf-try/work-145-26e0fade/call/37` (`51c7be4`, whose
script differs from the four-image run's `bc0c257` only by log lines and a
header comment), dispatched on 2026-11-25 at 15:23 UTC as run
[45644686875](https://github.com/Ming-Research/Firn-wf/actions/runs/45644686875)
(`headSha` read from the run) once the scratch run had ended; the images `base`, `base-twin`, `cross` and `head` as before;
tests `hset lpush hget sismember`, depth 16, 2 passes of 5 seconds (they size
the profiled runs and decide nothing); `profile_test`
`hset 16;lpush 16;hget 16;sismember 16`; `annotate`
`try_entry acquire_entry wf_bytes.make_key
wf_std.collections.hash_map.hash_map_try_put$instance$3d97ccfd215c1dd8
wf_std.collections.hash_map.hash_map_lookup$instance$5397e3116a8cc2ab
wf_commands.execute_client$instance$6afd30fe3ce447e9.resume
wf_commands.push_body wf_commands.hset_body wf_commands.run_hget.resume
wf_commands.record_memory_peak.resume wf_protocol.push_item`, the hottest
symbols of the four profiles above, whose names (instance hashes included) are
the same in all four images; no callers.

**Read.** Each `annotate-*.txt` lists a symbol's instructions with the share of
the symbol's samples at each, with its address. For (a), the `cross` and `head`
listings of each of the two library functions, aligned instruction by
instruction after the addresses and call targets are removed: the added and
removed instructions, and the instructions that carry the cycles in each. For
(b), the `base` and `cross` listings of each symbol of the `lpush` path:
identical when the instruction sequences are equal after the same removal, and
then with each image's start address modulo 64 and 4096 recorded; a symbol that
differs is shown with its difference. A path whose symbols are all identical,
with `cross`/`base` below 1, says the source edits changed no code of the path
and the effect lies in where it sits (H4); a symbol that differs names the edit.
The run attributes; it accepts or rejects nothing.

### Second profile run, results

Run [45644686875](https://github.com/Ming-Research/Firn-wf/actions/runs/45644686875)
succeeded (job 15:25:58 to 15:40:28 UTC; builds 4 min 1 s; Measure 9 min 56 s).
Its control held (idle 0.0242, loaded 0.1592), both passes were clean (highest
reading 0.0142), no profiled run was void (they read at most 0.0221), and every `annotate-*.txt` has the
listing of its symbol (176 files; those of a symbol the test does not run, such
as `hset_body` under `lpush`, are empty). Listings are compared with
rip-relative operands and call targets removed.

**The per-symbol instruction counts of the profiles do not give the size of a
difference.** The second run's whole-profile instructions per request are again
equal across the four images (`hset` 4950, 4945, 4946, 4962; `hget` 5094, 5115,
5110, 5111 for `base`, `base-twin`, `cross`, `head`), as the first run's were. The
per-symbol numbers move between the two runs by as much as the differences they
were read for: `hash_map_lookup` instructions per request were 154, 148, 150, 188
in the first run and 154, 152, 174, 187 in the second, so that `cross` reads 20
above `base` in the second run for an instruction sequence the listings below show
identical, and `head` reads above `cross` in both runs by 38 and 13 (`hash_map_try_put`:
14 and 13) where its listing has fewer instructions (91 against 97 and 314
against 321). So the first run's "+25% instructions in `hash_map_lookup`" is not
confirmed as a size, only as a sign that two runs repeat; the profile cannot say
whether the sign is a difference or the spread of sampling, and the program that
follows counts retired instructions exactly. The new home function is not a reason
for more probes: a model of the test's 99,996 keys (FNV-1a of `element:` and twelve
digits, the 262,144 buckets that 100,000 pairs need under both releases' growth rule,
linear probing) gives a present key 1.351 probes under `hash % buckets` and 1.288
under the top bits of `hash * 11400714819323198485`, with 0.789 and 0.815 of the
keys at their home bucket ([`probes.py`](../../experiments/hash-map-release/probes.py),
a model of the home function, not a replay of the server's growth). The cycles of the contended lock
move more: `try_entry` of `hset` was 573 and 639 per request in the first run
for `cross` and `head` and 599 and 890 in the second. `try_entry` is a `pause`
spin loop, and its samples measure the other driver's wait, not work.

**The source edits changed no instruction of any annotated function.** The eleven
annotated functions (`try_entry`, `acquire_entry`, `make_key`,
`hash_map_try_put`, `hash_map_lookup`, `execute_client`, `push_body`,
`hset_body`, `run_hget`, `record_memory_peak` and `push_item`) have the same
instruction sequence in `base`, `base-twin` and `cross`, from 93 to 1050
instructions each, `execute_client` and `push_body` of the `lpush` path among
them. Every one starts at a 64-byte boundary in all images and at a different
offset in the page: `try_entry` at 0x540, 0x340 and 0xcc0 of a page in `base`,
`cross` and `head`; `push_body` at 0x400, 0x2c0 and 0x1c0; `execute_client` at
0xc00, 0xa00 and 0x400. So the 2% `lpush` and 3% `sismember` that `cross` loses
against `base` come with no change to the code of the `lpush` path: the edits move
it, and what the listings cannot show is whether the move is the cause.

**What `head` changes in the annotated code.**

- `hash_map_lookup` and `hash_map_try_put` compute the home bucket with `imul`,
  `shr %cl` and `cmovae` where `base` and `cross` have a 64-bit `div` (and a
  32-bit `div` behind a test of the operands' width): 91 listed instructions
  against 97 and 314 against 321, none of the loop's. The probe loop is the same
  in both, with 96-byte slots (`lea (%rax,%rax,2)` and `shl $5`).
- `run_hget`, `hset_body` and `push_body` read the entry's fields 8 bytes further
  in (`0x40(%r13)` becomes `0x48(%r13)` and `0x48` becomes `0x50`; in `push_body`
  `0x50` becomes `0x58` too). `hset_body` and `push_body` have a frame 16 bytes
  larger (`0x58` to `0x68`, `0x78` to `0x88`) and copy 16 bytes through `movups`
  and `movaps` where `cross` copies one word; `push_body` has 306 instructions
  against 302. The map's header has a word more (`shift`), and what contains it
  moves.
- `try_entry`, `acquire_entry`, `make_key`, `execute_client`,
  `record_memory_peak` and `push_item` are identical between `cross` and `head`.

**Where `head` spends the cycles the map functions gain.** In each of the two
functions the samples of the cycles event sit mostly on one instruction (53% to
82%), the compare after the load of a probed slot's tag byte (`movzbl (slot),%edx`,
then `cmp $0x1,%edx`), the first touch of a slot in a table of 262,144 slots of 96
bytes (25 MB: the 100,000 fields of the test's one hash need more than the 98,304
that 131,072 buckets hold at three quarters, and both releases double a map at that
limit):

| function | image | samples | at the tag load | everywhere else |
| --- | --- | ---: | ---: | ---: |
| `hash_map_lookup` (`hget`) | base | 2385 | 1735 (72.7%) | 650 |
| | base-twin | 2446 | 1798 (73.5%) | 648 |
| | cross | 2842 | 2129 (74.9%) | 713 |
| | head | 3094 | 2544 (82.2%) | 550 |
| `hash_map_try_put` (`hset`) | base | 3891 | 2085 (53.6%) | 1806 |
| | base-twin | 3762 | 1981 (52.6%) | 1781 |
| | cross | 3903 | 2091 (53.6%) | 1812 |
| | head | 5785 | 3881 (67.1%) | 1904 |

The four profiled runs of a test hold the same number of requests (17,741,136 of
`hset`, 19,426,064 of `hget`), so samples compare. Away from the tag load `head`
has 163 samples fewer than `cross` in lookup (550 against 713: the division is
gone) and 92 more in put (1904 against 1812); the load itself has 415 samples more
in lookup (+19%) and 1790 more in put (+86%) than in `cross`, whose own lookup
lies 394 above `base`'s with the same instructions. One profiled run of each image
does not carry a per-function conclusion, but the direction is that of the
throughput intervals: the compiler's share of `hset` and `hget` shows as more
cycles at the first touch of a slot, and the instructions the map functions lose
do not make up for it.

**What this leaves open, and what is next.** A cost at the first touch of a slot
in a 25 MB table depends on how the table lies in memory (its alignment and
backing pages, the probe sequence of each key, the size of a slot) and not on the
instructions around it; the header of the map, and with it the entry that holds
it, has changed size. The next step is a program of a library map alone, the
`HashMap<Key, Bytes>` of this record without firn's server, built by both
releases and timed one against the other (cycles and retired instructions per
lookup and per replace).

### A library-map program on both releases, stated before measuring

Question. Does the release change alone, with no firn code, slow the library
`HashMap<Key, Bytes>` on the two operations the slow tests spend in it: a put on a
present field (HSET) and a lookup (HGET)? Firn's `Key`, `Bytes` and `KeyHash` are
used as they are (`pkg::bytes` is copied from `firn/bytes/`, not rewritten), and
the program ([`research/experiments/hash-map-release/`](../../experiments/hash-map-release/))
holds the 100,000 fields `element:` and twelve digits with three-byte values, built
by the map's own puts from four buckets as the server's hash is. Three entries come
from one source: `fill` builds the map, `replace` builds it and then puts a random
present field N times, `lookup` builds it and then looks up a random present field
N times, and each exits 1 unless it did what it counts (100,000 fields held; every
put a replacement; the lengths of N values found). Both releases build it with full
link-time optimization, as firn is measured.

Comparison. The two builds are run on one CPU in the same job, interleaved: each
repetition runs every entry once per executable, in an order that rotates. The old
build is also run from a byte-identical second copy, the twin, so that its ratio
to the old build is noise by construction. The cost of an operation is the entry's
user cycles minus those of `fill` (the same build's), over N, from `perf stat` on
the CPU; with no usable counters the cost is wall time and the result says so.
Retired instructions are counted the same way, to say whether a cycle difference
comes with an instruction difference. The summary is the mean log ratio of
new to old over the repetitions and its 95% interval (Student's t), for `replace`
and `lookup` alone.

Criterion, before any measurement. The program reproduces the slowdown for an
operation if the 95% interval of its new/old cycle ratio lies wholly above 1.000
and the twin's interval contains 1.000. The size is reported with the cycle
difference per operation, to read against the 21 to 106 cycles per request the
profiles show `head` above `cross` in `hash_map_try_put` and 27 to 32 in
`hash_map_lookup`; matching them is not a condition. It rejects the proposal that
the release alone makes the library map slower, in this program, if both
operations' intervals contain 1.000 or lie below it, or if the twin's interval
excludes 1.000 (the run's noise is then larger than what it was to read, and it is
not a result). A rejection leaves the profile evidence above as the whole of the
attribution, without a minimal example; it is not an exoneration of the release,
because this program has one thread, no network and no other keys, which the
server has. The machine is the shared 14900K: no void rule applies to this program,
and the share of the other CPUs' busy time over the measurement is printed with the
result, as the twin is its control.

The first run of the target is a sample (2,000,000 operations, 3 repetitions) to see
that the program builds and runs on both releases, and to time it; it counts for
nothing else. The operation count and repetitions of the run that counts are chosen
from its time and written here before that run.

The job is a scratch target of the Makefile, `hash-map-release`, which a later commit
removes; it always ends in failure so that the harness returns its output, which
stands in this record for the run's result.

#### Sample run, results, and the reading rule added before the counting run

The sample ([run 45654914775](https://github.com/Ming-Research/Firn-wf/actions/runs/45654914775),
job 30 s) did what it was for. After two source changes the program builds with
both releases, 2 to 3 s for each of the six builds, and exits 0 from all of them:
the pinned release rejects the binder `held`, a reserved word there that the old
release accepted, and the guard `if place < 12_u64` that the loop
bound decides (OP-5), and the program has neither now. `perf stat` counts user
cycles and retired instructions on the runner without root (a probe of `true`
counted 456,620 and 149,037). The sample took 2,000,000 operations and 3
repetitions on CPU 4.

It read nothing for cycles, and the reason is the machine: the other CPUs were
46.6% busy over the measurement (load average 14.3), and the cycles of one
operation in one run ranged from 498 to 783 between repetitions of the same
executable. The twin's intervals of the cycle ratio are [0.9507, 1.0600] for
`replace` and [0.8547, 1.1683] for `lookup`, 11% and 31% wide, against the new
build's [0.6986, 1.4852] and [0.8905, 1.0965]. Retired instructions are exact:
per operation `replace` 613.90 with the old build and 0.9948 of it with the new
one, `lookup` 475.21 and 0.9934, the twin 1.0000 both times (in `fill`, 60.01
million instructions with the old build and the twin, 59.19 million with the new
one). So the release does not execute more instructions of this program, as the
profiles also found, and its cycles are not read yet.

Two changes before the counting run, both about resolution and neither about the
result. The cost of `fill` is taken as its median over the repetitions of the same
executable, not that of the same repetition, because a run of `fill` (43 ms) is a seventh
of a short run and its own noise would enter every ratio. And a run is read for an
operation only if its twin's cycle interval lies within [0.985, 1.015] and holds
1.000, a width at which an effect of 2.5% (the smallest throughput difference of
the slow tests) can be seen; otherwise the operation is *unresolved* in that run,
and the run neither reproduces nor rejects. The reading of the criterion above
then is: `slower` (the new build's interval wholly above 1.000) reproduces;
`no difference seen` and `faster` reject it for this program. The counting run has
3,000,000 operations and 40 repetitions (about 100 s of measurement from the
sample's rates), and is repeated, up to three runs, while an operation is
unresolved; runs are not pooled, and if all three leave it unresolved the record
says that the shared machine did not allow a reading. The program prints the
reading, and `run.py selftest` holds a case for each (slower, faster, no
difference, the sample's own intervals as unresolved, a twin that excludes 1, a
twin too wide); each mutation tried, a threshold that reads a slower build from a
lower bound of 0.9 and a resolution of 0.5 to 2.0, fails the self-test.

#### First counting run, results

[Run 45661551696](https://github.com/Ming-Research/Firn-wf/actions/runs/45661551696)
(commit ce116cc, job 135 s): 3,000,000 operations, 40 repetitions, CPU 4. The other
CPUs were 64.3% busy over the measurement and the load average read 63, so the
machine was more loaded than in the sample.

| operation, cycles | old | new/old (95% interval) | twin/old (95% interval) | reading |
| --- | ---: | --- | --- | --- |
| `replace` | 589.71 per op | 1.0134 [0.9791, 1.0490] | 1.0149 [0.9840, 1.0468] | unresolved |
| `lookup` | 483.66 per op | 0.9493 [0.9164, 0.9833] | 0.9575 [0.9176, 0.9991] | unresolved |

The twin's intervals are 6% and 8% wide, against the 3% the rule allows, so neither
operation is read, and the two ratios move with their twins' (1.0134 with 1.0149,
0.9493 with 0.9575): the machine, not the build, moved them. Retired instructions are
exact and agree with the sample: the new build executes 0.9953 of the old build's
instructions per `replace` (612.15 per operation) and 0.9942 per `lookup` (473.79), the
twin 1.0000 of both, in 0 of 40 repetitions more; `fill` runs 60,013,027
instructions with the old build and 59,188,400 with the new one. Wall time is
unresolved as well (`replace` 1.0048 [0.9541, 1.0581] against its twin's 1.0240
[0.9710, 1.0799]; `lookup` 0.9164 [0.8462, 0.9923] against 0.9186 [0.8501, 0.9926]).

Read after the result and not part of the reading rule, from the 40 per-repetition
costs the target printed: the lowest cost and the mean of the ten lowest give
`replace` new/old 1.040 and 1.024 with twin/old 1.021 and 1.002, and `lookup` 0.954
and 0.971 with twin/old 1.007 and 0.987. They point to `replace` slower and
`lookup` faster in the new build by a few percent and do not agree with each other
within the twin's spread, so they say where a resolved run would look, not what it
finds. A second counting run follows with 2,000,000 operations and 100 repetitions,
the reading rule unchanged: shorter runs interleave the three builds more finely,
and more of them narrow the interval. The target now also prints the mean of the ten
lowest costs for cycles, described only.

#### Second counting run, results

[Run 45672136007](https://github.com/Ming-Research/Firn-wf/actions/runs/45672136007)
(commit fec1baa, job 197 s): 2,000,000 operations, 100 repetitions, CPU 4. The other
CPUs were 66.1% busy over the measurement and the load average read 37.

| operation, cycles | old | new/old (95% interval) | twin/old (95% interval) | reading |
| --- | ---: | --- | --- | --- |
| `replace` | 525.34 per op | 1.0071 [0.9866, 1.0281] | 1.0257 [1.0058, 1.0459] | unresolved: the twin excludes 1 |
| `lookup` | 414.88 per op | 0.9716 [0.9518, 0.9919] | 1.0087 [0.9857, 1.0324] | unresolved: the twin is 4.7% wide |

Retired instructions again agree with the earlier runs: the new build executes 0.9948
of the old build's instructions per `replace` (613.90 per operation) and 0.9934 per
`lookup` (475.21), the twin 1.0000, and the new build executes more in none of the 100
repetitions; `fill` runs 60,013,137 instructions with the old build and 59,188,390
with the new. Described and not read: the mean of the ten lowest costs gives
`replace` new/old 1.0146 against a twin's 1.0535, and `lookup` 0.9717 against 1.0015.
Wall time is unresolved as well (`replace` 1.0005 [0.9616, 1.0410] with its twin 1.0190
[0.9810, 1.0586]; `lookup` 0.9424 [0.9049, 0.9814] with its twin 0.9875 [0.9425, 1.0346]).

**Reading of the program.** Both counting runs leave both operations unresolved, so
by the rule stated before them the program neither reproduces the slowdown nor
rejects it, and by the owner's answer there is no third run. What the two runs do
fix is instructions: the library map built by the new release executes 0.47% to
0.66% fewer instructions per operation than the old release's build, exactly, in
three runs, which is the opposite of the instruction increase the first profile
read in `hash_map_lookup`. Cycles are not read, and the twin shows why: an identical
binary run from a second path costs up to 2.6% more cycles per `replace` (twin/old
1.0257, an interval that excludes 1) on this machine, whose other CPUs were 46% to 66%
busy, so a difference of the size looked for lies inside what the machine does to one
binary. The point estimates point where the first run's exploratory ones did: `lookup`
at 0.9493 and 0.9716 (new faster, with twins at 0.9575 and 1.0087) and `replace` at
1.0134 and 1.0071 (twins 1.0149 and 1.0257); none of them is a reading. This program
is therefore **not** a reproduction, and it is not handed over as one. It has one
thread, no entry holding the map, no lock and no network, which the server has; a
quiet machine, which the shared 14900K was not, may resolve it, and nothing here says
that the library map alone is exonerated.

## Attribution

What each slow cell's cause is, from the runs above, and what is still open.

**`hset` and `hget`: the release, that is the compiler with its standard library and
runtime.** `head` against `cross` (the same new source, the old release against the
new) is 0.9478 [0.9363, 0.9529] and 0.9435 [0.9340, 0.9624] in throughput over 16
clean passes (run 45631892786), and server CPU per request is 5.6% and 6.2% higher
(medians of the per-pass ratios, 16 and 15 of 16 passes); `cross` against `base` (new
source against old, old release) holds 1 for both, so neither Halo's nor firn's edits
are in it. The release is the only difference between `cross` and `head`, so it
includes the compiler, `std::collections::hash_map` and the runtime together; the
images cannot split them. What the profiles and listings add:

- the whole profile retires the same instructions per request in all four images
  (`hset` 4948 to 4951, `hget` 5168 to 5182 in the first run, 4945 to 4962 and 5094 to
  5115 in the second), so the release makes no image do more work in total, and IPC
  is lower in `head` (`hset` 1.842 against `cross`'s 1.917);
- the profile rule names the keyspace lock group (`try_entry`, the runtime's take of
  a key's lock) for the compiler's share, and not the hash-map group by itself;
- the code the two releases build differs, in the annotated functions, only in
  `hash_map_lookup` and `hash_map_try_put` (the home bucket from `imul`, a shift and a
  conditional move, where the old release divides) and in the offsets at which
  `hset_body`, `run_hget` and `push_body` read the entry, 8 bytes further in, and
  the size of their frames; the map's header has a word more (`shift`) and what
  holds it moved. The other annotated functions (`try_entry`, `acquire_entry`,
  `execute_client`, `make_key`, `record_memory_peak` and `push_item`) have the same
  instructions;
- the cycles the map functions gain sit at one instruction, the compare after the
  load of a probed slot's tag byte (+19% in `hash_map_lookup` and +86% in
  `hash_map_try_put` against `cross` in the second run), the first touch of a slot
  in a 25 MB table; a model of the test's keys gives the new home function fewer
  probes (1.288 against 1.351), not more;
- the program of the library map alone, built by both releases, executes fewer
  instructions in the new build and does not resolve a cycle difference (above).

So the cause of `hset` and `hget` is the release, and not firn's or Halo's code;
which part of the release (the map's code or layout, the runtime's lock, or how
either sits in the server) the evidence does not tell. The Whitefoot item that this
work hands to the owner states the firn numbers with their run ids and the listings'
differences, says that the library-only program does not reproduce it, and asks
Whitefoot to look at the pair on a quiet machine. Nothing in firn is changed for it
(constitution, article 4).

**`lpush` (and `sismember`): the source edits.** `cross` against `base` is 0.9793
[0.9730, 0.9997] for `lpush` and 0.9719 [0.9606, 0.9811] for `sismember`, `head`
against `cross` holds 1 for both, and `base-twin` against `base` holds 1. Every
annotated function of the `lpush` path (`execute_client`, `push_body`, `push_item`,
`acquire_entry`, `try_entry`, `record_memory_peak`) has the same instructions in
`base`, `base-twin` and `cross`; it starts at the same page offset in `base` and
`base-twin` and at a different one in `cross` and in `head`, so the edits changed no
code of the path; they moved it, and the cause is how it sits (Whitefoot#287, the
placement of code, is the related item there). Whether
Halo's or firn's edits move it was not split, because no edit changes an instruction
of the path, so a build with the old Halo could say only which side's addresses move.
No firn change that is not itself a placement of the same code is available, none is
made, and the 2% stays; whether to spend an experiment on placement is the owner's
choice.
