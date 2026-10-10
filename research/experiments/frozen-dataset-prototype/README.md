# Persistent dataset prototype: Frozen acceptance probes

Results: pending CI on wf-0c0a2eda83ae

These standalone probes exercise the ownership boundary of the persistent
dataset snapshot route. Whitefoot's [Frozen implementation, PR #338](https://github.com/Ming-Research/Whitefoot/pull/338)
supplies the missing immutable shared value: specification v0.123, revision
`0c0a2eda83ae0270a7c3e12f969aee5655a4301f`, compiler release
`wf-0c0a2eda83ae`. This is an expressibility experiment; the HAMT, retention
ledger and complete reserve protocol remain unimplemented.

## What changed and why

The managed probes now build uniquely owned values with `frozen_new` and
retain the same immutable value with `frozen_share`. Their nodes use
`Frozen<u8>` rather than `SharedRead<u8>`. SHARE-1 permits reading `inner`
without an atomic statement, including inside the registry's atomic block.
There is no writable node handle to relinquish, nested atomic statement or
dependent atomic target. Only the mutable publication/registry cell still
uses `Shared<Registry>` and SHARE-2 holds.

The two formerly rejected managed-next probes are positive acceptance tests.
Their filenames remain unchanged so historical diagnostics and CI artifacts
still identify the same witnesses. Each preserves its original scalar oracle
7 and cursor update from position 0 to 1. A separately spawned writer
publishes a new frozen live value 9 while the reader retains version 7. The
reader observes 7 without a hold before spawning the writer, while the writer
context is outstanding, and again after publication. It waits for publication
without holding registry state; the writer requires no reader acknowledgement.
Inside the registry statement, the reader records the captured and live bytes
and advances the cursor. It joins
the writer explicitly before checking the recorded bytes against 7 and 9
and the position against 1. No assertion's return edge implicitly joins the
writer before those observations (WAIT-3).
The direct-field and ordinary-reference forms separately replace the former
nested hold and dependent target. All successful observations exit 0;
returning the observed byte as exit 7 is no longer the oracle check.

`managed-atomic-control.wf` retains its separately available handle control,
now frozen, and checks both that handle and the registry root within one
registry statement. It also replaces the live registry root with 9 and
checks that the separately retained snapshot still reads 7 without a hold.
That external handle remains a diagnostic/ownership control, rather than
the registry-only managed-cursor API.

`revoke-rejected.wf` remains negative. Its intended operation is still
destructive replacement of a captured one-byte value with an empty array.
Writing `root^.inner` is forbidden by TYPE-2's readonly-path rule; a
replacement of the handle itself would merely release that handle and would
not revoke another holder's value. The explicit `writes(root)` row prevents
a missing write effect from being the reason for rejection:

```wf
fn revoke(root: &Frozen<Box<Array<u8>>>) -> result: unit writes(root) {
  let empty = box_array_filled::<u8>(count: 0_u64, value: 0_u8);
  set root^.inner = move empty;
  return unit;
}
```

The unchanged `control.wf` still tests the three original Shared ownership
consequences. It is an independent baseline for handle-local release, not a
Frozen library implementation or a substituted reserve protocol.

## Expected results and independent oracles

| Probe | Expected result | Purpose and oracle |
| --- | --- | --- |
| [control.wf](control.wf) | Compiles, exits 0 | Literal byte 7 and same-function PRE-2 heap comparisons: releasing a writer preserves the reader's allocation; last read-handle drop releases it; a registry retains a dropped reader's root until cleared; dropping a ticket does not decrement a separately retained ledger's outstanding count of 1. |
| [revoke-rejected.wf](revoke-rejected.wf) | Rejected citing TYPE-2 at `set root^.inner = move empty;` | An immutable captured generation cannot be overwritten or remotely revoked through its frozen value. Never executed. |
| [managed-atomic-control.wf](managed-atomic-control.wf) | Compiles, exits 0 | Separately retained and registry-owned frozen handles both read 7 inside the registry statement; cursor position persists as 1; replacing the registry root with 9 leaves the external snapshot at 7. |
| [managed-next-nested-rejected.wf](managed-next-nested-rejected.wf) | Compiles, exits 0 | Direct `registry^.root.inner` reads 7 within the registry hold, replacing the formerly nested node hold. A writer publishes live 9, position becomes 1, and a reader-held snapshot reads 7 without a hold before and after publication. |
| [managed-next-target-rejected.wf](managed-next-target-rejected.wf) | Compiles, exits 0 | `let byte = &registry^.root.inner;` and `byte^` read 7 within the registry hold, replacing the formerly dependent node target. The same independent live-9, position-1 and no-hold snapshot-7 oracles apply. The reference never escapes the block. |

| Program exit | Meaning |
| --- | --- |
| Any positive: 0 | Every specified observation for that probe matched. This does not establish the full library contract. |
| control: 1 | Writer drop changed heap while a read handle remained. |
| control: 2 / 3 | Captured length was not 1 / captured byte was not 7. |
| control: 4 | Last root drop did not return to the same-function baseline. |
| control: 5 / 6 | Reader-controller drop changed registry-held heap / registry lost its root. |
| control: 7 / 8 | Clearing the registry released no storage / registry drop did not return to baseline. |
| control: 9 / 10 / 11 | Ticket drop changed ledger count 1 / did not return to ledger-only heap / ledger drop did not return to baseline. |
| Any managed probe: 12 / 13 | Captured byte was not 7 / cursor position was not 1. |
| Any managed probe: 14 / 15 | Published live byte was not 9 / retained reader byte changed from 7. |
| run.sh: 81 | Negative compiled, or its source rejection did not match the registered TYPE-2 operation. |

`run.sh` records the compiler's own exit status and each positive program's
own exit status separately in `results.tsv`, together with an expectation
verdict. It tries every probe even after a mismatch. Positives must compile
and exit 0. The negative must produce compiler status 1 and exactly one
source error, `error[TYPE-2]: ReadonlyWriteTarget`, at
`revoke-rejected.wf:3:7`, with the registered write in its diagnostic. An
unrelated error, unexpected acceptance, internal failure, timeout or signal
termination fails the job. The old unconditional expressibility-stop exit
80 is removed: the runner exits 0 only when all five expectations hold.
Raw compile/run failures retain the first failure's status. Logs remain
available to inspect the actual diagnostic and runtime behavior.

## CI invocation

The branch-only [snapshot-witnesses workflow](../../../.github/workflows/snapshot-witnesses.yml)
installs the witness release separately from `whitefoot.pin`, exports `WFC`,
and runs this runner alongside the other snapshot witnesses on ubuntu-24.04.
It uploads all logs. Neither firn's compiler pin nor any submodule is moved.
The workflow is temporary and no canonical gate consumes these probes.

For CI after the workflow has installed the compiler:

```sh
WFC="$PWD/build/whitefoot/wf-0c0a2eda83ae/whitefootc" \
  sh research/experiments/frozen-dataset-prototype/run.sh
```

GNU `timeout` is required. Outputs go to `$OUT`, defaulting to
`$RUNNER_TEMP/frozen-dataset-prototype` or `/tmp/frozen-dataset-prototype`.
The runner retains its existing `whitefoot.pin` compiler fallback when
`WFC` is unset; use the explicit witness compiler above for this experiment.
The concurrent managed-next probes use `WF_DRIVERS=2 WF_WORKERS=1`, allowing
the spawned writer to progress while the reader waits. The two controls keep
`WF_DRIVERS=1 WF_WORKERS=1`. No timing or performance claim is made.
No local build, compilation, test or runner execution accompanies this edit.
Changes are left in the working tree for the handoff owner to commit and
push before CI can validate them.

## Remaining library obligations

The original pre-registration against Whitefoot
`fc98b8f1aee54a2d57c55dd25b099f2819320d3f` (v0.121) asked whether the
opt-in persistent dataset library could meet B1-B5 and B8 of the
[consistent-snapshots investigation](https://github.com/Ming-Research/Whitefoot/blob/main/research/investigations/consistent-snapshots/README.md#contracts-required-before-implementation).
The service-first reserve and opt-in library rulings remain in force.
These scalar probes close only the first immutable-node read obstruction if
CI passes; they do not implement or validate the four full programs below.

The intended representation remains a path-copying, fan-out-16 HAMT with
owned `Box<Array<u8>>` keys/values, frozen child edges, complete-hash collision
storage and one publication cell for root and generation. Frozen payloads
must contain no Shared, SharedRead or host handle at any depth (SHARE-1).
Hash choice, collision layout and generation exhaustion remain unselected.

The selected managed API keeps capture roots in writer-controlled registry
entries, returns a linear capture identifier, yields detached bytes from
`cursor_next`, and requires explicit `capture_close`. Publication and cursor
observation each occur in one statement; abort removes the entry and releases
its root during publication. No retaining node owner may escape that API.
The reader-held frozen handle in these scalar witnesses deliberately tests
snapshot lifetime; it is not a claim that external owners are revocable.

The exact original snapshot-only quantity is
`V = sum(bytes(a), a in (union of capture-reachable allocations) minus live-root-reachable allocations)`.
Allocation identities count once, including node backing, keys/values and
object overhead. Last-capture drop and abort must remove real retention,
not just zero counters. Frozen's ordinary drop still supplies no user-defined
ledger action (STOR-3), and releasing one handle does not release others
(SHARE-1). For example, retaining
`let reader = frozen_share::<u8>(frozen: &root);` leaves the old object live
after the writer replaces or releases `root`. Freezing a ticket containing
`ledger: Shared<Ledger>` is also forbidden by SHARE-1's payload closure;
this does not supply an automatic ledger-notification mechanism.

For the managed continuation, let `L` be live-root allocations, `C_c` the
allocations reachable from active capture c and `b(a)` a nonnegative charge
covering actual allocation bytes. Its proposed conservative charge is
`r_c = sum(b(a), a in C_c minus L)` and `S = sum(r_c, active captures c)`.
For m captures, `V_b <= S <= m * V_b`, where `V_b` counts the union once.
Before publication completes, abort oldest captures until `S <= reserve`;
close also subtracts the entire charge and releases the registry root.
This conditional bound does not establish a physical-byte formula, identity
ledger, incremental accounting algorithm, scratch limit or total heap bound.
Detached results, metadata and private batch construction need separate
budgets. Cleanup work is proportional to newly unreferenced allocations,
not a constant-time or wall-clock promise. These obligations remain open.

| Full program, still unimplemented | Fixed independent oracle and rejection criterion |
| --- | --- |
| Sequential (B1/B2/B3/B5) | Print a deterministic PRNG seed; replay set/delete/multi-key batch histories into an independent sorted association list. Every captured enumeration equals that generation, covering empty keys/values, replacement, absent deletes, collisions and multiple captures. Expected exit 0. |
| Concurrent (B3/B4/B5) | Writer publishes complete batches while reader captures at racing cuts and waits between cursor steps. Independently replay through the returned generation: no partial batch or mixed generation, and writer completes. Expected exit 0. |
| Reserve (B1/B4/B8) | Stall a small-reserve reader while writer replaces every key. Abort before excess retention is admitted, complete publication without reader acknowledgement, and observe actual retained bytes zero before waking the reader; its next cursor call reports Aborted. Independently verify final live state. Expected exit 0; timeout fails. |
| Reclamation (B1/B8) | Multiple captures share immutable nodes and match independent replay. After captures close and dataset drops, quiescent PRE-2 heap returns exactly to baseline within one activation, without deeper waiting frames between readings or subtracting a guessed frame constant. Expected exit 0. |

Any required inexpressible operation, oracle mismatch, abort awaiting a
stalled reader or retained storage past the cleanup boundary rejects the
proposed library contract. Compiler failure or an unrelated diagnostic does
not establish language rejection. No expected result is weakened to obtain
a green run. B6/B7 export, host-resource/fork behavior, performance and
durability are outside these probes. The prior retained runtime frame-chunk
finding stays with board item `gran-blg-ctx-spare-chunk`.

## Historical rejection on wf-fe5589ec5f45

These probes moved on 2026-10-10 from Whitefoot branch
`claude/snap-lib-proto` (`75d4ddddf` plus the managed-cursor working edits)
into Firn-wf. Earlier Whitefoot investigations reported update, capture,
enumeration and reclamation in the binary-tree
[expressibility witness](https://github.com/Ming-Research/Whitefoot/blob/2bb8a342539930f04ed661bcf1c677ed5bb7eb07/research/experiments/frozen-dataset-witness/README.md#results);
those were not runs of this prototype or evidence for its HAMT/reserve API.

[Firn-wf CI 38047195263](https://github.com/Ming-Research/Firn-wf/actions/runs/38047195263),
revision `abf063d`, compiler release `wf-fe5589ec5f45`, recorded the
pre-Frozen outcomes:

| Original probe | Historical outcome and diagnostic |
| --- | --- |
| control.wf | Compiled, exit 0. |
| revoke-rejected.wf | SHARE-2 ReadonlyWriteTarget at `set bytes^ = move empty;` (4:9). |
| managed-atomic-control.wf | Compiled, exit 0 with independently available targets. |
| managed-next-nested-rejected.wf | SHARE-2 WaitInsideAtomic at inner `atomic byte = &registry^.root` (16:5). |
| managed-next-target-rejected.wf | SHARE-2 AtomicKeyReadsTheState at dependent target `byte = &registry^.root` (15:39). |

Those rejections established the SharedRead traversal obstruction on that
release. The owner selected Frozen on board card `firn-q-frozen-type`, option
A, implemented as Whitefoot item `proof-frozen`; the managed-next probes
became its acceptance tests. The old runner returned 80 even when both
controls passed and all negatives were rejected. The Frozen expectations
above replace that stop while preserving immutable-generation protection.
Prior read-only reviews covered the pre-Frozen reductions and did not
validate the rewritten sources or the unimplemented full library.

## Frozen rewrite read-only review

An independent GPT-6 agent inspected all seven changed files against Firn-wf
`0958448784cb4f42f05e238728dacf00efb7b2b2`, unchanged control and affected
workflow context, the original oracles, project checklist, governing design
nodes and Whitefoot's v0.123 specification/conformance evidence. A limited
follow-up inspected the managed-next observation and explicit-join changes.
Findings: none within scope. Canonical syntax, ownership/effects, readonly
reference provenance, fixed oracles, WAIT-3 join placement and runner verdict
handling passed inspection. No production pin, submodule, design tree,
Redis ratchet or performance result changed.

Neither review executed builds, compilations, tests or scripts. Actual
compiler/runtime outcomes, representative-fault sensitivity of the runner
and compatibility of the other workflow witnesses remain unverified pending
CI. No new Whitefoot gap was identified by inspection; the existing ledger
and reserve obligations above remain open.
