# Function-parameter postcondition reduction ladder

This instrument re-reduces the FN-9 reported in [Firn CI run
38413960370](https://github.com/Ming-Research/Firn-wf/actions/runs/38413960370)
on `60d7105`. The observed failure is the normal-return length guarantee at
`firn/store/command-metadata.wf:209`, not the absence of contracts on raw
function parameters: FN-3 through FN-5 already specify them. The Whitefoot
owner reports that the earlier standalone and module-graph reductions are
accepted on Whitefoot main; its exact tested revision was not supplied.
That observation does not establish their result on either compiler used
here, and it does not explain the historical server failure.

## Historical boundary recovered

At `60d7105`, the public declaration in `firn/store/module.wfm:630` repeats
the complete adapter signature defined in `command-metadata.wf:202–210`.
The callback and adapter both publish:

```whitefoot
requires keys^.len == declared^.len;
ensures keys^.len == entry(keys)^.len;
```

There is one instantiation, `firn/commands/script.wf:1577`, supplying
`<Box<Slots<Span>>, Client, fn held_entries_run>` and `keys: slots`.
Its concrete function at line 1580 has the same sixteen ordered parameters,
effects and contracts as the formal `apply`. Thus there is no historical
contract-refinement discrepancy to remove. The caller obtains the entries
view with `let slots = &keys^[command_keys]` before forwarding it.

The outer adapter prepends `meta`, then supplies `due: &meta^.due` at the
callback's position eight. As a result, `keys` is outer parameter five but
callback parameter four, counting from zero. The old accepted reduction
used parameter zero for `window` on both sides and therefore lost this
potential remapping cause as well as the projection, Entries and generic
argument shapes.

## Controlled steps and limits

[cases.json](cases.json) is the ordered case inventory and defines every
step's delta. There are sixteen chain steps and six independent controls.
Every file is self-contained; step 00 is a complete two-module program.

Step 00 retains the historical sixteen-argument signatures and effect rows,
the public `.wfm` boundary, `A,C`, real `Entries<Entry>` and `KeySet`,
`&meta^.due` with `writes(meta.due)`, both Option arguments, a numeric return,
and the four selected-database clauses on WriteSequence, ExpiryQueue and
Meta, including the nested expiry bound. The callback's writes to invariant
fields go through declaring-module writers with narrow rows. Its concrete
types are `Box<Slots<Span>>` and `Client`, as in the historical instantiation.
One atomic map target supplies the entries; the reduction introduces no
multi-map alias requirement.

The callback changes entry payloads without resizing the entries view and
exhibits every declared runtime effect. Its command dispatcher and unrelated
payloads are reduced to small stand-ins; this is a signature-shaped
reduction, not an extracted build of the complete historical server. Entry,
the queue, write sequence, metadata, client and journal/undo retain their affine
capability through `nocopy`, but use counters instead of their original
storage. Journal/undo payloads omit their unrelated invariants. The historical
map-scanning caller is reduced to an equivalent
one-target KeySet acquisition. If even step 00 is accepted by both compilers,
these removed bodies, types or call-graph relationships must be restored in
a further reduction before attributing the original failure.

The important separate comparisons are:

- 01 flattens module visibility only, preserving writer calls and invariant
  logic; public fields become private fields in the standalone program.
- 02 replaces the projection with a direct due reference at outer position
  zero. It preserves the keys ordinal mismatch. Only 03 aligns the outer
  parameter order with the callback.
- 04 removes invariant clauses. 05 removes the unrelated state/effect
  channels, 06 removes the two Option channels, and 07 specializes A/C.
- 08 through 10 remove due, args and client individually. 11 removes the
  relational requirement and declared KeySet. 12 replaces Entries with
  Slots; 13 adds the nonempty requirement; 14 changes only the callback body
  to take/place; 15 changes the numeric return to unit.

Step 15 is structurally the accepted one-container example, with the same
`entry` guarantee, but retains unused stand-in declarations and a common
construction scaffold. `accepted-baseline.wf` is a byte-exact copy of the
previous accepted file, providing the actual endpoint without conflating
cleanup and naming changes with a reduction step.

`ordinal-before` and `ordinal-after` keep an identical dummy read and Slots
callback but put the dummy before or after window. They isolate parameter
order. `specialized-full-raw` and `direct-control` keep the full concrete
sixteen-argument projected shape, invariants and callback body; only the raw
function-parameter boundary differs. Comparing direct-control straight with
step 01 would also remove A/C, so the matched specialized control is needed.

`caller-view-control` restores the historical `held_run` whole-map parameter,
its locally built command KeySet, `let slots = &keys^[command_keys]`, local
undo/fallback and generic instantiation. Its adapter and callback match step
01. It tests the caller/view origin separately from the direct atomic Entries
acquisition used by the chain.

No ladder step is declared a known rejection or acceptance in advance.
The later rungs add or change prerequisites needed to reach the existing
accepted example; those changes are explicit in the inventory. Passing
and failing results need not be monotonic.

## Compilers and CI caller

The temporary [select-db-fnparam-ladder workflow](../../../../.github/workflows/select-db-fnparam-ladder.yml)
runs on pushes to `claude/select-db`. It downloads both compilers through
Whitefoot-kit's checksum and manifest verification:

| Role | Release tag | Full Whitefoot commit |
| --- | --- | --- |
| Historical pin | `wf-af7665be672d` | `af7665be672d29912d878dd5da8c3932bb6f296c` |
| Requested formal release | `wf-3adedae9bb1a` | `3adedae9bb1a9028e0afd95d2776e5330db4abc1` |

The supplied `3adedae9b` resolves to that full commit in the local Whitefoot
repository. Whitefoot's release workflow at that commit and the kit define
the tag as `wf-` plus its first twelve digits. Network access did not allow
an independent release-asset lookup here; the CI download and exact full
manifest match must verify availability. The formal compiler uses an
ephemeral pin file; the repository pin and production sources are untouched.
Download failure is an infrastructure failure, not a compiler result.

The workflow calls `run.py` with both tag/commit/download-directory triples,
the inventory and an artifact output directory. It uses only `--check` for
bundles and `--graph ... --check-modules` for the graph, with a sixty-second
per-invocation timeout. It emits no executable. `test_run.py` uses fake
subprocess results to verify the recorder in CI; it never invokes Whitefoot.
No compiler, test or check script was run locally.

The artifact and job summary contain all forty-four rows with step, compiler,
actual exit code and first diagnostic line. Full diagnostics, arguments,
timeouts and release manifests are retained alongside JSON and Markdown
tables. All ordinary rejections are recorded without skipping later steps.
Missing inputs, invalid arguments, malformed inventories and wrong compiler
provenance fail the recording job. Green recording means data was collected,
not that Firn or all witnesses compile.

The summary identifies each compiler's first passing chain step and last,
most reduced chain step matching FN-9 at the forwarding `return apply` with
the unchanged-length guarantee, and separately lists matching control
failures. Other FN-9 sites, other diagnostics and
timeouts are separate findings. This does not claim global minimality.

## Results

Pending owner-driven CI. The first passing step on `3adedae9bb1a` and the
smallest observed failing reduction are not yet known. The runner calculates
both from actual diagnostics; no result is inferred from an earlier Firn
reply, a specification expectation or the owner's different-main observation.
