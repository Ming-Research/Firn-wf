# Testing

How firn is checked against the reference Redis: the reference server every check uses, and below it the Redis suite, the network cases and the benchmarks.

### decision reference-server-is-redis-7-2-16-from-its-pinned-archive
Every reference run uses Redis 7.2.16 built from its release archive, whose SHA-256 `tests/redis-suite/reference.sh` pins and checks.
- replaced: Redis 7.0.15 as the reference was on main; the owner changed it, because 7.2 is the part Redis and Valkey share, still gets security fixes, and differs least from 7.0.
- rejected: an agent proposed Redis's newest release, 8.10.2, as the reference; the owner rejected it, because it differs most from 7.0 and costs most to move to, its bundled modules would join firn's differences, and Valkey users' behaviour would part from it.
- replaced: Ubuntu 24.04's redis-server package was on main; the owner changed it, because that package is Redis 7.0.15, and an unpinned package can change between two runs.
- rejected: an agent proposed Redis's apt repository or its official container image; the owner rejected it, because an unpinned package or image can change between two runs.