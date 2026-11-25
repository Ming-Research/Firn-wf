# Eviction

How a command over maxmemory makes room: which context checks the limit, what it gives back first, and how it picks a victim.

### decision each-command-context-checks-and-evicts
While maxmemory is set, each command's own context checks the limit before the command runs and evicts there, reading the heap again before each victim.
- rejected: an agent proposed one dedicated evictor context that commands wake or wait for; the owner rejected it, because another command can still come between its finishing and the command running, so it is no closer to Redis, and it adds wake-ups and waiting.
- rejected: an agent proposed a turn that orders every command's check and execution while a limit is set, as Redis's single thread does; the owner rejected it, because cache deployments set a limit and would lose parallel execution across cores.

### decision maxmemory-admission-without-aof-skips-meta
While firn keeps no AOF, admission compares the heap with maxmemory without a Meta statement and takes Meta only once over the limit, while with an AOF it takes Meta for every command.
- replaced: a Meta statement in every admission check, with or without an AOF, was on main; the owner changed it, because all cores queued on it: under an unreached 8 GiB limit, GET kept 0.117 and SET 0.145 of unlimited throughput on 8 cores (data: `research/investigations/memory-limit/README.md`).

### decision over-the-limit-gives-back-the-map-reserve-first
Over the limit, admission first gives back the keyspace map's reserve and reads the limit again, before it evicts a key or refuses a write.
- rejected: an agent proposed counting the map's spare table without giving it back before eviction; the owner rejected it, because firn would then evict earlier and keep less data than Redis.

### decision eviction-samples-from-a-random-map-scan-cursor
Eviction samples maxmemory-samples eligible keys from a random map_scan cursor into one pool of 16 candidates in Meta, then deletes the chosen key holding only its entry and Meta.
- rejected: an agent proposed first adding to Whitefoot a bounded random sampling operation that does not hold the whole map; the owner rejected it, because nothing showed that map_scan was not enough, and eviction would wait on a language change.
- rejected: an agent proposed that firn shard the keyspace or keep its own index of keys with an expiry; the owner rejected it, because every command that changes an expiry would have to keep it up, it costs more memory, and it works around a Whitefoot gap inside firn.
- rejected: an agent proposed a candidate pool for each context; the owner rejected it, because the owner's sampling choice requires Redis's one retained pool of candidates.
- rejected: an agent proposed a second shared object to hold the pool; the owner rejected it, because Meta already orders each removal with its AOF record, so another object would only add a hold.

### decision volatile-absence-search-restarts-only-on-a-new-expiry
A volatile policy's resumable search for an eligible key restarts only when a statement creates or moves an expiry, while allkeys policies use the live map count instead.
- rejected: an agent proposed restarting the search on every write command; the owner rejected it, because a stream of plain SETs without an expiry would then restart an empty volatile search forever.
