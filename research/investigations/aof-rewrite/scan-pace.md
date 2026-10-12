# A time-sliced live scan within the reserve

Status: implemented on the scan-within branch with Whitefoot
wf-8ffb6d93241f (specification v0.133); the network cases below have not
yet run in CI, and the 14900K comparison has not run. No completion or
performance result is claimed.

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
firn charges `after - before` with a subtraction the compiler proves.
`Needs(bytes)` inserts nothing: the scan sleeps outside every hold and
retries while `bytes` fits R less admission's fixed charge and the step
buffer, since journal staging drains and frees the rest, and aborts with
`reserve` otherwise.

The count hint starts at 64, the scan count this investigation first
proposed, halves after a step writing more than 64 KiB and doubles back
after one writing at most 32 KiB. When a step of several positions finds a
key copy, image or buffer growth that does not fit, it is discarded
unwritten, its cursor kept, and retried at hint 1; one position that does
not fit aborts with `reserve` as before. Steps run back to back; after
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
per-step sleep. A nonzero value scans one position per step and sleeps per
key in place of the slice pause, so these cases keep the pace they were
written against.

## Comparison and rejection criteria

Network cases: the quiescent 17-MiB key must abort with `reserve` with both
the independently sampled heap growth and the ledger peak within R; 100,000
keys must rewrite and restore under a concurrent writer in fewer than one
eighth as many steps as keys and within 30 s. Count hint 1 needs a step per
key, and its millisecond per step exceeds `rewrite_wait`'s 60 s.

On the idle 14900K through CI, repeat the million-key rewrite-during
workload at R = 16 MiB with `make firn-lto` builds of this branch and its
base, interleaved, with a base twin as noise control and Redis 7.0.15 under
identical data, connections, CPU, AOF and fsync settings. Record completion,
scan steps, rewrite duration, and throughput and latency during the rewrite.
The design is rejected if the rewrite still fails to complete that workload,
if client latency during the rewrite worsens beyond the control noise, or if
any case observes heap growth or a ledger charge beyond R.

## Results

None yet: CI has not run this branch's cases, and the 14900K comparison is
still to be run.
