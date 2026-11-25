# HSET and LPUSH after the move to wf-63b7cbb03820

Item #145. The move of firn from the Whitefoot release `wf-af7665be672d` to
`wf-63b7cbb03820` (#138, [record](whitefoot-v0165-pin.md)) left two cells
slower on the 14900K: `hset` and `lpush` at pipeline depth 16, 2.5% to 4.0%
and 1.7% to 2.7% slower over 24 passes, with 4.1% and 2.6% more server CPU
per request ([confirmation run](whitefoot-v0165-pin.md#confirmation-run)).
This record finds which of the compiler with its standard library, the Halo
sources or firn's own source edits causes each, and holds the evidence.

The old source does not build with the new compiler, but the new source
builds with the old one once the two `Inputs` fields the old release lacks are
left out of `main`'s destructuring: revision `1dddd17` (firn and Halo as on
this branch, pin `wf-af7665be672d`) built with `whitefootc wf-af7665be672d`
on the 14900K on 2026-11-25 (fast workflow run
[45610502247](https://github.com/Ming-Research/Firn-wf/actions/runs/45610502247),
the `firn` target). Its image, the **cross** image below, differs from the
head image only by the compiler with its standard library and runtime, and
from the base image only by the source edits of #138 (firn's and Halo's), so
measuring it beside both splits the regression between the two.

## What the sources of the two builds say

Read on 2026-11-25 in `git diff 7cf12df b08522a` (firn and its Halo commit)
and in Whitefoot between the two release commits `af7665be672d` and
`63b7cbb03820`.

- **Both tests write one key.** `redis-benchmark` 7.2.16 sends
  `HSET myhash element:__rand_int__ xxx` and `LPUSH mylist xxx`
  (`src/redis-benchmark.c`), so with two server CPUs both drivers take turns
  on that one key's atomic statement, and a longer statement shows as lower
  throughput and as more time spent waiting to acquire it. With `-r 100000`
  the hash holds at most 100,000 fields of 20 bytes, so after the first
  fraction of a run nearly every `HSET` replaces a field; the list grows by
  every request.
- **firn's edits leave both paths as they were.** Of the 42 files of `firn/`
  the move changed, the functions `HSET` and `LPUSH` run (`run_hset`,
  `hset_body`, `set_fields`, `put_fields`, `run_push`, `push_body`,
  `push_items`, `push_all`, `push_element`, `list_room`, `prepare_elements`,
  `push_item`, `clear_items`) differ only by the rename of `held`. The
  conditions deleted near them are in functions these commands do not run
  (`random_field`, `insert_item`, `lpos_finish`) or run only with the
  append-only file or a script's access journal, both off in the
  measurement (`log_reserve`, `record_access`).
- **Halo is not on either path.** firn's command code calls Halo only in
  `eviction_limit` (`halo::number::pow`), which runs under a memory limit, and
  the measurement sets none; Halo-wf#71 itself renames and deletes decided
  conditions. Halo's code is linked into the same image, so its size can still
  move where the hot code lies.
- **The list's front operation is unchanged in the library and its emitter.**
  `std::collections::deque` is the same file at both commits, and the emitter
  functions that write `place_front` and `place_back`
  (`boundary_slot` and `move_run_boundary` in
  `compiler/src/backend/emitter/runs.rs`) are the same at both. `LPUSH` and
  `RPUSH` run the same firn code but for `deque_push_front` against
  `deque_push_back`.
- **The hash map changed (Whitefoot#487).** A key's home bucket was
  `hash % count`; it is now the top bits of `hash * 11400714819323198485`,
  over a bucket count that is zero or a power of two of at least two
  (`hash_map_home`, with a bounds branch the checker requires), and the map
  gained a `shift` word. Every probe loop (`hash_map_find`,
  `hash_map_lookup`, `hash_map_edit`, `hash_map_try_put`, the rebuild)
  computes the home this way. A hash of 100,000 fields has 262,144 buckets in
  both builds: the old rule grows a map made for 4 pairs from 4 buckets and the
  new one from 8, each doubling once three quarters are used. firn's keys
  carry an FNV-1a hash (`window_hash`), so the old home took its low 18 bits
  and the new home takes the top 18 bits of its product.
- **The keyspace is not this map.** `store^.keys` is the compiler's shared
  concurrent map (`ConcurrentHashMap<Entry>`), the same for every command; the
  standard library's map holds a hash's fields and a set's members.

**The new home does not lengthen the probes of these fields.** The 100,000
fields `element:000000000000` to `element:000000099999` that `-r 100000`
draws from, hashed with FNV-1a as `window_hash` does, placed by linear
probing in 262,144 buckets (a successful search's mean cost does not depend on
the order of insertion), computed on 2026-11-25:

| home | distinct homes | mean buckets per hit | 99th percentile | longest |
| --- | ---: | ---: | ---: | ---: |
| `hash % 262144` (old) | 81,376 | 1.351 | 5 | 26 |
| top 18 bits of `hash * 11400714819323198485` (new) | 83,591 | 1.288 | 5 | 15 |
| a uniformly random home (seed 1), for reference | 83,246 | 1.308 | 5 | 17 |

So under H1 below the extra cost would come from the code of the map's
operations, not from longer walks. The computation, in Python:

```python
M = (1 << 64) - 1
def fnv1a(data):
    h = 14695981039346656037
    for b in data:
        h = ((h ^ b) * 1099511628211) & M
    return h
B = 262144
hashes = [fnv1a(b"element:%012d" % i) for i in range(100000)]
old = lambda h: h % B
new = lambda h: ((h * 11400714819323198485) & M) >> 46
def mean_probes(home_of):
    table, homes = [None] * B, []
    for i, h in enumerate(hashes):
        j = home = home_of(h)
        homes.append(home)
        while table[j] is not None:
            j = (j + 1) % B
        table[j] = i
    where = {i: j for j, i in enumerate(table) if i is not None}
    return sum((where[i] - homes[i]) % B + 1 for i in range(len(hashes))) / len(hashes)
```

**The map's change alone made no operation slower in Whitefoot's own
measurement.** Whitefoot#487's performance acceptance compared the map before
and after it under one compiler, the candidate's, on the 14900K
(`hash-map-bench.yml`, run 39716174131, recorded in Whitefoot's
`research/investigations/hash-scan/DESIGN.md`, "Third comparison"): of the 21
time cells 19 faster, one unchanged, one unresolved and none slower, lookup of
a present key at 1,000,000 pairs 0.994 [0.983, 0.9995]. Its keys and values are `u64` and it has no cell
that replaces a present key's value, which is what nearly every `HSET` here
does, so it does not cover firn's `HashMap<Key, Bytes>` with byte keys; it
does say that the library's new probe code, run by one compiler, is not
slower than the old.

**What else the compiler and runtime changed between the two releases** (80
merges on Whitefoot's `main`, `git log --first-parent af7665be672d..63b7cbb03820`),
read for what can reach these two paths, none of it measured apart:

- every generated function carries `uwtable` (Whitefoot#461), so the base image
  has no unwind tables and a DWARF-caller profile of it does not walk through
  Whitefoot frames; callers can be compared only between the cross and head
  images;
- enum variant payloads are read and written through body place suffixes
  (Whitefoot#467), which reaches `HashMapSlot`, `Bytes` and `Key`;
- a statement holding map entries that misses a shared object queues a FIFO
  claim (Whitefoot#388, `2c92dc5fa`), a change to the runtime's contended take;
- heap blocks are taken at a recorded 16-byte alignment (Whitefoot#471);
- a `set` evaluates its right-hand side before its target (Whitefoot#515).

## What the earlier runs already show

Server CPU per request in the first run (run
[40120506904](https://github.com/Ming-Research/Firn-wf/actions/runs/40120506904),
8 passes, depth 16, the `server_cpu_us_per_request` column of its
`compare.csv`): the median over passes of each pass's ratio, and in how many
passes the head was above the base.

| test | head/base | passes head above | base-twin/base | what the request does |
| --- | ---: | ---: | ---: | --- |
| hset | 1.034 | 7 of 8 | 1.005 | replaces a field of a hash of 100,000 |
| hget | 1.029 | 6 of 8 | 1.004 | reads a field of that hash |
| sadd | 1.017 | 4 of 8, 1 equal | 0.998 | re-adds a member of a set of 100,000 |
| sismember | 0.995 | 4 of 8 | 0.993 | reads a member of that set |
| hdel | 0.980 | 2 of 8 | 0.990 | mostly finds the drained hash empty |
| srem | 0.997 | 4 of 8 | 1.000 | mostly finds the drained set empty |
| lpush | 1.016 | 7 of 8 | 0.998 | pushes at the head of one list |
| rpush | 0.995 | 2 of 8 | 0.997 | pushes at its tail |
| lpop | 0.971 | 0 of 8 | 0.993 | pops its head |
| rpop | 0.988 | 1 of 8 | 0.993 | pops its tail |

The confirmation run (16 passes, run
[40851400945](https://github.com/Ming-Research/Firn-wf/actions/runs/40851400945))
has the head above the base in all 16 passes of both cells, `hset` 1.037 and
`lpush` 1.026 by this measure. `hget` leans the way `hset` does but was not
measured again, and 8 passes do not decide it.

## Hypotheses, and what a profile of each predicts

A profile gives each symbol's share of the server's cycles. Multiplied by the
profile's event count and divided by its requests it gives cycles per request
for that symbol, which can be set beside the other image's.

- **H1, the hash map's change.** The extra cycles of `HSET` lie in the
  standard library's hash-map functions (their symbols name
  `hash_map`), or in the firn function they are inlined into, while `RPUSH`
  shows none. `HGET`, which probes the same table, shows the same if it is
  slower.
- **H2, the compiler's code for firn's own command code.** The extra cycles
  lie in firn's command functions of the path (`wf_commands.push_body`,
  `wf_commands.run_push`, `wf_commands.put_fields`, ...) and not in the hash
  map or the runtime.
- **H3, the runtime's shared keyspace.** The extra cycles lie in the
  acquire and release functions (`wf_shared_acquire_as`, `acquire_entry`,
  `wf__shared_take`, `wf_shared_unlock_as`, `wf__table_lock_entry`), with no
  rise in the functions inside the statement. `RPUSH` and `SISMEMBER` take
  the same lock and are not slower, which already argues against it.
- Waiting to acquire grows whenever the statement grows, since the other
  driver waits longer. That growth follows whichever of H1 and H2 holds, and
  names no cause of its own.
- **H4, where the code lies.** The head runs the same instructions per
  request in the functions whose cycles grew, at fewer instructions per
  cycle, as Whitefoot's item on code placement records of Halo on the 14900K
  (two byte-identical hot functions at different page offsets, one workload
  4.6% slower at the same instruction count). A flat profile of cycles alone
  cannot tell this from H1 or H2.

H1 to H4 all lie in the compiler with its library and runtime, or in firn's
and Halo's source edits through what the compiler makes of them; the cross
image decides between these two before any profile is read.

## First profile run, stated before measuring

**Question.** In which functions does the head spend the extra server cycles
per `HSET` and per `LPUSH` request?

**Run.** `redis-bench.yml`, mode `compare`, on the 14900K: `base` and
`base-twin` are `7cf12df` (pin `wf-af7665be672d`), `head` is `b08522a` (pin
`wf-63b7cbb03820`), the images of the earlier runs. 2 server CPUs on the
P-cores 2 and 4, clients on CPUs 16-31; tests `hset lpush rpush hget
sismember sadd` at depth 16, 8 interleaved passes of 5 seconds, then one
profiled run of each test for each image (`perf record`, 4999 Hz, the
`profile_test` list), each image's flat profile by symbol.

**Measure.** For each image and test, cycles per request of each symbol, and
of these groups: the hash map (symbols naming `hash_map`), the list's push
(`push_body`, `push_items`, `push_all`, `push_element`, `list_room`,
`deque`), firn's other code (`wf_commands`, `wf_protocol`, `wf_store`,
`wf_server` and the rest of the image), the keyspace's lock (the five
runtime symbols named under H3 and the other `wf_shared`, `wf__shared`,
`wf__table` and `wf_guard` symbols), the allocator (`libc` allocation
symbols) and the kernel. `base-twin` against `base` is the measure's noise
for each symbol and group.

**Rule.** A group accounts for a test's regression if its head-minus-base
cycles per request is more than twice its twin-minus-base, in absolute value,
and at least half the head-minus-base of the whole profile. H1 holds for
`HSET` if the hash-map group accounts for it, with the lock group allowed to
grow beside it; H2 if the push group or firn's other code does. If no group
accounts for it, as when the two compilers inline the path differently and the
symbols do not correspond, the profile is inconclusive and the next step is a
profile with DWARF callers or the annotated instructions of the symbols that
moved. This profile is exploratory: it attributes, and its rule picks the
next step, rather than accepting or rejecting a change.

The passes give another 8 passes of throughput and CPU per request for the
six cells, judged with the twin as in the
[pin record](whitefoot-v0165-pin.md#performance-comparison-stated-before-measuring).

### Results: disturbed and stopped, no profile

Dispatched on 2026-11-25 as run
[45607701747](https://github.com/Ming-Research/Firn-wf/actions/runs/45607701747)
(from `main`, with the inputs above and `revisions` naming `7cf12df` and
`b08522a` in full). It measured from 11:51:42 to 12:07:58 UTC and failed in
pass 7: the measurement's session received TERM (exit 143), whose sender the
log does not name, before its profiles, which follow the eight passes; the
rows below are read from the job's log.

Since 2026-11-24 every agent's tests run on the same machine (`fast.yml` and
the fast workflows of the other projects on the `14900k` runners), and three
Firn fast jobs ran there during the measurement (11:53:29-11:55:34,
12:05:16-12:08:35 and 12:07:52-12:10:35; runs 45608586520, 45610400266 and
45610502247). The twin shows it: `base-twin` over `base` in `hset` rate is
0.540, 3.598, 3.820, 0.977, 0.973 and 1.740 in passes 1 to 6, and pass 6 of
`base`, from 12:05:26, ran at 11% (`sadd`) to 68% (`rpush`) of its pass-5
rate. Passes 4 and 5 are the
only ones whose twin ratios lie within 3% of 1 in all six tests; two passes
judge nothing, and they are shown only as a lead:

| test | head/base rate, passes 4 and 5 | twin/base rate | head/base CPU per request | twin/base CPU per request |
| --- | --- | --- | --- | --- |
| hset | 0.952, 0.934 | 0.977, 0.973 | 1.053, 1.072 | 1.024, 1.027 |
| hget | 0.949, 0.944 | 0.983, 0.991 | 1.052, 1.064 | 1.019, 1.012 |
| lpush | 0.984, 0.987 | 1.004, 1.016 | 1.017, 1.013 | 0.998, 0.983 |
| rpush | 1.007, 1.014 | 0.986, 1.004 | 1.004, 0.990 | 1.030, 1.000 |
| sismember | 0.979, 1.012 | 0.986, 1.019 | 1.022, 0.988 | 1.016, 0.982 |
| sadd | 0.991, 1.018 | 0.988, 1.016 | 1.006, 0.981 | 1.009, 0.983 |

`hget` leans as `hset` does, as in the first run of the pin comparison
(0.9711, 0.9456-1.0055, 8 passes), while `sismember`, a lookup in the same
library map with `unit` values, does not; the next run measures `hget` as a
cell of its own.

## Four-image run, stated before measuring

**Question.** Is each of `hset`, `lpush` and `hget` at depth 16 slower
because of the compiler with its library and runtime, or because of the
source edits of #138, and in which functions does the slower side spend its
extra cycles and instructions per request?

**Images.** `base` and `base-twin`, `7cf12df` (old source, old compiler);
`cross`, `1dddd17` (new source, old compiler); `head`, `b08522a` (new
source, new compiler). `cross` and `head` have the same firn and Halo
sources, so their symbols correspond as far as the two compilers inline
alike, and `cross` and `base` have the same compiler.

**Run.** `redis-bench.yml`, mode `compare`, on the 14900K with the placement
of the pin comparison (2 server CPUs on P-cores 2 and 4, clients on CPUs
16-31); tests `hset lpush rpush hget sismember` at depth 16, 16 interleaved
passes of 5 seconds; then one profiled run of `hset`, `lpush` and `hget` for
each image, recording cycles and retired instructions. How passes run beside
other jobs on the machine is settled before dispatch and added here.

**Split rule, for each of `hset`, `lpush` and `hget`.** The intervals are
those of the pin comparison (median of the per-pass ratios, distribution-free
95% interval), and a cell counts only if its `base-twin`/`base` interval holds
1. With `head`/`base` wholly below 1, the compiler's share is `head`/`cross`
and the source edits' share is `cross`/`base`:

- **the compiler** if `head`/`cross` is wholly below 1 and `cross`/`base` is
  not;
- **the source edits** if `cross`/`base` is wholly below 1 and `head`/`cross`
  is not;
- **both** if both are wholly below 1;
- **unresolved** if neither is, and then 16 more passes of the same images
  are pooled with these once; still neither, and the record says so with both
  intervals.

If `head`/`base` is not wholly below 1 for a cell, the regression did not
repeat in this run, and that cell is reported, not split. `rpush` and
`sismember` are the controls of `lpush` and `hget`: the same command code and
the same library map, which the pin comparison did not find slower.

**Profile rule.** For each image and test, cycles and instructions per
request of each symbol and of the groups of the first profile run. A group
accounts for the compiler's share if its `head`-minus-`cross` cycles per
request exceed twice its `base-twin`-minus-`base` in absolute value and are at
least half of the whole profile's `head`-minus-`cross`; for the source edits'
share the same with `cross`-minus-`base`. In a group that accounts for a
share, more instructions per request in the same proportion as the cycles is
H1 or H2 (more work); the same instructions at more cycles is H3's waiting or
H4. If no group accounts for it, the next step is the annotated instructions
of the symbols that moved most, in a second profiled run of `cross` and
`head` (`annotate`; DWARF callers only between those two, since `base` has no
unwind tables). The profile attributes and picks the next step; it accepts or
rejects nothing.
