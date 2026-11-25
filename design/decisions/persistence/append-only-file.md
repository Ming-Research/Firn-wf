# Append-only file

How firn replays its append-only file at startup, cuts a damaged end, writes each command's records to it, when replies wait for them, and when it syncs it.

### decision aof-multi-record-command-grouped-at-commit
A command that appends several records is wrapped in MULTI and EXEC at its commit, by inserting MULTI at the log length it noted at its start; one record stays bare.
- rejected: an agent proposed writing each command's records to a temporary buffer and copying them into the log; the owner rejected it, because every write command would then pay one more allocation and copy on its hot path.
- rejected: an agent proposed inserting MULTI as soon as a command's second record arrives; the owner rejected it, because every encoder would then have to prove the group's start lies in the log, which the rewrite code cannot, as Whitefoot cannot carry such a fact through a struct field.

### decision aof-sync-policy-everysec-only
firn syncs its append-only file only as Redis's appendfsync everysec does, and refuses the appendfsync directive.
- rejected: an agent proposed adding Redis's always and no modes; the owner rejected it, because no selected consumer sets appendfsync, and always needs a durability barrier on the reply path and a Whitefoot way for the writer to end the process, two to three times the work.

### decision aof-reply-after-covering-append
With an append-only file, firn sends a reply or a subscriber's frame only once every record it may show is in the file or released by a failed append, as Redis 7.2.16 appends before it replies.
- replaced: replying at once, before a writer context's next append every 10 ms, was on main; the owner changed it, because a SIGKILL then lost about the last 10 ms of acknowledged writes, which Redis 7.2.16 keeps.

### decision aof-group-commit-through-a-desk
A connection whose replies must wait takes the active file from a shared desk and appends every record collected so far; the connections waiting meanwhile send once the mark it publishes covers them.
- rejected: an agent proposed a writer context woken for each batch that hands it back (Firn-wf#97); the owner rejected it, because the handoff cut AOF throughput to 0.21-0.69 of main, 8-core SET to 0.37 of Redis (data: `research/investigations/aof-write-errors/reply-after-append.md`).

### decision aof-reply-dependency-is-the-commit-sequence
A reply waits for the commit sequence its command's own Meta statement noted, or else one Meta statement reads before the send; a subscriber's frame waits for the one read before it.
- rejected: an agent proposed that a reply wait only for its own command's records (Firn-wf#97); the owner rejected it, because a read could then show another client's write that a kill loses.
