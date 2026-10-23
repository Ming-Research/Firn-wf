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

Written before P2 runs, from P1's result; the segments of the table are the
candidates until then.

## Results

(P1 and P2 are filled in after the runs.)
