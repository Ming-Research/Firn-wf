# Access stamps

The LRU or LFU stamp each key carries, the statement that refreshes it, and how an abandoned script attempt undoes it.

### decision every-lookup-refreshes-the-access-stamp
Every entry holds Redis's 24-bit LRU clock or LFU stamp and counter in one u32, refreshed on every lookup Redis 7.2.16 refreshes, under every policy.
- rejected: an agent proposed keeping the stamp only while an LRU or LFU policy is set; the owner rejected it, because OBJECT IDLETIME would then answer wrongly under noeviction, unlike Redis.
- rejected: an agent proposed no stamp at all, with random eviction standing in for LRU; the owner rejected it, because OBJECT IDLETIME and OBJECT FREQ could not be answered, and the cache hit rate would be lower than Redis's.

### decision get-refreshes-its-stamp-in-its-own-statement
GET refreshes its stamp in the entry's own statement, through helpers that declare writes only to the stamp of a present entry, so a missing key claims no cell in the map.
- rejected: an agent proposed a Whitefoot field that can be updated atomically under a read-only hold; the owner rejected it, because the controls put GET's loss on missing keys, which the map's no-claim path removes without changing what atomic statements mean.
- rejected: an agent proposed refreshing the stamp in a separate statement only when its clock has moved; the owner rejected it, because with a large keyspace nearly every hit then enters the map twice, 6 to 8% below the build without stamps.
- rejected: an agent proposed keeping the stamps in a second map keyed the same way; the owner rejected it, because every miss then took a passing claim in the stamp map, which fell to 0.44 of the build without stamps on four CPUs.

### decision abandoned-script-attempt-restores-access-stamps
A script attempt that wrote nothing and is abandoned for its step budget restores each refreshed key's stamp and firn's LFU random state before it releases its keyspace statement.
- rejected: an agent proposed accepting the extra LFU increments of abandoned attempts; the owner rejected it, because OBJECT FREQ could then answer values that Redis's single run of the script never gives.
- rejected: an agent proposed not rerunning a script after its first refresh and holding the keyspace to its end; the owner rejected it, because a long script would then hold the keyspace for good, out of SCRIPT KILL's reach, against the approved rerun design.

### decision no-touch-mode-skips-the-refresh-by-a-test-in-every-refresh
A connection in CLIENT NO-TOUCH mode skips the access-stamp refresh, and every refresh tests the mode first, MSET's included, as Redis 7.2.16's lookupKey does.
- rejected: an agent proposed removing the per-key test from MSET's path and then measuring again; the owner rejected it, because one code's two builds differ by 1.7% on hset, so MSET's 0.5% is not shown to come from the change (data: `research/investigations/deployment-performance/README.md`).
- rejected: an agent proposed a criterion with a 1% noise floor for every cell; the owner rejected it, because accepting the mset and setmany-tx cells is one case, not a precedent, and the other cells are still judged by no interval wholly on the slow side.
