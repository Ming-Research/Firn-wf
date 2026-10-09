# Halo embedding leak probe

Disposable instrument for `exp/halo-gc-stats`, never merged. The leak-probe
workflow builds and runs entry `halo_leak`, appending three CSV rows to
`leak-min.txt`. Remove it with the experiment branch.

Question: does the counted heap growth seen in firn's `EVAL "return 1" 0`
also occur without firn's eval bookkeeping? One engine compiles `return 1`
once, then three functions run in sequence using the same cached ScriptId:

- `a`: start the script, settle its outcome, and verify its one numeric result.
- `b`: also create and install empty KEYS and ARGV tables before every call,
  matching firn's prepare for zero keys and arguments.
- `c`: also explicitly reset after every completed call, as firn does when
  returning its engine. Halo's start already resets internally in all variants.

Each function warms up for 1,000 calls, samples `heap_in_use`, runs 10,000
more, samples again, then runs another 10,000 and samples. Output is
`variant,heap_after_warmup,heap_after_10k,heap_after_20k`, without a header.
Printing occurs after all three samples. For each interval, divide the
difference between successive samples by 10,000 for bytes per call.

Growth already in `a` implicates the embedded execution path; additional
growth in `b` or `c` isolates argument setup or explicit reset. Flat samples
in all three reject reproduction through these public API calls and leave
firn's surrounding path for investigation. This is counted live allocation,
not RSS; normal Halo collection and retained capacity can affect differences.
No forced collection changes firn's default GC settings. The probe does not
install firn's Redis host or reproduce its pool, registry, protocol conversion,
or attempt snapshots; `return 1` needs no host calls. Unexpected outcomes,
results, or output failures produce a nonzero status, not a successful row.

Compilation and execution are intentionally deferred to CI; no measurements
have been obtained from this instrument yet.
