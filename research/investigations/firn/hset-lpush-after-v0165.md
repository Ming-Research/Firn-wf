# HSET and LPUSH after the move to wf-63b7cbb03820

Item #145. The move of firn from the Whitefoot release `wf-af7665be672d` to
`wf-63b7cbb03820` (#138, [record](whitefoot-v0165-pin.md)) left two cells
slower on the 14900K: `hset` and `lpush` at pipeline depth 16, 2.5% to 4.0%
and 1.7% to 2.7% slower over 24 passes, with 4.1% and 2.6% more server CPU
per request ([confirmation run](whitefoot-v0165-pin.md#confirmation-run)).
This record finds which of the compiler with its standard library, the Halo
sources or firn's own source edits causes each, and holds the evidence.

The old source does not build with the new compiler, nor the new source with
the old one, so the two builds cannot be varied one part at a time; the cause
is attributed from what each build's source says and from profiles of the two
builds.

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

## Profile of the two builds, stated before measuring

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

## Results

Dispatched on 2026-11-25 as run
[45607701747](https://github.com/Ming-Research/Firn-wf/actions/runs/45607701747)
(from `main`, with the inputs above and `revisions` naming `7cf12df` and
`b08522a` in full); not yet read.
