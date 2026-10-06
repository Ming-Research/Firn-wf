# Review checklist

The items firn's completion review checks beside the owner-wide review checks
G1–G3 and DC1–DC4.

## How to review

Read the task's requested outcome and constraints, the complete diff from the
base to the reviewed revision (including uncommitted and new files), the
actual validation results, the changed sections in context and the directly
affected definitions, interfaces, callers or cases. Start the reviewer with
this prompt, filled in, and the groups [AGENTS.md](../AGENTS.md#review) names
for the kind of change:

```text
You are reviewing a firn change you did not write. Do not edit files.
Task outcome and constraints: <...>
Base and head: <...>; validation already run: <commands, results, revision>.
Read the diff from the base (git diff <base>, plus untracked files), the
changed sections in context, and "How to review" in docs/review-checklist.md.
Check each group whose trigger applies, run make design-lint, and apply the
owner-wide review checks G1–G3 and DC1–DC4 (copied in design/skill/SKILL.md)
to the relevant tree nodes and ancestors. Do not rerun green suites. Report
Scope (your model, base..head, groups checked and skipped), Checks (what you
ran) and Findings (item ID, file:line, quoted text or missing evidence,
reason; quote both sides of a contradiction), or "none within scope".
```

Mark each item `pass`, `finding`, `unverified` or `not applicable`; missing
evidence is not a pass, and a question that needs an unrecorded reason is
`unverified`.

## A. Scope and layout — every change

- [ ] **A1 — Task fit.** Each changed artifact serves the requested outcome or
  a necessary dependency. No requirement is silently dropped, and no
  advertised fix is left as a stub or unconnected code.
- [ ] **A2 — Paths and connections.** Each added file or directory has a
  consumer, a home and a removal condition, and no root entry is added
  without approval. Added scripts and tests have a caller; documented
  commands name existing targets; moves and deletions update links and
  wiring.
- [ ] **A3 — Artifacts.** No scratch output, personal path, credential or
  machine-local setup, and no vendored Whitefoot source.

## D. Documentation — changed Markdown, comments or examples

- [ ] **D1 — Role.** Each changed passage fits its document's
  [role](../AGENTS.md#documents), without editing history.
- [ ] **D2 — References.** Changed references resolve to the intended file,
  heading or symbol and support their claim.
- [ ] **D3 — Current meaning.** Changed claims agree with their owning source.
  A goal, a proposal, a decision, an implemented capability and a dated
  measurement stay distinct.
- [ ] **D4 — Usability.** Instructions name real commands and prerequisites;
  changed runnable examples were run.

## C. Code and cases — changes to Whitefoot sources or tests

- [ ] **C1 — Observable case.** A fix has a case that distinguishes the faulty
  behavior from the intended result; new behavior has coverage for its normal
  use and relevant boundaries.
- [ ] **C2 — Independent expectation.** Expected results come from the
  [reference](../AGENTS.md#references-and-correctness), never firn's current
  output. A regression case fails before the fix and passes after.
- [ ] **C3 — General path.** The change implements the command's general
  behavior as Redis does. No test, client or benchmark selects a special
  path, and no fallback conceals an unsupported feature.
- [ ] **C4 — Interface fidelity.** Module bodies implement their `.wfm`
  interfaces as written. An interface, contract or effect row changed only
  with the architecture's approval, and none was weakened to let a body pass.

## T. Checks and pins — changes to tests, the Makefile, `.github/`, `whitefoot.pin` or a submodule

- [ ] **T1 — Preserved checks.** Every removed, skipped, narrowed or weakened
  test or check has a technical reason consistent with the requested change.
- [ ] **T2 — Effective checks.** New cases actually run; new check machinery
  shows that a representative wrong result is detected.
- [ ] **T3 — Local and CI correspondence.** CI runs the same Makefile targets
  as local `make check`; changed selection adds no omission or extra check.
- [ ] **T4 — The pins.** The [review items](../whitefoot-kit/downstream.md#review-items)
  of Whitefoot-kit hold for `whitefoot.pin` and the `whitefoot-kit` and
  `design/skill` submodules, and firn's checks pass with them.

## R. Experiments — changes under `research/`, or a choice an experiment selected

- [ ] **R1 — Prior criterion.** An experiment used to choose has the question,
  the comparison and the rejecting result in its investigation from before
  the measurement, and records its conditions and actual outcome.

## V. Validation — every change

- [ ] **V1 — Actual checks.** Applicable checks ran on the delivered content;
  commands, results and limitations are available, and focused success is not
  described as the gate.
- [ ] **V2 — Supported claims.** Counts, paths, revisions and quoted results
  were checked. A performance claim names workload, machine, engine versions
  and comparison; a causal claim has isolating evidence.
- [ ] **V3 — Delivery.** The PR's remote head is the reviewed revision, and
  its description gives the current result, its limitations and, under
  *Found along the way*, each defect or opportunity with its disposition. A
  ready PR carries the owner's approval of every design-tree change in
  `design/log.md`.
