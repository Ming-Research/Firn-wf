# Review checklist

firn's own review items, checked with the owner-wide review checks G1–G3 and
DC1–DC4 and against the owner-wide engineering standards.

## How to review

Start the reviewer with this prompt, filled in:

```text
You are reviewing a firn change you did not write. Do not edit files.
Task outcome and constraints: <...>
Base and head: <...>; validation already run: <commands, results, revision>.
Read the diff from the base (git diff <base>, plus untracked files) and the
changed sections in context. Apply the owner-wide review checks G1–G3 and
DC1–DC4 and engineering standards (copied in design/skill/SKILL.md) and each
group of docs/review-checklist.md whose trigger applies, and run
make design-lint. Do not rerun green suites. Report Scope (your model,
base..head, groups checked and skipped), Checks (what you ran) and Findings
(item ID, file:line, quoted text or missing evidence, reason; quote both
sides of a contradiction), or "none within scope".
```

## C. Code and cases — changes to Whitefoot sources or tests

- [ ] **C1 — Reference expectation.** Expected results come from the
  [reference](../AGENTS.md#references-and-correctness), Redis 7.0.15, never
  firn's current output; a fix's case fails before the fix and passes after.
- [ ] **C2 — General path.** The change implements the command's general
  behavior as Redis does. No test, client or benchmark selects a special
  path, and no fallback conceals an unsupported feature.
- [ ] **C3 — Interface fidelity.** Module bodies implement their `.wfm`
  interfaces as written. An interface, contract or effect row changed only
  with the architecture's approval, and none was weakened to let a body pass.

## T. Checks and pins — changes to tests, the Makefile, `.github/`, `whitefoot.pin` or a submodule

- [ ] **T1 — The ratchet.** A row leaves `tests/redis-suite/passing.tsv` or
  enters `unstable.tsv` only on `redis-suite-record.yml` evidence or a stated
  technical reason; new passes the ratchet reported are added.
- [ ] **T2 — Local and CI correspondence.** CI runs the same Makefile targets
  as local `make check`; changed selection adds no omission or extra check.
- [ ] **T3 — The pins.** The [review items](../whitefoot-kit/downstream.md#review-items)
  of Whitefoot-kit hold for `whitefoot.pin` and the `whitefoot-kit` and
  `design/skill` submodules, and firn's checks pass with them.

## R. Measurements — changes under `research/`, or a performance claim

- [ ] **R1 — Performance claims.** A claim names the workload, the machine,
  the engine versions compared and the comparison; an attribution follows the
  [method](../AGENTS.md#design-tree) of interleaved `make firn-lto` builds on
  the 14900K.

## D. Documents — changed Markdown

- [ ] **D1 — Roles.** Each changed passage fits its document's
  [role](../AGENTS.md#documents).
