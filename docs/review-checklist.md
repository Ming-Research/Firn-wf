# Review checklist

## A. Every change

- [ ] **A1 — No vendored Whitefoot.** Whitefoot enters only through
  `whitefoot.pin`; no Whitefoot source is copied into the repository.

## C. Code and cases — changes to Whitefoot sources or tests

- [ ] **C1 — Reference expectation.** Expected results come from Redis
  7.0.15's replies and files, observed or read from its source, or its own
  test suite.
- [ ] **C2 — Interface fidelity.** Module bodies implement their `.wfm`
  interfaces as written, and no contract or effect row was weakened to let a
  body pass.

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

- [ ] **R1 — firn's comparison.** A performance claim about firn names the
  engine versions compared, and an attribution compares `make firn-lto`
  builds on the 14900K.

## D. Documents — changed Markdown

- [ ] **D1 — Roles.** Each changed passage fits its document's
  [role](../AGENTS.md#documents).
