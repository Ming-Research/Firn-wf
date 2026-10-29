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
   before it appended in place. The second version (`7a4e4de`) sent there only
   the writes that can create a list. The third (`12dbbc5`, measured at
   `2d91961`) sends there only a push or move that creates a list on a key
   where a blocked client waits, `RENAME`, `RENAMENX`, `COPY` and a promoted
   connection's commands: every push or move that would create a list takes the
   event hub as well, in a second statement under its own entry hold, asks the
   registry (`key_awaited` in `firn/commands/blocking-runtime.wf`) whether a
   client waits on that key, and leaves for `run_list_producer` only if one
   does (`run_push` and `run_lmove` in `firn/commands/lists.wf`); the
   dispatcher calls them directly, as it called `run_push` in `main`. The
   fourth (the head of the PR after the measurement of `2d91961`) puts a bound
   in front of that second statement: Meta keeps an upper bound on the keys
   blocked clients wait on (`waiter_keys_bound`), a push or move that would
   create a list reads it in its first statement, and at zero creates the list
   in place there; only above zero does it take the second statement. In both
   versions `LINSERT`, `LPUSHX`, `RPUSHX`, the pushes and moves onto a live list
   and the creating ones where nobody waits append in place, as on `main`. Which
   of these costs time is measured in the results below, for the earlier
   versions and for the third (`2d91961`); the fourth version's runs are added
   after they are made.
   The design decision for the staged copy is
   [firn-q-list-producer-staging](../../../design/firn/append-only-file.md),
   ruled A: keep the staged copy, provided this record's measurement shows no
   regression; a regression caused by the copy itself falls back to restoring
   the in-place append, one caused by the allocation to a buffer the connection
   reuses. Since the third version no cell below runs the staged copy: it runs
   only when a client waits or for `RENAME`, `COPY` and promoted connections,
   and nothing here measures its cost.
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
main that the PR's last merge of main brought in (`dd8c06c` for the fourth
version, the tip of main when the PR last merged it; `7cf12df` for the runs of
the third version below, with the stream commands of main in both images), so
the two images differ by the PR alone; the first two A1 runs below, made before
the merge of `7cf12df`, used `05f7e89`, the tip of the merge before. A later
merge of main that leaves the PR's own change as it is does not repeat a
measurement.

### Cost on existing commands (question 1)

- **Cells.** The 20 tests of the redis-benchmark default suite of Redis 7.2
  (`SUITE_DEFAULT` in `redis-bench.sh`, 21 rows with the `LRANGE` fill, which
  `compare` sends outside the timed span; `xadd` is one of them, a stream
  command of main that the dispatcher of the head image reaches, since the
  fourth version, before it tries the four blocking commands; the third reached
  it after them), plus `linsert`, `lmove` and `lpush_new`, which
  `compare_client` defines for this measurement; each at depth 16 and depth 1 on
  2 server CPUs: 46 cells, with the append-only file off (the redis-bench
  default). With the file on (`appendfsync everysec`, rewrites off, a fresh
  directory per run, which the instrument checks with INFO) the cells are
  `set`, `lpush`, `rpush`, `linsert`, `lmove` and `lpush_new` at both depths: the
  copy into the file exists only there, `set` is the control that the PR does
  not touch. Of the list writes, none runs the held path with its staged copy
  on the head: `lpush`, `rpush`, `linsert` and `lmove` write onto a list that
  exists (the first version of these cells measured below ran that path too)
  and `lpush_new` creates a list where no client waits. Since the fourth version
  none of the five takes the event hub, because no client is blocked while the
  run lasts: `lpush_new` reads Meta's bound on the keys blocked clients wait on,
  found zero, in the statement it holds already, in addition to what `main` does;
  the other four run as `main` does. The third version took the event hub in a
  second statement for `lpush_new`.
- **Commands.** `linsert` is `LINSERT ins<depth>:<n> AFTER pivot xxx` on one of
  100,000 short lists filled with copies of the pivot outside the timed span (a
  list grows by a few hundred elements over the run, so the pivot scan stays one
  step); `lmove` is `LMOVE mylist mylist RIGHT LEFT` on a list of five, which
  rotates and keeps its length. Both send the 3-byte payload of the other tests
  through 50 connections. `lpush_new` is `LPUSH new<depth>:<n> xxx` with `n`
  drawn from 2,000,000,000 keys (`-r` is a C int in redis-benchmark), so that
  all but a fraction of a percent of the requests of a run of some ten million
  push onto a key that does not exist and create its list; the lists stay in the
  server for the rest of its pass, and the runs below name the test last in
  `COMPARE_TESTS`, so that the larger keyspace changes no other cell. Before the
  timed runs, each image's server receives 100 `LINSERT`s, 101 `LMOVE`s and 100
  `LPUSH`es of `lpush_new` from redis-benchmark itself, and the run fails unless
  the keys show the result (`verify_list_forms`: 101 elements, the last element
  moved to the front, and at least 99 new keys): a form that missed its key or
  wrote nothing would otherwise be measured as a fast miss.
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
- **Chance.** 46 cells each have about a 2.5% chance of lying wholly below 1
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
  rejects the claim its command rests on. For `lpush_new` it is "creating a list
  where no client waits, with one event-hub hold and one registry check added to
  the entry hold, costs nothing measurable" (A1 measures it with the file off,
  A2 with the in-place append to the file). For `lpush`,
  `rpush`, `linsert` and `lmove` it is "a write onto an existing list runs as it
  did before the PR". For every other cell it is "an ordinary command's path is
  unchanged", which the design states for `GET` and `SET` (blocking-lists.md:
  they gain no shared target and no registry lookup) and which the other
  commands that are not list producers rest on alike; the cause is then not the
  copy and the profile below does not apply to it. For the list producers, the
  profile of the first `lpush 16`, `lmove 16` and `lpush_new 16` pass of each
  image (`perf record`, flat, no call stacks) separates the hypotheses of
  question 1: share of cycles in the allocator and `memcpy` against share in the
  shared-acquire function and the hub. A share that is near zero in `base` and
  large in `head` names the part; no cause is concluded from one run.

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

### The fourth version (stated before its runs)

The fourth version is the head that merges. Its runs are A1, A2 and C of the
table below, dispatched on the head commit that carries this section, with the
images `base` (`dd8c06c`, the tip of main that the PR last merged), `head` and
`head-twin`, in the cells, depths, passes and placement stated above.

- **Question.** Does reading Meta's bound in the first statement, in place of
  the second statement that takes the event hub, bring a push that creates its
  list where nobody waits back to the rate of `main`; and does trying the stream
  commands before the blocking ones bring `xadd` at depth 1 back to it?
- **Prediction (inferred from one profile of the third version, not measured).**
  The four lock symbols of the third version's second statement, about 12% of
  the cycles in excess of `base`, are the 14% it lost at depth 16. No client is
  blocked in A1 or A2, so the bound stays zero and the second statement does not
  run: `lpush_new` at depth 16 and depth 1 is predicted to lie across 1 with the
  file off and with it on, and `xadd` at depth 1 across 1.
- **What rejects it.** `lpush_new` wholly on the slower side in A1 or A2 rejects
  "the second statement was the cost": the cost is then elsewhere, and the
  profile of the first `lpush_new 16` pass of `head` is read for the four
  symbols (a share near that of `base` shows the zero-bound path is the one
  taken, and the cost is not theirs). `xadd` at depth 1 wholly slower rejects
  "the name comparisons before the stream commands are the 0.5%". Any other cell
  wholly on the slower side is a regression by the rule above, and goes to the
  owner with its numbers.
- **Not covered.** No cell of A1 or A2 has a client blocked, so they measure the
  path at bound zero alone. A push that creates a list while some client is
  blocked takes the third version's two statements (0.857 of `main`'s rate at
  depth 16 when it ran for every creation), and no cell measures it; C has
  blocked clients but its pushes only serve them.
- **C** compares the hand-off with the six competitors as in the third
  version's run; the registration now also sets the bound, so the run shows
  whether that moves the numbers. Its result is read against the criteria of
  question 2 unchanged.

### The third and the fourth version at one placement (stated before its runs)

Round 15 of the review found that the third `Rejected:` line of
`design/firn/blocking-lists.md` rests on the third version's rates, which were
measured with the clients on the P-cores (note at the head of Results), while the
fourth version's rates are at the design placement. Nothing compares the two on
equal terms, and that comparison is what says whether the bound in Meta, its
registration bookkeeping and its stale-value correction buy anything over the
third version's second statement. Two runs of `compare` at the design placement
(servers on CPUs 2 and 4, clients on 16-31), F1 with the file off and F2 with it
on (`appendfsync everysec`), 6 passes of 5 s, with the images `base` (`dd8c06c`),
`third` (`2d91961`), `head` (`2f2ae10`, the code that merges) and `head-twin`,
tests `set lpush rpush linsert lmove lpush_new`, depths 16 and 1, profile
`lpush_new 16`. They could not be asked for earlier: the placement of the third
version's runs was found from the placement line of the fourth version's first A1,
after the third version had been rejected on them.

- **Question.** At one placement, does a push that creates its list where nobody
  waits run faster with the fourth version's read of the bound than with the third
  version's second statement, with the file off and with it on?
- **Prediction (inferred from the mechanism found in the fourth version's runs,
  not measured).** With the file off both versions take Meta's lock on every such
  push (the third in its second statement together with the event hub's, the
  fourth in its first statement), so `third` and `head` lie within noise of each
  other on `lpush_new` at depth 16, or `third` is slower by the cost of the hub's
  lock; `head`/`base` reproduces A1's 0.8338 (the same image and base as run
  40878646647). With the file on Meta is taken by both and the third also takes
  the event hub: `third` is slower than `head` on `lpush_new`.
- **What rejects it.** `third`/`head` wholly above 1 on `lpush_new` at depth 16
  with the file off rejects "the fourth is no worse": the bound then costs more
  than the statement it replaced, the third version is the faster of the two, and
  the owner's card must say so. `third`/`head` across 1 or above with the file on
  rejects "the bound pays with the file on". `head`/`base` for `lpush_new` 16 with
  the file off outside A1's interval (0.8118-0.8532) means this run does not
  reproduce A1 and is not read against it.
- **Reading.** Per cell the ratio `third`/`head` (`summary.py ratios` with
  `--base head --head third --twin head-twin`), not two ratios to `base`; the
  interval of `head-twin` against `head` is the noise. The result goes into the
  third `Rejected:` line as measured; if `third` is as fast or faster than `head`
  in a cell that matters, a new card replaces the ruling asked for on
  firn-q-list-producer-hold. Nothing is accepted by this record.
- **Not covered.** No client is blocked, as in A1 and A2.

## The runs

Each is a dispatch of `redis-bench.yml` on the PR's head commit; the runner
orders them on the 14900K, one at a time.

| run | mode and inputs | answers | expected duration |
|---|---|---|---|
| A1 | `compare`, `base`, `head`, `head-twin`, the 23 tests with `lpush_new` last, depths 16 and 1, 2 server CPUs, 6 passes of 5 s, profile `lpush 16;lmove 16;lpush_new 16` | question 1, file off; run on `da1e2c3` and `7a4e4de`, repeated on the third version (`2d91961`) and on the head that merges (the fourth version) | about 95 min (the 44-cell first A1 run 40049518328 took 85, builds included) |
| A2 | `compare`, the same images, `aof`, tests `set lpush rpush linsert lmove lpush_new`, the same profile | question 1, file on: the in-place append of `lpush_new` beside the event-hub check | about 25 min |
| B | `workloads` with `workload_images`, `head` and `head-twin` (and the `reference` line that `workload_images` adds), `blocking-handoff`, depths 16 and 1, 50 connections, 6 passes of 10 s | the floor of question 2 | about 10 min |
| C | `workloads` with `workload_images`, the six competitor lines and `head` and `head-twin`, `sched_report`, profile | question 2 | about 40 min |
| D | `compare`, `base`, `head`, `head-twin` and the throwaway images `va`, `vb`, `vc`, `lpush` and `lpush_new`, depths 16 and 1, 6 passes of 5 s, profile `lpush_new 16` | which code of the first statement is the cost of `lpush_new`, after A1 rejected the prediction of the fourth version (stated under the fourth version's results) | about 40 min |
| E | the same with `vc`, `vd`, `ve`, `vf`, `vg` | whether the entry read or the read of Meta is the cost, and whether the field's place in `Meta` matters | about 50 min |
| F1, F2 | `compare`, `base`, `third`, `head`, `head-twin`, tests `set lpush rpush linsert lmove lpush_new`, depths 16 and 1, 6 passes of 5 s, profile `lpush_new 16`; F1 with the file off, F2 with `aof` | whether the fourth version beats the third at one placement (stated under the fourth version) | about 35 min each |

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

Every run below is on the 14900K at the commit it names. The design places the
servers on CPUs 2 and 4 and the clients on CPUs 16-31. B and the first A1 at
`da1e2c3` (the PR's first version) and the second A1 at `7a4e4de` have that
placement (`source=override` in the run's placement line). The runs of the third
version, `2d91961` (A1, A2, a retest at depth 1, and C), follow them, and the
runs of the fourth version, the head that merges, follow those; **the third
version's four runs and the first A1 of the fourth were dispatched without
`server_cpus` and `client_cpus`, so the automatic placement put the clients on the
P-cores 6-15 (`source=hybrid`; the one-CPU configuration of C has server CPU 2
and clients 4-15)**: runs 40113222639, 40833091903, 40839887066 and 40843860452,
and 40866420392. They are measurements at that placement, not at the design's. The
fourth version's A1 was repeated at the design placement, and its D, E, A2 and C
were made there; the third version's runs were not repeated (runs F1 and F2 below
measure it at the design placement beside the fourth).

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

The file is off, so the staged copy into it does not run; `head` still
allocates the 64-byte effects buffer, which `base`, appending in place, does
not. The profile of the first
`lpush 16` pass has `wf_cmap_hold` (the runtime function that takes the
whole-keyspace hold) at 51.16% of the cycles in `head` and `try_entry` (the
per-key entry hold) at 14.13% in `base`; `lmove 16` has `wf_cmap_hold` at
48.16% in `head` and `try_entry` at 17.05% in `base`. The allocator and
`memcpy` are each below 1% in `head` (`malloc` 0.57%, `__memmove_avx_unaligned_erms`
0.36% in `lpush 16`). So the cost of these commands is the whole-keyspace,
metadata, event-hub and server-state hold that
[blocking-lists.md](../../../design/firn/blocking-lists.md) gave every list
producer in this first version (`run_list_producer`), which serialises the two server contexts,
and not the staged copy or its buffer: neither of the other two options of
[firn-q-list-producer-staging](../../../design/firn/append-only-file.md) (restore
the in-place append, reuse a buffer) would remove it. This is read from one
profile of one pass per image; the ratio of the cells is the measurement,
the attribution to the hold is inferred. The head is the variant that would
confirm it: its list writes onto a live list take only the key's entry, and the
repeated A1 on it shows whether the 8 cells return to `base`.

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

### A1 again: the narrowed hold (`7a4e4de`)

Run 40082579797 (`compare`, `base` = `05f7e89`, `head` = `head-twin` = `7a4e4de`,
the 20 tests of the default suite plus `linsert`, `lmove` and `lpush_new`,
depths 16 and 1, 2 server CPUs on CPUs 2 and 4, clients on CPUs 16-31, 6 passes
of 5 s, the append-only file off, profile `lpush 16;lmove 16;lpush_new 16`; the
placement is that of the first A1 run). 46
cells: 9 faster, 30 no detected change, 7 slower. The twin interval lies across
1 in 45 cells and wholly below 1 in one (`ping_mbulk` depth 1, 0.9875, interval
0.9818-0.9932), which is the noise of this run. The client is the limit
(CPUs busy above 0.90) in `lrange_500` and `lrange_600` at depth 16 (35 of the
828 pass rows of the three lines), which are not judged (the first is unchanged, the second faster).

**The claim "a write onto an existing list runs as it did before the PR" is
rejected for five cells, and the claim "creating a list under the hold costs
nothing measurable" for both `lpush_new` cells.** The 7 cells wholly on the slower
side:

| test | depth | base (requests/s) | head (requests/s) | head/base | 95% interval |
|---|---|---|---|---|---|
| lpush_new | 16 | 3208521 | 1006388 | 0.3130 | 0.3070-0.3185 |
| lpush_new | 1 | 735375 | 589262 | 0.8013 | 0.7879-0.8166 |
| lmove | 16 | 3935176 | 3707904 | 0.9393 | 0.8972-0.9688 |
| lpush | 16 | 4255664 | 4025858 | 0.9410 | 0.9291-0.9700 |
| rpush | 16 | 4255518 | 4045741 | 0.9508 | 0.9425-0.9605 |
| linsert | 1 | 687950 | 677634 | 0.9827 | 0.9743-0.9944 |
| lpush | 1 | 779102 | 768174 | 0.9883 | 0.9790-0.9982 |

Against the first A1 run (`da1e2c3`, every list producer held whole), the
narrowing returned the cells it was made for: `lpush`, `rpush`, `linsert` and
`lmove` at depth 16 went from 0.31 to 0.94-1.00 of `base`, and at depth 1 from
0.83-0.88 to 0.98-0.99; `lrange_100` at depth 1, 1.5% slower in the first run, is
unchanged now. So the attribution of that cost to the whole-keyspace hold, which
the first run inferred from one profile, holds. Two costs remain, and they have
different causes.

*Creation of a list (`lpush_new`, 0.3130 and 0.8013).* Nearly every request
creates its list, so nearly every request takes the whole-keyspace hold. The
first `lpush_new 16` pass of `head` spends 38.78% of its cycles in
`wf_cmap_hold` (the runtime function that takes the whole-keyspace hold) and
10.66% in `wf__table_lock_entry`, against 5.53% in the entry lock and no whole
hold in `base`; the allocator takes about 2% in `head` (`__libc_malloc2` 0.79%,
`_int_malloc` 0.71%, `malloc` 0.51%). The cost is the hold, not the staged
buffer or its copy, which the file being off does not run in any case; the copy
is measured by A2. The ratio, 0.31 at depth 16, is the one the first A1 run found
for every list producer (0.3077 to 0.3167), which is what the same hold taken on
every request would give.

*Writes onto an existing list (0.94-0.99 at five cells).* These commands take no
whole hold now, but the profile of the first `lpush 16` pass of `head` has
`wf_context_allocate` at 2.14% (1.19% in `base`), `wf__context_frame_release`
at 1.04% (0.85%), and `run_list_write_entries.resume` (0.98%) and
`run_list_write.resume` (0.65%), the two functions that the command now passes
through before `run_push` and that `base` does not have; `lmove 16` has
`wf_context_allocate` at 2.06% (1.48%) and `wf__context_frame_release` at 1.72%
(1.15%). The dispatcher called `run_push` directly in `base`; in `head` it calls
`run_list_write`, which calls `run_list_write_entries`, which calls `run_push`,
three suspendable functions in place of one. This is read from one profile of one
pass per image, so the attribution of the 2% to 6% to these frames is inferred;
removing them is the test, and the head after this run does so: the dispatcher
calls `run_push`, `run_linsert` and `run_lmove` directly, `run_list_write` and
`run_list_write_entries` are deleted, and a creating push takes the hub under its
entry hold instead of the whole keyspace (the third version, above).

Table, all 46 cells (head/base and twin/head: median, 95% interval):

| test | depth | head/base | interval | verdict | twin/head | interval |
|---|---|---|---|---|---|---|
| get | 1 | 1.0014 | 0.9866-1.0168 | no detected change | 0.9977 | 0.9866-1.0151 |
| get | 16 | 1.0424 | 1.0311-1.0550 | faster | 1.0089 | 0.9957-1.0150 |
| hset | 1 | 1.0142 | 0.9931-1.0284 | no detected change | 0.9992 | 0.9685-1.0229 |
| hset | 16 | 1.0003 | 0.9683-1.0292 | no detected change | 1.0000 | 0.9849-1.0261 |
| incr | 1 | 0.9952 | 0.9797-1.0384 | no detected change | 1.0044 | 0.9801-1.0217 |
| incr | 16 | 1.0121 | 1.0059-1.0223 | faster | 1.0029 | 0.9952-1.0099 |
| linsert | 1 | 0.9827 | 0.9743-0.9944 | slower | 0.9995 | 0.9853-1.0104 |
| linsert | 16 | 0.9958 | 0.9781-1.0110 | no detected change | 0.9986 | 0.9944-1.0113 |
| lmove | 1 | 0.9850 | 0.9700-1.0059 | no detected change | 1.0021 | 0.9893-1.0144 |
| lmove | 16 | 0.9393 | 0.8972-0.9688 | slower | 0.9887 | 0.9635-1.0113 |
| lpop | 1 | 0.9936 | 0.9878-1.0044 | no detected change | 0.9960 | 0.9889-1.0176 |
| lpop | 16 | 1.0197 | 1.0087-1.0314 | faster | 0.9968 | 0.9838-1.0170 |
| lpush | 1 | 0.9883 | 0.9790-0.9982 | slower | 1.0011 | 0.9852-1.0094 |
| lpush | 16 | 0.9410 | 0.9291-0.9700 | slower | 1.0037 | 0.9719-1.0262 |
| lpush_new | 1 | 0.8013 | 0.7879-0.8166 | slower | 0.9958 | 0.9814-1.0136 |
| lpush_new | 16 | 0.3130 | 0.3070-0.3185 | slower | 1.0096 | 0.9890-1.0316 |
| lrange_100 | 1 | 0.9989 | 0.9935-1.0038 | no detected change | 0.9997 | 0.9924-1.0085 |
| lrange_100 | 16 | 1.0199 | 1.0036-1.0307 | faster | 1.0008 | 0.9880-1.0135 |
| lrange_300 | 1 | 0.9897 | 0.9814-1.0010 | no detected change | 1.0033 | 0.9914-1.0137 |
| lrange_300 | 16 | 1.0054 | 0.9914-1.0205 | no detected change | 1.0080 | 0.9934-1.0152 |
| lrange_500 | 1 | 1.0017 | 0.9962-1.0159 | no detected change | 0.9942 | 0.9834-1.0029 |
| lrange_500 | 16 | 1.0000 | 0.9905-1.0264 | no detected change | 0.9990 | 0.9866-1.0048 |
| lrange_600 | 1 | 0.9996 | 0.9863-1.0242 | no detected change | 0.9962 | 0.9919-1.0040 |
| lrange_600 | 16 | 1.0035 | 1.0009-1.0061 | faster | 0.9985 | 0.9866-1.0032 |
| mset | 1 | 1.0052 | 0.9817-1.0273 | no detected change | 1.0037 | 0.9729-1.0291 |
| mset | 16 | 1.0585 | 1.0193-1.0999 | faster | 0.9316 | 0.8993-1.0235 |
| ping_inline | 1 | 1.0220 | 0.9853-1.0754 | no detected change | 0.9955 | 0.9836-1.0057 |
| ping_inline | 16 | 1.0421 | 1.0272-1.0503 | faster | 1.0096 | 0.9884-1.0182 |
| ping_mbulk | 1 | 1.0104 | 0.9887-1.0420 | no detected change | 0.9875 | 0.9818-0.9932 |
| ping_mbulk | 16 | 1.0502 | 1.0439-1.0662 | faster | 0.9977 | 0.9923-1.0088 |
| rpop | 1 | 1.0037 | 0.9955-1.0073 | no detected change | 1.0064 | 0.9942-1.0128 |
| rpop | 16 | 1.0051 | 0.9964-1.0147 | no detected change | 1.0026 | 0.9868-1.0128 |
| rpush | 1 | 0.9805 | 0.9453-1.0105 | no detected change | 1.0153 | 0.9773-1.0415 |
| rpush | 16 | 0.9508 | 0.9425-0.9605 | slower | 0.9978 | 0.9799-1.0126 |
| sadd | 1 | 0.9868 | 0.9705-1.0031 | no detected change | 1.0035 | 0.9932-1.0270 |
| sadd | 16 | 1.0151 | 0.9885-1.0329 | no detected change | 0.9856 | 0.9588-1.0168 |
| set | 1 | 1.0170 | 0.9994-1.0335 | no detected change | 0.9971 | 0.9879-1.0054 |
| set | 16 | 1.0012 | 0.9917-1.0168 | no detected change | 0.9994 | 0.9848-1.0071 |
| spop | 1 | 0.9883 | 0.9753-1.0079 | no detected change | 1.0064 | 0.9878-1.0249 |
| spop | 16 | 1.0046 | 0.9850-1.0226 | no detected change | 1.0087 | 0.9738-1.0379 |
| xadd | 1 | 1.0143 | 1.0001-1.0269 | faster | 0.9951 | 0.9823-1.0069 |
| xadd | 16 | 1.0220 | 0.9818-1.0803 | no detected change | 1.0044 | 0.9669-1.0575 |
| zadd | 1 | 0.9931 | 0.9713-1.0158 | no detected change | 1.0128 | 0.9735-1.0464 |
| zadd | 16 | 1.0093 | 0.9912-1.0291 | no detected change | 0.9871 | 0.9785-1.0091 |
| zpopmin | 1 | 0.9957 | 0.9814-1.0166 | no detected change | 1.0213 | 0.9938-1.0355 |
| zpopmin | 16 | 1.0003 | 0.9686-1.0294 | no detected change | 1.0127 | 0.9796-1.0459 |

### The third version (`2d91961`)

`2d91961` has the code of the third version (`12dbbc5`, narrowed again by the
`stage_ready` flag) and differs from the last commit reviewed (`74e7ec4`) by a
deleted temporary workflow. The base is `7cf12df`, the tip of main merged by then
(the earlier A1 runs used `05f7e89`, so a cell is compared with its twin in the
same run and not with the same cell of an earlier run).

**A1** (run 40113222639, `compare`, AOF off, 46 cells, 6 passes of 5 s, 2 server
CPUs on CPUs 2 and 4, clients on the P-cores 6-15 (automatic placement, see the
note at the head of Results), profile `lpush 16;lmove 16;lpush_new 16`; UTC 16:48
to 18:07). 46 cells: 8 faster (`get`, `lmove`, `lpop`,
`ping_inline`, `ping_mbulk`, `rpop`, `set`, `zadd`, all at depth 16), 29 no
detected change, 9 wholly on the slower side. The twin interval lies wholly off 1
in 5 cells (`get` 1, `set` 1, `set` 16 and `lrange_100` 1 faster, `lrange_500` 1
slower); the widest twin half-width is `mset` 16 (7.3%, above the 5.2% bound; its
head/base interval, 0.9644-1.1076, does not decide). The client is the limit in
`lrange_100` (16), `lrange_300`, `lrange_500` and `lrange_600`, which are not
judged. The 9 cells:

| test | depth | base (requests/s) | head (requests/s) | head/base | 95% interval | twin/head, 95% interval |
|---|---|---|---|---|---|---|
| lpush_new | 16 | 3216458 | 2764612 | 0.8573 | 0.8511-0.8714 | 0.9977, 0.9811-1.0120 |
| lpush_new | 1 | 747712 | 717578 | 0.9602 | 0.9486-0.9671 | 1.0042, 0.9979-1.0169 |
| lpush | 16 | 4249900 | 4129357 | 0.9689 | 0.9572-0.9866 | 0.9968, 0.9819-1.0191 |
| lpush | 1 | 792200 | 784178 | 0.9884 | 0.9851-0.9959 | 1.0051, 0.9982-1.0093 |
| rpush | 1 | 790316 | 783370 | 0.9914 | 0.9845-0.9983 | 1.0028, 0.9974-1.0082 |
| sadd | 1 | 800721 | 794387 | 0.9918 | 0.9845-0.9990 | 1.0039, 0.9952-1.0103 |
| xadd | 1 | 703299 | 698732 | 0.9904 | 0.9818-0.9976 | 1.0012, 0.9912-1.0178 |
| zadd | 1 | 641836 | 632160 | 0.9838 | 0.9624-0.9999 | 1.0099, 0.9872-1.0283 |
| lrange_100 | 1 | 608366 | 605025 | 0.9963 | 0.9924-0.9988 | 1.0040, 1.0007-1.0065 |

`lrange_100` at depth 1 does not count, by the rule above: its twin lies wholly
off 1 (the same code differs by 0.4%). Against the second A1 run, the cell that
creates a list went from 0.313 and 0.801 to 0.857 and 0.960, `lmove` 16 and
`linsert` 1 are no longer slower (1.0559 and 1.0025), and `lpush` 16 went from
0.941 to 0.969. The profile of the first `lpush_new 16` pass of each image
reads: `wf__shared_take` 6.11% in `head` (not among the first twenty symbols of
`base`), `wf__table_lock_entry` 8.95% (5.98% in `base`), `wf_shared_unlock_as`
2.28% (1.14%), `wf__table_unlock_entry` 1.51%. A creating push runs two atomic
statements, the first (entry and Meta) finding no live list, the second (entry,
event hub and Meta) asking the registry; the symbols above are the second
statement's locks, which together make up about 12% of the cycles (the excess over
`base` of the four symbols), against the 14% lost (read from one profile, so
inferred; runs D and E of the fourth version show that the same symbols come from
the lock of Meta, which any read of Meta forces an AOF-off push to take, so this
attribution to the second statement is superseded). `lpush` 16 spends 16.1% of
its cycles in `try_entry`, the spin on the entry of the one key two server CPUs
push to, against 14.5% in `base`; that a longer hold per request lengthens the spin
is inferred.

**A2** (run 40833091903, `compare`, AOF on with `appendfsync everysec`, `set`,
`lpush`, `rpush`, `linsert`, `lmove`, `lpush_new`, depths 16 and 1, same images
and passes; UTC 07:04 to 07:29). 12 cells: 3 wholly slower, 9 no detected
change, no twin interval off 1. `lpush_new` 16 is 0.9310 (0.9049-0.9458; twin
0.9814, 0.9689-1.0094) and `lpush_new` 1 is 0.9715 (0.9580-0.9862); `lmove` 1 is
0.9972 (0.9923-0.9998). `lpush` 16 is 0.9874 (0.9768-1.0009), `lpush` 1 1.0040,
`rpush` 0.9910 and 0.9963, `linsert` 1.0101 and 1.0001, `lmove` 16 0.9984, and the
control `set` 1.0092 and 0.9986. With the file on the creating push loses less
than with it off (0.931 against 0.857) because the append takes a larger share of
the request. A write onto an existing list shows no loss, so the staged copy, which
runs only for a push that serves a waiting client, `RENAME`, `COPY` and a promoted
connection, is not in any cell and its cost stays unmeasured.

**A1 retest at depth 1** (run 40839887066, `compare`, AOF off, `rpush`, `sadd`,
`xadd`, `zadd`, `lrange_100`, `lpush`, `get`, `set` at depth 1, 12 passes of 5 s;
UTC 08:07 to 08:34), for the depth 1 cells of A1 that were wholly slower and are
not list creations, with `get` and `set` as controls. 7 of 8 cells show no detected change (`lpush` 0.9963, 0.9921-1.0001;
`rpush` 0.9985; `sadd` 0.9963, 0.9907-1.0004; `zadd` 0.9972; `lrange_100` 0.9978;
control `get` 1.0020 and `set` 1.0016), so the A1 results for these were noise.
`xadd` 1 is wholly slower again: 0.9946 (0.9888-0.9985; twin 0.9976,
0.9911-1.0006). `xadd` is a stream command, which the dispatcher reaches after
trying the four blocking commands by name; `main` has no such step. That the name
comparisons cost the 0.5% is inferred, not tested.

**C** (run 40843860452, `workloads` with `workload_images`, `blocking-handoff`,
`reference` (Redis 7.2.16), `redis-latest`, `valkey-latest`, `valkey-io-latest`,
`dragonfly-latest`, `garnet-latest` and the images `head` and `head-twin`, 50
consumers, depths 16 and 1, 1 and 2 server CPUs, 6 passes of 10 s, `sched_report`
and `profile`; UTC 08:40 to 09:36). `summary.py pubsub` with the floor 1.12% of B:
the twin half-widths of the 4 cells are 0.39% to 0.94%, none above the floor. No
cell is client-limited (firn's client work fraction is 0.04 and 0.23). All 8
criteria (4 cells, a rate and a p99 each) are not met. The cells with 2 server
CPUs, the ones the acceptance row names:

| depth | line | rate (elements/s) | p99 (ms) |
|---|---|---|---|
| 16 | valkey-io-latest (fastest) | 297734 | 0.080 |
| 16 | valkey-latest | 276305 | 0.058 |
| 16 | reference | 270647 | 0.059 |
| 16 | redis-latest | 243308 | 0.066 |
| 16 | dragonfly-latest | 215001 | 0.086 |
| 16 | garnet-latest | 125471 | 1.015 |
| 16 | head | 100631 | 0.758 |
| 16 | head-twin | 101128 | 0.754 |
| 1 | valkey-io-latest (fastest) | 136445 | 0.011 |
| 1 | redis-latest | 118537 | 0.010 |
| 1 | valkey-latest | 117852 | 0.010 |
| 1 | reference | 116447 | 0.009 |
| 1 | dragonfly-latest | 109758 | 0.017 |
| 1 | garnet-latest | 78146 | 0.158 |
| 1 | head-twin | 78057 | 0.096 |
| 1 | head | 77752 | 0.097 |

`head` is 0.338 of the fastest at depth 16 (needs 1.4) and 0.570 at depth 1 (needs
1.0), with a p99 of 0.758 ms against 0.080 and 0.097 against 0.011. Against Redis
7.2.16 it is 0.372 and 0.668. With 1 server CPU the fastest is `redis-latest` at
depth 16 (283236 against `head` 84987, 0.300) and `reference` at depth 1 (120397
against 78462, 0.652). `head` is faster than `garnet-latest` only with 1 server
CPU (45990 against 78462 at depth 1, 58456 against 84987 at depth 16); with 2 it
is slower than every competitor in both cells (`garnet-latest` 125471 against
100631, and 78146 against 77752). The first pass of `head` under `perf` spends 45.65% of its cycles at depth
16 and 22.74% at depth 1 (2 CPUs) in `wf_shared_acquire_as`, the runtime's
shared-acquire function (30% in the pub/sub measurement), which is where the
prediction of question 2 puts the cost; that the wait is the listening lock of
board item coord-wfbl-03-04 is inferred from the symbol, since neither the
scheduling counts of the server log (borrows and stolen contexts per driver) nor
the profile count the contexts woken per write or the wait on the lock, so the
instrument the plan said would be missing is missing; it belongs to that item and
is not built here.

**Reading of the third version's runs.** The creating push still loses 14% (AOF off,
depth 16) and 4% (depth 1) in A1 and 7% and 3% in A2, `lpush` 16 loses 3% in A1
and none in A2, `xadd` 1 loses 0.5%, and the hand-off reaches 0.34 and 0.57 of the
fastest competitor. By the criteria stated above each is a cell wholly on the
slower side or below its threshold, and goes to the owner on the board with these
numbers; nothing in these runs is accepted by this record.

### The fourth version (`2f2ae10`)

`2f2ae10` is the head that merges up to the records: it has the code of the
reviewed `c6b86a3` (`3f101f0`, the commit of the full CI, differs from it by a
deleted temporary workflow alone) and one more section of this record (the one
stated before the runs). The base is `dd8c06c`. The runs below are at the design placement (servers
on CPUs 2 and 4, clients on CPUs 16-31) unless named otherwise.

**First A1** (run 40866420392, `compare`, AOF off, 46 cells, 6 passes of 5 s,
automatic placement: clients on CPUs 6-15, so not the design's). The creating push
at depth 16 was 0.8396 (0.8244-0.8492; twin 0.9955, 0.9792-1.0147) and at depth 1
0.9758 (0.9640-0.9862); `lpush` 16 0.9580, `rpush` 16 0.9777; five cells at depth
1 (`incr`, `lpop`, `sadd`, `set`, `zadd`) were wholly slower by less than 1% with
twins across 1; `xadd` 1 was 1.0014 (0.9957-1.0129). 9 cells slower, 8 faster, 29
without a detected change. The prediction stated above, that `lpush_new` lies
across 1, was rejected by this run; the placement was found wrong from its
placement line, and the runs below repeat it.

**A1** (run 40878646647, same inputs at the design placement, profile `lpush
16;lpush_new 16` with `callers`). 4 cells wholly slower, 7 faster (`get`, `linsert`,
`lpop`, `ping_inline`, `ping_mbulk`, `sadd` and `set`, all at depth 16), 35 no
detected change; the client is the limit (median busy fraction above 0.9) in
`lrange_500` 16 and `lrange_600` 16, which are not judged. The 4 cells (rates are
medians over the passes):

| test | depth | base (requests/s) | head (requests/s) | head/base | 95% interval | twin/head, 95% interval |
|---|---|---|---|---|---|---|
| lpush_new | 16 | 3216436 | 2687939 | 0.8338 | 0.8118-0.8532 | 1.0005, 0.9771-1.0297 |
| lpush_new | 1 | 736360 | 732457 | 0.9910 | 0.9772-0.9998 | 0.9828, 0.9707-1.0026 |
| lpush | 16 | 4237934 | 4090312 | 0.9728 | 0.9562-0.9919 | 1.0082, 0.9829-1.0220 |
| mset | 1 | 614804 | 605798 | 0.9902 | 0.9710-0.9958 | 0.9923, 0.9795-1.0156 |

The `head/base` column of this table and of the tables below is the median of the
per-pass ratios, not the quotient of the two rate medians (2687939/3216436 =
0.8357 for `lpush_new` 16).

`xadd` 1 is 0.9896 (0.9755-1.0268) and `xadd` 16 0.9792 (0.9453-1.0186): trying the
stream commands before the blocking ones removed the 0.5% of the third version.
The prediction stated above is rejected again: the creating push at depth 16 is
wholly slower, at 0.834 and not across 1. The profile of its first `lpush_new 16`
pass, as the rule above asks, shows the four lock symbols, not at the share of
`base` but at more than three times it (cycles in G; `base` 57.8 in total, `head`
71.7): `wf__shared_take` 5.83 (none in `base`), `wf_shared_acquire_as` 4.98 (1.27),
`wf_shared_unlock_as` 3.51 (0.61), the three together 12.4 of the 13.9 that `head`
has more; `acquire_entry` (10.60 against 10.45) and `wf__table_lock_entry` (3.18
against 3.09) are the same. The `callers` option gave no callers: perf's unwinding
printed addresses such as `0x12` for these functions. The profile of `lpush` 16
(`head` 49.2 G, `base` 48.2 G) has none of the three in excess.

**Runs D and E, stated before they were made** (written in the item's log on 29
October before each run; the sides, the prediction and what rejects it are copied
here). The question after A1: the second statement was predicted to be the cost,
and its locks are the excess, but with the bound at zero it should not run.
Which code is the cost? The side branches, throwaway branches on `2f2ae10` that
were deleted after the runs, each changing one place of `run_push`
(`firn/commands/lists.wf`) and, for two, the position of the field in `Meta`:

- D, with `base`, `head` and `head-twin` (the noise): `va`, the first statement's
  test `waiter_keys_bound != 0` changed to `== 7`, so that with no blocked client
  the second statement cannot run; `vb`, the second statement deleted; `vc`, the
  first statement's `list_kind` and read of the bound deleted, so that the first
  statement is main's with the new signature. Prediction (inferred, not measured):
  `va` equals `head` (the bound is zero). Rejected if `va` is wholly faster than
  `head` (the bound is not zero in the run, a defect); `vb` faster than `head`
  would show that the second statement's existence costs, `vc` faster that the
  cost is in the first statement's probe, neither that it is in the shape of the
  function.
- E, with the same three: `vc` again; `vd`, `list_kind` kept and the read of the
  bound removed (`held > client^.unix_us`, never true and not removable by the
  compiler); `ve`, the read of the bound kept (for every push, a live list
  included) and `list_kind` removed; `vf`, `ve` with `waiter_keys_bound` moved to
  be the first field of `Meta` (and the constructor in `store.wf` reordered, which
  the compiler requires: the first attempt of E, run 40898679237, stopped there
  and measured nothing); `vg`, `ve` with the field right after `commit_seq`, which
  every write statement writes. Prediction (inferred): `vd` returns to `base` and
  `ve` is as slow as `head`; `vf` and `vg` faster than `ve` would show that the
  cache line of the field is the cost. Rejected if `vd` is as slow as `head`
  (reading the entry is the cost) or if `vd` and `ve` are both fast and `head`
  slow.

D (run 40893273232, `compare`, AOF off, `lpush` and `lpush_new`, depths 16 and 1,
6 passes of 5 s, profile `lpush_new 16`), head/base median (95% interval):

| image | lpush_new 16 | lpush_new 1 | lpush 16 | lpush 1 |
|---|---|---|---|---|
| head | 0.8290 (0.8177-0.8491) | 0.9799 (0.9744-0.9881) | 0.9651 (0.9447-0.9803) | 0.9883 (0.9548-1.0066) |
| va | 0.8212 (0.8131-0.8462) | 0.9770 (0.9666-0.9955) | 0.9805 (0.9614-0.9973) | 0.9946 (0.9851-1.0036) |
| vb | 0.8318 (0.8107-0.8487) | 0.9840 (0.9724-0.9953) | 0.9832 (0.9534-1.0000) | 0.9939 (0.9773-1.0072) |
| vc | 1.0027 (0.9796-1.0305) | 0.9876 (0.9710-1.0068) | 0.9930 (0.9789-1.0068) | 0.9848 (0.9754-0.9975) |
| head-twin / head | 0.9773 (0.9605-1.0117) | 0.9903 (0.9740-0.9997) | 1.0114 (0.9747-1.0351) | 0.9909 (0.9665-1.0320) |

`va` and `vb` against `head`: 0.9873 (0.9709-1.0208) and 0.9972 (0.9841-1.0071)
for `lpush_new` 16, no detected change; `vc` against `head` 1.2019 (1.1862-1.2257),
faster. The profiles (`lpush_new` 16, cycles in G): `va` and `vb` have the excess of
`head` (`wf__shared_take` 5.63 and 5.35), `vc` has none (`wf_shared_acquire_as`
0.97, `wf_shared_unlock_as` 0.56, against 1.28 and 0.68 in `base`). So the bound is
zero, the second statement does not run, its existence costs nothing, and the cost
is in the first statement's probe (prediction held).

E (run 40900755503, same inputs, 8 images):

| image | lpush_new 16 | lpush_new 1 | lpush 16 | lpush 1 |
|---|---|---|---|---|
| head | 0.8305 (0.8201-0.8364) | 0.9741 (0.9624-0.9977) | 0.9665 (0.9497-0.9937) | 0.9916 (0.9858-1.0031) |
| vc | 1.0086 (0.9758-1.0225) | 0.9940 (0.9752-1.0231) | 0.9904 (0.9688-1.0168) | 0.9927 (0.9376-1.0179) |
| vd | 0.9915 (0.9731-1.0021) | 1.0139 (0.9948-1.0207) | 0.9771 (0.9662-0.9926) | 0.9791 (0.9669-1.0000) |
| ve | 0.8351 (0.8223-0.8472) | 0.9772 (0.9602-1.0068) | 0.8702 (0.8396-0.8841) | 0.9938 (0.9677-1.0032) |
| vf | 0.8327 (0.8135-0.8541) | 0.9910 (0.9738-1.0045) | 0.8571 (0.8414-0.8752) | 0.9755 (0.9627-0.9900) |
| vg | 0.8353 (0.8256-0.8486) | 0.9880 (0.9080-1.0326) | 0.8585 (0.8493-0.8699) | 0.9754 (0.9644-0.9875) |
| head-twin / head | 1.0061 (0.9785-1.0342) | 1.0075 (0.9958-1.0147) | 0.9999 (0.9594-1.0244) | 0.9990 (0.9705-1.0172) |

Against `head`: `vc` 1.2096 (1.1768-1.2361) and `vd` 1.1884 (1.1667-1.2186) faster,
`ve` 1.0052 (0.9889-1.0270) no detected change, for `lpush_new` 16. The excess
symbols of `head` appear in `ve` (`wf__shared_take` 5.15 G of 69.4), `vf` (5.65 of
71.2) and `vg` (5.27 of 70.5) and not in `vc` and `vd`. The reading of the
prediction: `vd` returns to `base` and `ve` is as slow as `head` (held); `vf` and
`vg` are not faster than `ve`, so the cache line of the field is not the cost, and
the cost is the read of Meta itself, not the entry read of `list_kind`
(about 2% to 3% on a live list at depth 16: `vd` 0.9771, `vc` 0.9904, `vc` against
`head` 1.0214). Reading the field for every push, a live list included (`ve`), costs
13% there (0.8702).

**Mechanism** (read from instructions and the runs, not counted). In `base` the
instructions of `run_push.resume` after `wf__table_lock_entry` are `test %r14b,%r14b;
je ...`, with `r14b` a byte loaded from the store before the call (the source's
`logged = store^.logging`), and the jump skips the call to `wf__shared_take`: the compiled statement takes Meta only on the path that uses it,
the logging path. With the file off a push of `base` holds only its key's entry, and
the two server cores share no lock; this is the decision of `firn/command-parts`
(while the keyspace keeps no append-only file, a statement that reads nothing else
from Meta holds only its keys' entries). The fourth version reads
`waiter_keys_bound` in the first statement for every push that does not find a live
list, so the statement takes Meta's lock on every such push and the two cores queue
on it: the `wf__shared_take`, acquire and unlock symbols are that lock, and a
creating push loses 17% at depth 16. With the file on, Meta is taken anyway
(`wf__shared_take` is 4.57 G in `base`'s A2 profile of `lpush_new` 16, none in its
A1 profile), so one more field read adds nothing (A2 below). That Meta's lock is the
shared lock is inferred from this and from D and E; no counter counted its
acquisitions.

**A2** (run 40907743576, `compare`, AOF on with `appendfsync everysec`, `set`,
`lpush`, `rpush`, `linsert`, `lmove`, `lpush_new`, depths 16 and 1, 6 passes of 5
s, profile `lpush_new 16;lpush 16`, design placement). 12 cells: 2 wholly slower, 3
faster, 7 no detected change. `lpush_new` 16 is 1.0123 (1.0034-1.0268; twin 0.9986,
0.9582-1.0315; 2126414 requests/s in `base`, 2160193 in `head`) and `lpush_new` 1
1.0000 (0.9813-1.0233). The slower cells: `rpush` 16 0.9681 (0.9553-0.9821; twin
1.0106, 0.9943-1.0241; 2893148 and 2815431) and `lpush` 16 0.9816 (0.9674-0.9980;
twin 1.0041, 0.9835-1.0184; 2842284 and 2805004). Faster: `linsert` 16 1.0141, `set`
16 1.0483 and `lpush_new` 16. The others: `linsert` 1 0.9970, `lmove` 16 0.9734
(0.9422-1.0043), `lmove` 1 0.9975, `lpush` 1 0.9888, `rpush` 1 0.9897, `set` 1
1.0140. The profile of `lpush_new` 16 has no excess (`base` 54.8 G, `head` 54.7);
that of `lpush` 16 has 1.8 G more of 49.3, of which `run_push.resume` itself is
0.56 and `wf__shared_take` 0.26: the probe's own code (a second read of the entry,
a branch and a local), about 2% to 3% on an existing list, not a lock.

**C** (run 40912672893, `workloads` with `workload_images`, `blocking-handoff`,
`reference`, `redis-latest`, `valkey-latest`, `valkey-io-latest`, `dragonfly-latest`,
`garnet-latest` and the images `head` and `head-twin`, 50 consumers, depths 16 and
1, 2 server CPUs on CPUs 2 and 4, clients on 16-31, 6 passes of 10 s,
`sched_report`, profile). `summary.py pubsub` with the floor 1.12% of B: twin
half-widths 1.10% and 0.36%, none above the floor; firn's client work fraction is
0.043 and 0.299, so no cell is client-limited. The 4 criteria (2 cells, a rate and
a p99 each: the rate at least 1.4 times the fastest competitor's at depth 16 and at
least 1.0 at depth 1, the p99 not above its) are not met:

| depth | line | rate (elements/s) | p99 (ms) |
|---|---|---|---|
| 16 | valkey-io-latest (fastest) | 299524 | 0.080 |
| 16 | reference (Redis 7.2.16) | 291255 | 0.055 |
| 16 | redis-latest | 284699 | 0.056 |
| 16 | valkey-latest | 272046 | 0.059 |
| 16 | dragonfly-latest | 211884 | 0.088 |
| 16 | garnet-latest | 125890 | 0.946 |
| 16 | head | 100142 | 0.758 |
| 16 | head-twin | 100255 | 0.758 |
| 1 | valkey-io-latest (fastest) | 119259 | 0.012 |
| 1 | reference (Redis 7.2.16) | 114125 | 0.010 |
| 1 | redis-latest | 111718 | 0.010 |
| 1 | valkey-latest | 110930 | 0.010 |
| 1 | dragonfly-latest | 101073 | 0.017 |
| 1 | head | 76695 | 0.082 |
| 1 | head-twin | 77233 | 0.080 |
| 1 | garnet-latest | 75182 | 0.141 |

`head` is 0.334 of the fastest at depth 16 (needs 1.4) and 0.643 at depth 1 (needs
1.0), with a p99 of 0.758 ms against 0.080 and 0.082 against 0.012; against Redis
7.2.16 0.344 and 0.672. It is faster than `garnet-latest` only at depth 1 and by
2%. The first pass of `head` under `perf` spends 45.23% of its cycles at depth 16
and 20.65% at depth 1 in `wf_shared_acquire_as`, as the third version's run did
(45.65% and 22.74%): the cost the second question predicted is in the runtime's
shared-acquire function; that the wait is the listening lock of board item
coord-wfbl-03-04 is still inferred from the symbol, and no counter counts the
contexts woken per write.

**Reading of the fourth version's runs.** The version meant to read a number
instead of taking the event hub did not remove the loss where it was measured, the
file off: a push that creates its list is 0.83 of `main` at depth 16 (0.99 at depth
1), against 0.86 and 0.96 for the third version at the other placement, and the
prediction that it would lie across 1 was rejected by A1 and the cause was found
in the first statement's read of Meta (D, E and the mechanism above). With the file
on the creating push is not slower (1.01 and 1.00). An existing list loses 2% to 3%
at depth 16 to the probe's code (`lpush` with the file off 0.973, with it on 0.982,
`rpush` with it on 0.968), the stream commands no longer lose, and `mset` 1 loses
1% in A1 (0.9902, the upper end of the interval 0.9958). The hand-off is at 0.334
and 0.643 of the fastest competitor. By the criteria stated above these are cells
wholly on the slower side and criteria not met, and they go to the owner on the
board with these numbers; nothing in these runs is accepted by this record. An
information about waiters that costs no shared lock on an AOF-off push has to come
from what the statement already holds, the key's own entry; a mark left on the key
by a registration is such a design and is not part of this version.
