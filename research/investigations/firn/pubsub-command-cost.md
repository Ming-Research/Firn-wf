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

### P2: reverting the segments

Run 39906790106 (`compare`, 8 interleaved passes of 5 s, depth 16, 2 server CPUs
on CPUs 2 and 4, clients on CPUs 16-31; no sample is client-limited, the
largest client busy fraction is 0.64; the images are those of the criteria
above, built from the commits `33a5ef1` to `fa0dcb4`). Server CPU time per
request in microseconds, mean of the 8 passes:

| test | before | main | main-twin | v1 | v2 | v3 | v4 | v5 | v6 |
|---|---|---|---|---|---|---|---|---|---|
| `get` | 0.2575 | 0.2742 | 0.2736 | 0.2690 | 0.2558 | 0.2546 | 0.2526 | 0.2521 | 0.2526 |
| `set` | 0.3021 | 0.3185 | 0.3177 | 0.3130 | 0.3115 | 0.3056 | 0.3029 | 0.3019 | 0.3023 |
| `ping_mbulk` | 0.2345 | 0.2521 | 0.2514 | 0.2449 | 0.2339 | 0.2305 | 0.2299 | 0.2300 | 0.2295 |
| `lrange_100` | 1.1869 | 1.2043 | 1.2106 | 1.2075 | 1.1827 | 1.1954 | 1.1889 | 1.1775 | 1.1829 |

The ratios of `summary.py ratios --metric cpu` (median of the 8 per-pass
ratios and its 95% interval; head over base, above 1 is faster):

| head / base | `get` | `set` | `ping_mbulk` | `lrange_100` |
|---|---|---|---|---|
| main / before | 0.936 (0.933-0.945) | 0.950 (0.939-0.959) | 0.929 (0.923-0.937) | 0.984 (0.978-0.994) |
| main-twin / main | 1.002 (0.997-1.007) | 0.998 (0.993-1.012) | 1.002 (0.996-1.010) | 1.002 (0.984-1.006) |
| v1 / main (S1, `execute`) | 1.020 (1.016-1.023) | 1.018 (1.006-1.030) | 1.027 (1.023-1.036) | 0.998 (0.990-1.004) |
| v2 / v1 (S1, `command_kind`) | 1.053 (1.045-1.058) | 1.003 (0.996-1.014) | 1.045 (1.040-1.054) | 1.021 (1.016-1.026) |
| v3 / v2 (S2) | 1.000 (0.998-1.011) | 1.020 (1.014-1.025) | 1.015 (1.004-1.026) | 0.990 (0.985-0.994) |
| v4 / v3 (S3) | 1.008 (1.001-1.015) | 1.008 (1.002-1.017) | 1.000 (0.995-1.010) | 1.008 (0.997-1.014) |
| v5 / v4 (S4) | 1.002 (0.996-1.008) | 1.003 (0.994-1.013) | 1.002 (0.993-1.006) | 1.006 (1.002-1.018) |
| v6 / v5 (S5) | 1.000 (0.990-1.007) | 1.003 (0.989-1.009) | 1.002 (0.994-1.011) | 0.990 (0.988-1.003) |
| v6 / before (all removed) | 1.020 (1.011-1.028) | 0.998 (0.992-1.007) | 1.022 (1.011-1.033) | 1.005 (0.997-1.010) |

- The hypothesis that S1 to S5 are the whole cost is not rejected: `v6`
  against `before` is nowhere wholly on the slower side.
- S1, the two `pubsub_kind` calls, is most of it for the cheap commands: 7% of
  the CPU per request on `get` and 7% on `ping_mbulk`, 2% on `set` and 2% on
  `lrange_100`. S2 costs `set` 2% and `ping_mbulk` 1.5%, S3 about 1% on `get`
  and `set`. S4 (the two tasks) and S5 (the field order) cost nothing that
  twin-wide intervals show. `set` is back at `before` only when S1, S2 and S3
  are all removed (`v2`/`before` 0.973 (0.963-0.977), `v3`/`before` 0.990
  (0.983-0.994), `v6` at 0.998 (0.992-1.007)): S2 and S3 are the subscriber features
  themselves and can be made cheap, not removed.
- Where it goes, `get` (flat profiles of the first pass, nanoseconds per
  request, 4300-8200 samples per function): `execute_client` with
  `admission_writes` 44.1 `before`, 51.0 `main`, 49.3 `v1`, 43.9 `v2`, 43.0
  `v3`; `execute` 8.8, 11.7, 10.9, 8.8, 8.7; `subscriber_refused` 1.4 in
  `main` and 0 in `v3`. In `before` the compiler had inlined `admission_writes`
  into `execute_client`; in `main` and `v1` it stands as a function of its own,
  and in `v2` and later it does too, at 18.9 and 18.0 ns against 23.0 in
  `main`: the same loop costs 4 ns more in `main`.
- The loops dominate. The instructions the profile annotates show that, in
  `before`, 41% and 23% of the cycles of `execute_client` (18.3 and 10.2 ns)
  are two loops that compare the command's kind with each entry of a list, the
  longer being the 61-entry scan of `command_writes` and the shorter the
  34-entry scan of `command_denyoom` (the two loops' cycles are in the ratio
  1.79, as the entries are), and that every command runs to the end of both, since
  `GET`, `SET` and `PING` are not in the lists that decide a refusal. In
  `main` the same two scans cost 22.3 and 10.8 ns: about 33 ns of the 274 ns
  a `get` costs, and 28.5 ns of 257 ns in `before`.

## Cause

The cost of a command's admission is a scan of two lists, 95 comparisons for
every command that is on neither list, about 30 ns, which is 11% of the CPU a
`get` takes. The scans were there before Firn-wf#100 and their cost depends on
what the compiler does with the functions around them; Firn-wf#100 changed
that twice: it put the names of nine publish/subscribe commands, compared by
nine inlined `name_is` loops, at the head of `command_kind` and of `execute`,
which grows the functions every command passes through (S1: 7%), and it added
the subscriber checks (S2) and the promotion check in `serve` (S3), 1-2% each,
which cannot go. The slowdown of the cheap commands is the sum; the larger
cost of `lrange_100` in `after` that the main line lost again (see P1) is
probably the same kind of effect, in the runtime's entry lock (deduced, not
tested).

The root is that a decision of constant cost is made with a scan whose cost
changes with the code around it; any later change that grows a hot function can
move it by several percent again.

## The fix

1. `command_writes` and `command_denyoom` read a 256-byte table indexed by the
   command's kind instead of scanning a list (`commands/admission.wf`). The
   tables are generated from the lists they replace and checked equal for
   kinds 0 to 999; the subscription kinds that `command_denyoom` listed in code
   are in the table. Expected: about 30 ns less per command on every command,
   more than the 18 ns of the slowdown.
2. `command_kind` and `execute` try the publish/subscribe names last, after the
   names that are not publish/subscribe do not match, so that no other command
   pays for them (S1). A publish/subscribe command pays the chain of the other
   names first: `execute` reaches `run_pubsub` after its chain of comparisons
   and the two lookups after it instead of at its head. The delivery path
   costs nanoseconds there against microseconds for a delivery
   (pubsub-throughput.md: 350 thousand deliveries a second), but a later
   change that makes a publication cheap should classify once and route by
   kind.
3. S2 and S3 stay.

### Run P3: the fix, probe

Written before P3 runs. The question: does the fix bring the ordinary commands
to the rate of `before`, and which of its parts does it?

- **Images.** `before` (`00e2bd1`), `main` (`7f8b4f8`), `main-twin`, `f1`
  (`bc7489e`, the tables only) and `f123` (`7a584d4`, the tables and the
  classification order).
- **Settings and measure.** As P2: CPUs 2 and 4 for the servers, 16-31 for the
  clients, depth 16, `get`, `set`, `ping_mbulk` and `lrange_100`, 6 passes of
  5 s, no profile; the ratio of server CPU time per request.
- **Reading.** The fix works when `f123`/`before` is wholly on the faster side
  or holds 1 for every test. It is rejected as insufficient when that interval
  is wholly on the slower side for any test; S2 and S3 would then need work of
  their own. `f1`/`main` and `f123`/`f1` give the part of each step; a part
  whose interval holds 1 does nothing measurable and is questioned in review.
