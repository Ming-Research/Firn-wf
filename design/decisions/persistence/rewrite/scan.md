# Rewrite scan

How a rewrite's scan steps through the live keyspace within the reserve, and how it shares the driver with clients.

### decision aof-rewrite-scan-step-within-reserve
Each scan step calls map_scan_within with the reserve left after its 64-KiB output buffer as the limit, and charges the KeySet growth that key_set_bytes reports.
- replaced: map_scan with count hint 1, its KeySet charged after the call, was on main; the owner changed it, because the call allocates a step's keys before any charge, so one long key went past R.
- rejected: an agent proposed charging the whole scan limit before the call and refunding what the KeySet did not keep; the owner rejected it, because the ledger's peak would then read nearly R at every step and hide the attempt's real use.

### decision aof-rewrite-scan-paced-by-time-slice
The scan runs steps back to back and pauses outside every hold about once a millisecond, with a per-step key hint that starts at 64 and halves or doubles by the bytes a step writes.
- replaced: A 100-microsecond sleep after every one-key step was on main; the owner changed it, because the driver's millisecond timer made each step cost about a millisecond, so a million-key rewrite could not finish under writes (data: `research/investigations/aof-rewrite/scan-pace.md`).
- rejected: an agent proposed only moving the journal to disk and keeping the sleep after each step; the owner rejected it, because a rewrite would then take over 100 seconds, and the journal and the append-only file on disk would both grow.

### decision aof-rewrite-honors-rdb-key-save-delay
CONFIG answers Redis's hidden rdb-key-save-delay; a nonzero value scans one position per step, and a positive one sleeps that many microseconds per key.
- rejected: an agent proposed a firn-only test hook to keep a rewrite in progress; the owner rejected it, because Redis's own suite uses this parameter for that.
- rejected: an agent proposed test data large enough to outlast a time-sliced scan; the owner rejected it, because such cases race the scan.