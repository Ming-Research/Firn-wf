Decision: SCAN, KEYS and RANDOMKEY enumerate the keyspace map through Whitefoot's `map_scan`, whose position cursor returns each key present throughout a scan exactly once while other clients write, SCAN taking one `map_scan` step per request from the client's cursor and KEYS every step in one statement, because that meets Redis 7.0.15's SCAN guarantee, every key present for the whole iteration returned, with no work added to the commands that write keys, instead of an index of keys kept beside the map, which every write would have to update.

Decision: RANDOMKEY picks uniformly among the keys, up to fifteen, that one `map_scan` step of count 15 gathers from a random position, stepping on only when that step finds none, and starts over after removing a picked key found expired, because that is how Redis 7.0.15's dbRandomKey samples through dictGetFairRandomKey and dictGetRandomKey, instead of the first live key at or after a random position.

Rejected:
- An index of keys kept beside the keyspace map: rejected because every write would update it, where the map's own cursor enumerates the keys.
- The first live key at or after a random position: rejected because a key's chance then grows with the empty run before it, where Redis averages it over fifteen keys.
