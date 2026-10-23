# Ordinary-command cost of publish/subscribe

## The question

Firn-wf#100 (publish/subscribe) made `redis-benchmark`'s ordinary commands
slower: in the before/after comparison of
[pubsub-throughput.md](pubsub-throughput.md#before-and-after-publishsubscribe)
(run 39573904480), 15 of 38 cells were wholly on the slower side, 13 of them at
depth 16, with medians from 0.947 to 0.991 and 0.889 for `lrange_100` at depth
16. Which part of Firn-wf#100 costs the time, and what removes the cost? The
run had no profile, so no cause has been measured.

## What the source shows

Read from the diff of `00e2bd1` (before) and `10c20af` (after); nothing here is
measured. A connection that never subscribes pays, for every command:

| segment | where | added work per command |
|---|---|---|
| S1 | `command_kind` (`commands/script.wf`) and the head of `execute` (`commands/dispatch.wf`) | `pubsub_kind` runs twice: one length comparison for a name under six bytes, and up to nine `name_is` calls (each rejecting at once on a different length, or on the first byte) for `LRANGE`, `ZPOPMIN` and other longer names |
| S2 | `execute_client` (`commands/transaction.wf`), `admission_arity` and `command_denyoom` (`commands/admission.wf`) | `kind == 193`, `subscriber_mode`, `subscriber_refused` (a call that reads `client^.resp` and the three subscription counts), and a few more comparisons |
| S3 | `serve` (`server/server.wf`) | `client.pubsub_id != 0` after every command, the `promoted` flag, and the code that hands the connection to `serve_promoted` in the same function |
| S4 | `main` (`server/server.wf`) | two tasks, `dispatch` and `timer`, spawned at start; each waits in an `atomic ... when` statement on the event hub |
| S5 | `Client` (`protocol/module.wfm`), `Keyspace` (`store/module.wfm`) | four `u64` fields (`pubsub_id`, `sub`, `psub`, `ssub`) before `name`, which moves the offsets of every field after `id`, among them `name`, `authenticated`, `locked` and `resp`; and a `pubsub` handle in `Keyspace` |
| S6 | the whole program | `execute`, `serve` and `execute_client` grow; the compiler's inlining and the code layout under `make firn-lto` may change in ways no segment above names |

The cost shows at depth 16 and not at depth 1 (deduction from the run above):
a server does the most command work between its reads at depth 16, so a cost
per command shows there, while a cost per wake-up or per read would show at
depth 1 as well.

## Criteria, stated before measuring

The compiler pin is the same in all images (`wf-af7665be672d` at `00e2bd1`,
`10c20af` and `7f8b4f8`).

### Run P1: profile

- **Images.** `before` (`00e2bd1`), `after` (`10c20af`), `main` (`7f8b4f8`, the
  main line when the item started, which holds the changes merged after
  Firn-wf#100) and `main-twin` (`7f8b4f8` again, the noise control), by the
  compare mode of `redis-bench.yml`.
- **Settings.** The 14900K, 2 server CPUs, placement as in AGENTS.md, depth 16,
  the tests `get`, `lrange_100` and `ping_mbulk`, 4 interleaved passes of 5 s;
  a profile with call stacks (`perf record --call-graph dwarf`, 999 Hz, the
  workflow's `callers` input) of each image under `get` and under `lrange_100`
  at depth 16.
- **Comparison.** The server CPU time per request (`compare.csv`) and the rate
  of `after`, `main` and `main-twin` against `before`; per symbol, the cycles
  per request, which is the symbol's share of the profile's samples times the
  CPU time per request of the profiled test.
- **What it decides.** A symbol counts as grown when its cycles per request in
  `after` exceed those in `before` by more than `main` and `main-twin` differ
  in the same symbol. The deduction above (a cost per command, in firn's own
  command code) is rejected if the extra cycles per request of `after` stand in
  the kernel or in the runtime's scheduling and locking symbols
  (`wf_executor_loop`, `wf_shared_acquire_as`, the TCP send path) instead; then
  S4 and S5 are tested before S1 to S3. If `main` runs as fast as `before` in
  this run, the slowdown is not reproduced and the cause cannot be located by
  this comparison. P1 also says whether the main line still carries the whole
  slowdown, which the acceptance row of the item measures.

### Run P2: reverting segments

Written after P1 (below) and before P2 runs. The question: which of S1 to S5
makes an ordinary command cost more, and do they account for all of it?

- **Images.** `before` (`00e2bd1`), `main` (`7f8b4f8`), `main-twin` (`7f8b4f8`
  again) and six images that remove the segments from `main` one after another,
  each on top of the one before (the branch `firn-pubsub-cmd-regression-exp`,
  where the experiment images are commits that no pull request merges):
  `v1` has no publish/subscribe dispatch at the head of `execute` (S1, the
  second `pubsub_kind` call); `v2` also none in `command_kind` (S1, the first
  call); `v3` also none of the subscriber checks of `execute_client`,
  `admission_arity` and `command_denyoom` (S2); `v4` also nothing of the
  promotion to the subscriber loop in `serve` (S3); `v5` also does not spawn the
  two event hub tasks (S4); `v6` also keeps the four subscription fields after
  the rest of `Client` (S5). Publish/subscribe commands do not work in these
  images and the run sends none; the correctness pass of the compare mode still
  checks their replies to the commands it sends.
- **Settings.** The 14900K with the placement of AGENTS.md set by hand, servers
  on CPUs 2 and 4 and clients on CPUs 16-31 (P1's automatic placement put the
  clients on P-cores, where `lrange_100` was client-limited), 2 server CPUs,
  depth 16, the tests `get`, `set`, `ping_mbulk` and `lrange_100`, 8
  interleaved passes of 5 s; a flat profile of `get` at depth 16 for every image
  with the instructions of `execute` and `execute_client` annotated.
- **Measure.** The ratio, pass by pass, of the server CPU time per request
  of the image it is compared with to that of the image compared (`summary.py
  ratios --metric cpu`; below 1 is slower, as for the rate), with the 95%
  interval of the mean log over the 8 passes; the rate ratio beside it. CPU time
  per request is the measure because it does not depend on the client, which
  limits the rate of a test whose client is busy.
- **Reading.** A segment is a cause when the image without it, against the image
  with it (`v1`/`main`, `v2`/`v1`, ..., `v6`/`v5`), has an interval wholly on
  the faster side, and `main-twin` against `main` has an interval holding 1. The
  hypothesis that S1 to S5 are the whole cost is rejected if `v6` against
  `before` is still wholly on the slower side for `get` or `ping_mbulk`; the
  cost left then belongs to what the compiler does with the larger functions
  (S6) or to other changes of Firn-wf#100, and the annotated profiles are read
  for where. Because the images are cumulative, a segment's cost may depend on
  the ones removed before it; the table reports what was measured, not a sum.

## Results

### P1: the profile

Run 39902843039 (`compare`, 4 passes of 5 s, depth 16, 2 server CPUs; the
placement chosen by the script: servers on CPUs 2 and 4, clients on P-cores
6-15, not the placement of AGENTS.md, which P2 sets). 15 of the 16 samples of
`lrange_100` are client-limited (client busy fraction up to 0.96), so its rates
are the client's; `get` and `ping_mbulk` are not (at most 0.27). Server CPU time
per request, mean of 4 passes, and its ratio to `before` as `summary.py ratios
--metric cpu` gives it (median of the per-pass ratios, 95% interval; below 1 is
slower):

| test | before | after | main | main-twin | after/before | main/before | main-twin/main |
|---|---|---|---|---|---|---|---|
| `get` | 0.2520 us | 0.2678 us | 0.2700 us | 0.2718 us | 0.940 (0.934-0.949) | 0.931 (0.908-0.959) | 0.989 (0.975-1.012) |
| `ping_mbulk` | 0.2290 us | 0.2440 us | 0.2482 us | 0.2475 us | 0.937 (0.925-0.952) | 0.920 (0.908-0.937) | 1.000 (0.981-1.025) |
| `lrange_100` | 1.1458 us | 1.3228 us | 1.1745 us | 1.1778 us | 0.864 (0.848-0.885) | 0.977 (0.967-0.984) | 0.997 (0.982-1.012) |

- The slowdown is in the server and still on the main line: `main` costs 7% more
  CPU per request than `before` on `get` and 8% on `ping_mbulk`, as `after`
  did; `main-twin` is within 1% of `main`. The 15% extra of `after` on
  `lrange_100` is gone in `main` (2.5% extra left), so the main line has
  changed that cost without anyone aiming at it.
- Profiles with call stacks (flat shares times CPU time per request, nanoseconds
  per request; `get`; 8-9K samples each, so a symbol under about 0.5 ns
  is within sampling noise): the kernel's share is the same (114.4 `before`,
  115.9 `after`, 116.6 `main`, 117.3 `main-twin`), and the rise is in firn's
  code, 137.0 `before`, 150.8 `after`, 152.4 `main`, 153.5 `main-twin`, 14 ns
  of the 16 ns. In `after` against `before`: `execute_client` and
  `admission_writes` together +3.4 (45.3 to 48.7; `admission_writes`, 22.6 ns,
  stands as a symbol of its own in `after` and was inlined into `execute_client`
  in `before`), `execute` +4.1 (8.1 to 12.2), `subscriber_refused` +0.9,
  `pubsub_kind` +0.4, and about 5 ns spread thinly over `access_existing_entry`,
  `record_memory_peak`, `wf_context_allocate` and others. The two background
  tasks of S4 and the runtime's executor and locking symbols show no rise.
  The same symbols rise in `main` and `main-twin`.
- For `lrange_100` the profile differs: `try_entry` (the entry lock in the
  runtime) is 363.6 ns per request in `before`, 497.4 in `after`, 385.8 in
  `main` and 373.8 in `main-twin`, and accounts for 134 of the 177 ns the
  request gained in `after`. Nothing in the diff of Firn-wf#100 touches that
  function, so it is the layout or inlining of the program (S6), which the main
  line lost again.
- The deduction of the question (a cost per command in firn's own code) holds;
  it is not rejected. The cost is spread over `execute`, `execute_client` and
  what the compiler did to the functions around them, so P2 is run.
