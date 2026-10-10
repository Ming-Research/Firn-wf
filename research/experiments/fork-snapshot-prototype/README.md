# Restricted-fork snapshot prototype

Status: blocked on a Whitefoot-callable capsule/export contract, tracked by
status-board item `firn-wf-snap-fork`. The native runtime support alone does
not expose this experiment to Whitefoot programs.

## Pre-registered question, comparison and rejection criteria

Written before any compilation, execution or measurement of this experiment.

**Question.** Can a continuously mutating `Shared<ConcurrentHashMap<u64>>`
be captured at one sequence-numbered cut, exported by a restricted child
while its parent continues serving, and checked against an independent
serial-history oracle using canonical Whitefoot?

**Comparison.** The other routes are the merged [reconciled live scan and
after-image log, Firn-wf PR #42](https://github.com/Ming-Research/Firn-wf/pull/42)
and the [Frozen persistent keyed dataset prototype](../frozen-dataset-library/README.md).
The scan route reconciles changes over an enumeration interval; the Frozen
route retains an immutable root; this route would retain a kernel COW image
at a single cut. Compare exact dataset recovery, parent service gaps, parent
heap growth, retained memory and export completion under the same keyed
insert/replace/delete trace and dataset sizes. The existing prototypes have
different instruments and workloads; their individual runs cannot rank the
routes without a matched comparison. This does not change firn's production
snapshot design.

**Prospective witness.** Every writer operation publishes its command and
sequence number in the same atomic statement as its map mutation. Capture
the sequence and fork while holding the whole map and journal together,
with output preparation outside the hold. Replay the command prefix through
that sequence into an independent array model; do not derive the oracle
from the exported map or reuse the mutation helper. Check every key/value
and absence, rejecting duplicates, missing entries and extra bytes. Require
writer progress both before and after the cut and a changed post-cut key.
Select the post-cut live map in a wrong control and require the same checker
to report a snapshot mismatch. A crash, timeout, compiler failure or I/O
failure is not that expected mismatch. A future runner must fail on any
wrong positive result, undetected control, unexpected status or incomplete
report.

**Measurements.** Report whole-map acquisition delay, held capture duration
and the interval between the last writer completion before capture and its
first completion afterward. The last interval includes scheduling and is an
observed service gap, not a proof of maximum runtime pause. Sample parent
`std::process::heap_in_use` before capture, after capture and after child
completion, stating which oracle/log storage remains live. This meter counts
parent logical allocations, not child COW retention. Measure child peak RSS
through `/proc` only if a supported source interface supplies the necessary
child identity and file authority; otherwise report it as unmeasured. RSS
does not replace the investigation's child `Private_Dirty` reserve proxy.

Start with a timed small CI sample and inspect duration and spread before
scaling. Hosted Ubuntu 24.04 runs establish expressibility and correctness;
precise comparative timings require the idle 14900K through CI, matched
compiler/settings, interleaved runs and a base twin. Record kernel, glibc,
architecture, page settings and driver/worker counts with those runs.

**Rejection.** Any inconsistent cut, wrong entry, forbidden child runtime
access, stalled parent writer or undetected wrong control rejects the
witness claim. An operation unavailable in canonical Whitefoot blocks this
route; do not substitute C, a custom host module or a pre-fork dataset copy.
For production eligibility, the [fork-route investigation](../../investigations/consistent-snapshots/fork-route.md)
also requires a service-first reserve and confirmed cleanup. Poll-and-kill
alone proves no strict memory or cleanup bound. Those obligations remain
open even if a future correctness witness passes. Service-gap acceptance
limits must be fixed before comparative measurements can select this route;
this record makes no performance prediction.

## Results

Results: pending CI on wf-a3205af0bbea.

No witness has been compiled or run. CI execution of this route additionally
requires the source interface described below; the named release cannot
express it. Parent service gap, parent heap changes and child peak RSS are
all unmeasured. No `.wf` program, `run.sh` or fork workflow entry is supplied:
an invented API or a runner that only reports success would conceal the gap.
The existing snapshot workflow's `WITNESS_RELEASE` is updated as requested;
`whitefoot.pin` and submodules are unchanged.

## Gaps

### The capsule has no Whitefoot-callable capture or exporter boundary

Inspected release `wf-a3205af0bbea`, Whitefoot main
`a3205af0bbea6e8356279f160f3fcd523f085ed1`, specification v0.124.
[PR #337](https://github.com/Ming-Research/Whitefoot/pull/337) explicitly
excludes a Whitefoot-callable exporter. Its
[native header](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea/compiler/src/backend/completion/fork_capsule.h)
says "Native witness surface only; this is not a Whitefoot callback ABI."
It exposes C preparation, arm, held capture, parent-start and finish
functions with raw descriptors, a captured pointer and an audited C encoder.
Its child closure forbids inherited allocators, TLS, locks, runtime helpers
and cleanup. Calling ordinary map helpers in that child is not authorized.

The [standard-library graph](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea/lib/std/modules.wfg)
registers no capsule module, and
[`std::process`](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea/lib/std/process/module.wfm)
declares no child creation, capture, identity or reap operation.
[Specification PRE-2 and MOD-8/MOD-10](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea/spec/kernel-spec.md)
fix the host interfaces and require ordinary modules' definitions; declaring
a local function with the C symbol's name cannot supply the missing host
operation. PR #337 adds a
[native C test](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea/compiler/src/backend/completion/fork_capsule_test.c),
not a Whitefoot example: it exports captured bytes after parent mutation and
free, checks descriptor/ring isolation and reaping, and explicitly makes no
Whitefoot map/library claim.

Minimal semantic example (requirements, not runnable Whitefoot syntax):

```text
Shared map: "a" -> 11, "b" -> 22; shared journal sequence: 7
writer statement: update a key and append its command with the next sequence
capture statement, holding the whole map and journal:
    record cut = 7
    invoke nonsuspending capsule capture of this map at cut
parent: continue writer statements, including "a" -> 99 at sequence 8
child: export cut 7 with exactly {"a": 11, "b": 22}, then exit
parent: reap and compare against independent replay through sequence 7
wrong control: export the live sequence-8 map; the same comparison must fail
```

Even this two-key example lacks an expressible capture invocation and a
checked child traversal/output body. The missing boundary must specify
transitive data/call/release closure, nonsuspending capture under SHARE-2,
output ownership, and parent-owned completion/reaping. It belongs to
Whitefoot's language/runtime owners, not a downstream C bridge. Reopen when
a released, specified source interface and maintained Whitefoot example
support this example; then implement the keyed witness and fail-on-error
runner and wire it into the snapshot workflow. This is a contract gap found
by inspection, not an observed compiler rejection.
