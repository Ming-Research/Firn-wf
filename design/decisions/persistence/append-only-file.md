# Append-only file

How firn replays its append-only file at startup and what it logs then, cuts a damaged end, writes each command's records to it, and when it syncs it.

### decision aof-multi-record-command-grouped-at-commit
An ordinary command that appends several records is wrapped in MULTI and EXEC at its commit, by inserting MULTI at the log length it noted at its start; one record stays bare.
- rejected: an agent proposed writing each command's records to a temporary buffer and copying them into the log; the owner rejected it, because every write command would then pay one more allocation and copy on its hot path.
- rejected: an agent proposed inserting MULTI as soon as a command's second record arrives; the owner rejected it, because every encoder would then have to prove the group's start lies in the log, which the rewrite code cannot, as Whitefoot cannot carry such a fact through a struct field.

### decision aof-sync-policy-everysec-only
firn syncs its append-only file only as Redis's appendfsync everysec does, and refuses the appendfsync directive.
- rejected: an agent proposed adding Redis's always and no modes; the owner rejected it, because no selected consumer sets appendfsync, and always needs a durability barrier on the reply path and a Whitefoot way for the writer to end the process, two to three times the work.

### decision aof-load-failure-logs-one-generic-line
When the append-only file does not load, firn logs Fatal error loading the DB, check server logs. Exiting. and exits with status 4, and the log names no cause.
- rejected: an agent proposed logging Redis's specific line for each cause, from aof.c, and not the generic line; the owner rejected it, because every failure point in startup.wf must be matched with Redis's text and tested, one to two days more.
- rejected: an agent proposed logging the specific line and then the generic one; the owner rejected it, because it costs the same work, and Redis's append-only file path never writes the generic closing line.

### decision aof-replay-leaves-out-two-redis-lines
Replaying the append-only file, firn does not log Redis's Revert incomplete MULTI/EXEC transaction line or its Error truncating the AOF file line, and the manual lists both as differences from Redis.
- rejected: an agent proposed writing both, with a replay-result field that tells an incomplete block from a clean one and a line where cutting the tail fails; the owner rejected it, because it changes persistence's public structure and callers, needs two tests against Redis and takes half a day.