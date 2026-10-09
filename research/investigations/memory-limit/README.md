# Bounding firn's memory as Redis's maxmemory does

## The question

The deployment milestone needs memory-bounded operation: a cache that
evicts under a limit, and a session or rate-limit store that refuses writes
rather than lose data when full
([TODO](../../../docs/todo.md#server), "Complete firn's standalone
deployment workloads"). Redis 7.0.15 does both with `maxmemory`,
`maxmemory-policy` and its `OOM` refusal. firn has none of it: `CONFIG SET
maxmemory` is refused, and `INFO` reports a constant `maxmemory:0`.
Whitefoot now lets a program read the bytes its heap holds and its resident
set ([memory statistics](https://github.com/Ming-Research/Whitefoot/pull/277)).
What should firn count, when should it check and evict, and which keys may
it evict, so that the selected consumers behave as they do on Redis?

## What Redis 7.0.15 does

From its source at the 7.0.15 tag (`evict.c`, `server.c`, `object.c`,
`db.c`, `script.c`, `multi.c`, `src/commands/*.json`):

- **What is compared.** `used_memory`, the allocator's usable bytes of every
  live allocation, less the append-only buffer and replica buffers beyond
  the backlog (`mem_not_counted_for_evict`), against `maxmemory`; 0, the
  default, means no limit.
- **When.** `processCommand` calls `performEvictions` before every command
  while `maxmemory` is set. It evicts until under the limit or until a time
  limit set by `maxmemory-eviction-tenacity` (default 10, about 500 µs),
  continuing from the event loop after that.
- **Refusal.** When nothing more can be evicted, a command flagged
  `denyoom` gets `-OOM command not allowed when used memory >
  'maxmemory'.`. Among the consumers' commands `SET`, `MSET`, `SETEX`,
  `INCR`, `INCRBY` are `denyoom`; `GET`, `MGET`, `EXPIRE`, `DEL`, `PTTL`,
  `SCAN` are not. Every command queued after `MULTI` is refused, and `EXEC`
  is refused when a queued command is `denyoom`; `DISCARD` never is. A
  script without a shebang runs, and its first `denyoom` call before any
  write is refused with the same error.
- **Policies.** `noeviction` (default), `allkeys-lru`, `allkeys-lfu`,
  `allkeys-random`, `volatile-lru`, `volatile-lfu`, `volatile-random`,
  `volatile-ttl`; `volatile-*` consider only keys with an expiry and act as
  `noeviction` when there are none.
- **Approximation.** Each round samples `maxmemory-samples` (default 5) keys
  with `dictGetSomeKeys`, keeps the best 16 candidates in a pool, and evicts
  the best. Each object carries 24 bits: a seconds clock for LRU, or a
  minute stamp and an 8-bit logarithmic counter for LFU. Every lookup except
  `EXISTS`, `TYPE`, `TTL`, `PTTL`, `OBJECT` and their kin refreshes it, under
  every policy, so `OBJECT IDLETIME` works under `noeviction`.
- **Report.** `INFO` gives `used_memory`, `used_memory_rss` (refreshed every
  100 ms), `used_memory_peak`, `maxmemory`, `maxmemory_policy`,
  `mem_fragmentation_ratio`, `mem_not_counted_for_evict`, and `evicted_keys`
  in its stats.

## What can be observed

Redis's suite runs `unit/maxmemory` only against a server it starts
itself, so the ratchet skips it. Tests elsewhere that run against firn need
`CONFIG SET maxmemory`, the `OOM` refusals of `MULTI`, `EXEC` and scripts,
`OBJECT IDLETIME` and `FREQ`, `RESTORE ... IDLETIME` and `FREQ`, and the
`used_memory` and `mem_not_counted_for_evict` fields (`unit/multi`,
`unit/scripting`, `unit/info`, `unit/dump`, `unit/introspection-2`,
`unit/tracking`). No test checks which of several candidates is evicted.
Which key goes first is observable to an application only as a cache's hit
rate.

The consumers say little: Django warns that a cache that evicts can log
users out of cache-backed sessions; connect-redis and rate-limiter-flexible
give every key an expiry and document no policy.

## Proposals

These are the direction-setting choices. Each needs the owner's ruling
before implementation; the rest follows Redis.

1. **What is counted.** `used_memory` is Whitefoot's `heap_in_use`, the
   requested bytes of live allocations, and `used_memory_rss` its resident
   set; the limit compares `heap_in_use` less firn's append-only pending
   bytes. Redis counts usable sizes, so firn's count for the same data is
   lower by the allocator's rounding. This follows the memory-statistics
   proposal the owner approved; it is recorded here, not reopened.
2. **Which policies.** All eight, against the subset the consumers' likely
   deployments use (`noeviction`, `allkeys-lru`, `volatile-lru`). The
   sampling, the refusal and the reporting are shared; `random` needs
   nothing more, `ttl` can take the nearest expiry from the expiry queue
   firn already keeps, and LFU adds an 8-bit counter beside the stamp.
3. **The access stamp.** Each entry gains 32 bits holding Redis's 24-bit
   LRU clock or LFU stamp and counter, refreshed by every lookup Redis
   refreshes, under every policy, as Redis does. That makes `GET`'s
   statement write the entry it reads. The alternative, stamping only under
   an LRU or LFU policy, leaves `OBJECT IDLETIME` wrong under `noeviction`.
4. **When to check.** Before every command while `maxmemory` is nonzero,
   as Redis does, with one comparison of a cached setting when it is zero.
   The command's own context evicts, so several contexts may evict at
   once; each evicts until the count it reads is under the limit.
5. **Sampling.** `maxmemory-samples` keys from a random cursor of the
   keyspace's `map_scan`, keeping Redis's pool of 16 best candidates, for
   the `lru`, `lfu` and `random` policies; `volatile-*` skip keys without an
   expiry.

## Measurements, stated before implementing

- **Cost of the stamp.** firn's `redis-benchmark` `get` and `set` at
  depths 1 and 16 on one and two CPUs, with and without the stamp, same
  source, interleaved with twins on the i9-14900K. The stamp is rejected in
  that form if `get` loses more than its twins' spread, and then the next
  step is a stamp written only when the clock it holds has advanced.
- **Cost of the check.** The same comparison with `maxmemory` set far above
  the dataset, against unset.
- **Memory per key.** The resident set after one million sessions, as in
  the [deployment measurement](../deployment-performance/README.md#results-1),
  with and without the stamp.
- **Eviction quality.** A Zipf-distributed `GET`/`SET` workload under
  `allkeys-lru` with a limit at half the dataset: firn's hit rate against
  Redis 7.0.15's. firn's is rejected if it is more than two percentage
  points below Redis's.

## Step 1 implementation record

The [access/configuration draft record](step-1.md) lists every lookup site,
its Redis oracle, validation still required for the current working changes,
and the owner's two additional A rulings: snapshot memory settings once per
request read, and restore first-refresh stamps and LFU random state before
abandoning an unwritten script attempt. The owner selected all eight policies and
the all-policy access stamp in the board rulings `firn-maxmemory-policies` A
and `firn-access-stamp` A. Eviction and OOM refusal are the following step.
