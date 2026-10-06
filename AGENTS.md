# Firn-wf — agent instructions

Firn-wf holds firn, a server of Redis's protocol written in Whitefoot
(`firn/`, one module program, `firn/modules.wfg`), growing toward Redis
scripting through Halo, the Lua engine of
[Halo-wf](https://github.com/Ming-Research/Halo-wf).

## Goal and priorities

firn serves Whitefoot: it is the real server that shows what Whitefoot gives a
concurrent, persistent network program under its guarantees, and exposes what
Whitefoot still lacks. Its milestones ([design](design/firn.md)) are
standalone Redis application workloads, cache and session storage and
scripted conditional updates, used by existing applications through their
ordinary clients with their business logic unchanged, and then the keyspace
scaled past two cores on the 14900K.

When priorities conflict:

1. reach the next end-to-end compatibility, workload or performance
   experiment;
2. keep replies and persisted state identical to the reference, with every
   safety check Whitefoot requires;
3. keep the implementation understandable and easy to change;
4. add only the evidence needed to trust the current result;
5. defer robustness, infrastructure and polish that no current experiment
   needs.

## References and correctness

- The reference is Redis 7.0.15 on x86-64 Linux: its replies, the
  append-only files it writes and reads, its own test suite
  (`tests/redis-suite/`), and its source where the documentation is silent.
  Correctness is judged by it, never by firn's own earlier output.
- A command implements Redis's general behavior; no test, client or benchmark
  selects a special path in the server.
- The language is the pinned Whitefoot commit: read its specification,
  maintained programs and standard-library interfaces at that commit as
  [whitefoot-kit/downstream.md](whitefoot-kit/downstream.md#reading-the-language)
  describes; the compiler's diagnostics and repairs are the other source.
- Research records written while firn lived in the Whitefoot repository cite
  Whitefoot paths and commands of their time; they are evidence, not current
  instructions.

## Design tree

The live trees are the root node files under `design/` other than `log.md`,
each with its subdirectory; the change log is `design/log.md`; the research
record is `research/investigations/` (`firn/` holds the server's design,
rulings, criteria and measurements) and `research/experiments/` (the
instruments and their results); the maintained TODO is `docs/todo.md`; the
form and readiness checks are `make design-lint` and `make design-ready`,
which run `lint.py` from the `design/skill/` submodule.

A design decision here is a choice between viable alternatives that changes
accepted requests or replies, persisted state, a safety or trust condition,
a shared interface or representation, a significant performance commitment
or a standing project rule. These decisions are firn's implementation rules:
read the nodes a change touches and their ancestors before changing code.

An investigation's question, comparison and rejecting result go in
`research/investigations/<name>/` before it measures. A performance change is
attributed with a same-source before-and-after comparison of interleaved runs
of `make firn-lto` builds on the 14900K, with a twin of the base as a noise
control, and a falsifier.

## Architecture

The owner and the primary agent own the architecture: the design tree, the
module graph, the Whitefoot module interfaces (`.wfm`) with their contracts
and effect rows, and the way firn hosts Halo. An implementer that finds an
interface insufficient reports the gap with a minimal example instead of
editing it.

## Branch and main boundary

These are the complete approval and merge rules:

1. Work-branch changes need no approval, including the design tree, code,
   tests, gate wiring, the pins and documentation, except that new
   repository-root entries require owner approval. A PR becomes ready only
   after the owner has approved every decision it needs, including every
   design-tree change; the approval is recorded in `design/log.md` only then,
   and `make design-ready` checks the record.
2. Every change merged into `main` requires owner approval of the exact
   revision to be merged.
3. The exact revision merged into `main` must pass `make check` before the
   merge.
4. A change that moves `whitefoot.pin` or a submodule names the revisions it
   adopts and why. A revision merged into `main` pins a Whitefoot release
   `wf-<12 hex>` of a commit on Whitefoot's `main`, never an experiment
   release, and submodule commits on their repositories' `main`.

**Exact revision** is the complete tree that will enter `main`, the pins
included; if it changes after approval or after its successful check, rules
2 and 3 apply to the new revision. No other workflow step is an approval or
merge precondition.

## Checks

- `make check`, the gate, runs in CI on every push: it downloads the pinned
  compiler (`make compiler`), builds firn and runs its network cases
  (`tests/`), the Redis 7.0.15 suite ratchet (`tests/redis-suite/`, without
  `--tolerant`) and the design lint. The ratchet requires every test in
  `passing.tsv` to pass and reports new passes to add in the same PR;
  `unstable.tsv` records exclusions from repeated CI runs, and the
  dispatch-only `redis-suite-record.yml` workflow produces both lists for
  review and commit.
- The gate needs git, curl, Python 3, Rust stable with Cargo, `tclsh` 8.5 or
  later, `redis-cli` and `redis-benchmark`, the submodules
  (`git submodule update --init`), `/usr/bin/clang` and, on Linux, LLD; the
  Redis suite runner needs Linux's GNU tools.
- `make design-ready` and `make pin-ready`, before marking ready and in CI on
  ready PRs and main: every design-tree change is approved in the log, and
  `whitefoot.pin` names no experiment release.

## Review

The completion review's reviewer gets the prompt in
[the review checklist](docs/review-checklist.md); its scope and the findings
fixed go in the PR's review section.

## Whitefoot

firn builds with the compiler release `whitefoot.pin` names, through the
`whitefoot-kit/` submodule. Its
[downstream.md](whitefoot-kit/downstream.md) holds the rules for the pin,
trying an unmerged Whitefoot change, upgrading Whitefoot and recording a
Whitefoot gap (under *Whitefoot requirements* in `docs/todo.md`); a change to
them is made in [Whitefoot-kit](https://github.com/Ming-Research/Whitefoot-kit).
An upgrade whose compiler changed code generation compares firn's benchmarks
before and after on the 14900K
(`research/experiments/redis-bench/redis-bench.sh`).

## Reports

A completion report also names any `whitefoot.pin` or submodule moved and any
Whitefoot gap filed.

## Documents

`README.md` introduces and navigates, `firn/README.md` describes the server's
commands and invocation, `design/` holds the decisions and their log,
`docs/review-checklist.md` the review items, `docs/todo.md` open defects and
Whitefoot requirements until resolved, and `research/` questions, experiments
and results.
