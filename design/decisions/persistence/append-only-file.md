# Append-only file

How firn replays its append-only file at startup, cuts a damaged end, writes each command's records to it, and when it syncs it.

### decision aof-multi-record-command-grouped-at-commit
An ordinary command that appends several records is wrapped in MULTI and EXEC at its commit, by inserting MULTI at the log length it noted at its start; one record stays bare.
- rejected: an agent proposed writing each command's records to a temporary buffer and copying them into the log; the owner rejected it, because every write command would then pay one more allocation and copy on its hot path.
- rejected: an agent proposed inserting MULTI as soon as a command's second record arrives; the owner rejected it, because every encoder would then have to prove the group's start lies in the log, which the rewrite code cannot, as Whitefoot cannot carry such a fact through a struct field.

### decision aof-sync-policy-everysec-only
firn syncs its append-only file only as Redis's appendfsync everysec does, and refuses the appendfsync directive.
- rejected: an agent proposed adding Redis's always and no modes; the owner rejected it, because no selected consumer sets appendfsync, and always needs a durability barrier on the reply path and a Whitefoot way for the writer to end the process, two to three times the work.