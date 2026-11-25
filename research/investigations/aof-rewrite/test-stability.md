# Intermittent failures of two rewrite cases

Board item: `firn-rw-memory-tests-flaky`.

## Question and rejection criteria

Two network cases that rewrite the append-only file under a memory limit,
`rewrite_scan::reconciled_scan_long_history_completes_under_writes_without_duplicate_effects`
and
`memory_limit::allkeys_lru_cache_at_limit_rewrites_and_restores_surviving_data`,
failed in CI on revisions whose changes did not touch them and passed on rerun.
Is the margin the cases give the rewrite unfounded, or does firn's reserve
accounting differ between runs?

Firn's accounting would be at fault if a failing run's in-use total departed
from the charges the design nodes list (`design/firn/aof-rewrite/capture.md`):
a charge not made, or made twice. A test fix would be
accepted only if the case that failed is red every time before it and green
after it under a condition that reproduces the failure; where no such
condition is found, if the number of green whole-suite runs `n` makes an
all-green result unlikely (below 5%) for the failure rate measured before the
fix.

## What failed, from CI history

The `check` workflow's network job ran 302 times from 2026-10-13 to
2026-10-21 (243 reached the tests). Every run below was read with `gh`.

| Case | Executions | Red | Verdict |
| --- | --- | --- | --- |
| `long_history` | 243 | 4 (1851bc1, 6880a70, 9db74aa, 36e145b) | intermittent on main; three were red on the first attempt and green on rerun of the same commit |
| `cache_at_limit` | 70 since c0ada67 | 1 (run 39547781759, 11c692b) | intermittent on main |
| `pressure_after_s1…` | 36 after 8d20576 | 0 | the earlier reds were development commits of Firn-wf#86 |
| `allkeys_lru_*aborted_rewrite*` | none on main | none | the cases existed only in development commits of Firn-wf#86 |

The three red runs of `long_history` that were read in full (39533585022 at
36e145b, 39521620733 at 9db74aa, 39514254640 at 6880a70) show the same INFO:
`aof_last_bgrewrite_status` failed with reason `reserve`, `cut=1`,
`scan_done=1`, and `peak = used = 16721192` against a reserve of 16777216.

## Why `long_history` fails

The case holds a 128-MiB history and writes continuously from a worker while
the rewrite runs; it passes if S1 (the cut that rotates the collecting buffer)
does no work proportional to the history.

16721192 is the sum of every doubling's full new capacity of the collecting
buffer, from 128 KiB to 8 MiB, 128 KiB × (2⁷ − 1) = 16646144, plus 75048 of the
rewrite's other charges (the difference). The design has the encoder charge a
growing buffer's full new capacity, overlap included (`design/firn/aof-rewrite/capture.md`,
the rotation decision); it does not say when the old capacity is refunded.
`log_reserve` in `firn/store/store.wf` adds each growth to the statement's
charge and refunds nothing, and the total at the abort, which equals the sum
of all seven doublings, shows that none was refunded while the buffer grew. The
reserve therefore holds the rewrite only while the backlog between the cut and
the publication of the new increment stays at or below 4 MiB; past it the
doubling to 8 MiB leaves 16777216 − 16721192 = 56024 bytes, and the worker's
next charge, the 64-KiB readback buffer (65560 bytes), fails.

What varies between runs is the time from the cut to publication, which holds
several fsyncs. Measured on CI runners with the whole network suite
(32 executions, run 39569260143): 1.2 to 5.6 ms, median 1.7 ms, with the
worker writing one to three rounds. The worker writes about 13 MB/s, so 4 MiB
takes about 0.33 s. Under injected disk contention, two loops writing 600 MB
files with `fsync` in `/tmp` (run 39572426406), the original case was red in
59 of 60 runs with the same INFO as above, and the cut-to-publication time was
0.6 to 0.9 s with 8 to 12 MB of backlog. Six busy loops on the CPU gave 0 of 60.
In the gate, the case and `post_cut_backlog` each build a 134-MB fixture and
start close together in the suite's alphabetical order, so each can make the
other's fsyncs wait (inferred from the injected result, not measured in the
gate).

The accounting is stable: each failing run matches the design exactly. The
case's assumption that fsyncs return in milliseconds is what failed.

## Why `cache_at_limit` fails

The reserve R is a twentieth of `maxmemory`: 854212 bytes. The live scan steps
start at a count hint of 64 keys and charge their key set, key copies, images
and 64-KiB buffer before the hint adapts. With 16-KiB values the rewrite's
peak use of R was 820606 to 850735 bytes (96.1 to 99.6% of R) in the 201 green
executions of the diagnostics (100 alone and 30 in the first whole-suite run
39569260143, 71 in the second, 39575213690), reached in the first few steps.
A statement's capture allowance is the limit less what is in use when the
statement starts; a SET that starts during that peak sees about 3 KB, cannot
capture its 16-KiB image, and aborts the rewrite with `capture`. The three red
executions (2 of 32 in the first run, 1 of 72 in the second) all show
`used = 851009` against R = 854212 at the abort, 3203 bytes left. The
collision needs the worker thread to start late, so that the four SETs after
the `BGREWRITEAOF` reply land in the first steps; that happens only under the
whole suite's load, never alone (100 of 100 green, and 0 of 150 under injected
CPU or disk contention).

With 1-KiB values the rewrite's peak use of R was 426984 to 560748 bytes (50.0
to 65.7% of R = 853393) over the 72 whole-suite executions of the second run,
which leaves at least 292645 bytes of allowance; a SET of a 1-KiB value needs
little more than 1 KiB.

## Reproduction and detection of the old behavior

| Condition | Case | Result |
| --- | --- | --- |
| `long_history` alone, injected disk contention | original | 59 of 60 red (run 39572426406); 19 of 20 red (run 39579432492) |
| the same, files in `/dev/shm` | fixed | 0 of 60 (run 39572426406); 0 of 60 (run 39579432492) |
| `long_history` alone, six busy CPU loops | original | 0 of 60 (run 39572426406) |
| the same, files in `/dev/shm` | fixed | 0 of 60 (run 39579432492) |
| `long_history` with the log boundary made unknown, files in `/dev/shm` or on disk | stand-in for the full-history reread | 40 of 40 red on each (run 39575213690) |
| `cache_at_limit` alone, injected CPU or disk contention | original | 0 of 150 each |
| `cache_at_limit`, writes continuing until the rewrite ends | 16-KiB values / 1-KiB values | 40 of 40 red / 0 of 40 (run 39575213690) |

The stand-in is the setup of `post_cut_backlog`: a file-size limit makes one
append fail, which leaves the increment's boundary unknown under the I/O
contract, and the exceptional switch-time reread then reads the whole 134-MB
history, the work the design rejected as a routine ("Full-history reread at
every S1 switch" in `design/firn/aof-rewrite.md`). The case moved to memory
still fails on it, so it still detects work proportional to the history.

The continuous-write variant fails by `rotation_reserve`, not `capture`: its
backlog grows without limit, which is a different abort. It does not reproduce
the red seen in CI, so `cache_at_limit` has no deterministic reproduction and
the `n` rule below applies to it.

## The fixes

- `long_history` keeps its files in `/dev/shm`. It checks that S1 does no work
  proportional to the history; fsync latency is not what it measures. The
  other rewrite cases keep their files on disk.
- `cache_at_limit` uses 1-KiB values instead of 16-KiB, and fills 512 batches
  of 32 keys at most instead of 128. The test still fills the cache until
  eviction starts; eight MiB of cache at 1 KiB is about 8 thousand SETs, 16
  times as many as before.

No firn source or design node changes.

## Number of runs

The failure rate of the original `cache_at_limit` under the gate's load, the
whole suite run with the gate's command and parallelism, pools 1 of 70 (CI
history since c0ada67), 2 of 32 (run 39569260143) and 1 of 72 (run
39575213690): 4 of 174, p = 2.30%.

n = ceil(ln 0.05 / ln(1 − p)) = ceil(−2.9957 / −0.023258) = 129. If the case
were unfixed, 129 all-green executions would have a probability of about 5%.
Run 39579432492 ran the whole suite 132 times with the fixed cases (12 legs of
11): both cases passed in all 132. The probability of 132 all-green executions
at the unfixed rate is (1 − 0.0230)^132 = 4.6%.

`long_history`'s natural rate, 4 of 243 (1.65%), would need 180; the injected
disk contention replaces that, since it makes the original case red in almost
every run and the fixed case green in all.

## Other intermittent failures seen, not these cases

Four of the 132 executions failed in four other cases, once each, with a
socket read failing in the test client (`network.rs:127` or `:288`):
`firn_answers_the_value_types_as_redis_does_with_hashes_and_sorted_sets`,
`firn_answers_the_value_types_as_redis_does`,
`aof_write_errors::aof_latched_error_shutdown_and_signals_exit_zero_on_both_routes`
and `collection_scan::zscan_score_text_uses_current_cardinality_before_match`.
Earlier diagnostic runs saw `pubsub::soft_output_limit_*`,
`collection_scan::multi_resp2_oracle`,
`firn_script_clients_direct_unpack_boundaries` and
`firn_script_clients_direct_arity_and_execution_errors`. None is analyzed here.

## Weaknesses of the design noticed

Neither is changed here; each is a design decision.

- The first scan step starts at 64 keys whatever R is, so a small R with large
  values lets one step hold nearly all of it; a statement during that step has
  no allowance and the rewrite aborts with `capture`. A slow start of the count
  hint, or a cap on a step's share of R, would avoid it.
- Rotation charges for a doubling collecting buffer are not refunded when the
  buffer moves (the design is silent on the refund), so with R = 16 MiB the
  backlog is bounded by 4 MiB, a quarter of R, because the doublings up to 8 MiB
  already charge R.
