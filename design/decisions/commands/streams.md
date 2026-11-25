# Streams

How firn stores a stream, so that trimming it answers as Redis's does.

### decision stream-stored-in-redis-nodes
A stream is kept as Redis 7.2.16 keeps it, in nodes ordered by master ID, each with its entries in ID order, tombstones, live and deleted counts and its listpack size in bytes.
- rejected: an agent proposed an ordered map of live entries with a second map summing up each node; the owner rejected it, because every write must change both maps and keep them about the same entries, and finding an entry's node takes a second lookup.
- rejected: an agent proposed treating `~` as exact trimming, with no nodes; the owner rejected it, because XLEN and XTRIM after an approximate trim would then answer unlike Redis 7.2.16, failing eight cases of its own suite.
- rejected: an agent proposed storing each node's real listpack bytes; the owner rejected it, because it needs the whole encoder and decoder and a decode on every read, while firn reads and writes no RDB file that would use those bytes.