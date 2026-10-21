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

- The reference is Redis 7.2.16 on x86-64 Linux: its replies, the
  append-only files it writes and reads, its own test suite
  (`tests/redis-suite/`), and its source where the documentation is silent.
  Its `redis-server`, `redis-cli` and `redis-benchmark` are built from the
  release archive, whose SHA-256 `tests/redis-suite/reference.sh` pins, and
  every reference run uses that build; Ubuntu's package is 7.0.15.
- The language is the pinned Whitefoot commit: read its specification,
  maintained programs and standard-library interfaces at that commit as
  [whitefoot-kit/downstream.md](whitefoot-kit/downstream.md#reading-the-language)
  describes; the compiler's diagnostics and repairs are the other source.

## Paths

- Research record: `research/investigations/` (`firn/` holds the server's
  design, rulings, criteria and measurements) and `research/experiments/`
  (the instruments and their results). Records from before firn moved here
  cite Whitefoot's paths of their time, such as `apps/firn/`.
- Review checklist: [docs/review-checklist.md](docs/review-checklist.md).
- Performance comparisons build firn with `make firn-lto` and run
  `research/experiments/redis-bench/redis-bench.sh` on the 14900K through the
  `redis-bench.yml` workflow; an upgrade of Whitefoot uses them for the
  comparison
  [downstream.md](whitefoot-kit/downstream.md#upgrading-whitefoot) asks for.
  The servers run on P-cores and the load clients on all 16 E-cores
  (CPUs 16-31), for every competitor alike; a cell whose clients average over
  90% busy is marked client-limited.

## Checks

- `make check`, the gate, runs in CI on every push, its targets in four
  parallel jobs (`check.yml`) so that the answer takes as long as the
  slowest: it downloads the pinned compiler (`make compiler`), builds firn
  and runs the Redis 7.2.16 suite ratchet (`tests/redis-suite/`, without
  `--tolerant`); builds firn and runs its network cases (`tests/`); runs
  the design lint; and compiles the paired safety witnesses
  (`research/experiments/safety-witnesses/`). Each faulty program must receive
  its specified source diagnostic and each correction must build with the pin.
  `make gate-sync` fails when the targets `check.yml` runs
  and those of `make check` differ, so a target added to one is added to the
  other. The suite runs as two shards side by side
  (`tests/redis-suite/run-shards.sh`): `ratchet.py assign` splits its units
  by the times in `unit-seconds.tsv`, which puts `unit/type/list` alone in
  the first shard, and `ratchet.py merge` fails unless the shards together
  ran every unit of the suite exactly once. The ratchet requires every test
  in `passing.tsv` to pass and reports new passes to add in the same PR;
  `unstable.tsv` records exclusions from repeated CI runs, and the
  dispatch-only `redis-suite-record.yml` workflow produces both lists for
  review and commit, running the suite in the gate's two shards so that its
  evidence holds under the gate's load.
- The gate needs git, curl, GNU `patch`, Python 3, Rust stable with Cargo,
  `tclsh` 8.5 or
  later, a C compiler and `make` (the suite builds the Redis 7.2.16 tools it
  needs from source and caches them), the submodules
  (`git submodule update --init`), `/usr/bin/clang` and, on Linux, LLD of
  the LLVM major the pinned release names (`make toolchain` installs it,
  `make toolchain-check` checks it); the Redis suite runner needs Linux's
  GNU tools.
- The `aof-sync-fault` job is part of the check workflow
  (`.github/workflows/check.yml`) and runs beside `make check` on every push.
  A merge needs it green on the exact revision, like the gate. It is not part
  of `make check`, because it needs a device only root can make. The job
  makes the device with `sudo tests/aof_sync_fault_device.sh setup MOUNT
  UID:GID`, a device-mapper device whose writes and flushes fail on command
  and which turns the kernel's periodic writeback off, so that only the
  server's fsync and the harness's umount write during a failing window,
  takes the redis-server 7.2.16 that `tests/redis-suite/reference.sh` builds
  as the oracle, runs `check-writeback` (a file made before a 40 s failure
  must survive it), runs `make aof-sync-fault` and
  removes the device with `teardown MOUNT` even when a step failed. The
  target runs the eight cases of `tests/aof_sync_fault.rs` that it selects,
  ignored in `make check`, against firn and the oracle, one after the other,
  each on a filesystem made anew by `reset` (`remount` fails the run when
  `e2fsck -fn` finds the metadata damaged),
  and fails unless `FIRN_SYNC_FAULT_MOUNT` names that mount, `REDIS_SERVER`
  names the oracle and all eight cases passed. It needs Linux with passwordless
  sudo, `dmsetup` and the dm-flakey module; `setup` may install
  `linux-modules-extra-$(uname -r)` through `apt-get` on Ubuntu to get it.
- `make pin-ready`, with the readiness check: `whitefoot.pin` names no
  experiment release.

## Whitefoot

firn builds with the compiler release `whitefoot.pin` names, through the
`whitefoot-kit/` submodule, whose
[downstream.md](whitefoot-kit/downstream.md) holds the rules for the pin,
trying an unmerged Whitefoot change, upgrading Whitefoot and recording a
Whitefoot gap; a change to them is made in
[Whitefoot-kit](https://github.com/Ming-Research/Whitefoot-kit).

Before writing Whitefoot code, read the kit's
[README.md](whitefoot-kit/README.md) and its writing guide
[docs/writing.md](whitefoot-kit/docs/writing.md).

## Reports

A completion report also names any `whitefoot.pin` or submodule moved and any
Whitefoot gap filed.

## Documents

`README.md` introduces and navigates, `firn/README.md` describes the server's
commands and invocation, `docs/review-checklist.md` holds the review items,
and `research/` holds questions, experiments and results.
