# Moving firn to Whitefoot wf-63b7cbb03820

Board item: `firn-pin-wf-main` (Firn-wf#138). firn moves from the Whitefoot
release `wf-af7665be672d` (specification v0.146) to `wf-63b7cbb03820`
(specification v0.165), the newest release of Whitefoot's `main` when the item
started. This record states what moved, what the new compiler required of the
source, and the 14900K comparison of the two builds with the criteria written
before the run.

## What moved

- `whitefoot.pin`: `wf-af7665be672d` to `wf-63b7cbb03820`. The release holds
  Whitefoot#486 (window `clear` keeps storage and capacity), #510 (a shared
  object holds linear states, `shared_close`), #517 (test contexts take turns
  with a parked file) and #487 (a resumable bounded `hash_map_scan`). It does
  not hold #460 (guards written as helper calls wake by condition): that pull
  request merged after the release's commit, and no release holds it yet.
- `deps/halo-wf`: `56878ac` to `1cc1b08`, Halo-wf's `main` after Halo-wf#71,
  which renames the names the new prelude and grammar took and deletes the
  conditions the checker decides. Halo's own `whitefoot.pin` stays at
  `wf-af7665be672d`; firn compiles Halo's sources with its own compiler.
- `whitefoot-kit`: `56fd13b` to `92a9806`, the newest commit of the kit's
  `main`.
- Both releases are built against LLVM 22 (`linux_llvm_major` in their
  manifests), so no host toolchain moves.

## What the source needed

The edits change no behavior of firn. The compiler was run only in CI: a
temporary workflow checked each module of the graph, and a discovery script
applied the compiler's own repairs round by round in a disposable workspace
(the compiler reports one redundant condition per function body, so the
repair needs several rounds) and wrote them as a patch, which was read hunk
by hunk before it was applied.

- **`held` is a reserved word** (FORM-3, specification v0.147). 376 uses of
  the five new names in 30 files (the list-ceiling fixture and two doc
  strings included) replace it by role: `held_bytes` (filled bytes of a read
  buffer), `held_state`, `held_engine`, `held_call` and `held_value`. None of
  the new names existed before.
- **A redundant condition is a hard error** (OP-5, v0.152). The checker now
  refuses a condition it can decide from the facts at hand, such as an index
  guard after a loop bound that already proves the index in range. 80 such
  guards in 27 files of `firn/` and 17 in `tests/list_ceiling.wf` (the
  discovery script's tally) are deleted; each is a branch that could not run.
  Two follow-on errors needed hand edits: a `return` made unreachable by a
  deleted guard (`eviction_pool_take`, `rightmost`) and two `glyph`
  declarations that collided once the guard between them was gone
  (`aof_hex`, now `high_glyph` and `low_glyph`).
- **`Inputs` destructuring names every field** (GRAM-10). The two fields the
  host API added, `host_paths` and `process_id`, are bound and unused in
  `server.wf`.
- **The list-ceiling fixture** (`tests/list_ceiling.py`) builds a copy of
  firn's graph with the ceiling constant replaced textually by 8. With that
  constant one growth guard in the copy of `lists.wf` is decided by the
  checker (OP-5), so the assembler removes that exact block from the copy
  and fails if it does not find it exactly once. Production `lists.wf` keeps
  the guard. Whitefoot has no source overlay, so the fixture cannot carry the
  constant any other way; this is recorded for the architect.
- **Halo-wf** needed no edit in firn: its sources, at the commit named
  above, already follow the new rules.
- **`redis-bench.sh`** gained the `sismember`, `srem`, `hget` and `hdel` cases
  the comparison below uses.

The gate was green on the pushed head before this record's last edits: the
Redis 7.2.16 suite ratchet, firn's network tests, the AOF sync-fault job, the
paired safety witnesses, the design lint and the release workflow.

**Statements the pin move made false or left unchecked.**

- `design/firn/startup.md` and `firn/README.md` said the pinned Whitefoot has
  no resource-limit facility. The new `std::process` has `open_files_limit`
  and `raise_open_files_limit`, so both now name the release they held for,
  and the node records that its reopening condition is met. Whether firn
  raises `RLIMIT_NOFILE` at startup as Redis does is a behavior change,
  left to firn-ops-pin-followup, not part of this item.
- `docs/safety.md` and the safety-witnesses README cite specification
  passages, source lines and a C-source census read at `wf-af7665be672d`.
  Many line anchors differ at the new commit. Both documents now say they
  were audited at the older release and not read again; the re-audit is a
  separate piece of work.
- `design/firn/command-parts.md` rests on the old compiler's behavior of
  acquiring a named target at its first use on a path. The comparison below
  covers `SET` and the other commands but does not observe that behavior.
- Whitefoot#460 is in no release yet, so `firn-group-commit`'s measurement
  cannot use it with this pin.

## Performance comparison, stated before measuring

**Question.** Does the firn built with `wf-63b7cbb03820` serve any of
`redis-benchmark`'s default tests, or the set and hash reads and removals
below, slower than the firn built with `wf-af7665be672d`?
The two builds differ by the pin move and the source edits it required; the
comparison does not separate the compiler from those edits.

**Sides.** Three images, interleaved by `redis-bench.sh compare` through the
`redis-bench.yml` workflow (`mode compare`, `make firn-lto` for each):
`base` is `origin/main` before this pull request (pin `wf-af7665be672d`),
`base-twin` is the same image measured again, and `head` is the pull
request's head. The twin differs from the base by this run's noise alone.

**Workload.** 23 tests at pipeline depths 16 and 1: the 19 default tests of
`redis-benchmark` (`ping_inline ping_mbulk set get incr lpush rpush lpop rpop
sadd hset spop zadd zpopmin lrange_100 lrange_300 lrange_500 lrange_600
mset`; the `LRANGE` list fill is the 20th row of its default output and is
done outside the timed span) and `sismember srem hget hdel`, which
`redis-bench.sh` fills outside the timed span with 100,000 members or fields
(`SADD`, `SISMEMBER`, `SREM`, `HSET`, `HGET` and `HDEL` are the commands that
exercise the standard library's `HashMap`, whose layout Whitefoot#487 changed). `SREM` and `HDEL` drain their set or hash within the
first fraction of a run, so most of their requests find nothing to remove;
the same holds for every image. `xadd` is left out as in
[Pub/sub throughput](pubsub-throughput.md#criteria-stated-before-measuring):
its stream grows without bound over a run. 2 server CPUs on the P-cores 2
and 4, the clients on the E-cores 16-31 (AGENTS.md), 8 interleaved passes of
5 seconds each with the image order reversed on even passes, no `reference`
line, no profile.

**Measure.** For each cell (test and depth) the ratio head/base of the
requests per second in each pass, and its median over the passes; the 95%
interval of the mean log ratio over the passes (Student's t), and the same
for base-twin/base, as `summary.py ratios` prints them.

**Verdict per cell, picked from no threshold.** The interval of head/base
wholly below 1: slower, of any size. Wholly above 1: faster. Across 1: no
detected change. A cell counts only if the twin's interval holds 1. A cell
whose client work fraction (`compare-limited.csv`) has a median above 0.90 in
any image is client-limited: its verdict is reported and not judged. A cell
that does not count, or whose interval is too wide to say whether the
head is slower than the twin's noise, is repeated with more passes in one
follow-up run, or reported unresolved.

**Noise floor.** No floor is recorded for this class of workload (single-key
commands in `compare` mode on 2 server CPUs). This run measures it: each
cell's twin half-width is the usual noise of that cell on this host, and the
architect is asked to copy the widest per depth to the 14900K record. The
run counts only if every cell's twin interval holds 1.

**What each result shows.**

- Every cell across 1 or above: the pin move costs no detected throughput on
  these tests.
- A cell wholly below 1: a regression of the interval's size. It is listed in
  the acceptance with its size and goes to its own item or a card; the cause
  is not named without a profile.
- A cell wholly above 1: an improvement, reported with its size.
- The comparison cannot say which of the compiler, the Halo sources or the
  edits to firn's own source moved a cell; a regression that needs that goes
  to its own item.

**Cost.** 23 tests, 2 depths, 3 images, 8 passes of 5 seconds are 92 minutes
of timed samples. A functional probe on a GitHub-hosted runner (run
40098317675: both images, all 23 tests at both depths, 2 passes of 2 seconds)
measured every cell of both images and took 19 minutes in the measuring step
for 6.1 minutes of timed samples; its pilots (the runs that size each cell's
request count) and per-run fills are the rest. Scaled to the real run that
puts the measuring step at about 100 to 125 minutes, above the 110-minute
limit the workflow had on that step, so this pull request raises the step's
limit to 150 minutes and the job's to 180. The probe's first dispatch (run
40097124162) passed without measuring anything: with 1 server CPU the
hosted runner's 4 vCPUs left no client core, and the script skipped the
count; the second dispatch names the CPUs.

## Results

Not yet measured.
