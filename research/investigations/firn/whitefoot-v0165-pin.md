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
(both deleted before the merge)
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

**Measure.** For each cell (test and depth) the ratio of the requests per
second in each pass, its median over the passes, and the 95% interval of the
mean log ratio over the passes (Student's t), as two invocations of
`summary.py ratios` on the run's `compare.csv` print them: `--base base --head
head` gives head/base, and `--base base --head base-twin` gives base-twin/base,
the same image against itself, whose interval is the cell's noise. The tool's
`--twin` option is not used: it divides the twin by `head`, which moves when
the head really differs from the base.

**Verdict per cell, picked from no threshold.** The interval of head/base
wholly below 1: slower, of any size. Wholly above 1: faster. Across 1: no
detected change. A cell counts only if its base-twin/base interval holds 1;
a cell whose twin interval excludes 1 does not count, and no other cell is
affected. With 46 cells, about 2 twin intervals (46 times 5%, assuming
independent cells; deduced, not measured) exclude 1 by chance in a quiet run,
and about 1 head/base interval (46 times 2.5%) lies wholly below 1 by chance
alone; the run reports how many of each it found, set against those
expectations, and whether the excluded cells cluster in one pass or one
image position, which would point at a disturbed host. A cell whose client busy fraction (`client_busy_mean` in `compare-limited.csv`)
has a median above 0.90 in any image is client-limited: its verdict is
reported and not judged. The cells that do not count and those whose
interval is too wide to say whether the head is slower than the twin's
noise get more pairs in one follow-up run of the same images, those tests at
both depths and 16 passes; a cell that still does not count is reported
unresolved. A cell wholly below 1 is decided, and the follow-up does not
revisit it.

**Noise floor.** No floor is recorded for this class of workload (single-key
commands in `compare` mode on 2 server CPUs). This run measures it: each
cell's twin half-width is the usual noise of that cell on this host, and the
architect is asked to copy the widest of the counting cells per depth to the
14900K record.

**What each result shows.**

- Every cell across 1 or above: the pin move costs no detected throughput on
  these tests.
- A cell wholly below 1: a regression of the interval's size, of any size. It
  is listed in the acceptance with its size, and with the count of such cells
  against the one expected by chance, and goes to its own item or a card; the
  cause is not named without a profile.
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

### First run

Run [40120506904](https://github.com/Ming-Research/Firn-wf/actions/runs/40120506904)
of `redis-bench.yml` on the 14900K (Intel Core i9-14900K, Linux 7.0.0-38, clang
and LLD 22.1.8, `redis-benchmark` 7.2.16), mode `compare`, built from head `b08522a`: `base` is `7cf12df` (pin
`wf-af7665be672d`), `base-twin` is `7cf12df` again, `head` is `b08522a` (pin
`wf-63b7cbb03820`). Servers on CPUs 2 and 4 (P-cores), clients on CPUs 16-31
(E-cores), 8 passes of 5 seconds, no AOF, no profile. The measuring step took 96
minutes. The tables below are `summary.py ratios` on the run's `compare.csv`,
once with `--head head` and once with `--head base-twin`; the last column is the
largest over the three images of the median over passes of the client CPUs'
busy fraction (`client_busy_mean` in `compare-limited.csv`).

| test | depth | head/base median | 95% interval | verdict | base-twin/base median | 95% interval | client busy |
| --- | ---: | ---: | --- | --- | ---: | --- | ---: |
| ping_inline | 16 | 0.9994 | 0.9958-1.0062 | no detected change | 0.9996 | 0.9838-1.0061 | 0.275 |
| ping_mbulk | 16 | 1.0005 | 0.9936-1.0107 | no detected change | 0.9984 | 0.9899-1.0052 | 0.277 |
| set | 16 | 1.0230 | 1.0116-1.0367 | faster | 1.0075 | 0.9972-1.0258 | 0.233 |
| get | 16 | 0.9967 | 0.9847-1.0109 | no detected change | 0.9956 | 0.9878-1.0125 | 0.227 |
| incr | 16 | 0.9924 | 0.9896-1.0003 | no detected change (not counted: twin excludes 1) | 0.9953 | 0.9885-0.9999 | 0.225 |
| lpush | 16 | 0.9823 | 0.9668-0.9903 | slower | 1.0003 | 0.9882-1.0153 | 0.147 |
| rpush | 16 | 1.0080 | 0.9939-1.0285 | no detected change | 1.0084 | 0.9895-1.0269 | 0.146 |
| lpop | 16 | 1.0276 | 1.0229-1.0390 | faster | 1.0067 | 0.9963-1.0163 | 0.145 |
| rpop | 16 | 1.0201 | 1.0125-1.0345 | faster (not counted: twin excludes 1) | 1.0131 | 1.0068-1.0226 | 0.149 |
| sadd | 16 | 0.9835 | 0.9521-1.0125 | no detected change | 1.0017 | 0.9671-1.0332 | 0.150 |
| hset | 16 | 0.9679 | 0.9519-0.9886 | slower | 1.0021 | 0.9865-1.0206 | 0.138 |
| spop | 16 | 1.0013 | 0.9777-1.0343 | no detected change | 1.0027 | 0.9917-1.0091 | 0.168 |
| zadd | 16 | 1.0083 | 0.9989-1.0202 | no detected change | 1.0006 | 0.9907-1.0094 | 0.047 |
| zpopmin | 16 | 1.0058 | 0.9738-1.0291 | no detected change | 0.9868 | 0.9710-1.0177 | 0.150 |
| lrange_100 | 16 | 1.0095 | 0.9997-1.0366 | no detected change | 1.0032 | 0.9815-1.0170 | 0.594 |
| lrange_300 | 16 | 1.0055 | 0.9030-1.0524 | no detected change | 1.0034 | 0.9676-1.0199 | 0.848 |
| lrange_500 | 16 | 1.0008 | 0.9952-1.0115 | no detected change (client-limited, not judged) | 0.9990 | 0.9950-1.0072 | 0.923 |
| lrange_600 | 16 | 1.0050 | 1.0017-1.0124 | faster (client-limited, not judged) | 1.0024 | 0.9989-1.0093 | 0.925 |
| mset | 16 | 1.0195 | 0.9640-1.0707 | no detected change | 1.0027 | 0.9808-1.0473 | 0.100 |
| sismember | 16 | 1.0061 | 0.9737-1.0724 | no detected change | 1.0057 | 0.9665-1.0707 | 0.144 |
| srem | 16 | 1.0006 | 0.9621-1.0798 | no detected change | 1.0067 | 0.9510-1.0670 | 0.162 |
| hget | 16 | 0.9711 | 0.9456-1.0055 | no detected change | 0.9952 | 0.9872-1.0109 | 0.150 |
| hdel | 16 | 1.0187 | 0.9833-1.0456 | no detected change | 1.0078 | 0.9863-1.0334 | 0.166 |
| ping_inline | 1 | 0.9990 | 0.9877-1.0099 | no detected change | 0.9978 | 0.9874-1.0058 | 0.431 |
| ping_mbulk | 1 | 0.9953 | 0.9799-1.0128 | no detected change | 0.9951 | 0.9846-1.0135 | 0.443 |
| set | 1 | 0.9929 | 0.9862-1.0074 | no detected change | 1.0050 | 0.9919-1.0134 | 0.431 |
| get | 1 | 1.0151 | 1.0053-1.0278 | faster | 1.0034 | 0.9898-1.0146 | 0.426 |
| incr | 1 | 1.0059 | 0.9490-1.0410 | no detected change | 1.0063 | 0.9916-1.0312 | 0.422 |
| lpush | 1 | 1.0029 | 0.9686-1.0216 | no detected change | 0.9994 | 0.9772-1.0196 | 0.399 |
| rpush | 1 | 0.9954 | 0.9644-1.0181 | no detected change | 0.9847 | 0.9474-1.0180 | 0.402 |
| lpop | 1 | 0.9962 | 0.9761-1.0035 | no detected change | 1.0022 | 0.9399-1.0249 | 0.406 |
| rpop | 1 | 1.0125 | 0.9898-1.0692 | no detected change | 1.0042 | 0.9853-1.0542 | 0.415 |
| sadd | 1 | 0.9990 | 0.9787-1.0237 | no detected change | 1.0009 | 0.9727-1.0315 | 0.409 |
| hset | 1 | 0.9990 | 0.9814-1.0202 | no detected change | 0.9969 | 0.9778-1.0321 | 0.406 |
| spop | 1 | 0.9978 | 0.9747-1.0124 | no detected change | 0.9921 | 0.9745-1.0105 | 0.412 |
| zadd | 1 | 0.9991 | 0.9810-1.0227 | no detected change | 0.9979 | 0.9823-1.0196 | 0.323 |
| zpopmin | 1 | 0.9993 | 0.9880-1.0112 | no detected change | 0.9918 | 0.9787-1.0023 | 0.408 |
| lrange_100 | 1 | 1.0013 | 0.9963-1.0130 | no detected change | 0.9990 | 0.9899-1.0089 | 0.568 |
| lrange_300 | 1 | 0.9995 | 0.9880-1.0095 | no detected change | 0.9983 | 0.9897-1.0080 | 0.727 |
| lrange_500 | 1 | 0.9986 | 0.9900-1.0161 | no detected change | 0.9938 | 0.9105-1.0408 | 0.826 |
| lrange_600 | 1 | 1.0057 | 1.0014-1.0094 | faster | 1.0018 | 0.9972-1.0072 | 0.884 |
| mset | 1 | 0.9975 | 0.9701-1.0199 | no detected change | 1.0063 | 0.9907-1.0205 | 0.324 |
| sismember | 1 | 0.9996 | 0.9797-1.0186 | no detected change | 1.0067 | 0.9321-1.0408 | 0.399 |
| srem | 1 | 1.0031 | 0.9875-1.0194 | no detected change | 1.0051 | 0.9940-1.0186 | 0.409 |
| hget | 1 | 0.9989 | 0.9949-1.0015 | no detected change | 0.9975 | 0.9762-1.0051 | 0.411 |
| hdel | 1 | 1.0123 | 0.9960-1.0226 | no detected change | 1.0102 | 0.9882-1.0193 | 0.410 |

**Counts against the expectations stated above.** 46 cells; 2 do not count
(`incr` and `rpop` at depth 16: the twin's interval excludes 1, 2 expected) and
2 are client-limited (`lrange_500` and `lrange_600` at depth 16, above 0.90),
which leaves 42 judged cells (about 1 wholly below 1 and about 1 wholly above 1
expected by chance, deduced).

- **Wholly below 1, 2 cells (about 1 expected by chance): `hset` at depth 16,
  0.9679 median, 1.1% to 4.8% slower; `lpush` at depth 16, 0.9823 median, 1.0%
  to 3.3% slower.** Neither cell is slower at depth 1 (`hset` 0.9990, 0.9814 to
  1.0202; `lpush` 1.0029, 0.9686 to 1.0216). Seen against chance alone, 2 where
  1.05 are expected is not unusual (a Poisson count of 1.05 gives 2 or more
  about one time in four; deduced); the criterion above does not weigh that,
  so each cell is a regression of its size.
- Wholly above 1, 4 cells (about 1 expected): `set` at depth 16 (2.3%, 1.2% to
  3.7%), `lpop` at depth 16 (2.8%, 2.3% to 3.9%), `get` at depth 1 (1.5%, 0.5%
  to 2.8%) and `lrange_600` at depth 1 (0.6%, 0.1% to 0.9%). `lrange_600` at
  depth 16 is faster by 0.5% (0.2% to 1.2%) but client-limited, so it is
  reported and not judged; `rpop` at depth 16 (2.0%, 1.3% to 3.5%) does not
  count.
- The other 36 judged cells are across 1.
- **Disturbed host?** The excluded twin intervals do not cluster in one pass or
  one image position. The median over the 46 cells of base-twin over base per
  pass is 0.990 to 1.005 in all 8 passes; `incr` at depth 16 has the twin
  below 1 in 7 of 8 passes (0.982 to 1.006), `rpop` at depth 16 has it above 1
  in all 8 (1.005 to 1.035), in odd and even passes alike, so the direction is
  the same whichever image runs first. `rpop` at depth 16 has `head` above
  `base` in all 8 passes as well, so `base` is the low image in that cell, and
  the cell cannot say anything about the head. One pass of one cell is far from
  the rest: `lrange_300` at depth 16 in pass 8 (`head` 0.779 of `base`, twin
  0.926), which is why that cell's interval is wide.
- **Noise floor of this class** (widest counting twin half-width, as the
  architect is asked to copy; the median is given for context): depth 1,
  `lrange_500`, 6.5% (23 counting cells, median 1.6%, narrowest 0.5%);
  depth 16, `srem`, 5.8% (21 counting cells, median 1.4%, narrowest 0.5%).
  Half-width is half the interval's width as a ratio.

**Follow-up chosen from the first run.** The criteria above do not define "too
wide to say whether the head is slower than the twin's noise", and this reading
was fixed after the tables were read: a judged cell whose head/base interval
has a half-width above the floor of its depth. At depth 1 none does (widest
`incr`, 4.6%); at depth 16 `lrange_300` does (7.5%) and `srem` by a hair (5.9%,
against the floor of 5.8% that `srem`'s own twin sets). The follow-up run
measures the same three images again for `incr`, `rpop`, `lrange_300` and
`srem` at both depths with 16 passes of 5 seconds (4 tests, about 40 minutes).
`hset` and `lpush` at depth 16 are decided and are not in it.

### Follow-up run

Run [40835968034](https://github.com/Ming-Research/Firn-wf/actions/runs/40835968034)
of the same workflow on the same host and placement, with the same three
images (`base` and `base-twin` `7cf12df`, `head` `b08522a`), the four tests
above at both depths, 16 passes of 5 seconds. The measuring step took 32
minutes. The firn sources are those of the first run; only this record and
`docs/safety.md` differ at the commit the run was dispatched from. The columns
are those of the first run's table.

| test | depth | head/base median | 95% interval | verdict | base-twin/base median | 95% interval | client busy |
| --- | ---: | ---: | --- | --- | ---: | --- | ---: |
| incr | 16 | 1.0066 | 0.9994-1.0150 | no detected change | 0.9953 | 0.9765-1.0066 | 0.228 |
| rpop | 16 | 0.9869 | 0.9786-1.0005 | no detected change | 0.9966 | 0.9822-1.0051 | 0.180 |
| lrange_300 | 16 | 1.0073 | 0.9965-1.0081 | no detected change | 1.0016 | 0.9812-1.0049 | 0.847 |
| srem | 16 | 0.9929 | 0.9682-1.0274 | no detected change | 1.0035 | 0.9847-1.0378 | 0.170 |
| incr | 1 | 1.0017 | 0.9934-1.0053 | no detected change | 0.9956 | 0.9774-1.0036 | 0.423 |
| rpop | 1 | 0.9965 | 0.9872-1.0025 | no detected change | 0.9939 | 0.9802-1.0024 | 0.416 |
| lrange_300 | 1 | 0.9975 | 0.9886-1.0021 | no detected change | 0.9996 | 0.9914-1.0038 | 0.728 |
| srem | 1 | 0.9953 | 0.9706-1.0153 | no detected change | 1.0018 | 0.9924-1.0194 | 0.410 |

Every twin interval holds 1, so all 8 cells count, and every head/base interval
is across 1: no detected change, and no cell is left unresolved. The follow-up
replaces the first run's verdict for these 8 cells. Two things the verdict does
not say:

- `rpop` at depth 16 does not repeat its first run: there `head` was 2.0%
  above `base` (and `base` was the low image, so the cell did not count); here
  `head` is 1.3% below `base` (median 0.9869) with the interval's upper end at
  1.0005, across 1 by 0.0005. The rule decides it as no detected change. The
  two runs' medians are 3.3% apart, more than the twin half-width of either
  run (0.8% and 1.2%), so a single run of this cell is not a precise
  measurement of it.
- Pass 8 of this run was disturbed: the median over these 8 cells of
  base-twin over base is 0.943 in that pass and 0.987 to 1.010 in the other
  15. Pass 8 of the first run was the lowest too (0.990 against 0.998 to
  1.005 in the other 7), with the one outlier cell `lrange_300` at depth 16.
  The criteria have no rule for dropping a pass, so none is dropped; the
  intervals are wider by it and every twin interval holds 1 with it in.

### Verdict

44 judged cells (46 less the 2 client-limited), none unresolved.

- **Regressions (interval wholly on the slower side), 2 cells: `hset` at
  depth 16, median 3.2% slower, interval 1.1% to 4.8% slower; `lpush` at depth
  16, median 1.8% slower, interval 1.0% to 3.3% slower.** About 1.1 cells
  (44 times 2.5%) are expected below 1 by chance, and two or more would occur
  about three times in ten (Poisson, deduced), so the run does not separate
  these two from chance, and the criterion above does not weigh that. Neither
  cell is slower at depth 1. The cause is not named: `hset` is a `HashMap`
  command and Whitefoot#487 changed that layout, but `lpush` is not, and no
  profile was taken. The pair is handed to the item `firn-pin-hset-lpush-slow`;
  merging this pull request with the two regressions is the owner's ruling.
- **Improvements, 4 cells (about 1.1 expected):** `set` at depth 16 (2.3%
  faster, 1.2% to 3.7%), `lpop` at depth 16 (2.8%, 2.3% to 3.9%), `get` at
  depth 1 (1.5%, 0.5% to 2.8%), `lrange_600` at depth 1 (0.6%, 0.1% to 0.9%).
- **Not judged, 2 cells (client-limited):** `lrange_500` and `lrange_600` at
  depth 16 (client busy 0.923 and 0.925). Both are across 1 or above it, so
  neither shows firn behind.
- **Across 1, 38 cells**, among them the 8 of the follow-up.
- **Noise floors for the 14900K record**, from the first run's counting twin
  cells (8 passes): depth 1, 6.5% (`lrange_500`); depth 16, 5.8% (`srem`). Each
  is set by one cell with an outlier pass. Among the four tests measured again
  with 16 passes the widest twin half-width is 1.4% at depth 1 and 2.7% at
  depth 16 (`srem`).
- The comparison covers one 14900K, one pair of server cores and 2 server CPUs;
  it cannot say which of the compiler, the Halo sources or the edits to firn's
  own source moved a cell.
