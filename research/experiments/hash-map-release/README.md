# hash-map-release

A program of the library `HashMap<Key, Bytes>` alone, built by two Whitefoot
releases and timed one against the other in one job. Item #145 uses it to ask
whether a release change slows the map's put on a present field (HSET) and its
lookup (HGET) with no firn server around them; the question, the criterion fixed
before the runs and the results are in
[`research/investigations/firn/hset-lpush-after-v0165.md`](../../investigations/firn/hset-lpush-after-v0165.md).

- `modules.wfg`, `repro/`: the program. `pkg::bytes` is not here: `run.py` copies
  `firn/bytes/` into the build directory, so `Key`, `Bytes` and `KeyHash` are
  firn's own. Three entries: `fill` builds a map of 100,000 fields (`element:` and
  twelve digits, three-byte values) by the map's puts; `replace` and `lookup` build
  it and then put on, or look up, a random present field `operations` times
  (`const operations` in `repro/repro.wf`, replaced by `--operations`).
- `run.py`: builds each entry with each compiler (`--full-lto`), runs them
  interleaved on one CPU under `perf stat` (user cycles and instructions), and
  prints per operation the ratio of the new build to the old with its 95%
  interval, the same for the old build run from a second copy (the twin, the
  measure of noise), and a reading. `python3 run.py selftest` checks the interval
  and the reading.

- `probes.py`: the probes of a present-field lookup under the old and the new home
  function for the test's keys, modelled; no compiler is needed (`python3 -B -I probes.py`).

To run the timing, with a compiler and language package of each release (`make compiler
PIN=FILE` downloads a release that FILE names into `build/whitefoot/<release>/`):

```sh
python3 -B research/experiments/hash-map-release/run.py --root . --out build/hash-map-release \
  --old wf-AAAAAAAAAAAA=build/whitefoot/wf-AAAAAAAAAAAA/whitefootc,build/whitefoot/wf-AAAAAAAAAAAA/language \
  --new wf-BBBBBBBBBBBB=build/whitefoot/wf-BBBBBBBBBBBB/whitefootc,build/whitefoot/wf-BBBBBBBBBBBB/language \
  --operations 3000000 --reps 40
```

It needs Linux, `taskset` and `perf` that can count user cycles; without perf it
reports wall time only. Run it on a quiet machine: the reading rule declares an
operation unresolved when the twin's interval is wider than ±1.5%.
