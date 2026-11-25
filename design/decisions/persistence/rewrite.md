# AOF rewrite

How firn rewrites its append-only file from the live keyspace: the scan and journal that make the new base, the one switch at S1, and how a rewrite starts and meets shutdown.

### decision aof-rewrite-base-from-live-scan-and-journal
A rewrite writes its base from bounded scan steps over the live keyspace after S0, then applies the writes journaled during the scan in sequence order, which gives the state at S1.
- replaced: Replaying the base and closed incremental files into a private keyspace was on main; the owner changed it, because that copy is a second whole dataset in memory, and maxmemory should stay close to what the process really uses.
- rejected: an agent proposed counting the rewrite's copy within maxmemory and evicting during the rewrite; the owner rejected it, because every rewrite would then evict about half the cache and drop the hit rate, where Redis evicts nothing for a rewrite.
- rejected: an agent proposed putting off a rewrite while current use plus the copy would pass a further limit; the owner rejected it, because the append-only file would keep growing while memory is tight, and it adds a setting Redis does not have.
- rejected: an agent proposed keeping the replay rewrite on main and offering the scan only as a mode for comparison; the owner rejected it, because that keeps the second copy of the data and two sets of code to maintain.
- rejected: an agent proposed replaying the scan window's commands over the scanned images without first journaling each key's complete image; the owner rejected it, because a scanned image can already hold a later write, so INCR and collection changes would apply twice.
- rejected: an agent proposed sorting the journaled images so that the base holds each key once; the owner rejected it, because that needs an external sort or more memory, while writing the images in sequence order needs the fewest parts and the most direct proof.

### decision aof-rewrite-switches-once-at-s1
The sole writer switches to a new incremental file once, at S1 when the scan ends, so the new base is paired only with the commands after S1.
- replaced: Switching at the start of the rewrite, as the replay rewrite did, was on main; the owner changed it, because a base that holds the state at S1, paired with the commands from S0, would apply INCR and other non-idempotent writes twice.
- rejected: an agent proposed switching at both S0 and S1; the owner rejected it, because it leaves one more incremental file for the scan window and one more round of file work, for the same cut.

### decision aof-rewrite-cut-before-a-retained-multi
At the switch, an old incremental file that ends inside an unfinished block, which only a MULTI kept at startup leaves, is cut before that block's first MULTI and synced.
- rejected: an agent proposed refusing the rewrite until a restart while such a MULTI remains; the owner rejected it, because every write appended after that MULTI would then be lost at the next restart, where Redis's rewrite from its live dataset keeps them.
- rejected: an agent proposed cutting the unfinished block away at startup; the owner rejected it, because the loaded file would then differ from the file Redis keeps.
- rejected: an agent proposed cutting before the latest MULTI; the owner rejected it, because with nested MULTIs that leaves an earlier unfinished block in a file that is no longer last, and such a file must replay whole.

### decision aof-rewrite-carries-the-block-boundary
The writer carries the current incremental file's block boundary from startup's parse and advances it with each append, so the switch rereads the file only when that state is unknown.
- replaced: Rereading the whole current incremental file at each switch was on main; the owner changed it, because on the 14900K writes piled up in the reread until the reserve ran out and the rewrite aborted; Redis 7.2.16 does not reread (data: `research/investigations/aof-rewrite/scan-pace.md`).
- rejected: an agent proposed keeping the reread and only yielding by time slice during it; the owner rejected it, because the switch would still read the whole history, and the longer the history, the more new writes pile up.
- replaced: Replaying the current file into a private keyspace to check its blocks at the switch was on main; the owner changed it, because that builds a second copy of the dataset before the switch, while syntax and block classification need no keyspace.

### decision aof-rewrite-answers-started-at-admission
BGREWRITEAOF replies that the rewrite started once S0 admits it; a later failure shows only in INFO, as aof_last_bgrewrite_status:err.
- rejected: an agent proposed replying only once the rewrite completes or fails; the owner rejected it, because Redis's BGREWRITEAOF answers as soon as the rewrite starts in the background.

### decision aof-rewrite-refused-while-appendonly-is-off
BGREWRITEAOF with appendonly off answers Redis's generic error that a rewrite cannot start.
- rejected: an agent proposed answering with an unknown-command error or claiming that a rewrite started; the owner rejected it, because no rewrite can start then: publishing one needs an authoritative incremental file.

### decision aof-rewrite-installation-holds-off-shutdown
Installation and accepting a shutdown are ordered by the aof_installing flag in Meta: a shutdown accepted first cancels installation, and one arriving later waits until installation ends.
- rejected: an agent proposed letting the acceptance of a shutdown race the publication of the install manifest; the owner rejected it, because Redis's event loop also finishes its install handler before it runs SHUTDOWN, and file operations cannot run inside Whitefoot atomic statements.