# Frozen persistent keyed dataset

Results: pending CI on wf-0c0a2eda83ae

This is the next expressibility experiment for status-board item
`firn-wf-snap-lib-proto`, not a production replacement for firn's keyspace.
The [scalar acceptance probes](../frozen-dataset-prototype/README.md) precede
it. The governing [investigation](https://github.com/Ming-Research/Whitefoot/blob/main/research/investigations/consistent-snapshots/README.md#contracts-required-before-implementation),
[library direction](https://github.com/Ming-Research/Whitefoot/blob/main/design/language/data-model/frozen-datasets.md)
and [retention policy](https://github.com/Ming-Research/Whitefoot/blob/main/design/language/data-model/frozen-datasets/retention-budget.md)
remain in force. Language reference: Whitefoot v0.123, SHARE-1, PRE-1,
TYPE-9 and OWN-1; witness release `wf-0c0a2eda83ae`.

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

No new language gap identified by source inspection. Compilation and runtime
behavior remain unverified until CI; no local build, compiler, runner or
test invocation is authorized for this handoff. Existing reserve and ledger
obligations remain as described above.

## Read-only review

An independent Codex reviewer (GPT-6 family) inspected all five experiment
files and the workflow diff against `eeee118d6f311755f2871f2c9253c31836e8954b`,
the project checklist, governing design nodes and v0.123 contracts. Findings:
none within scope. Grammar, ownership, trie paths, independent oracles,
publication, heap-charge derivation and runner verdicts passed source review.
Runtime correspondence, compiler acceptance and actual control sensitivity
remain unverified. No builds, compilers, tests, lint or runners were executed.
The implementing agent subsequently clarified that the post-join traversal
follows all M publications and that total growth, rather than separately
attributed runtime storage, is compared with B; no behavior changed.

## Gaps

### A recursive read of a Frozen tree has no acceptable effect row (wf-0c0a2eda83ae)

`map_lookup_at` in `map.wf` recurses into a `Branch`'s child handles. Run
[38093338376](https://github.com/Ming-Research/Firn-wf/actions/runs/38093338376)
rejects every row offered for it (EFF-2):

- `pure` is refused: the body reads `root.inner.Leaf.key` and `.byte`.
- That exact row is refused: the recursive call adds
  `root.inner.Branch.zero.inner.Leaf.key` and the like, one level deeper per call, so no finite
  list of paths covers a recursion of unbounded depth.
- `reads(root)` was refused earlier on `map_is_empty` as wider than the body's reads.

Minimal form:

```text
enum Node { Empty(); Leaf(key: u64, byte: u8); Branch(zero: Frozen<Node>, one: Frozen<Node>); }
fn lookup(root: &Frozen<Node>, key: u64, mask: u64) -> result: Option<u8> <row?> {
  match &root^.inner {
    Empty() => { return None<u8>(); }
    Leaf(key: k, byte: b) => { if k^ == key { return Some<u8>(value: b^); } return None<u8>(); }
    Branch(zero: z, one: o) => { ... return lookup(root: z, key: key, mask: next); }
  }
}
```

Acceptance: this recursion compiles with a finite row, for instance because a frozen object's
contents can never be written and so reading them needs no row, or because a covering row such
as `reads(root)` is admitted. The prototype stops here rather than rewriting the lookup as a
loop over local handles, which would avoid the row without the language accepting the natural
form. Filed as status board item `proof-bl-frozen-recursive-row`.
