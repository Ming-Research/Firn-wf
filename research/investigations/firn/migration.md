# Moving Redis 7.2.16 data to firn

Board item: `firn-bl-01-02-rest`, migration row.

## Question

Can an operator move Redis 7.2.16's database 0 to firn through either its
multipart command-only AOF or a client-side copy, preserving complete values
and exact absolute expiry deadlines for every data type firn currently
implements? Missing or extra keys, different type/value bytes, or any
changed expiry deadline rejects the path.

## Method

`tests/migration.rs` runs in the gate's existing network target. `make test`
obtains `REDIS_SERVER` through `tests/redis-suite/reference.sh`, which builds
the SHA-256-pinned Redis 7.2.16 archive; the network job caches that same
build as the Redis-suite and AOF-sync-fault jobs. The test fails without the
oracle or when it reports a different version. No system Redis package is
used. No compiler, server, test or measurement runs on the owner's Mac.

Both paths populate an independent Redis source with strings (binary, empty,
integer and more than 1 MiB), lists, hashes, sets and sorted sets (tied,
negative and infinite scores), including permanent and expiring keys. firn
implements only `SELECT 0`, so the dataset has one database; streams are
outside its implemented types.

The file path sets `appendonly yes` and `aof-use-rdb-preamble no`, requests
`BGREWRITEAOF`, waits for `aof_rewrite_in_progress:0` and
`aof_last_bgrewrite_status:ok`, and includes writes in the incremental file
after the rewrite. Redis stops cleanly before the entire append directory
is copied unchanged to a fresh firn working directory. firn loads the copied
manifest, base and incremental files with the same appendfilename. Redis
restarts in its original directory and uses its own loader to reconstruct
the live comparison oracle from the source files. The two servers have
separate directories: Redis never loads the copy that firn can modify. The
zeroed-increment negative control in [Result](#result) demonstrates that
missing incremental contents are detected against this oracle.

The online path keeps both servers running but makes no concurrent source
mutations. firn's command table has no `MIGRATE`, `DUMP` or `RESTORE`, so the
test helper uses `SCAN` and per-type reads/writes: `GET`/`SET`,
`LRANGE`/`RPUSH`, `HGETALL`/`HSET`, `SMEMBERS`/`SADD` and
`ZRANGE WITHSCORES`/`ZADD`. Before reading the type and full value, it reads
the source's `PEXPIRETIME`: `-1` means permanent, a positive value is the
absolute expiry deadline in epoch milliseconds, and `-2` rejects a missing
key. After copying a value with an expiry, it installs that unchanged
deadline on firn with `PEXPIREAT`. Permanent keys receive no expiry.
This helper is not an atomic snapshot or a user-facing utility.

The comparison independently enumerates both key sets and reads `TYPE` and
every complete value from each server. It compares exact bytes, preserving
list and sorted-set ordering and canonicalizing only the unspecified order
of hash field/value pairs and set members. A copied value cannot pass merely
because it matches a fixture expected by the copying code. Permanent keys
must have `PEXPIRETIME == -1` on both servers. For every expiring key,
both servers must return exactly the same positive `PEXPIRETIME`; `-2`
fails because the key is missing. Both paths use this comparison, with no
elapsed-time correction or TTL sampling tolerance.

## Result

The owner-supplied CI evidence records both migration cases passing in
[gate run 38415877392](https://github.com/Ming-Research/Firn-wf/actions/runs/38415877392)
at revision `0f775cb`. In
[negative-control run 38417367903](https://github.com/Ming-Research/Firn-wf/actions/runs/38417367903),
a temporary branch zeroed the destination's copied incremental AOF contents
while retaining its filename and manifest entry. The AOF comparison failed
at `database 0: complete key sets` because `incremental-only` was missing.
The online control changed one byte of `list:persistent`; its complete-value
comparison identified that key and reported `complete exact bytes differ`.
An earlier control
omitted the incremental file and made firn exit with status 4 before
listening. It tested loader refusal rather than the comparison, so the
zeroed-increment control replaced it.

[Gate run 38419322241](https://github.com/Ming-Research/Firn-wf/actions/runs/38419322241)
at revision `fa3cda4` exercised the documented named options and the
`appendonly.aof` filename. The owner-supplied evidence records the AOF case
passing and the online case failing for `large:expires`:
`firn renewed TTL (299899 > source 299897)`. Firn's remaining TTL was 2 ms
higher even though it was sampled later. The relative-expiry helper read
`PTTL`, subtracted elapsed copy time plus a 1 ms margin, then sent
`PEXPIRE`. That subtraction omitted the unbounded delay before firn
installed the relative expiry, so it extended the deadline; the failure
does not justify treating the difference as test noise. The earlier passing
run used this relative-expiry method and sampled PTTL comparisons, so it
does not establish exact deadline preservation.

The current correction reads source `PEXPIRETIME`, installs the same deadline
with `PEXPIREAT`, and compares exact deadlines in both paths. It awaits new
CI; no local build or test was run for these edits. The planned cutover path
is the command-only AOF import, because the existing loader and multipart
layout provide the mechanism without adding firn commands or shipping a
separate copy utility. The
README records the steps and the online helper's limits.

## Compatibility limits found while reading

- The loader parses command-form AOF, not Redis's default RDB base preamble.
  `aof-use-rdb-preamble no` followed by a successful rewrite is required;
  changing the setting without rewriting does not convert an existing base.
- Only database 0 and the five types above are supported. Reading
  `firn/persistence/persistence.wf` shows replay executes each command and
  clears the reply buffer without testing whether the command returned an
  error. Combined with rejection of `SELECT 1`, this implies an AOF from
  several databases could silently apply later records in database 0.
  This is a code-inspection finding, not an executed reproduction. Fixing
  replay's treatment of command errors and implementing other databases
  are outside this task; importing such a file is unsafe.
- Sorted-set score replies use `%.17g`, whereas Redis 7.2.16 uses
  `fpconv_dtoa`; the existing `firn-score-text-72` board item covers the
  difference for values such as `0.1`. The migration comparison remains
  byte-exact and the dataset covers the requested score categories; it does
  not prove reply-text equality for all doubles.
- Source and destination clocks must be synchronized for absolute epoch
  deadlines to represent the same expiration time. The online fixture keeps
  the source quiescent, uses five-minute expiries and rejects a missing key
  when reading its deadline or value. A
  production client must handle concurrent changes, expirations during
  transfer, retries and partial copies before cutover; the test helper
  supplies none of those operational guarantees.
- A dump-based online path would need Redis-compatible `DUMP` serialization
  and `RESTORE` decoding/expiry behavior. Native `MIGRATE` would additionally
  need a server-to-server transfer and failure protocol. No commands, pin,
  submodule or Whitefoot source change belongs to this experiment.
