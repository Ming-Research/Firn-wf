# Shared guard fairness probe

## Question and prior criterion

With two runtime drivers on two server CPUs, does a single shared guarded
checkout develop acquisition tails disproportionate to its serialized hold
time as the number of competing contexts rises from 2 to 8 to 50?

Firn's limiter-script requests have reported p99 latency of 0.5–2.3 ms versus
Redis's 0.05–0.4 ms, with firn driver OS scheduling delays at most 0.05 ms.
Those observations motivate this experiment; they do not yet establish an
engine guard cause. First compare measured engine waits with the network
tail. If waits are small, reject the guard as the dominant explanation for
that workload and investigate the remaining request path.

Before running: a fairness concern is supported if p99 acquisition delay
grows much faster than `N × hold_ns_mean` across these context counts and
repeats across passes. Inspect `p99_ns / (N × hold_ns_mean)` and maximum
delay, rather than raw delay alone. Approximately proportional growth, or
tails small relative to the network p99, rejects this proposed explanation
under these conditions. An increasing ratio is evidence to investigate
runtime admission/wake policy, not proof that waking all waiters causes it.
If noisy, repeat or increase K only after the pilot's runtime and spread are
known. No measurements have been made by this change.

## Instrument

`probe.wf` is independent of firn and Halo. Exactly N spawned contexts each
perform K guarded acquisitions of one `Shared<State>`, 4096 dependent integer
mixing steps while held, and an unguarded release. The state holds `out`, an
acquisition counter, and an observable work checksum. Clock handles are shared
from the process monotonic clock, one per context. Each context stores delays
locally in a preallocated buffer; results are joined after all spawns have
been launched and sorted outside the runtime measurement using the standard
priority queue. There are no per-sample result aggregation locks.

The CSV columns are `contexts,rounds,count,p50_ns,p99_ns,max_ns,hold_ns_mean,
runtime_ns,checksum`. Percentiles are exact nearest-rank values over all N×K
acquisitions, without binning. Runtime includes context creation, acquisitions
and joins; it excludes flattening, sorting and output. There is no start
barrier: early acquisitions can precede later context creation. The pilot and
larger K help assess this finite-run effect. The counter and sample count must
both equal N×K, with a nonempty sample set, or the process exits unsuccessfully.

Each acquisition adds three clock reads: before the first attempt, at entry
to the successful guarded block, and near the end of the release block. The
measured hold includes the pure work, the release lock wait and this clock
overhead; it excludes the final counter/checksum updates and lock release.
The probe therefore measures instrumented admission, not an unperturbed
runtime. The checksum makes the work observable. Passing this probe cannot
rule out contention specific to firn's other pool operations.

## CI and the 14900K

Run only in CI; use the idle `14900k` runner for timings. With the repository
and compiler prepared by CI:

```sh
git submodule update --init whitefoot-kit
make compiler
sh research/experiments/guard-fairness/run.sh
```

`run.sh` uses `WFC` if supplied; otherwise it reads `whitefoot.pin` and uses
`build/whitefoot/<release>/whitefootc`. It compiles with full LTO, runs a small
N=2, K=100 pilot, then N=2,8,50 with `WF_DRIVERS=2` under `taskset -c 0,1`.
Defaults are K=1000 and two passes, reversing the order on the second pass.
`K`, `PASSES` and `OUT` can be set by CI. The program accepts only these N
values and K in 1..20000. Keep the host model, CPU topology, frequency policy,
load, compiler release and revision with the output. Fixed logical CPUs 0,1
must have their topology recorded; this script does not select physical cores.

## Firn checkout instrumentation

The experiment branch stores `EngineProbe` in `EnginePool`. Checkout and
return update it under the existing pool statements, without a new lock or
touching the keyspace Meta on each request. `INFO scriptprobe` snapshots it
once. CONFIG RESETSTAT clears it and advances an epoch so an engine acquired
before reset cannot add its hold to the new period. A script still waiting
across reset contributes its full wait when it succeeds in the new epoch.
SCRIPT FLUSH preserves the measurements.

One checkout adds three reads of the connection's existing monotonic clock,
two duration subtractions, saturating totals, maxima and one histogram update.
SCRIPT LOAD now receives the same clock through command dispatch. There is
no clock or counter operation on a false guard evaluation. Long scripts can
take the engine several times; the counters count checkouts, not EVALSHA
requests. Holds include engine creation/replacement, compilation, execution,
reply construction, engine reset and return-lock delay. The timestamp is
taken before the hold aggregate update and final release, so that small tail
is excluded. The added clock time and longer pool statements are unmeasured
perturbations; no nanosecond overhead estimate is claimed. At the pinned
runtime, each clock read calls `clock_gettime(CLOCK_MONOTONIC)` through the
standard clock implementation; the handle stores no shared clock state.

INFO emits CRLF-delimited `name:value` lines under `# Scriptprobe`, in this
order:

```text
engine_checkouts
engine_wait_ns_total
engine_wait_ns_max
engine_wait_lt10us
engine_wait_lt100us
engine_wait_lt1ms
engine_wait_lt10ms
engine_wait_ge10ms
engine_retries_total
engine_retries_max
engine_hold_ns_total
engine_hold_ns_max
```

The histogram buckets are disjoint: [0,10us), [10us,100us), [100us,1ms),
[1ms,10ms), [10ms,infinity). Available counters are unsigned decimal values
and saturate rather than wrap. `scriptprobe` is included in ALL/EVERYTHING,
excluded from DEFAULT, and can be selected explicitly, case-insensitively.
An INFO during a held checkout includes its count/wait but not its hold yet.

### Exact retries need runtime support

Both retry fields are **-1 (unavailable)**, including after reset; they are
not measured zeroes. This is a proposed experiment-only exception to the
normal reporting policy, pending the owner's scope decision. Task 1's retry
portion remains incomplete. At the pinned Whitefoot revision, SHARE-2 says
the guard's footprint “writes no path,” and SHARE-3 explicitly makes the
number of guard evaluations unobservable.
A counter helper in the guard is therefore forbidden:

```whitefoot
fn ready(out: u64, retries: &u64) -> yes: Bool writes(retries) {
  if out != 0_u64 {
    set retries^ = retries^ +sat 1_u64;
  }
  return out == 0_u64;
}
```

Using `when ready(out: p^.out, retries: &retries)` would violate SHARE-2.
This is a specification witness, not a compiler-diagnostic observation.
Counting attempts in an explicit polling loop would change the waiting policy
being measured. The runtime owner needs a hook returning the number of false
evaluations for one atomic statement, without replacing its guard or wake
behavior. Acceptance: unchanged guarded checkout with 0 false evaluations
reports 0, controlled held-out cases report each actual false evaluation,
and firn can aggregate those values under its existing pool statements.
No Whitefoot source or compiler pin is changed here.

### Workload capture

Dispatch `redis-bench` in workloads mode with `probe_info: true`, or set
`FIRN_PROBE_INFO=1` for the shell harness. Original consumers warm both firn
and Redis for five seconds (override `WORKLOAD_WARMUP_SECONDS`), outside their
measured runs. Memory workloads retain their existing warm-up and RESETSTAT.
For firn, the client resets after warm-up and SCRIPT LOAD/connection setup,
before releasing measured workers. After they have finished, the shell reads
INFO and prints each line as:

```text
firn-2,2,limiter-script,8,1,engine_checkouts:123
```

The prefix is line name, CPU count, workload, connections, pass. AOF lines
and memory variants retain their full line names. Text goes to
`scriptprobe.txt`, separate from rate/latency CSVs. The measurement start,
deadline, workers and elapsed-time calculation are unchanged; extra commands
execute outside that interval. Opt-in warm-up changes the starting keyspace,
so compare enabled runs against enabled Redis runs. Optional perf reruns do
not reset or emit probe records.

## Change inventory and validation status

These are uncommitted changes on `exp/engine-probe`, based on main
`c1144b666f57b2bd5cf567f1686e7abd6e387003`; the experiment branch is never to
be merged.

- Pool state and updates: `firn/script_pool/module.wfm`,
  `firn/script_pool/pool.wf`.
- Checkout timestamps and clock propagation: `firn/scripting/entry.wf`,
  `firn/scripting/module.wfm`, `firn/commands/dispatch.wf`.
- INFO and RESETSTAT: `firn/commands/info.wf`, `firn/commands/server.wf`.
- Harness and dispatch: `research/experiments/redis-bench/redis-bench.sh`,
  `research/experiments/redis-bench/workload/src/main.rs`,
  `research/experiments/redis-bench/workload/src/eviction.rs`,
  `.github/workflows/redis-bench.yml`.
- CI cases: `tests/network.rs` adds probe accounting/reset assertions and
  includes the new section in the existing ALL/EVERYTHING expectations.
  The workload client's cases cover reset after SCRIPT LOAD, worker admission
  after successful reset, and refusal preventing measured work.
- Standalone instrument: `research/experiments/guard-fairness/probe.wf`,
  `research/experiments/guard-fairness/run.sh`, this README.
- Proposed experiment decisions: `design/firn/scripts/probe.md`,
  `design/firn/reported-facts.md` (the explicit unavailable-value exception).

No build, program execution, test, lint or timing measurement was run locally.
No CI result for these uncommitted sources is available. Compilation, all
added cases and the 14900K measurements remain unverified. RESETSTAT crossing
an outstanding checkout is protected by the epoch in the implementation but
has no targeted test; sequential reset is covered by an added, unrun case.

A separate read-only Codex agent reviewed the complete base-to-working-tree
diff and untracked sources, relevant design commitments, affected consumers
and the project's checklist. Its exact model variant was unavailable. It
found the missing retry measurements (blocked on runtime support) and a
reporting-policy conflict (fixed by the explicit proposed exception). A
divisor-domain concern was resolved by the nonempty-sample validation. The
limited follow-up review found no further findings in those repairs. This is
source inspection evidence only, not gate or owner approval.

`whitefoot.pin` and all submodule pins are unchanged. `firn/modules.wfg`
already declares the scripting/commands time and script-pool edges used by
this change; no new firn module dependency is introduced. `docs/todo.md` is
untouched. The Whitefoot retry requirement is recorded above as a semantic
witness; no Whitefoot repository change or external gap filing was made.
