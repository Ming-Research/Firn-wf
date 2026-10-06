# Firn-wf — agent instructions

Firn-wf holds firn, a server of Redis's protocol written in Whitefoot. It
answers Redis's clients over TCP for strings, lists, sets, hashes and sorted
sets in one shared keyspace, with expiry and an append-only file, and is
growing toward Redis scripting through Halo, the Lua engine of
[Halo-wf](https://github.com/Ming-Research/Halo-wf). Whitefoot, the language
and its compiler, is pinned as a compiler release named in `whitefoot.pin`,
and the `design-tree` skill as the `design/skill/` submodule.

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
test suite (`research/experiments/redis-compat/`). Its source is the oracle
for what a command does where the documentation is silent.

**The language.** The pinned Whitefoot commit defines the language. Releases
carry only the compiler, so read the language in the Whitefoot repository at
that commit: the specification `spec/kernel-spec.md`, which is normative, the
maintained programs under `tests/programs/`, `docs/patterns.md` and the
standard library's interfaces `lib/std/**/module.wfm`. The commit is the
`commit` field of the release manifest that `make compiler` keeps as
`build/whitefoot/<release>/whitefoot-release.json`. Read a file with, for
example,

```sh
gh api 'repos/Ming-Research/Whitefoot/contents/spec/kernel-spec.md?ref=<commit>' \
  -H 'Accept: application/vnd.github.raw'
```

or `git show <commit>:<path>` in a local Whitefoot clone. The compiler's
diagnostics and repairs are the other source. firn never vendors Whitefoot
source; see [The Whitefoot boundary](#the-whitefoot-boundary).

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
   ancestors, verify the worktree and PR state on resumption, and settle the
   direction with the owner as the `design-tree` skill describes.
2. **While working,** state why each material choice fits its evidence and
   record an experiment's criterion before using it to choose. Change the
   code and the design tree together on a Draft PR, and update what a changed
   conclusion affects in the same work.
3. **At completion,** run the [checks](#checks) and the [review](#review),
   then hand the work back as the skill describes, adding the validation run
   and its revision, what remains unverified, any pin moved or Whitefoot gap
   filed, and what the work found along the way.
4. **After the owner approves** every decision, write the log entry and mark
   the PR ready (rule 1 below).

**Judge a design by its merits, not by the work it takes.** No design
judgment weighs the existing code, tests or documents a choice would change,
nor the effort of changing them.

**Fix or record what you notice.** When work exposes a defect elsewhere, such
as a bug, an awkward interface, duplicated logic or a stale document, fix it
in the same change if it is small and within the files you are changing;
otherwise add an item to `docs/todo.md` with its impact, the change you would
make and when to reopen it. List each in the PR's *Found along the way*
section with its disposition.

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

Use a PR as the owner's review surface from the start, as a Draft until rule 1
below lets it become ready. Push coherent progress to the same branch and keep
its description and actual validation results current. A series of dependent
PRs is stacked, each on the branch of the one before it. Updating a
work-branch PR never authorizes a merge into `main`.

**The design tree.** The `design-tree` skill (`design/skill/`, a submodule of
[Design-skill](https://github.com/Ming-Research/Design-skill) that firn never
edits, linked from `.claude/skills/` and `.agents/skills/`) is the one
recurring procedure; a change to it is made in Design-skill. Its live trees
are the root node files under `design/` other than `log.md`, each with its
subdirectory, which the Makefile finds and lints; its log is `design/log.md`,
its research record `research/investigations/` and `research/experiments/`,
its TODO `docs/todo.md`, and its checks `make design-lint` and
`make design-ready`.

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
   built from a commit on Whitefoot's `main` and submodule commits on their
   repositories' `main`.

**Exact revision** is the complete tree that will enter `main`, the pins
included; if it changes after approval or after its successful check, rules
2 and 3 apply to the new revision. No other workflow step is an approval or
merge precondition.

## Checks

- `make check`, the gate, in CI on every push and on the revision to merge.
  It downloads the pinned compiler (`make compiler`), builds firn and runs
  every firn check and the design lint. It needs git, curl, Python 3, the
  submodules (`git clone --recurse-submodules` or
  `git submodule update --init`) and the toolchain the compiler links with:
  `/usr/bin/clang`, and on Linux LLD, which CI installs.
- `make design-ready`, before marking ready and in CI on ready PRs and main:
  every design-tree change is approved in the log.
- Build and test through CI, not on a developer's machine; run a build or
  test locally only when CI cannot do it or the owner asks, and say so.
- Precise timing and performance run on the owner's i9-14900K self-hosted
  machine (runner labels `self-hosted`, `14900k`), never on a hosted runner
  or a laptop. Other projects share it: announce a long run before starting
  it.

## Review

One review per task, when the work is complete and before the handoff, and
whenever the owner asks for one. Start a separate, read-only agent that did
not implement the change, with the prompt in
[the review checklist](docs/review-checklist.md#how-to-review): GPT-6 Astra
at high effort and every applicable group for a change to code, tests, gate
wiring, a pin, the design tree or guidance; GPT-6.1 Sol at high effort and
groups A, D, M and V, plus R for a material choice, when only research
records or other prose changed. Fix every finding and review again as the
`design-tree` skill's workflow describes, push, verify that the remote head
is the reviewed revision, and fill the PR's review section.

## Upgrading Whitefoot

The owner periodically has an agent move every downstream project to the
latest Whitefoot. For firn:

1. Take the Whitefoot `main` commit to adopt; its gate must have passed.
   If Whitefoot has no release `wf-<12-character hash>` for it, or the
   release was deleted (releases older than 30 days are deleted, except the
   newest), dispatch Whitefoot's compiler release workflow for that commit:
   `gh workflow run compiler-release.yml -R Ming-Research/Whitefoot -f commit=<hash>`.
2. On a work branch, set `whitefoot.pin` to `release = wf-<hash>`.
3. Read what changed between the old and new pinned commits that can affect
   firn: `spec/log.md` and the specification, the standard library's
   interfaces, and the compiler's diagnostics.
4. Adapt firn to the new language and compiler, without weakening any check.
5. Run CI (`make check`); when the compiler's code generation changed,
   compare firn's benchmarks before and after on the 14900K.
6. Open the PR naming both commits, both specification versions and every
   change firn needed; it merges under rules 2 to 4.

A pin whose release is gone gets the same commit dispatched again.

## The Whitefoot boundary

- firn builds with exactly the pinned release, and moving the pin is a
  deliberate change under rule 4. firn never vendors Whitefoot source or
  pins Whitefoot as a submodule.
- A change firn needs in Whitefoot is made in Whitefoot, under Whitefoot's
  own AGENTS.md, as a branch and PR in its repository. A main release exists
  only for a commit on Whitefoot's `main`, so firn's `main` adopts the change
  after it merges there; an experiment branch may pin an experiment release
  (`wf-exp-<hash>`) of the unmerged commit to try it first.
- When a missing Whitefoot feature would bend firn's implementation or
  architecture, add the feature to Whitefoot instead of working around it.
  State the gap as its minimal semantic example, apart from the server code
  that exposed it, and record it under *Whitefoot requirements* in
  `docs/todo.md` until Whitefoot resolves it. A problem that belongs to the
  server alone is fixed in firn, not by generalizing the language.

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
