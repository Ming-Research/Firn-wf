# A time-sliced live scan within the reserve

Status: the scan-within implementation was introduced with Whitefoot
wf-8ffb6d93241f (specification v0.133). The later continuous-write sample
below finished scanning but failed during switch validation. The proposed
boundary-state fix passes the gate (run
[38383983743](https://github.com/Ming-Research/Firn-wf/actions/runs/38383983743)
on cf1492d); its writer cost has not yet been compared on the 14900K.

## Question

Can the rewrite's live scan cross a million keys in seconds under write load
while every byte it allocates stays within the reserve R? The owner's
`firn-q-rw-pace` option A asks for both: the scan yields its driver once per
time slice of about 1 ms instead of sleeping after each one-key step, and R
covers only scan and buffer memory, the journal itself going to disk
([journal-disk.md](journal-disk.md)).

## Prior evidence

On the 14900K (redis-bench rewrite-during, 1,000,000 keys, 64-byte values,
50 writing connections; runs
[38142167582](https://github.com/Ming-Research/Firn-wf/actions/runs/38142167582)
and [38142529829](https://github.com/Ming-Research/Firn-wf/actions/runs/38142529829)),
the rewrite aborted after 2,266 scan steps at R = 16 MiB and after 67,854
at R = 429 MB, where Redis 7.0.15 finished in about 1.6 s. Each step used
`map_scan` with count hint 1 and then slept 100 microseconds. Whitefoot's
driver waits for its earliest timer in whole milliseconds rounded up
(`wf_timer_wait_ms`, `compiler/src/backend/completion/bridge.c` at
Whitefoot 8ffb6d932), so a step costs about a millisecond when nothing else
wakes the driver; this is a reading of the source, not a measurement.

The same scan charged a step's KeySet only after `map_scan` returned, so one
step could exceed R by its keys
([stage-2 limitation](scan-log.md#reserve-and-the-scan-exception)); the
quiescent 17-MiB-key case stayed ignored for it.

## Design

Each step holds the whole map and Meta, makes a KeySet with capacity 0,
charges the bytes `key_set_bytes` reports for it, and calls
`map_scan_within` with the reserve then left after the step's 64-KiB output
buffer as its limit. The call's heap growth, temporary storage included,
stays within that limit, and its postconditions bound the KeySet's bytes
after the call between their value before and that value plus the limit, so
firn charges `after - before`, for either result, with a subtraction the
compiler proves. `Needs(bytes)` inserts nothing: the scan sleeps outside
every hold and retries while `bytes` fits R less admission's fixed charge,
the KeySet's bytes and the step buffer, since journal staging and statement
scratch drain and free the rest, and aborts with `reserve` otherwise.

Key copies, images and output-buffer growth are charged before allocation
without aborting. A step with one that does not fit is discarded unwritten,
its cursor kept. A step of several positions is retried at count hint 1 at
once. A one-position step records its whole need so far, every charge it
made plus the one that did not fit, and waits and retries as `Needs` does
while that need could fit R less admission's fixed charge, aborting with
`reserve` otherwise. That need is a lower bound of the step's total, so a
step that can never fit aborts at the latest on the retry that reaches its
larger charges with every drainable byte free.

The count hint asks for about that many keys per step: the specification
leaves the extent to the runtime, and the pinned runtime stops once it holds
at least that many keys, keeping whole positions together. It starts at 64, the scan
count this investigation first proposed, halves after a step writing more
than 64 KiB and doubles back after one writing at most 32 KiB. Steps run
back to back; after
each, the scan reads the monotonic clock and sleeps 100 microseconds once
1 ms has passed since it last slept, which yields the driver to clients.

`key_set_new` states no byte bound, so the creation charge follows the
allocation; with capacity 0 the pinned runtime allocates nothing then. This
is reported as a contract gap rather than worked around.

### Testing delay

Cases that write during a rewrite relied on the slow scan to stay in
progress. They now set Redis 7.0.15's hidden `rdb-key-save-delay`, which
Redis's `rewriteAppendOnlyFileRio` (`aof.c:2337`) applies after each key and
Redis's own suite sets for the same purpose, to 100 microseconds, the old
per-step sleep. Any nonzero value scans one position per step; a positive
one sleeps per key in place of the slice pause, so these cases keep the pace
they were written against. Every sleep is rounded up to whole milliseconds
by the driver's timer. A delay is taken in slices of at most 100 ms that end
when CONFIG SET changes it or the rewrite is cancelled, so SHUTDOWN stops a
rewrite delayed by `2147483647`, the value Redis's `unit/type/set` sets,
within about 100 ms; Redis kills its rewrite child instead.

## Comparison and rejection criteria

Network cases: the quiescent 17-MiB key must abort with `reserve` with both
the independently sampled heap growth and the ledger peak within R; 100,000
keys must rewrite and restore under a concurrent writer in fewer than one
eighth as many steps as keys and within 30 s. Count hint 1 needs a step per
key, and its millisecond per step exceeds `rewrite_wait`'s 60 s. 100 keys
of 1 MiB at R = 16 MiB must rewrite each key's image once and restore, in
more steps than keys, which only a discarded step explains. A rewrite delayed
by `2147483647` microseconds per key must stop within five seconds of
SHUTDOWN, and finish once CONFIG SET returns the delay to 0.

No deterministic case covers the wait path. It needs part of R held by
journal staging or statement scratch at the moment a step does not fit,
and the sole writer drains staging on its next turn and scratch is released
when its statement commits; holding either from outside needs a server test
hook or stalled host I/O.

On the idle 14900K through CI, repeat the million-key rewrite-during
workload at R = 16 MiB with `make firn-lto` builds of this branch and its
base, interleaved, with a base twin as noise control and Redis 7.0.15 under
identical data, connections, CPU, AOF and fsync settings. Record completion,
scan steps, rewrite duration, and throughput and latency during the rewrite.
The design is rejected if the rewrite still fails to complete that workload,
if client latency during the rewrite worsens beyond the control noise, or if
any case observes heap growth or a ledger charge beyond R.

## Results

### Switch validation under continuous writes

**Observation (2026-10-13, supplied attribution).** The 14900K sample
[run 38379619656](https://github.com/Ming-Research/Firn-wf/actions/runs/38379619656),
at [760d189](https://github.com/Ming-Research/Firn-wf/commit/760d189fa164f9dda3a5f3b77e30c758ebc97393),
reported 15,625 scan steps, scan_done=1 and cut=1, then aborted with
rotation_reserve at R = 16,777,216 bytes. The persistence, store, design and
compiler-pin paths have no differences from the implementation baseline
[c5571e8](https://github.com/Ming-Research/Firn-wf/commit/c5571e8c6dacde521587d57fbbf15185df40d626).
These are supplied log observations, not a new measurement or evidence of
data loss; neither scan_done nor cut establishes a successful switch.

**Attribution.** At S1 the sole writer suspends ordinary draining while it
rereads the whole current increment from byte zero, using 64-KiB reads and
a 100-microsecond sleep after each chunk. Client writes after S1 therefore
accumulate in the collecting buffer. The following accounting is
**reconstructed** from the code and the benchmark's `firn.aof` layout:
61,824 bytes of fixed overhead + 589,872 bytes of boundary-reader storage +
8,257,680 bytes for six collecting-buffer expansions from 128 KiB through
4 MiB = 8,909,376 bytes used. The next expansion, **4 MiB to 8 MiB**, needs
8,388,632 bytes, exceeding the remaining 7,867,840 bytes by 520,792.
The failed charge is not added, and prior growth charges stay until cleanup.
INFO reports the use, not the failing capacity. The roughly 525-MB file
dump was taken after abort; its size is not an exact S1 file size. Actual
reread pause duration, scan-to-S1 delay and firn's phase write rate were not
measured. The abort obeyed the reserve policy, but failure to finish rejects
the workload criterion above.

Redis 7.2.16 flushes the old buffer and opens/publishes the new increment
before fork and does not reread the old increment at that switch
([AOF source](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2427-L2464)).
The cached source archive was checked against the SHA-256 pinned by
`tests/redis-suite/reference.sh`. Firn must retain rotation at S1: its live
scan plus journal produces base(S1), and moving rotation to S0 would replay
already-incorporated INCR, APPEND and list effects twice.

**Fix (proposed, implemented for CI).** Keep a scalar boundary with the
writer's active file: known complete syntax, whether a block is open, its
first MULTI offset and file length. Startup computes it during existing
replay, after accounting for startup's actual cut. A partial-command cut
retains the preceding block; a clean-EOF cut before the latest nested MULTI
retains an earlier unmatched MULTI, if one exists. Each completely appended
buffer advances the state using the existing streaming RESP parser and
shared MULTI/EXEC classifier. Nested MULTI preserves the first offset; EXEC
clears the open-block flag. Positive short writes are retried before this
update; a successful rollback leaves the previous state intact. A failed
append without successful rollback, a failed switch truncation, unexpected
generated syntax, or opening an existing unlisted increment invalidates the
state. Only unknown state takes the existing charged, cancellable disk reread;
a successful reread restores the cache even if a later switch step fails.
A successful switch truncation updates the cache immediately; manifest
publication resets it for the adopted empty file, even if directory sync
then reports failure. Thus a known cache describes exactly the bytes the
former reader would parse, under the existing sole-writer assumption.

The cache and decoder are fixed-size; RESP values are traversed in the
already-owned append buffer without copying or retaining historical payloads.
Their small fixed storage fits the existing writer overhead. R, conservative
growth charges, S1 ordering, truncation policy and manifest failure semantics
are unchanged. The healthy path removes O(history) work from switching;
exceptional fallback can still exhaust the rotation reserve. CI correctness,
the old-revision failure control and a 14900K comparison (including added
ordinary-append parsing cost and latency) remain unverified.

**Regression coverage.** The long-history continuous-write
case keeps more than 128 MiB of overwritten SET history at maxmemory zero
(R = 16,777,216 bytes), writes through an observed S1, requires rewrite
success, and compares live and restarted INCR counts, APPEND bytes and RPUSH
order against acknowledged requests. Run against main's full-history reader
(control run [38383712698](https://github.com/Ming-Research/Firn-wf/actions/runs/38383712698),
network job, main plus only the test files), the case failed as predicted:
aof_last_bgrewrite_status err, abort reason rotation_reserve, cut 1,
scan_done 1, rewrite peak and used 8,922,432 bytes. The existing post-cut abort case now first
forces an EFBIG append failure with the existing Linux file-limit fixture,
restores the limit and waits for append recovery. Its unknown boundary needs
the exceptional reread; at maxmemory 64 MiB (R = 3,355,443 bytes), writes
after S1 must still cause rotation_reserve,
leave the old manifest selected, and restore exactly. Thus ordinary history
no longer implies an abort, while genuine collecting-buffer pressure remains
covered without removing or increasing the reserve. A new nested clean-EOF
case checks that startup removes the latest MULTI but switch truncation still
removes the first retained one. Existing unfinished-block, nested partial-tail,
transaction-closes-block and failed-switch cases are unchanged.

### Per-append boundary parse cost

Carrying the boundary means each successful AOF append also runs the
streaming parser over the appended bytes. The approval card set the
criterion before measuring: on the i9-14900K with AOF on, SET and MSET
throughput of the change against main stays within the range the twin of
main shows. Runs used `redis-bench.yml` compare mode from
`claude/bench-hybrid-placement`, with AOF everysec, automatic rewrite off,
depth 16, 50 connections, clients on CPUs 16-31 and 6 interleaved passes.
Builds were main (c6f6058), a twin of main and the change (b292f60). A
2-pass probe ([38391181421](https://github.com/Ming-Research/Firn-wf/actions/runs/38391181421))
confirmed that AOF was on in every line.

| Cores | Test | main median (req/s) | change/main | twin/main | change per pass | twin per pass |
|---|---|---|---|---|---|---|
| 2 ([38391672715](https://github.com/Ming-Research/Firn-wf/actions/runs/38391672715)) | SET | 3,523,884 | 0.958 | 0.957 | 0.908-1.071 | 0.920-1.068 |
| 2 | MSET | 953,236 | 0.970 | 1.021 | 0.933-1.022 | 0.944-1.081 |
| 8 ([38392398506](https://github.com/Ming-Research/Firn-wf/actions/runs/38392398506)) | SET | 1,187,236 | 1.003 | 0.877 | 0.745-1.264 | 0.578-1.783 |
| 8 | MSET | 862,453 | 0.989 | 0.954 | 0.914-1.034 | 0.812-1.193 |

Each median ratio of the change lies inside the twin's per-pass range, so
the per-append parse costs nothing these runs can resolve. With AOF on, the
twin's own spread is wide: 4% at 2 cores, and more at 8 cores, where SET
passes range from 0.58 to 1.78 of main. The comparison therefore bounds the
cost only to that noise. It is not evidence of zero cost.

These runs also show that with AOF on, main's SET at 8 P-cores (1.19M/s)
is a third of its rate at 2 (3.52M/s). The single AOF writer or its fsync
path limits throughput as cores rise. The change does not cause this, and
it is recorded for the core-scaling work.
