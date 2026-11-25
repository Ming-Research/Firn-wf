# Script attempts

How a script's attempts hold keys and Meta, retry, roll back and stop for SCRIPT KILL, and what an attempt checks before its first write.

### decision script-attempt-is-one-budgeted-statement
A script runs in attempts, each one atomic statement with a budget of interpreter steps; an attempt that runs out before any effect is dropped and run again with twice the budget.
- rejected: an agent proposed releasing the statement around each of a script's commands; the owner rejected it, because another client's command could then come between them, which Redis does not allow.
- rejected: an agent proposed keeping a written script's progress between statements; the owner rejected it, because Redis lets no other client's command come between a script's commands, and a write cannot be taken back.

### decision script-holds-declared-keys-then-falls-back
A script on a never-subscribed connection first holds its declared KEYS and Meta; a command needing another key or the whole map drops the attempt untouched and reruns it on the whole keyspace.
- replaced: A script holding every key and Meta for its whole run was on main; the owner changed it, because then two scripts could not run at once even on two engines, and two cores served scripts more slowly than one.
- rejected: an agent proposed answering an error when a script that has written reaches an undeclared key; the owner rejected it, because standalone Redis allows a script to use keys it did not declare.

### decision script-fallback-restores-logical-copies
Before a command first changes a declared entry, the attempt saves a logical copy, and a fallback restores its members, values, scores, list order, expiry and stamps, but not the collection's layout.
- rejected: an agent proposed copying every declared entry into a private copy when the attempt starts; the owner rejected it, because large keys the script never uses would then be copied too.
- rejected: an agent proposed an exact snapshot of each collection's layout; the owner rejected it, because collection scans return the whole collection with cursor 0, so no cursor spans an attempt, no result depends on layout, and a rehash already changes it.

### decision script-attempts-hold-meta-throughout
A script attempt holds Meta for its whole run, so scripts that write different keys still run one at a time.
- rejected: an agent proposed designing a new Meta structure first and making scripts concurrent in one change; the owner rejected it, because a first step that keeps Meta held removes the scripts' wait for the engine sooner.

### decision script-kill-checked-between-attempts
SCRIPT KILL stops a script with no effects when an attempt runs out of budget and the script takes the engine again; it answers OK while any script runs and NOTBUSY while none does.
- rejected: an agent proposed checking for a kill inside an attempt; the owner rejected it, because the attempt holds the keyspace's statement, and the check would nest the pool's statement inside it.

### decision script-oom-checked-before-first-accepted-write
A script refuses a DENYOOM command for memory only before its first accepted WRITE command, which a WRITE_DIRTY flag apart from the effects flag records, by the memory state taken first.
- rejected: an agent proposed treating a write that changes nothing as unwritten for the memory check; the owner rejected it, because Redis's scriptVerifyOOM counts every accepted WRITE command, a DEL of a missing key included.
- rejected: an agent proposed checking memory again inside a script; the owner rejected it, because Redis's scriptVerifyOOM refuses DENYOOM only before the first accepted WRITE command.

### decision script-aof-error-checked-before-first-write
During an AOF error a script may read, but its first accepted WRITE command is refused with MISCONF, raised by redis.call and returned by redis.pcall; its later writes are not checked.
- rejected: an agent proposed refusing scripts at entry while an AOF error stands; the owner rejected it, because Redis lets such a script read and checks only before its first accepted write.