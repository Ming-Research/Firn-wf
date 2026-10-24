# Blocking lists: performance

## The questions

1. Did Firn-wf#113 (the blocking list commands) slow the commands that existed
   before it? Its first version (`da1e2c3`, measured in A1 below) sent all the
   top-level list producers (`LPUSH` and `RPUSH` with their `X` forms,
   `LINSERT`, `LMOVE`, `RPOPLPUSH`, `RENAME`, `COPY`) through the held-command
   parts that `EXEC` and scripts use, because a push can wake a blocked client.
   Read from `run_list_producer` in `firn/commands/blocking-dispatch.wf`: each
   such command holds the keyspace, metadata, event hub and server state
   together, allocates a 64-byte effects buffer and stages the hub's wake-ups,
   and, with the append-only file on, copies the staged effects into it, where
   before it appended in place. The head sends there only the writes that can
   create a list (`run_list_write` in the same file); `LINSERT`, `LPUSHX`,
   `RPUSHX` and the pushes and moves onto a live list hold only their keys'
   entries, as on main. Which of these costs time is measured for the first
   version in the results below; the head's own runs are made before the merge.
   The design decision for the staged copy is
   [firn-q-list-producer-staging](../../../design/firn/append-only-file.md),
   ruled A: keep the staged copy, provided this record's measurement shows no
   regression; a regression caused by the copy itself falls back to restoring
   the in-place append, one caused by the allocation to a buffer the connection
   reuses.
2. How fast does firn hand list elements to blocked clients, against Redis,
   Valkey, Dragonfly and Garnet, and what does the runtime's listening lock cost
   there? Source reading of the design says one write wakes every context
   listening on the table, not only the ones waiting for its key, and that the
   listening lists share one lock (board item coord-wfbl-03-04). That is a
   prediction, not a result: the pub/sub measurement of the same runtime
   ([pubsub-throughput.md](pubsub-throughput.md)) put 30% of firn's cycles in
   the runtime's shared-acquire function and firn at 1/19 of the fastest
   competitor at depth 16, so a result below the criterion below is expected
   here as well, and not evidence of a defect of this change.

## Criteria, stated before measuring

The definitions of medians, twins and intervals are those of the firn
investigation's [criteria](DESIGN.md#criteria-stated-before-measuring); the
placement is [measurement-sessions](../../../design/firn/measurement-sessions.md):
on the 14900K the servers on CPUs 2 and 4 (P-cores) and the clients on CPUs
16-31 (E-cores), for every line and the noise floor alike. Every run is
dispatched on the head commit of the PR that merges; the base is the revision of
main that the PR's last merge of main brought in (`05f7e89`, the point where the
PR and main meet and the tip of main when the PR merged it, with the stream
commands of main in both images), so the two images differ by the PR alone. A
later merge of main that leaves the PR's own change as it is does not repeat a
measurement.

### Cost on existing commands (question 1)

- **Cells.** The 20 tests of the redis-benchmark default suite of Redis 7.2
  (`SUITE_DEFAULT` in `redis-bench.sh`, 21 rows with the `LRANGE` fill, which
  `compare` sends outside the timed span; `xadd` is one of them, a stream
  command of main that the head image's dispatcher reaches after trying the
  four blocking commands), plus `linsert` and `lmove`, which `compare_client`
  defines for this measurement; each at depth 16 and depth 1 on 2 server CPUs:
  44 cells, with the append-only file off (the redis-bench default). With the
  file on (`appendfsync everysec`, rewrites off, a fresh
  directory per run, which the instrument checks with INFO) the cells are
  `set`, `lpush`, `rpush`, `linsert` and `lmove` at both depths: the copy into the file
  exists only there, `set` is the control that the PR does not touch.
- **Commands.** `linsert` is `LINSERT ins<depth>:<n> AFTER pivot xxx` on one of
  100,000 short lists filled with copies of the pivot outside the timed span (a
  list grows by a few hundred elements over the run, so the pivot scan stays one
  step); `lmove` is `LMOVE mylist mylist RIGHT LEFT` on a list of five, which
  rotates and keeps its length. Both send the 3-byte payload of the other tests
  through 50 connections. Before the timed runs, each image's server receives 100
  `LINSERT`s and 101 `LMOVE`s from redis-benchmark itself, and the run fails
  unless the lists show the result (`verify_list_forms`: 101 elements, and the
  last element moved to the front): a form that missed its key or wrote nothing
  would otherwise be measured as a fast miss.
- **Images.** `base`, `head` and `head-twin` (the head again, as the noise
  control: two builds of one source, measured in the same interleaving), 6
  passes of 5 s per cell, each pass in the reverse order of the one before, no
  `reference` line. `summary.py ratios` prints, per cell, head/base with the 95%
  interval of the mean log ratio over the passes, and twin/head likewise.
- **Regression.** A cell whose head/base interval lies wholly below 1 is a
  regression, of any size; wholly above 1, an improvement; across 1, no detected
  change (the engineering rule; no threshold is picked). A cell whose client is
  the limit (the client CPUs busy more than 0.90, as `compare-limited.csv`
  marks) is reported and not judged. A cell that does not decide is repeated
  with more passes before it is called.
- **Chance.** 44 cells each have about a 2.5% chance of lying wholly below 1
  with no change in the code, so one cell in a run may. The control for that is
  the twin of the same cell: a cell whose twin/head interval lies wholly off 1
  in the same run, or whose twin half-width is above the floor (Noise), does not
  count and is repeated with more passes. Every other cell wholly on the slower
  side is a regression, whichever command it is: the acceptance row (b) sends it
  to the owner with its numbers and with the number of cells whose twin lay off
  1 in the run, and it is not read away against that number.
- **Noise.** The class has no recorded floor. The only earlier measurement of
  these cells is run 39573904480 (pub/sub, same machine, settings and tool): the
  twin half-widths of its 38 cells run from 0.5% to 5.2% (`zpopmin`, depth 16).
  A cell here whose twin half-width is above 5.2% does not count and is repeated
  with more passes; this run's twin half-widths are then proposed to the
  architect as the class floor for the record of the 14900K.
- **What rejects.** A cell wholly on the slower side, in either AOF setting,
  rejects the claim its command rests on. For `lpush`, `rpush`, `linsert` and
  `lmove` it is "the staged copy costs nothing measurable". For every other cell
  it is "an ordinary command's path is unchanged", which the design states for
  `GET` and `SET` (blocking-lists.md: they gain no shared target and no registry
  lookup) and which the other commands that are not list producers rest on
  alike; the cause is then not the copy and the profile below does not apply
  to it. For the list producers, the profile of
  the first `lpush 16` and `lmove 16` pass of each image (`perf record`, flat,
  no call stacks) separates the hypotheses of question 1: share of cycles in the
  allocator and `memcpy` against share in the shared-acquire function and
  the hub. A share that is near zero in `base` and large in `head` names the
  part; no cause is concluded from one run.

### Hand-off to blocked clients (question 2)

- **Workload `blocking-handoff`** (`research/experiments/redis-bench/workload/`).
  50 consumer connections each hold `BLPOP bench:q:<i> 0` on a key of their own
  (finite timeouts are refused by firn until firn-req-extended-float). One
  producer connection pushes 64-byte elements with `LPUSH`, `depth` pushes to
  `depth` different keys per batch, waits for all the replies, and pushes to a
  key only after its consumer has received its previous element: no key holds
  more than one element. A consumer sends its next `BLPOP` as soon as it has an
  element. Each element starts with its number among its key's elements and its
  send time; a consumer checks the key, the length, the padding and that the
  numbers arrive in sequence, so a repeated, reordered or altered element
  fails the run at once. A lost element is found by silence: the producer
  pushes nothing more to its key, so once the others have everything the
  consumers receive nothing, and the run fails when a client thread has gone 30
  seconds without an element. A competitor that fails this way is recorded as a
  skipped line (`skip,<line>,blocking-handoff incomplete`) and firn failing it
  stops the run. A run ends with one more element for every key, the drain,
  after which a consumer does not block again, so that no `BLPOP` outlives its
  connection; the warm-up, the measured run and the next cell on the same
  server all use the same keys. Garnet 2.2.3 serves the `BLPOP` of a connection
  that has closed. A probe on a hosted runner (run 40019254666): a connection
  blocks on a key and closes, a second blocks on the same key, a push is
  answered `:1`, the second receives nothing and the list is empty. The client
  without the drain (run 39941574831: warm-up 1 s, then the measured run, on a
  fresh Garnet) stopped at its first measured run with every list empty: the
  first push of the run went to a connection the warm-up had closed. RESP2,
  loopback.
- **Measure.** The rate is the elements received per second over the span from
  the start of the run to the last arrival. An element's latency runs from just
  before the producer writes its batch to its arrival at its consumer; the p99 is
  over all elements. A cell is the workload at one depth; a line's value is the
  median over 6 interleaved passes (10 s after a 5 s warm-up, discarded), each
  pass in the reverse order of the one before.
- **Rate and latency.** The criteria of the pub/sub measurement, which `summary.py
  pubsub` applies: at depth 16 firn's rate is at least 1.4 times the fastest
  competitor's, at depth 1 at least the fastest's; firn's p99 is no higher than
  that of the fastest competitor of the cell; a cell whose client work fraction
  has a median above 0.90 in firn's line or in the fastest competitor's asks only
  that firn is not behind, and its p99 is reported and not judged.
- **Competitors.** The lines and the pins of the pub/sub record: `reference`
  (Redis 7.2.16), `redis-latest`, `valkey-latest`, `valkey-io-latest`,
  `dragonfly-latest`, `garnet-latest`, each with its defaults on the server's 2
  CPUs apart from the thread counts `latest.sh` sets. A server that answers `BLPOP`
  with an unknown-command error is left out of the workload's cells. A server
  that stops handing out elements gets a skip line for the cell it stops in and
  no value in the cells after it in that pass; the fastest competitor of a
  cell is the fastest of the lines that have a value in it, and a line that
  stopped in some passes only is summarised over the passes that gave a value
  (`summary.py` reads the rows present).
- **Noise.** A first run measures `head` against `head-twin` alone (6 passes, both
  depths) and takes the widest half-width of the twin/head intervals as the floor
  of this workload class; the comparison run follows with `--floor` set to it, and
  a cell whose twin half-width is above the floor does not count.
- **Not a criterion: the lock.** The comparison run sets `sched_report`, so each
  firn line's server log, kept as `workload-server-<line>-<cpus>-<pass>-blocking-handoff.txt`,
  ends with the runtime's scheduling counts, and the first pass of each firn
  cell is profiled. Whether either holds a count of contexts woken per write or
  the wait on the listening lock is not known before the run; if neither does,
  that is the missing instrument for coord-wfbl-03-04, filed there and not built
  here. The profile's share in the shared-acquire function is reported as it is.
- **Not covered.** Finite timeouts, several blocked clients on one key,
  `BRPOP`, `BRPOPLPUSH` and `BLMOVE`, several producers, RESP3, the append-only
  file on, service by `EXEC` or a script, and a push that finds the consumer not
  yet blocked again (the client cannot tell that path from a wake-up, and every
  server runs the same protocol; its share is unmeasured).
- **What rejects.** A cell below its threshold stands as measured, with its gap.
  The acceptance row (b) sends it to the owner with the numbers before the merge.

## The runs

Each is a dispatch of `redis-bench.yml` on the PR's head commit; the runner
orders them on the 14900K, one at a time.

| run | mode and inputs | answers | expected duration |
|---|---|---|---|
| A1 | `compare`, `base`, `head`, `head-twin`, the 22 tests, depths 16 and 1, 2 server CPUs, 6 passes of 5 s, profile `lpush 16;lmove 16` | question 1, file off | about 70 min (the 38-cell run 39573904480 took 60) |
| A2 | `compare`, the same images, `aof`, tests `set lpush rpush linsert lmove`, the same profile | question 1, file on | about 20 min |
| B | `workloads` with `workload_images`, `head` and `head-twin` (and the `reference` line that `workload_images` adds), `blocking-handoff`, depths 16 and 1, 50 connections, 6 passes of 10 s | the floor of question 2 | about 10 min |
| C | `workloads` with `workload_images`, the six competitor lines and `head` and `head-twin`, `sched_report`, profile | question 2 | about 40 min |

The instrument was tried first on GitHub-hosted runners, for function only (no
rate from them counts). A hosted runner has two physical cores and the automatic
placement skips the cell for want of a distinct client core, so those runs name
`server_cpus` and `client_cpus`. At 7cad8fc: run 39927345922 (`workloads`,
`blocking-handoff` against `reference` and firn, 20 consumers, depths 16 and 1)
and run 39927349103 (`compare` with the append-only file on: `set`, `lpush`,
`linsert`, `lmove`). The check of the forms was watched to fail and to pass at
its commit 4357091 (run 39929633261, the real forms: passes and measures); with
the `LINSERT` pivot changed to one that is not in the list it stopped the run
with `LLEN '1'` (run 39929636578), and with `LMOVE` changed to `LEFT LEFT` it
stopped the run with the list `a b c d e` (run 39929639885). The two broken
forms were on throwaway branches, deleted after the runs.

The hand-off against the competitors was tried the same way (function only):
run 39935935258 at 232cb66 (`workloads`, `blocking-handoff`, 20 consumers,
depths 16 and 1, warm-up 1 s, 3 s runs, one pass, `redis-latest`,
`valkey-latest`, `valkey-io-latest`, `dragonfly-latest`, `garnet-latest` and
firn) stopped at Garnet's first cell with `no element for 30 seconds`, the
cause given under the workload above. The same inputs at aa2a6b7 (run
40020579726, the drain in place) gave both cells for all six lines, Garnet's
94000 and 45059 elements included. The drain test was watched to fail on two
broken clients (run 40020673115, throwaway branch deleted after it): a
consumer that blocks again after the drain (222 bytes sent after it, 0
expected), and a producer that never drains (the run ends in `server closed
connection`). The harness's reaction to a line
that stops handing out elements was run with a client whose stall limit is 1 ms
(run 40021985233, throwaway branch deleted after it): `valkey-latest` finished
depth 16, stopped at depth 1 and was recorded as `skip,valkey-latest,blocking-handoff
incomplete: ... (connections=20 pipeline=1)`, and firn, next in the order, stopped
the run with `firn-1 stopped handing out elements in blocking-handoff`.

## Results

Every run below is on the 14900K at `da1e2c3`, the PR's first version, with the
servers on CPUs 2 and 4 and the clients on CPUs 16-31; the runs of the head that
merges are added after them.

### B: the floor of the hand-off class

Run 40043320074 (`workloads` with `workload_images`, `head` and `head-twin`,
`blocking-handoff`, 50 consumers, depths 16 and 1, 6 passes of 10 s). The run
also carried the `reference` line (Redis 7.2.16), which `workload_images`
keeps; the figures are `summary.py pubsub` on the run's `pubsub.csv`.

| depth | head (elements/s) | head-twin (elements/s) | twin / head, median | 95% interval | half-width |
|---|---|---|---|---|---|
| 16 | 101304 | 100934 | 0.9944 | 0.9898-1.0076 | 0.89% |
| 1 | 78384 | 78261 | 0.9985 | 0.9875-1.0098 | 1.12% |

The floor of the class is 1.12%, the wider half-width, and the comparison run C
is summarised with it. The `reference` line, for later comparison with C: Redis
7.2.16 hands over 286191 elements/s at depth 16 (p99 0.056 ms) and 114052 at
depth 1 (p99 0.010 ms), against firn's 101304 (p99 0.760 ms) and 78384 (p99
0.081 ms): firn is 0.354 and 0.687 of Redis, and the cell criteria need 1.4 and
1.0. The client work fraction is 0.044 and 0.300 for firn, so these cells are
not client-limited.

### A1: cost on existing commands, append-only file off

Run 40049518328 (`compare`, `base` = `05f7e89`, `head` = `da1e2c3`, `head-twin`
= `da1e2c3`, the 22 tests, depths 16 and 1, 2 server CPUs, 6 passes of 5 s,
profile `lpush 16;lmove 16`). Per cell, `summary.py ratios`: head/base median
over the passes with the 95% interval of the mean log ratio, and twin/head
likewise. 44 cells: 11 faster, 24 no detected change, 9 slower; the twin
interval lies across 1 in 43 cells and wholly below 1 in one (`lmove` depth
16, 0.9579-0.9960). The client is the limit (CPUs busy above 0.90) in
`lrange_500` and `lrange_600` at depth 16, which are not judged (both are
faster or unchanged).

The 8 cells of the four list producers are wholly on the slower side, and
`lrange_100` at depth 1 is a ninth, by 1.5% (0.9848, interval 0.9758-0.9987,
twin interval 0.9942-1.0237):

| test | depth | base (requests/s) | head (requests/s) | head/base | 95% interval |
|---|---|---|---|---|---|
| lpush | 16 | 4242766 | 1330255 | 0.3135 | 0.3002-0.3284 |
| rpush | 16 | 4206656 | 1328298 | 0.3167 | 0.3076-0.3277 |
| linsert | 16 | 2168374 | 686202 | 0.3130 | 0.3066-0.3225 |
| lmove | 16 | 3952587 | 1209726 | 0.3077 | 0.2931-0.3205 |
| lpush | 1 | 776783 | 674080 | 0.8761 | 0.8438-0.9319 |
| rpush | 1 | 770692 | 676096 | 0.8789 | 0.8634-0.9095 |
| linsert | 1 | 684838 | 587560 | 0.8631 | 0.8407-0.8836 |
| lmove | 1 | 771334 | 643328 | 0.8321 | 0.8191-0.8418 |

The file is off, so the staged copy into it does not run, and the 64-byte
buffer is allocated in both images' paths alike. The profile of the first
`lpush 16` pass has `wf_cmap_hold` (the runtime function that takes the
whole-keyspace hold) at 51.16% of the cycles in `head` and `try_entry` (the
per-key entry hold) at 14.13% in `base`; `lmove 16` has `wf_cmap_hold` at
48.16% in `head` and `try_entry` at 17.05% in `base`. The allocator and
`memcpy` are each below 1% in `head` (`malloc` 0.57%, `__memmove_avx_unaligned_erms`
0.36% in `lpush 16`). So the cost of these commands is the whole-keyspace,
metadata, event-hub and server-state hold that
[blocking-lists.md](../../../design/firn/blocking-lists.md) gives every list
producer (`run_list_producer`), which serialises the two server contexts,
and not the staged copy or its buffer: neither of the other two options of
[firn-q-list-producer-staging](../../../design/firn/append-only-file.md) (restore
the in-place append, reuse a buffer) would remove it. This is read from one
profile of one pass per image; the ratio of the cells is the measurement,
the attribution to the hold is inferred and would be confirmed by a
`head` variant whose producers take only the key's entry.

The other 35 cells have no interval wholly on the slower side. The 11 faster
cells are all at depth 16 (0.6% to 5.2% above `base`, `get` 1.0468,
`ping_mbulk` 1.0523); their cause was not investigated, since the criteria
ask only about the slower side.

Table, all 44 cells (head/base and twin/head: median, 95% interval):

| test | depth | head/base | interval | verdict | twin/head | interval |
|---|---|---|---|---|---|---|
| get | 1 | 1.0160 | 0.9931-1.0255 | no detected change | 0.9999 | 0.9883-1.0167 |
| get | 16 | 1.0468 | 1.0370-1.0581 | faster | 1.0024 | 0.9898-1.0098 |
| hset | 1 | 1.0120 | 0.9917-1.0451 | no detected change | 0.9991 | 0.9541-1.0271 |
| hset | 16 | 1.0089 | 0.9766-1.0338 | no detected change | 0.9977 | 0.9908-1.0099 |
| incr | 1 | 1.0028 | 0.9911-1.0215 | no detected change | 0.9959 | 0.9891-1.0094 |
| incr | 16 | 1.0116 | 1.0057-1.0167 | faster | 0.9985 | 0.9920-1.0102 |
| linsert | 1 | 0.8631 | 0.8407-0.8836 | slower | 1.0102 | 0.9913-1.0253 |
| linsert | 16 | 0.3130 | 0.3066-0.3225 | slower | 0.9983 | 0.9629-1.0253 |
| lmove | 1 | 0.8321 | 0.8191-0.8418 | slower | 1.0051 | 0.9927-1.0119 |
| lmove | 16 | 0.3077 | 0.2931-0.3205 | slower | 0.9790 | 0.9579-0.9960 |
| lpop | 1 | 1.0158 | 0.9824-1.0625 | no detected change | 0.9971 | 0.9810-1.0178 |
| lpop | 16 | 1.0321 | 1.0127-1.0598 | faster | 0.9898 | 0.9798-1.0137 |
| lpush | 1 | 0.8761 | 0.8438-0.9319 | slower | 0.9991 | 0.9895-1.0141 |
| lpush | 16 | 0.3135 | 0.3002-0.3284 | slower | 1.0153 | 0.9731-1.0492 |
| lrange_100 | 1 | 0.9848 | 0.9758-0.9987 | slower | 1.0114 | 0.9942-1.0237 |
| lrange_100 | 16 | 1.0179 | 1.0023-1.0442 | faster | 0.9996 | 0.9735-1.0221 |
| lrange_300 | 1 | 0.9983 | 0.9916-1.0076 | no detected change | 1.0087 | 0.9989-1.0206 |
| lrange_300 | 16 | 1.0103 | 1.0074-1.0151 | faster | 0.9999 | 0.9966-1.0048 |
| lrange_500 | 1 | 1.0062 | 0.9963-1.0128 | no detected change | 0.9888 | 0.9825-1.0030 |
| lrange_500 | 16 | 1.0021 | 0.9968-1.0062 | no detected change | 0.9998 | 0.9959-1.0064 |
| lrange_600 | 1 | 1.0009 | 0.9970-1.0040 | no detected change | 0.9967 | 0.9896-1.0010 |
| lrange_600 | 16 | 1.0062 | 1.0013-1.0093 | faster | 0.9973 | 0.9943-1.0029 |
| mset | 1 | 1.0107 | 0.9899-1.0209 | no detected change | 0.9955 | 0.9802-1.0120 |
| mset | 16 | 1.0305 | 0.9674-1.0960 | no detected change | 0.9826 | 0.9431-1.0422 |
| ping_inline | 1 | 1.0031 | 0.9968-1.0093 | no detected change | 0.9981 | 0.9860-1.0075 |
| ping_inline | 16 | 1.0352 | 1.0205-1.0490 | faster | 1.0006 | 0.9917-1.0235 |
| ping_mbulk | 1 | 1.0065 | 0.9885-1.0265 | no detected change | 0.9976 | 0.9772-1.0076 |
| ping_mbulk | 16 | 1.0523 | 1.0429-1.0625 | faster | 0.9992 | 0.9886-1.0166 |
| rpop | 1 | 1.0101 | 0.9902-1.0510 | no detected change | 1.0015 | 0.9756-1.0193 |
| rpop | 16 | 1.0205 | 1.0105-1.0299 | faster | 0.9976 | 0.9827-1.0183 |
| rpush | 1 | 0.8789 | 0.8634-0.9095 | slower | 0.9905 | 0.9788-1.0075 |
| rpush | 16 | 0.3167 | 0.3076-0.3277 | slower | 0.9895 | 0.9489-1.0714 |
| sadd | 1 | 1.0242 | 0.9992-1.0723 | no detected change | 0.9848 | 0.9158-1.0450 |
| sadd | 16 | 1.0102 | 0.9962-1.0389 | no detected change | 1.0047 | 0.9847-1.0119 |
| set | 1 | 1.0004 | 0.9942-1.0155 | no detected change | 1.0036 | 0.9935-1.0160 |
| set | 16 | 1.0183 | 0.9888-1.0312 | no detected change | 1.0003 | 0.9890-1.0250 |
| spop | 1 | 1.0060 | 0.9869-1.0286 | no detected change | 0.9990 | 0.9500-1.0300 |
| spop | 16 | 1.0474 | 1.0152-1.0835 | faster | 0.9796 | 0.9340-1.0301 |
| xadd | 1 | 1.0022 | 0.9828-1.0184 | no detected change | 0.9965 | 0.9863-1.0145 |
| xadd | 16 | 1.0114 | 0.9406-1.0514 | no detected change | 0.9958 | 0.9465-1.0578 |
| zadd | 1 | 0.9977 | 0.9782-1.0197 | no detected change | 0.9925 | 0.9864-1.0006 |
| zadd | 16 | 1.0035 | 0.9871-1.0219 | no detected change | 0.9962 | 0.9711-1.0093 |
| zpopmin | 1 | 1.0136 | 0.9970-1.0268 | no detected change | 0.9927 | 0.9764-1.0029 |
| zpopmin | 16 | 1.0383 | 1.0179-1.0820 | faster | 0.9917 | 0.9622-1.0036 |
