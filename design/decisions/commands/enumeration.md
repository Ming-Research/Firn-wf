# Enumeration

How firn walks the keyspace and a collection's elements: SCAN, KEYS, RANDOMKEY, the glob matcher, SSCAN, HSCAN and ZSCAN.

### decision keyspace-enumeration-through-map-scan
SCAN, KEYS and RANDOMKEY walk the keyspace through Whitefoot's `map_scan`, SCAN taking one step per request from the client's cursor and KEYS every step in one statement.
- rejected: an agent proposed an index of keys kept beside the keyspace map; the owner rejected it, because every write would have to update it, where the map's own cursor already returns each key present throughout a scan exactly once.

### decision randomkey-samples-fifteen-keys
RANDOMKEY picks uniformly among the keys one `map_scan` step of count 15 gathers from a random position, wrapping at the table's end, as Redis's dbRandomKey draws.
- rejected: an agent proposed the first live key at or after a random position; the owner rejected it, because a key's chance then grows with the empty run before it, where Redis averages it over fifteen keys.

### decision scan-excludes-expired-keys
SCAN removes and records a matched key it finds expired and leaves it out of the reply under every TYPE argument, none included, as Redis 7.2.16 does.
- replaced: returning the removed key's name under TYPE none, as Redis 7.0.15 does, was on main; the owner changed it, because Redis 7.2.16's SCAN treats a key whose lookup finds nothing as a type that does not match.

### decision glob-match-nesting-limit
The shared glob matcher stays iterative and returns no match past Redis 7.2.16's nesting depth of 1000, counting only each entry into a star with more pattern after it.
- replaced: a matcher with no limit on nesting was on main; the owner changed it, because Redis 7.2.16 returns no match above depth 1000, even where an unlimited matcher would match the name.
- rejected: an agent proposed limiting the total number of stars in a pattern; the owner rejected it, because Redis 7.2.16 limits the depth of nesting, counted after adjacent stars collapse, not the number of stars.

### decision collection-scans-return-whole-collection
SSCAN, HSCAN and ZSCAN return the whole MATCH-filtered collection in firn's element order, in one held statement, with cursor 0.
- rejected: an agent proposed answering these scans only for small collections and refusing large ones until Whitefoot's map can resume a scan; the owner rejected it, because applications would fail on collections with many members, and their validation would wait for Whitefoot.
- rejected: an agent proposed keeping Redis's encoding history and listpack order to reproduce Redis's batches; the owner rejected it, because the selected scope allows firn's element order and batch boundaries, and a complete held walk already returns every element present throughout.
- rejected: an agent proposed reading the map's buckets or resuming a whole-map visitor at a numeric offset; the owner rejected it, because a rebuild may move elements present throughout the scan and skip them between calls.
- rejected: an agent proposed returning part of a large collection with cursor 0, or dropping elements above a compact size; the owner rejected it, because a full iteration would then miss elements present throughout it.

### decision zscan-score-text-by-collection-size
ZSCAN writes scores with Redis 7.2.16's d2string text when the whole sorted set has at most 128 members of at most 64 bytes each, and with the %.17g writer otherwise.
- rejected: an agent proposed one score formatter for every size; the owner rejected it, because Redis 7.2.16 writes 0.1, 1e17 and 2/3 one way for a compact set and another way for a large one (data: `research/investigations/firn/collection-scan.md`).
- rejected: an agent proposed keeping sorted-set encoding history to choose the score text; the owner rejected it, because the rule rests on the current whole set and keeps no Redis encoding state, though a set that grew and shrank then differs from Redis.
- rejected: an agent proposed choosing the score text from the MATCH result only; the owner rejected it, because Redis's encoding belongs to the whole sorted set, so a long member that does not match still selects the large set's text.