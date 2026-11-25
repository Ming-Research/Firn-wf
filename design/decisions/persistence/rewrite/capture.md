# Rewrite capture

What a rewrite records of the writes made while it scans, the memory it may use for that, and when a failed rewrite is tried again.

### decision aof-rewrite-reserve-outside-maxmemory
A rewrite may use a reserve R of maxmemory/20, or 16 MiB without maxmemory; its charges are left out of maxmemory, and passing R aborts the rewrite.
- replaced: Counting rewrite memory in maxmemory, with R plus headroom under the limit, was on main; the owner changed it, because a cache kept at its limit by eviction could never rewrite, and Redis 7.2.16 does not count its rewrite child (data: `research/investigations/aof-rewrite/scan-log.md`).
- rejected: an agent proposed still counting it, but evicting room for R and headroom before each rewrite; the owner rejected it, because every rewrite would then evict about a tenth more of the data and lower the hit rate, unlike Redis.

### decision aof-rewrite-failure-stops-automatic-rewrites
Any failed rewrite attempt stops automatic rewrites until a manual BGREWRITEAOF succeeds; recovery rewrites after a sync error still run.
- rejected: an agent proposed retrying automatically, as Redis 7.0.15's aofRewriteLimited does, delaying only after three failures; the owner rejected it, because reserve or value-size pressure can persist, and repeated attempts would use rewrite resources without the owner acting.

### decision aof-rewrite-images-staged-in-chunks
A statement that journals images gathers each changed key's complete image in its own scratch, then publishes one framed unit into fixed 64-KiB staging chunks.
- rejected: an agent proposed journaling borrowed parts of entries, exporting a second keyspace, or releasing a whole journal at once under Meta; the owner rejected it, because an EXEC or script must commit its final images with its effects, and cleanup must release owners of bounded size.

### decision aof-rewrite-journal-streams-to-disk
The sole writer drains journal staging outside Meta to temp-F.journal in appendonlydir, and the rewrite reads it back through one charged 64-KiB buffer after S1.
- replaced: Keeping the whole journal in memory was on main; the owner changed it, because under steady writes the journal grew toward another copy of the dataset and used up the reserve, so rewrites aborted at 16 MiB and at 429 MB alike (data: `research/investigations/aof-rewrite/journal-disk.md`).

### decision aof-rewrite-collecting-buffer-charged-to-reserve
Growth of the collecting buffer while the writer seals and switches is charged to R through a statement-local LogBudget, committed with the mutation under Meta.
- rejected: an agent proposed counting on reuse of the two buffer owners to bound their capacity; the owner rejected it, because maxmemory leaves out ordinary AOF buffers, and writes of constant-sized values could still grow the backlog after S1 without bound.

### decision aof-rewrite-startup-serves-past-a-leftover-journal
Startup serves even when it cannot remove an unlisted leftover journal; the next rewrite must remove it before opening a new journal, or refuses with journal_io.
- rejected: an agent proposed refusing service while such a leftover remains; the owner rejected it, because the journal adds nothing to the recovered state, and Redis 7.0.15 also loads its selected files and ignores a failed unlink of temporary files.

### decision aof-rewrite-one-key-statements-journal-bytes
A statement opened for one key journals its own bytes while it has changed only one key already journaled in the window; first writes, other statements, EXEC and scripts journal complete images.
- rejected: an agent proposed keeping only the latest image of each key; the owner rejected it, because each write would still copy the whole key, and the journal would have to change from a byte stream to a structure replaced by key.
- rejected: an agent proposed journaling only the appended bytes when EXEC or a script keeps growing one key; the owner rejected it, because no measurement showed a rewrite failing from it, and no selected application does it.

### decision aof-rewrite-scripts-journal-images
A script journals complete images even when it declares one key.
- rejected: an agent proposed choosing one-key byte capture for a script from its numkeys alone; the owner rejected it, because complete images hold for both declared-key attempts and whole-map retries, and the one-key byte path was not yet verified for scripts.

### decision aof-rewrite-copies-large-keys-whole
A scan step or a capture copies a key's whole image in one hold, limited only by R, so the largest key a rewrite carries is about R/2.
- rejected: an agent proposed a work ceiling of 1024 payload visits or 64 KiB per scan step and per statement; the owner rejected it, because any key above 512 elements or about 64 KiB then aborts every rewrite, where the replay rewrite and Redis rewrite such keys.
- rejected: an agent proposed splitting a key across scan steps; the owner rejected it, because that needs resumable iteration over Whitefoot's HashMap, which it does not offer.