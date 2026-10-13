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

**All five programs must be accepted under that specification.** These are
predictions, not compiler results. No witness has been compiled locally.

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
runs only on pushes to `claude/aof-load-rules`, on `ubuntu-latest`. It mirrors
the gate's checkout with submodules, `make compiler`, and `make toolchain`.
It compiles every witness separately using the downloaded pinned
`build/whitefoot/<release>/whitefootc`, without running the resulting programs.
It prints complete diagnostics and captures each compiler exit status directly,
then continues after failures. The log and artifact include the source
revision, release manifest, toolchain versions and per-file summary.

The observation job is intentionally always green: only the individual
compiler exits and diagnostics are evidence of acceptance or rejection.
`SETUP_ERROR`, an untrusted setup, or unrelated parsing/toolchain errors do
not establish a field-proof defect. CI results are pending; append the run
link and per-witness outcomes here when available. Remove the temporary
workflow after collecting evidence, before the branch becomes ready. Retain
these witnesses as the downstream semantic reproductions for any filed gap.

## Interpreting results and the known gap

Board item `coord-wfbl-01-32` describes a missing current-value record for
struct fields, with copying a field into a local making subtraction proofs
go through. Firn's failed local-copy attempt means the earlier attribution
to that gap is not established. These cases separate the possible causes:

- If the local control fails, first investigate its diagnostic; the intended
  comparison is not established.
- If the control and straight-line cases pass but the reference loop fails,
  the defect is in loop/backedge handling of field facts, rather than simply
  proving subtraction from the current fields.
- If the straight-line call and subtraction fail after the writes, the
  compiler fails to establish or retain reference-field write facts. This
  overlaps the known item's field-image mechanism, but identical root cause
  requires compiler investigation.
- If only subtraction fails, its domain-query path matches the known item's
  symptom more directly than the contract or invariant paths do.
- In the postcondition case, an FN-9 failure in `g` concerns proving the
  producer's exit-state relation; acceptance of `g` followed by an FN-8
  failure at the consumer instead isolates delivery to the caller.
- If all five pass, these minimal cases do not reproduce Firn's failure.
  Reduce the remaining call/control-flow context into another standalone
  witness; do not infer that the Firn diagnostic is explained.

No new checker gap is classified or filed from predicted outcomes alone.
