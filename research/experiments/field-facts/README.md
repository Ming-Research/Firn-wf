# Reference-field proof witnesses

These standalone programs isolate the proof failure seen in startup replay:
the loop backedge does not discharge `reader^.cursor <= reader^.held` after
writing both fields from locals whose ordering is known. The local-copy change
still failed in gate run 38412917609 on Firn revision 79b1d2e. Do not change
Firn's spelling again on the strength of these experiments.

## Specification and predictions

The pin is `wf-af7665be672d`, full Whitefoot revision
`af7665be672d29912d878dd5da8c3932bb6f296c`. Rule references below refer to
the [kernel specification at that revision](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md),
read from the pinned source, rather than a newer specification.

**All five original programs must be accepted under that specification.**
The owner reported compiler exit 0 for all five in
[run 38417441277](https://github.com/Ming-Research/Firn-wf/actions/runs/38417441277)
on `tmp/field-facts`. This confirms that these standalone forms do not
reproduce the replay failure. No witness has been compiled locally.

| Program | Question isolated | Why acceptance is required |
| --- | --- | --- |
| [reference_loop.wf](reference_loop.wf) | Does writing ordered locals back through a reference establish the field loop invariant on the backedge? | ENT-3.S5 gives the post-write equalities; ENT-5 preserves consequences over surviving values; ENT-6 assigns the written fields their commit-value images. INV-1 admits these tracked reference fields and checks the next-header relation against those images. |
| [local_loop.wf](local_loop.wf) | Does the same calculation preserve a local-variable invariant? This is the control. | ENT-3.S12 imports `f`'s result bound. ENT-3.S5 and ENT-6 establish the local assignment image; INV-1 then proves `a <= b` at the next header. |
| [reference_call.wf](reference_call.wf) | Can a straight-line pair of reference-field writes discharge another function's requirement? | ENT-3.S5, ENT-5 and ENT-6 establish the same ordered field images without a loop. FN-8 checks the consumer's requirement in that state. |
| [reference_subtraction.wf](reference_subtraction.wf) | Can those same writes discharge the unsigned subtraction domain? | ENT-3.S5 and ENT-6 establish `r^.a <= r^.b`; OP-2 requires exactly this bound for `r^.b - r^.a`, using the shared numeric disposition MSR-4. |
| [reference_postcondition.wf](reference_postcondition.wf) | Can a writing callee prove and deliver an exit-state field relation? | FN-9 admits an unrouted clause naming only the exit state of a writable reference even with a `unit` result. MSR-3 interprets it at the caller's resolved exit place; ENT-3.S12 delivers it, and FN-8 must discharge the consumer's requirement. |

Each file has its own `main`. The struct has no type invariant that could
supply the desired bound implicitly. The reference loop starts with the bound
as a requirement, but the straight-line probes do not. The postcondition
witness deliberately enters with unordered fields, so a surviving entry fact
cannot explain acceptance. The counted loops have two iterations and no
runtime guards. Each scalar-producing `f` has `ensures next <= bound`.

For the field writes, let the captured upper value be B and `f`'s result N.
The call establishes N <= B. Writing `a = N` gives that field image N;
writing the sibling `b = B` gives its image B. The latter write cannot turn
the immutable relation N <= B into a relation about different values. The
backedge, requirement and subtraction therefore ask for that same relation.

## Evidence collection

The temporary workflow [tmp-field-facts.yml](../../../.github/workflows/tmp-field-facts.yml)
runs only on pushes to `tmp/field-facts`, on `ubuntu-latest`. It mirrors
the gate's checkout with submodules, `make compiler`, and `make toolchain`.
It compiles every witness separately using the downloaded pinned
`build/whitefoot/<release>/whitefootc`, without running the resulting programs.
It prints complete diagnostics and captures each compiler exit status directly,
then continues after failures. The log and artifact include the source
revision, release manifest, toolchain versions and per-file summary.

The observation job is intentionally always green: only the individual
compiler exits and diagnostics are evidence of acceptance or rejection.
`SETUP_ERROR`, an untrusted setup, or unrelated parsing/toolchain errors do
not establish a field-proof defect. The owner reported compiler exit 0 for
all 24 original and shape witnesses on revision `eccfd7b`, on `tmp/field-facts`.
Results for the extraction and graph witnesses below are pending. Remove the temporary
workflow after collecting evidence, before the branch becomes ready. Retain
these witnesses as the downstream semantic reproductions for any filed gap.

## Replay timeline and the known gap

Board item `coord-wfbl-01-32` describes a missing current-value record for
struct fields, with copying a field into a local making subtraction proofs
go through. The five passing cases show that reference-field writes do
establish facts, direct subtraction uses them, loop backedges can preserve
them, and a writable-reference postcondition reaches the caller in these
forms. That item is not an explanation for the replay failure. This
supersedes the attribution still present in replay's doc string; Firn source
is intentionally unchanged during this investigation.

In `aof_record`, the optional `aof_fill` call completes before `cursor` and
`held` are copied from the reader. Thus the local `held` captures the
post-fill value, not the loop-header value. The scanner's `event.next <= end`
postcondition is instantiated with that local as `end`. Both field stores
(`cursor = next`, then `held = held`) precede the position store and the
event-state branch. On the state-one path the only later call writes the
separate local argument collection; it does not write the reader.

INV-1 requires ordinary body effects and edge cleanup in source order before
checking the next-header invariant batch against the complete frozen state.
The first invariant in the list is not checked before the later `held`
store. On paper this backedge has `reader^.cursor <= reader^.held`: its two
current values are exactly N and B with N <= B supplied by the scanner.
The remaining question is which additional call, projection, control-flow
or invariant context prevents the checker from carrying that relation.

No new checker gap is classified or filed without a failing standalone case.

## Additional loop-shape ladder

Each row starts from its listed parent and changes the indicated feature.
The inert storage and capacity steps separate representation changes from
the nested write and slice-borrow changes. All 19 shape cases were predicted
to compile under the same pinned specification and the owner reported exit 0
for every one on revision `eccfd7b`.
They remain small standalone programs, not alternate Firn implementations.

| Witness | Parent | Difference isolated |
| --- | --- | --- |
| [shape_01_event.wf](shape_01_event.wf) | `reference_loop.wf` | Return the bound in an owned event struct's `next` field rather than a scalar. |
| [shape_02_uncounted.wf](shape_02_uncounted.wf) | 01 | Replace the counted loop with an ordinary loop and explicit termination. |
| [shape_03_event_branch.wf](shape_03_event_branch.wf) | 02 | Add the event-state branch before the fallthrough backedge. |
| [shape_04_storage.wf](shape_04_storage.wf) | 03 | Add inert nested syntax, boxed bytes, EOF and position storage, with entry bounds for the new storage. |
| [shape_05_capacity.wf](shape_05_capacity.wf) | 04 | Add the capacity header invariant through the boxed array. |
| [shape_06_nested_call.wf](shape_06_nested_call.wf) | 05 | Pass `&reader^.syntax` to a writing callee that publishes the event bound and syntax-field guarantees. |
| [shape_06_bounds.wf](shape_06_bounds.wf) | 06 | Copy cursor and use scalar start/end parameters, adding the lower result bound and ordered-endpoint requirement. |
| [shape_07_slice_call.wf](shape_07_slice_call.wf) | `06_bounds` | Keep a slice of `reader^.bytes.inner[0..held]` live across that call, with its read effect and length requirement. |
| [shape_08_fill_branch.wf](shape_08_fill_branch.wf) | 07 | Add the preceding EOF/empty-window branch and a whole-reader writer, then reread cursor and held. Its result match has a nonempty success arm. |
| [shape_09_eof_argument.wf](shape_09_eof_argument.wf) | 08 | Read `reader^.eof` as an additional parser argument. |
| [shape_10_empty_match.wf](shape_10_empty_match.wf) | 09 | Remove only the harmless statement in the preceding match's success arm. |
| [shape_11_four_invariants.wf](shape_11_four_invariants.wf) | 10 | Add the two nested header invariants, giving the same four-relation header shape as replay. |
| [shape_12_type_invariants.wf](shape_12_type_invariants.wf) | 11 | Add Reader's four struct invariants and Syntax's probe struct invariant. |
| [shape_13_position_write.wf](shape_13_position_write.wf) | 12 | Copy and write the sibling position field after the window stores. |
| [shape_14_sibling_call.wf](shape_14_sibling_call.wf) | 13 | Add the conditional unrelated argument-collection writer after the window stores. |
| [shape_15_implicit_fill.wf](shape_15_implicit_fill.wf) | 14 | Remove fill's written postconditions; Reader's type invariants supply its four exit guarantees, as in `aof_fill`. |
| [shape_16_implicit_probe.wf](shape_16_implicit_probe.wf) | 15 | Remove the parser's written probe postcondition; Syntax's type invariant supplies it, as in the real scanner. |
| [shape_17_header_requirement.wf](shape_17_header_requirement.wf) | 16 | Add the parser's header-length entry requirement, as in the real scanner. |
| [shape_18_event_order.wf](shape_18_event_order.wf) | 17 | Declare state before next in the returned event, matching the real event's field order. |

The event-field steps use CALL-4, FN-9 and ENT-3.S12 for result-field data
and ENT-3.S5/ENT-6 for its local copy. Both loop forms and their branching
backedges are admitted by INV-1 and FN-1. Nested effects and the unrelated
writer are resolved through EFF-2 and OWN-7; ENT-5 kills facts only for the
resolved overlapping storage. Slice formation uses REF-4 and OP-4, with the
capacity relation providing its bound; EFF-5 permits the disjoint syntax write
and byte-slice read. The whole-reader fill deliberately invalidates the old
field facts, but its declared guarantees and the subsequent local copies
establish the new bounds before the scanner. Branches and matches use
ENT-3.S1 and ENT-5; EOF is an ordinary copied Bool argument. MSR-1, INV-1
and TYPE-11 govern the nested measures and the additional invariant sites.
The last four cases distinguish written from implicit guarantees, exercise
the parser's real entry requirement, and put its bounded result field at the
same declaration ordinal as the real event. Dropping duplicate written
clauses does not weaken the guarantee: TYPE-11 requires the callee to prove
and deliver those same exit relations.

For compactness, the unrelated argument writer updates a local scalar
instead of a collection, and an extra iteration counter terminates the
ordinary loops after two visits. That counter never guards a proof-sensitive
call or write. The scanner body only supplies its signature's guarantees;
it does not implement AOF parsing. The final case still omits the real file
I/O/waiting context, scanner's other event fields and its internal branches,
the argument collection, and the complete state-two-through-six return
chain. Passing this ladder would leave those differences to reduce next.

Find the first failing step and compare it with its passing parent. A new
parsing, ownership or call-requirement error must be evaluated separately;
it is not evidence of the original backedge failure. If every step passes,
the remaining differences in the real parser and surrounding call context
still need reduction. No new attribution follows merely from having added
these cases.

## Module lookup and graph witnesses

`firn/modules.wfg` registers `pkg::persistence` at the persistence directory.
By MOD-2, both its direct files `scanner.wf` and `replay.wf` are implementation
records of that one module. MOD-3 gives them a shared declaration inventory:
`aof_syntax_next`, `AofSyntax` and `AofEvent` are private declarations visible
to `aof_record`, whose call is unqualified. It does not use an alias or cross
another package or module boundary.

`firn/persistence/module.wfm` does not declare `aof_syntax_next`; it exports
the external persistence API. Therefore it has no copy of the scanner's
endpoint postcondition, and none is needed for this private call. The real
definition in `scanner.wf` declares `ensures event.next <= end`, the lower
endpoint guarantee, and the header-length guarantee. A missing exported
interface clause does not explain the failure. MOD-7 requires exported
definitions to repeat their complete interface contracts; MOD-8 makes
cross-module clients use the interface instead of the implementation.

Two graph witnesses separately test the exported and actual private-call
arrangements, both with entry `field_facts`:

| Graph | Question | Spec prediction |
| --- | --- | --- |
| [graph/modules.wfg](graph/modules.wfg) | Two modules: scanner exports its types and functions; caller invokes the scanner through an alias. The `.wfm` repeats all six scanner clauses, including `ensures event.next <= end`. | Accept: MOD-4/7/8 preserve the function identity and declared guarantees; TYPE-11 supplies the probe bound. FN-9, ENT-3.S12 and INV-1 establish the window backedge. |
| [graph/same-module/modules.wfg](graph/same-module/modules.wfg) | One module: private scanner types/functions in `scanner.wf`, private replay loop in `replay.wf`, with only main exported. This mirrors Firn's cross-record private lookup. | Accept: MOD-2/3 make the private declarations visible across implementation records; the same endpoint contract and invariant proof apply. |

Both graph callers keep the four-invariant header, byte slice, nested syntax
writer, EOF argument, event-field copy and pair writeback. The exported
case exposes readonly Syntax fields because TYPE-11 requires this at a
public type boundary; the private control has private fields. They remain
reduced loops, not the full replay extraction.

The workflow now compiles each graph with
`whitefootc --graph <modules.wfg> --entry field_facts -o <binary>`.
It records separate full diagnostics in `graph_exported.compiler.log` and
`graph_same_module.compiler.log`, and includes both direct exit statuses in
the existing summary. Rejections still do not stop later compilation or
turn the observation job red.

## Real loop extraction

[real_extract.wf](real_extract.wf) contains the real `AofReader`, `AofSyntax`,
`AofEvent` and `AofRecord` declarations unchanged, including field order and
type invariants. It copies the entire `aof_fill` body and uses the real
standard-library Clock, HandleFactory and ReadFile types. The scanner's
signature, effect row and all three requires/three ensures clauses are
unchanged; only its body is reduced to bounded events and syntax updates.

The `aof_record` signature and body are copied verbatim **except one
packaging adaptation**: `pkg::protocol::push_arg` becomes local `push_arg`.
PROG-1/2 gives a single-file bundle only the root module `pkg` and standard
library; MOD-5 does not permit an unregistered `pkg::protocol` namespace.
The local helper retains the real argument-collection implementation and
effect row. Span has the same fields but becomes private, as MOD-6 requires
in a source bundle. Keyspace is reduced to cancellation state and the
cancellation reader is correspondingly reduced; no server keyspace behavior
is modeled. The pause/sleep helpers retain their real bodies.

The copied loop has no artificial iteration counter and retains its real
state-one-through-six branch chain, `Result<AofRecord, u8>` returns and
collection ownership. Its original doc string is also copied; its old
attribution to the known field-image item is preserved source text, not a
conclusion of this investigation. FN-7 makes source acceptance independent
of entry reachability, so a trivial main allows checking every ordinary body
without acquiring or executing file I/O.

The extraction is predicted to compile under the pin: the scanner boundary
supplies the same N <= B relation, TYPE-11 preserves the fill's exit bounds,
and INV-1 checks the fully written backedge. This is not a claim that the
reduced scanner performs valid replay. A failure would provide a much closer
standalone reproducer; acceptance would leave the real keyspace/dependency
context and scanner internals outside this extraction to investigate.
