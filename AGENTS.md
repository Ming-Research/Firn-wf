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

When priorities conflict, first:

1. reach the next end-to-end compatibility, workload or performance
   experiment;
2. keep replies and persisted state identical to the reference, with every
   safety check Whitefoot requires.

## References

- The reference is Redis 7.0.15 on x86-64 Linux: its replies, the
  append-only files it writes and reads, its own test suite
  (`tests/redis-suite/`), and its source where the documentation is silent.
- The language is the pinned Whitefoot commit: read its specification,
  maintained programs and standard-library interfaces at that commit as
  [whitefoot-kit/downstream.md](whitefoot-kit/downstream.md#reading-the-language)
  describes; the compiler's diagnostics and repairs are the other source.

## Paths

- Research record: `research/investigations/` (`firn/` holds the server's
  design, rulings, criteria and measurements) and `research/experiments/`
  (the instruments and their results). Records from before firn moved here
  cite Whitefoot's paths of their time, such as `apps/firn/`.
- Maintained TODO: the [shared status board](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip),
  the only TODO; repositories keep no TODO file. firn's backlog is in
  `firn-server` and `firn-ops`; Whitefoot gaps are in "firn 需要的 Whitefoot 工作"
  (`firn-wf`) and Whitefoot's own areas. Code, documents and checks that
  refer to unfinished work cite its board item key.
- Review checklist: [docs/review-checklist.md](docs/review-checklist.md).
- Performance comparisons build firn with `make firn-lto` and run
  `research/experiments/redis-bench/redis-bench.sh` on the 14900K through the
  `redis-bench.yml` workflow; an upgrade of Whitefoot uses them for the
  comparison
  [downstream.md](whitefoot-kit/downstream.md#upgrading-whitefoot) asks for.

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
  (`git submodule update --init`), `/usr/bin/clang` and, on Linux, LLD of
  the LLVM major the pinned release names (`make toolchain` installs it,
  `make toolchain-check` checks it); the Redis suite runner needs Linux's
  GNU tools.
- `make pin-ready`, with the readiness check: `whitefoot.pin` names no
  experiment release.

## Whitefoot

firn builds with the compiler release `whitefoot.pin` names, through the
`whitefoot-kit/` submodule, whose
[downstream.md](whitefoot-kit/downstream.md) holds the rules for the pin,
trying an unmerged Whitefoot change, upgrading Whitefoot and recording a
Whitefoot gap; a change to them is made in
[Whitefoot-kit](https://github.com/Ming-Research/Whitefoot-kit).

## Reports

A completion report also names any `whitefoot.pin` or submodule moved and any
Whitefoot gap filed.

## Documents

`README.md` introduces and navigates, `firn/README.md` describes the server's
commands and invocation, `docs/review-checklist.md` holds the review items,
the [shared status board](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip)
tracks open defects and Whitefoot requirements in the areas named above, and
`research/` holds questions, experiments and results.
