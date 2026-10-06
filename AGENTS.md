# Firn-wf — agent instructions

Firn-wf holds firn, a server of Redis's protocol written in Whitefoot. It
answers Redis's clients over TCP for strings, lists, sets, hashes and sorted
sets in one shared keyspace, with expiry and an append-only file, and is
growing toward Redis scripting through Halo, the Lua engine of
[Halo-wf](https://github.com/Ming-Research/Halo-wf). Whitefoot, the language
and its compiler, is pinned as a compiler release named in `whitefoot.pin`
and fetched through the `whitefoot-kit/` submodule, and the design tree's lint
comes from the `design/skill/` submodule of Design-skill.

The owner-wide agent instructions, which every Claude and Codex session loads
(`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`), govern reports, the ledger,
decision cards, pull requests, completion and the design tree. This file adds
what is firn's own: its goal, paths, checks, review checklist and reviewer
models, merge rules and project rules.

## Project goal

firn exists to serve Whitefoot: it is the real server that shows what
Whitefoot gives a concurrent, persistent network program under its
guarantees, and exposes what Whitefoot still lacks. Its milestones
([design](design/firn.md)) are standalone Redis application workloads, cache
and session storage and scripted conditional updates, used by existing
applications through their ordinary clients with their business logic
unchanged, and then the keyspace scaled past two cores on the owner's
i9-14900K.

When priorities conflict, use this order:

1. reach the next end-to-end compatibility, workload or performance
   experiment;
2. keep replies and persisted state identical to the reference, with every
   safety check Whitefoot requires;
3. keep the implementation understandable and easy to change;
4. add only the evidence needed to trust the current result; and
5. defer robustness, infrastructure and polish that no current experiment
   needs.

## Authority and reading

`design/` holds the decisions firn is built on, each with its reason and
refused alternatives. Work is not planned in a document up front: a selected
direction gets `research/investigations/<name>/` for its design,
measurements and rejected alternatives, and its surviving decision goes to
the design tree. `research/investigations/firn/` holds the server's design,
its owner's rulings, its criteria and its measurements;
`research/experiments/` holds the instruments and their results. Read only
the material relevant to the task, and do not turn research into an implied
implementation requirement.

**The reference.** firn's behavior is judged against Redis 7.0.15 on x86-64
Linux: its replies, the append-only files it writes and reads, and its own
test suite (`tests/redis-suite/`, with historical results under
`research/experiments/redis-compat/`). Its source is the oracle
for what a command does where the documentation is silent.

**The language.** The pinned Whitefoot commit defines the language; read its
specification, maintained programs and standard-library interfaces at that
commit as [whitefoot-kit/downstream.md](whitefoot-kit/downstream.md#reading-the-language)
describes. The compiler's diagnostics and repairs are the other source.

A finished task is not evidence: a claim cites a design-tree decision, an
investigation, a measurement with its workload, environment and comparison,
or the reference's own behavior. Research records written while firn lived
in the Whitefoot repository cite Whitefoot paths and tooling of their time,
such as `compiler/target/gate/whitefootc` and `.github/run-check.pl`; they
stay as historical evidence, and their commands are not current
instructions.

## How work proceeds

A *material choice* changes accepted requests or replies, persisted state, a
safety or trust condition, a shared interface or representation, a
significant performance commitment or a standing project rule; only a
material choice between viable alternatives is a design decision. Restoring
decided behavior or editing prose without changing its meaning is routine.

1. **Before starting,** read the affected design-tree nodes and their
   ancestors, and verify the worktree and PR state on resumption.
2. **While working,** state why each material choice fits its evidence and
   record an experiment's criterion before using it to choose. Change the
   code and the design tree together on a Draft PR, and update what a changed
   conclusion affects in the same work.
3. **At completion,** run the [checks](#checks) and the [review](#review).
   The completion report also names any `whitefoot.pin` or submodule moved
   and any Whitefoot gap filed.

**Judge a design by its merits, not by the work it takes.** No design
judgment weighs the existing code, tests or documents a choice would change,
nor the effort of changing them.

**Verify with observations that could have come out otherwise.** A passing
result is evidence only if a wrong result would have failed it. Make each new
check fail once for each way it can fail, and never check a transform against
its own output. Resolve every commit id, path, count and measurement with a
tool when you write it. Another agent's or a reviewer's report is a lead to
verify, not evidence. A green result reached by weakening a requirement does
not answer the original question.

**Size a run before starting it.** Before any build, test batch, measurement
or experiment, run the smallest useful sample, time it and look at its
spread, then choose the scale; repeat or lengthen only where the spread is
too large to decide. Never open with a run of hours.

A series of dependent PRs is stacked, each on the branch of the one before
it. Updating a work-branch PR never authorizes a merge into `main`.

**The design tree.** The owner-wide instructions' design-tree part applies,
with these roles: the live trees are the root node files under `design/`
other than `log.md`, each with its subdirectory, which the Makefile finds and
lints; the change log is `design/log.md`, the research record
`research/investigations/` and `research/experiments/`, the maintained TODO
`docs/todo.md`, and the form and readiness checks `make design-lint` and
`make design-ready`. Both run `lint.py` from the `design/skill/` submodule of
[Design-skill](https://github.com/Ming-Research/Design-skill), which firn
never edits.

**Investigations and performance.** An investigation decides something.
Before measuring, write the question, the comparison that could answer it
either way and the result that would reject the proposal; the surviving
decision goes to the tree. Attribute a performance change with a same-source
before-and-after comparison of interleaved runs of `make firn-lto` builds on
the 14900K, with a twin of the base as a noise control, and a falsifier.

## Agents

- The owner and the primary agent own the architecture: the design tree, the
  module graph, the Whitefoot module interfaces (`.wfm`) with their contracts
  and effect rows, and the way firn hosts Halo.
- Subagents are Codex models chosen by difficulty: GPT-6.1 Sol at high
  reasoning effort for simple tasks, GPT-6 Astra at high or max for complex
  ones. Fable is the last resort, used sparingly, only when the primary agent
  and both Codex models have failed at the task.
- An implementer that finds an interface insufficient reports the gap to the
  primary agent with a minimal example instead of editing it.

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

- `make check`, the gate, in CI on every push and on the revision to merge.
  It downloads the pinned compiler (`make compiler`), builds firn and runs
  its network cases (`tests/`), the Redis 7.0.15 suite ratchet
  (`tests/redis-suite/`, without `--tolerant`) and the design lint. The
  ratchet requires every test in `passing.tsv` to pass and reports new
  passes to add in the same PR; `unstable.tsv` records exclusions from
  repeated CI runs. The dispatch-only `redis-suite-record.yml` workflow
  produces both lists for review and commit. The gate needs git, curl,
  Python 3, Rust stable (with Cargo), `tclsh` 8.5 or later, Redis's client
  tools (`redis-cli` and `redis-benchmark`), the submodules
  (`git clone --recurse-submodules` or
  `git submodule update --init`) and the toolchain the compiler links with:
  `/usr/bin/clang`, and on Linux LLD, which CI installs alongside `tcl`
  and `redis-tools`. The Redis suite runner needs Linux's GNU tools.
- `make design-ready` and `make pin-ready`, before marking ready and in CI
  on ready PRs and main: every design-tree change is approved in the log, and
  `whitefoot.pin` names no experiment release.
- Build and test through CI, not on a developer's machine; run a build or
  test locally only when CI cannot do it or the owner asks, and say so.
- Precise timing and performance run on the owner's i9-14900K self-hosted
  machine (runner labels `self-hosted`, `14900k`), never on a hosted runner
  or a laptop. Other projects share it: announce a long run before starting
  it.

## Review

The owner-wide completion review, and any review the owner asks for, starts
its reviewer with the prompt in
[the review checklist](docs/review-checklist.md#how-to-review): GPT-6 Astra
at high effort and every applicable group for a change to code, tests, gate
wiring, a pin, the design tree or guidance; GPT-6.1 Sol at high effort and
groups A, D, M and V, plus R for a material choice, when only research
records or other prose changed. Its scope and the findings fixed go in the
PR's review section.

## Building with Whitefoot

firn builds with the Whitefoot compiler release `whitefoot.pin` names, through
the `whitefoot-kit` submodule
([Whitefoot-kit](https://github.com/Ming-Research/Whitefoot-kit)) that every
project written in Whitefoot shares. Its
[downstream.md](whitefoot-kit/downstream.md) holds the rules: the pin and its
checks, trying an unmerged Whitefoot change with an experiment release or
`make WHITEFOOTC=<path>`, upgrading Whitefoot, and where a Whitefoot gap is
recorded (*Whitefoot requirements* in `docs/todo.md`). A change to those rules
is made in Whitefoot-kit, and firn adopts it by moving the submodule.

For firn, an upgrade whose compiler changed code generation compares firn's
benchmarks before and after on the 14900K
(`research/experiments/redis-bench/redis-bench.sh`).

## Code and tests

- firn's implementation rules are its design decisions in `design/`; read the
  nodes a change touches and their ancestors before changing code.
- Correctness is judged by the reference, independent of firn: Redis
  7.0.15's replies and files, observed or read from its source, or its own
  test suite, never firn's own earlier output.
- A command implements Redis's general behavior; no test, client or
  benchmark selects a special path in the server.
- Never delete, disable, narrow or unwire a test or check merely to make
  `make check` green. A deliberately retired test leaves an honest technical
  explanation in the same change.

## Repository structure and hygiene

The repository root and every established directory are a curated, closed
set. Follow this by judgment and keep moving.

- Do not add a repository-root entry without owner approval. Put new material
  in the existing directory that owns its kind; if none fits, ask.
- Every new file, directory, script or document earns its place before it is
  created: name what it serves, its home and the condition under which it is
  removed.
- No bulk dumps. A script ships wired to a caller; a document ships into an
  existing home and is kept current or deleted.
- Prefer native tooling; a new script must justify why the native path cannot
  do the job.
- Supersede in place: when new material replaces old, update, merge or delete
  the old in the same change.
- Repository artifacts, identifiers, comments, diagnostics, fixtures, test
  names and file names use English.
- Each document keeps its role: `README.md` introduces and navigates,
  `firn/README.md` describes the server's commands and invocation, this file
  holds the goal, authority, process and rules, `design/` the decisions and
  their log, `docs/review-checklist.md` the review items, `docs/todo.md` open
  defects and Whitefoot requirements until resolved, `research/` questions,
  experiments and results, and the PR description the current change. None
  narrates editing history.

## Communication

Describe server and language work with precise, neutral technical wording:
name the concrete rule, failure and expected behavior, and report material
risks accurately.

## Data safety

Preserve unrelated user changes in a dirty worktree. Never discard, overwrite
or rewrite work outside the requested change boundary.
