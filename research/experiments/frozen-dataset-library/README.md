# Frozen persistent keyed dataset

Results: registered criteria pass on wf-bae97fdcf890; see Results below.

This is the next expressibility experiment for status-board item
`firn-wf-snap-lib-proto`, not a production replacement for firn's keyspace.
The [scalar acceptance probes](../frozen-dataset-prototype/README.md) precede
it. The governing [investigation](https://github.com/Ming-Research/Whitefoot/blob/main/research/investigations/consistent-snapshots/README.md#contracts-required-before-implementation),
[library direction](https://github.com/Ming-Research/Whitefoot/blob/main/design/language/data-model/frozen-datasets.md)
and [retention policy](https://github.com/Ming-Research/Whitefoot/blob/main/design/language/data-model/frozen-datasets/retention-budget.md)
remain in force. Language reference: Whitefoot v0.123, SHARE-1, PRE-1,
TYPE-9 and OWN-1; pre-registration witness release `wf-0c0a2eda83ae`.

## Pre-registered question, comparison and rejection criteria

Written before any compilation or execution of this experiment.

**Question.** Can an opt-in persistent map of `u64` keys to single-byte
values retain version zero as one `Frozen<Node>`, traverse it without any
atomic statement, and publish replacements while retaining storage bounded
by the number of changes and path depth, independently of dataset size?

**Comparison.** Build keys `[0, N)` with byte `key % 251`. A separately
constructed ordinary array is the version-zero oracle. A spawned writer
performs M single-key publications in groups of three: replace existing key
`r` with 255, insert key `N+r` with 254, delete existing key `N-1-r`.
The independent final array model assigns these three ranges directly; it
does not call the map, replay the writer helper, or derive expected bytes
from any tree. M is a multiple of three and `M/3 < N/2` in every case.

The reader retains its snapshot before spawning the writer, enumerates it
while the writer context is outstanding, explicitly joins, and enumerates
the snapshot again after all M publications. Each enumeration compares every
key/value and absence with the array, rejects duplicates and unexpected
keys, and computes count and a wrapping checksum. Lookup is checked against
every array slot separately. The final live root must equal the final model.
The writer has no reader acknowledgement, cursor or completion dependency.
This proves lifetime across publication if it passes; scheduling need not
place a publication inside a particular traversal. It makes no service
latency, fairness or simultaneous-instruction claim.

**Heap criterion.** Both `heap_in_use` samples are in the same witness
activation, with both models and traversal scratch allocated before the
first sample. The first sample precedes spawn; the second follows explicit
join, retaining version zero and the final live root. No I/O occurs between
samples. Require `after <= before + B(M,d)`, where

```
d = 64
C = 512 bytes per newly allocated frozen node
F = 262144 bytes of fixed runtime/context allowance
B(M,d) = C * M * (d + 1) + F
```

C covers this node's tag, largest payload (two handles), frozen header
and the selected release's [512-byte minimum pool grant](https://github.com/Ming-Research/Whitefoot/blob/0c0a2eda83ae0270a7c3e12f969aee5655a4301f/compiler/src/backend/completion/bridge.c#L995).
The [frozen allocator](https://github.com/Ming-Research/Whitefoot/blob/0c0a2eda83ae0270a7c3e12f969aee5655a4301f/compiler/src/backend/completion/bridge.c#L2802)
uses that pool. This is a source-derived charge, not a measured result.
F is an explicit prospective allowance, not a measured constant subtracted
from the observations. Total growth beyond B fails; F must not be
silently enlarged. This is a retained program-heap bound after updates,
not a sampled peak, RSS, allocator-reserve bound or production reserve
protocol. Private path construction can transiently hold an additional
path. Heap growth is reported even when negative.

CI first times three small runs (`N=64, M=6`) to inspect duration/spread.
Only if all pass, each takes at most 10 seconds and their spread is at most
5 seconds does it expand to `(N,M)=(1024,12), (8192,12), (8192,24)`.
These time limits size the CI experiment; they are not performance criteria.
The same B applies to both N values at M=12. Every positive must exit zero.

Two deliberate wrong controls use the same checks: selecting the final live
root for the old-version comparison must exit 70, and retaining a deep copy
of the final tree at `N=8192, M=12` must exit 72 at the heap check. The latter
establishes that this bound detects a dataset-sized copy rather than merely
accepting a generous allowance. The library itself has no control modes.
Unexpected control success, another failure code, timeout, signal, missing
output or compiler failure fails the runner; a compiler failure is not by
itself evidence of a language gap. Logs and actual exit statuses are kept.

**Reject this prototype's claim** for any wrong snapshot/live entry,
checksum/count mismatch, stalled writer, premature release, exceeded heap
bound or undetected wrong control. A necessary operation that cannot be
expressed stops this route and belongs under Gaps with a minimal witness;
do not change the oracle or use a workaround to get a pass.

## Representation and ownership

`map.wf` implements a binary radix trie over all 64 key bits, most significant
bit first. A node is empty, a leaf containing a key and byte, or a branch
with two frozen children. Leaves occur after 64 branches. One frozen root
is a version, including the empty version. Each update copies only its search
path, shares the other children, and allocates at most 65 frozen nodes.
Deletion collapses a branch only when both children are empty. Missing-key
deletion preserves the empty subtree. Lookup and enumeration need no hold.
Old roots remain readable until their last handle is released. No unique
owner is duplicated and no mutable/host handle enters a frozen payload.

The registry holds a root and generation together. The sole writer constructs
a complete replacement privately and publishes the pair in one atomic
statement. Capturing shares that root inside one atomic statement and returns
an owned handle, never a reference into the registry. A reader neither owns
nor holds the mutable live state while traversing. This prototype uses one
writer; it does not claim multi-writer conflict resolution or multi-key batches.
Boundary cases exercise the empty map, key zero, the high bit, maximum u64,
replacement, absent deletion, deletion to empty and separately retained roots.

The binary trie is an intentionally simple research representation; it is
not a selection of the eventual HAMT/hash/collision layout or a change to
firn's production design. B1/B2/B3/B4/B5 receive partial ownership, closure,
single-publication, shared-reader and version-local lookup evidence here.
B8's service-first reserve/abort, managed cursors, exact snapshot-only ledger,
bounded cleanup and stalled-I/O obligations remain unimplemented. A retained
external Frozen handle is not revocable. B6/B7, log boundaries, export,
durability, arbitrary-size values and comparative performance remain outside
this experiment; passing it does not complete the registered library route.

## Running in CI

The temporary [snapshot-witnesses workflow](../../../.github/workflows/snapshot-witnesses.yml)
installs the witness release independently of firn's pin and invokes
`run.sh`. The runner compiles `map.wf`, `witness.wf`, `main.wf` and a generated
configuration containing only N, M and the two explicit control flags.
It uses `WF_DRIVERS=2 WF_WORKERS=1` on ubuntu-24.04. GNU `timeout` and
`/usr/bin/time` are required. After CI installs the compiler:

```sh
WFC="$PWD/build/whitefoot/wf-0c0a2eda83ae/whitefootc" \
  sh research/experiments/frozen-dataset-library/run.sh
```

Outputs go to `$OUT`, defaulting to `$RUNNER_TEMP/frozen-dataset-library`
or `/tmp/frozen-dataset-library`, and are uploaded by the workflow.
The runner deliberately defaults to the witness release, not the older
production pin. No pin, submodule, Redis ratchet or canonical gate changes.

Each run prints ten `NN=decimal` fields, with twenty-digit decimal values:

| Field | Meaning |
| --- | --- |
| 00 / 01 | N / M |
| 02 / 03 | Heap before spawn / after join, in bytes |
| 04 | Allowed growth B, in bytes |
| 05 | Bytes released by the last snapshot handle, with the live root retained |
| 06 / 07 | Version-zero / final model checksum, each required to match enumeration |
| 08 / 09 | Program exit code / trie depth |

Both absolute heap readings are retained; `03 - 02` is the signed growth,
without subtracting runtime estimates. The last-snapshot release must lower
the heap; this checks real release but does not assert exact whole-process
return to baseline or bounded cleanup time.

| Exit | Meaning |
| --- | --- |
| 0 | All positive observations matched |
| 70 | Snapshot enumeration, checksum or lookup mismatch; required for wrong-live |
| 71 | Final live model, publication generation or writer completion mismatch |
| 72 | Heap growth exceeds B; required for wrong-copy |
| 73 | Full-u64 boundary cases failed |
| 74 | Last snapshot release freed no storage |
| 75 | Reporting I/O failed |
| runner 81 / 82 / 83 | Wrong runtime status / incomplete report / small sample needs inspection |

Compile failure keeps the compiler's actual status. Program statuses,
expectations and sizing times appear separately in `results.tsv`.

## Gaps

### Recursive readers' effect rows (resolved)

On wf-0c0a2eda83ae the compiler's suggested repair for a recursive reader of a Frozen tree listed
exact leaf paths one level deeper on each attempt, which never converges (run
[38093338376](https://github.com/Ming-Research/Firn-wf/actions/runs/38093338376)). The
specification admits a covering prefix: `reads(root.inner)` is accepted on every recursive reader
(run [38093834993](https://github.com/Ming-Research/Firn-wf/actions/runs/38093834993) onward), so
the program is written as the language intends. The misleading repair is Whitefoot's diagnostic
item `proof-bl-frozen-recursive-row`. A function that only matches a node's tag reads nothing
below the handle and is `pure`.

### Lowering failure (resolved)

With the rows corrected the checker accepted the program, but lowering stopped with
`lowering failure in Lowering: InvalidCheckedProgram`
(run [38094105415](https://github.com/Ming-Research/Firn-wf/actions/runs/38094105415), commit
6ea0cd6; status board item `proof-bl-frozen-lowering`). This was
[Whitefoot #347](https://github.com/Ming-Research/Whitefoot/issues/347):
a by-value match on a slice element that bound a payload took the wrong address.
Release `wf-bae97fdcf890` fixes that lowering defect.
[Run 38099307234](https://github.com/Ming-Research/Firn-wf/actions/runs/38099307234)
at commit `9fdd51a` compiles all six library configurations successfully and
runs every positive and both wrong controls; the prototype no longer waits
on this lowering fix. The oracle and criteria remain unchanged.

## Results

### CI run 38099307234 — wf-bae97fdcf890

[Snapshot-witnesses run 38099307234](https://github.com/Ming-Research/Firn-wf/actions/runs/38099307234)
tested commit `9fdd51a` (`9fdd51a0e27600cad99c8d1283d6acc2ab2c14ba`)
on GitHub-hosted ubuntu-24.04 with compiler release `wf-bae97fdcf890`,
`WF_DRIVERS=2 WF_WORKERS=1`. The log records the checked-out revision at
line 157 and `release = wf-bae97fdcf890` at line 547. The original oracle,
C = 512, F = 262144, d = 64 and rejection rules above are unchanged.

Evidence line numbers below refer to the supplied full CI log
`snap-38099307234.log`, including its CI prefixes. Quoted fields reproduce
the payload after those prefixes. Runtime statuses are the shell's actual
statuses, separately from field 08. Fields 06/07 print the independent
**model** checksums; successful enumeration/count/lookup comparisons are
established by the program's exit 0, not by printed checksums alone.
[The witness](witness.wf) checks all those comparisons before selecting code
0; it does not print individual entries or counts.

| Pre-registered criterion | Verdict | Exact log evidence and interpretation |
| --- | --- | --- |
| Registered comparison: keys `[0,N)`, bytes `key % 251`, independent version-zero/final arrays, three-operation replace/insert/delete groups, M divisible by 3 and M/3 < N/2. | **Pass** | Configuration fields at lines 614–615, 624–625, 634–635, 645–646, 655–656, 665–666, 675–676, 685–686 record `(64,6)`, `(1024,12)`, `(8192,12)` or `(8192,24)`. All satisfy the arithmetic constraints. Source inspection confirms the model arrays are assigned directly without calling the map or writer; actual positive exit-0 rows 697–699, 701, 703, 705 establish agreement. |
| Compilation succeeds for every positive and control; no necessary operation remains blocked by lowering. | **Pass** | Lines 696, 700, 702, 704, 706, 708 record compile status 0 for all six configurations (exact rows below). |
| Three `N=64, M=6` samples pass, each at most 10 seconds, spread at most 5 seconds, before expansion. | **Pass** | Lines 697–699: `smoke run-1 0 0 0.00`, `smoke run-2 0 0 0.00`, `smoke run-3 0 0 0.00` (tab-separated in the log). Line 644: `small-sample wall_seconds min=0 max=0 spread=0`; larger cases follow at lines 645–674. These are sizing observations at the timer's reporting precision, not a performance result. |
| Every positive exits zero, including `(1024,12), (8192,12), (8192,24)`. | **Pass** | Actual status rows 697–699, 701, 703, 705 all record 0 against expected 0; reported status fields at lines 622, 632, 642, 653, 663, 673 are each `08=00000000000000000000`. |
| Version-zero entries, absences, duplicates/unexpected keys, enumeration count/checksum and separate per-slot lookup match the independent array before spawn, with the writer context outstanding, and after join. | **Pass** | The same six actual exit-0 rows and field-08 lines establish all three `verify` checks; any mismatch selects 70. Model checksum fields are 89440/115543 at lines 620–621 (repeated 630–631, 640–641), 69052970/70026860 at 651–652, 4211824848/4214992986 at 661–662, and 4211824848/4218302676 at 671–672, each printed as a twenty-digit field. |
| Final live entries, absences, count/checksum and lookup match the independently assigned final model; all M publications complete and generations are 0/M. | **Pass** | Actual exit-0 rows 697–699, 701, 703, 705 and the six zero field-08 lines establish `live_ok`, `generation_ok` and `completed == operation_count`; failure selects 71. No timeout or stall replaces the recorded status. |
| Retained version zero survives publication; traversal needs no atomic statement and writer completion has no reader acknowledgement/cursor dependency. | **Pass** | The six positive exit-0 rows establish lifetime across spawn and join, with no premature-release failure. Source inspection of `enumerate`, `verify` and `map_lookup` shows no atomic statement; `writer` only updates/publishes and returns M. The log does not locate a publication inside a traversal, which the registration explicitly does not require. |
| Full-u64 boundaries: empty map, zero/high-bit/maximum keys, replacement, absent deletion, deletion to empty and independently retained roots. | **Pass** | Actual positive exit-0 rows 697–699, 701, 703, 705 establish `boundary_cases`; its failure would select 73 before the other checks. |
| Heap samples share one witness activation; both models and traversal scratch precede the first, first precedes spawn, second follows explicit join with old/final roots retained, and no I/O occurs between them. | **Pass** | Field-02/03 pairs at lines 616–617, 626–627, 636–637, 647–648, 657–658, 667–668 are the `before`/`after` returned by the same `witness` call. Source inspection confirms allocation, spawn/join and retention order; `main.wf` performs reporting I/O only after `witness` returns. These placement facts are source evidence interpreting the logged measurements, not separately printed runtime events. |
| Retained program-heap growth after join satisfies `after <= before + 512*M*65 + 262144`, with no subtraction/enlargement. | **Pass** | Exact fields at lines 616–618 (repeated 626–628, 636–638), 647–649, 657–659 and 667–669 are listed below. Signed growth is +171520, +178176, +181248 and +187904 bytes, respectively; each is below its unchanged B. |
| The same M=12 bound applies independently of N in the registered size comparison. | **Pass** | Lines 649 and 659 both read `04=00000000000000661504`. N=1024 grows +178176 bytes; N=8192 grows +181248 bytes. Both satisfy that same bound; this comparison does not prove a bound for every N or M. |
| Dropping the last snapshot handle lowers heap with the live root retained. | **Pass** | Field 05 is `05=00000000000000037376` at lines 619, 629, 639; `05=00000000000000042496` at 650; `05=00000000000000044032` at 660; `05=00000000000000051200` at 670. All are positive; no positive selects 74. Exact baseline return or bounded cleanup time is not required here. |
| Wrong-live is detected by the old-version checks with exactly exit 70. | **Pass** | Line 683: `08=00000000000000000070`; actual row 707: `wrong-live run-1 70 70 0.03` (tabs in log). This rejects selecting the final live root as version zero. |
| Wrong-copy at N=8192, M=12 is detected by the heap check with exactly exit 72. | **Pass** | Lines 687–689: `02=00000000000008621880`, `03=00000000000017254200`, `04=00000000000000661504`; growth +8632320 > 661504 bytes. Line 693: `08=00000000000000000072`; actual row 709: `wrong-copy run-1 72 72 0.03` (tabs in log). The bound detects a retained dataset-sized deep copy. |
| No unexpected control success/status, timeout, signal, missing/malformed report or reporting-I/O failure; all ten fields appear in order and status agrees. | **Pass** | Eight complete field-00–09 blocks at lines 614–623, 624–633, 634–643, 645–654, 655–664, 665–674, 675–684, 685–694. Every field 09 is `09=00000000000000000064`. Actual statuses at 697–709 agree with field 08; line 710: `== frozen-dataset-library exit=0`. The runner rejects a missing/malformed report or wrong status before that exit. |

The exact tab-separated status rows at lines 695–710 are:

```text
case	phase	exit_status	expected	wall_seconds
smoke	compile	0	0	2.87
smoke	run-1	0	0	0.00
smoke	run-2	0	0	0.00
smoke	run-3	0	0	0.00
small	compile	0	0	2.86
small	run-1	0	0	0.01
large	compile	0	0	2.92
large	run-1	0	0	0.04
changes	compile	0	0	2.87
changes	run-1	0	0	0.03
wrong-live	compile	0	0	2.91
wrong-live	run-1	70	70	0.03
wrong-copy	compile	0	0	2.89
wrong-copy	run-1	72	72	0.03
== frozen-dataset-library exit=0
```

Absolute heap evidence (payloads quoted exactly; growth is calculated as
field 03 minus field 02):

| Case | Log lines | Before | After | B | Signed growth (bytes) |
| --- | --- | --- | --- | --- | --- |
| smoke, all three | 616–618; 626–628; 636–638 | `02=00000000000000107176` | `03=00000000000000278696` | `04=00000000000000461824` | +171520 |
| small | 647–649 | `02=00000000000001111352` | `03=00000000000001289528` | `04=00000000000000661504` | +178176 |
| large | 657–659 | `02=00000000000008621880` | `03=00000000000008803128` | `04=00000000000000661504` | +181248 |
| changes | 667–669 | `02=00000000000008622168` | `03=00000000000008810072` | `04=00000000000001060864` | +187904 |
| wrong-copy | 687–689 | `02=00000000000008621880` | `03=00000000000017254200` | `04=00000000000000661504` | +8632320 |

**Conclusion.** The prototype's claim **stands within this registered
experiment**: every acceptance criterion passes, and no rejection condition
is observed. Version zero remains readable across completed publication,
the final live root matches the independent model, retained heap satisfies
the prospective bound for all tested sizes/change counts, last-snapshot
release frees storage, and both deliberate wrong controls are detected.
There are no failed or unverified acceptance criteria in this run.

This does not complete the library route or establish production snapshot
support. It does not test universal size bounds, sampled peak heap, RSS,
allocator reserves, exact whole-process return to baseline, bounded cleanup
time, service latency, fairness, simultaneous instructions or publication
inside a particular traversal. Multi-writer conflict resolution, multi-key
batches, arbitrary-size values, B8's reserve/abort, managed cursors,
snapshot-only ledger and stalled-I/O obligations, B6/B7, log boundaries,
export, durability and comparative performance remain outside this
experiment.

**Whole-job failure remains.** The earlier `frozen-dataset-witness` still
exits **9** in this same run: line 566 reads
`step=009 before=00000000000000001536 after=00000000000000009728 difference=+00000000000000008192`;
line 567 reads `witness run exit status: 9`, and line 596 reads
`== frozen-dataset-witness exit=9`. Its
[Results](../frozen-dataset-witness/README.md#results) and
[wf-0c0a2eda83ae rerun](../frozen-dataset-witness/README.md#rerun-on-wf-0c0a2eda83ae-2026-10-10)
already explain exit 9 as the caller-baseline check counting an 8192-byte
runtime frame-arena spare; that witness's pre-registered failure remains
a failure, and its concurrent phase is not reached. This run's diagnostic
again records +8192 on the first frame-only return (line 592, step 024) and
+0 on repetition (line 594, step 026). The workflow sets its overall status
to 1 for any nonzero witness (line 557); line 786 records
`##[error]Process completed with exit code 1.`. The library's exit 0 does
not make the overall job pass.
