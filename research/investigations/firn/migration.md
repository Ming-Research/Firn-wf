# Moving Redis 7.2.16 data to firn

Board item: `firn-bl-01-02-rest`, migration row.

## Question

Can an operator move Redis 7.2.16's database 0 to firn through either its
multipart command-only AOF or a client-side copy, preserving complete values
and remaining expiry times for every data type firn currently implements?
Missing or extra keys, different type/value bytes, an extended TTL, or TTL
loss exceeding the stated elapsed-time tolerance rejects the path.

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
manifest, base and incremental files with the same appendfilename.

The online path keeps both servers running but makes no concurrent source
mutations. firn's command table has no `MIGRATE`, `DUMP` or `RESTORE`, so the
test helper uses `SCAN` and per-type reads/writes: `GET`/`SET`,
`LRANGE`/`RPUSH`, `HGETALL`/`HSET`, `SMEMBERS`/`SADD` and
`ZRANGE WITHSCORES`/`ZADD`. It samples `PTTL` before the read and accounts for
elapsed copying time before setting a destination expiry with `PEXPIRE`.
This helper is not an atomic snapshot or a user-facing utility.

The comparison independently enumerates both key sets and reads `TYPE` and
every complete value from each server. It compares exact bytes, preserving
list and sorted-set ordering and canonicalizing only the unspecified order
of hash field/value pairs and set members. A copied value cannot pass merely
because it matches a fixture expected by the copying code. Permanent keys
must have `PTTL == -1`. Redis's expiring-key PTTL is sampled first; firn's
must remain positive and no greater, with loss at most the elapsed time
between samples plus 250 ms. The elapsed time includes network and runner
scheduling delay; the allowance covers rounding and small clock differences.

## Result

Pending CI. No local build or test was run, and no commit was made. The gate
must establish both cases on the edited revision before either path is
claimed to work. The planned cutover path is the command-only AOF import,
because the existing loader and multipart layout provide the mechanism
without adding firn commands or shipping a separate copy utility. The
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
- Relative `PEXPIRE` is installed after the client computes its remaining
  time. Its network/scheduling delay is not known in advance; subtracting
  elapsed time plus 1 ms does not guarantee an identical absolute deadline.
  The test checks the requested sampled PTTL bounds, which can fail under
  asymmetric delays and can miss an extension smaller than the interval
  between samples. CI under gate load remains unverified. A production
  client needing exact deadlines could use the already supported
  `PEXPIRETIME` and `PEXPIREAT`, with synchronized source/destination clocks.
- A dump-based online path would need Redis-compatible `DUMP` serialization
  and `RESTORE` decoding/expiry behavior. Native `MIGRATE` would additionally
  need a server-to-server transfer and failure protocol. No commands, pin,
  submodule or Whitefoot source change belongs to this experiment.
