# Memory limit

What firn counts against maxmemory, which policies it offers, which settings a command reads, and how the heap peak is kept.

### decision used-memory-counts-requested-heap-bytes
used_memory is Whitefoot's heap_in_use, the requested bytes of live allocations, and maxmemory is compared with it less the excluded AOF and rewrite bytes, never below zero.
- rejected: an agent proposed reporting firn's count as equal to Redis's allocator-usable bytes; the owner rejected it, because Whitefoot's heap measurement gives requested bytes and does not show the allocator's rounding that Redis's count includes.

### decision maxmemory-offers-all-eight-policies
firn offers all eight Redis maxmemory policies: noeviction, allkeys-lru, allkeys-lfu, allkeys-random, volatile-lru, volatile-lfu, volatile-random and volatile-ttl.
- rejected: an agent proposed offering only noeviction, allkeys-lru and volatile-lru and refusing the rest in CONFIG SET; the owner rejected it, because sampling, refusal and reporting are shared, the other five add little, and a deployment set up with one of them would fail to start.

### decision memory-settings-read-once-per-request-read
Each connection copies the memory settings once per read, beside its clock, and every command of that read, EXEC's queue and each attempt of a script use that copy.
- rejected: an agent proposed that every command hold the shared server state with its keys to read the settings; the owner rejected it, because all commands would then queue on one object, a 7 to 10% loss on two CPUs at depth 16, while Redis itself leaves old stamps after a policy switch.
- rejected: an agent proposed first designing a way to publish settings concurrently, such as a version number with lock-free reads; the owner rejected it, because no form for it had been chosen, and it would hold up all of maxmemory.

### decision config-set-refreshes-the-connections-memory-settings
After a connection runs CONFIG SET, it copies the memory settings again before it runs the rest of the commands in the same read.
- rejected: an agent proposed reading the latest settings before every command; the owner rejected it, because that is the shared hold already turned down for the per-read copy, and every command would pay for it, even where no limit is set.
- rejected: an agent proposed letting CONFIG SET take no effect on the rest of its read; the owner rejected it, because Redis's tests and real setup scripts send CONFIG SET maxmemory and then write at once.

### decision maxmemory-leaves-out-aof-buffers-and-rewrite-charges
The maxmemory comparison leaves out both AOF buffers' allocated capacity and the rewrite's charged bytes, less the rotation charges that the two buffers already cover.
- rejected: an agent proposed leaving out only the unwritten bytes of the two buffers; the owner rejected it, because their spare capacity would then count, and a buffer grown by eviction's own DEL records would cause more eviction.
- rejected: an agent proposed leaving out only the buffer held in Meta; the owner rejected it, because the same unwritten bytes would then count or not count as the writer takes the buffer over.

### decision heap-peak-raised-past-the-connections-last-seen-peak
After each command a connection takes Meta to raise the recorded heap peak only when its reading is above the peak it last saw recorded.
- replaced: a Meta statement after every command to raise the peak, as Redis 7.0.15 does, was on main; the owner changed it, because every core then queued on Meta: on 8 cores, depth-16 GET ran at 2.98M/s with it and 19.2M/s without (data: `research/investigations/memory-limit/README.md`).
- rejected: an agent proposed raising the peak once per read of pipelined commands; the owner rejected it, because a later command in the pipeline can free what an earlier one allocated and hide a peak that Redis records.
