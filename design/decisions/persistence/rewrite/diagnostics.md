# Rewrite diagnostics

What INFO reports about a rewrite: its reserve, its progress, why it stopped and which cleanup steps failed.

### decision aof-rewrite-info-reserve-and-scan-peak
INFO's firn-only fields report a rewrite's admitted reserve, the largest heap growth seen across one scan step, and whether the attempt crossed S1.
- rejected: an agent proposed checking the reserve accounting against the reservation ledger itself; the owner rejected it, because the long-key check needs allocation evidence independent of the ledger, under the rule that INFO reports real facts.

### decision aof-rewrite-reports-first-failure
INFO reports the first cause that stopped a rewrite, latched under Meta before cleanup, with the reserve's use and peak, the journal's record counts and the scan's progress.
- rejected: an agent proposed letting a later cleanup error replace the first cause; the owner rejected it, because an error without its first cause cannot tell which resource or I/O condition stopped the attempt.

### decision aof-rewrite-reports-cleanup-failures
INFO counts an attempt's failed cleanup steps in firn_aof_rewrite_cleanup_failed, apart from its first cause, and keeps the last leftover-journal observation in firn_aof_rewrite_journal_leftover.
- rejected: an agent proposed reporting cleanup errors only through the first failure cause; the owner rejected it, because cleanup can fail after another cause has already stopped the attempt.
- rejected: an agent proposed resetting the leftover-journal observation with each attempt's counters; the owner rejected it, because a leftover is found at startup, before any attempt exists.