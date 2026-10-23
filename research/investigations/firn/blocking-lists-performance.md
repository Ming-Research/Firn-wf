# Blocking lists: performance

## The questions

1. Did Firn-wf#113 (the blocking list commands) slow the commands that existed
   before it? It sends the top-level list producers (`LPUSH` and `RPUSH` with
   their `X` forms, `LINSERT`, `LMOVE`, `RPOPLPUSH`, `RENAME`, `COPY`) through
   the held-command parts that `EXEC` and scripts use, because a push can wake a
   blocked client. Read from `run_list_producer` in
   `firn/commands/blocking-dispatch.wf`: each such command holds the keyspace,
   metadata, event hub and server state together, allocates a 64-byte effects
   buffer and stages the hub's wake-ups, and, with the append-only file on,
   copies the staged effects into it, where before it appended in place. Which of
   these costs time is not measured. The design decision for the staged copy is
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
main the PR starts from (`ac291c1a1`, the revision the PR's last merge of main
brought in), so the two images differ by the PR alone. A later merge of main that
leaves the PR's own change as it is does not repeat a measurement.

### Cost on existing commands (question 1)

- **Cells.** The 19 tests of the redis-benchmark default suite of Redis 7.0
  (`SUITE_DEFAULT` in `redis-bench.sh`, 20 rows with the `LRANGE` fill, which
  `compare` sends outside the timed span), plus `linsert` and `lmove`, which
  `compare_client` defines for this measurement; each at depth 16 and depth 1 on 2
  server CPUs: 42 cells, with the append-only file off (the redis-bench
  default). With the file on (`appendfsync everysec`, rewrites off, a fresh
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
- **Chance.** 42 cells each have about a 2.5% chance of lying wholly below 1
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
  seconds without an element. RESP2, loopback.
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
  with an unknown-command error is left out of the workload's cells.
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
| A1 | `compare`, `base`, `head`, `head-twin`, the 21 tests, depths 16 and 1, 2 server CPUs, 6 passes of 5 s, profile `lpush 16;lmove 16` | question 1, file off | about 70 min (the 38-cell run 39573904480 took 60) |
| A2 | `compare`, the same images, `aof`, tests `set lpush rpush linsert lmove`, the same profile | question 1, file on | about 20 min |
| B | `workloads` with `workload_images`, `head` and `head-twin` only, `blocking-handoff`, depths 16 and 1, 50 connections, 6 passes of 10 s | the floor of question 2 | about 10 min |
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
