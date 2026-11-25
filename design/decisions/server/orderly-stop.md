# Orderly stop

How firn stops on SHUTDOWN or a stop signal: how waits end, what the signals do, the stop during the AOF replay, and the writer's last attempt.

### decision shutdown-fires-its-own-cancellation
A shutdown request is recorded in server state and fires the current receive wake generation and a separate shutdown cancellation that main, the signal context and the expiry context watch.
- replaced: a deadline of a second at most on each wait, to poll, was on main; the owner changed it, because polling cannot end a wait with no deadline, arms a timer per parked receive and delays exit, 1.103 s against a 500 ms bound (data: `research/investigations/orderly-stop/README.md`).
- rejected: an agent proposed that every context follow the replaceable receive generations; the owner rejected it, because a CONFIG SET timeout must wake receives without ending the accept, signal and expiry waits.
- rejected: an agent proposed cancelling a never-promoted connection's send at shutdown; the owner rejected it, because that would also cut off replies firn sends now, and Redis 7.0.15 promises no flush of ordinary replies at exit.

### decision config-timeout-replaces-the-receive-wake-generation
Server state owns the current receive wake generation, and a CONFIG SET timeout that changes the limit puts a new one in the same statement and fires the old one after it.
- replaced: a receive deadline of at most a second, to notice CONFIG changes, was on main; the owner changed it, because it arms timers and wakes every idle client each second, while replaceable generations deliver a new limit at once without polling.
- rejected: an agent proposed applying a new timeout only after a client's next request; the owner rejected it, because Redis applies it to connections already idle, so firn's replies would differ from the reference.
- rejected: an agent proposed asking Whitefoot for one wait that watches several cancellations; the owner rejected it, because a cancellation fires only once, so the change source would still need replacing, and the language change would save nothing.

### decision shutdown-save-takes-the-failed-save-path
SHUTDOWN SAVE takes Redis 7.0.15's failed-save path: it is refused unless FORCE is also given.
- rejected: an agent proposed ignoring SAVE; the owner rejected it, because firn writes no snapshot, so a save can only fail, and Redis answers a failed save on this path.
- rejected: an agent proposed answering SHUTDOWN SAVE with a syntax error; the owner rejected it, because firn writes no snapshot, so a save can only fail, and Redis answers a failed save on this path.

### decision stop-signals-request-the-orderly-stop
SIGTERM and SIGINT request the orderly stop that SHUTDOWN requests, through one stop listener that main opens before the AOF replay and a context waits on without a deadline.
- replaced: a signal context that polled the request every tenth of a second was on main; the owner changed it, because the shared shutdown cancellation now ends the signal wait when another context requests shutdown.
- rejected: an agent proposed replaying the AOF before opening the stop listener, so that a file that does not load is reported first; the owner rejected it, because a signal could then not stop the replay, and firn would replay, open the next increment and write the manifest only to stop.

### decision first-stop-signal-restores-the-host-default
On the first stop signal the signal context closes its listener before it records the request, so any later SIGTERM or SIGINT ends firn at once.
- rejected: an agent proposed keeping the listener through the drain and ignoring a second SIGTERM, as Redis does; the owner rejected it, because if the stop then hangs, for example on a script that never ends, nothing short of SIGKILL could end firn.
- rejected: an agent proposed first giving Whitefoot a way for one context to end the program with a status; the owner rejected it, because firn's signal stop would wait on that language work, and how the other contexts' handles end must be designed first.

### decision stop-request-ends-the-aof-replay
A stop request during the AOF replay ends it at its next check, before the first command and then every 1024th, and firn exits 0 without writing appendonlydir.
- replaced: a stop signal during the replay ending firn by the host default was on main; the owner changed it, because Redis 7.2.16 exits with status 0 there and leaves the append-only directory unchanged.
- rejected: an agent proposed finishing the replay and then stopping in order; the owner rejected it, because that writes appendonlydir and delays the stop by the rest of the replay.
- rejected: an agent proposed that the replay poll a cancellation watch of its own; the owner rejected it, because the signal context records the request in server state before it fires its source, so a watch would add a handle without seeing the request sooner.

### decision aof-writer-makes-one-last-attempt-at-exit
After every client leaves, the everysec writer makes one forced append and one sync attempt, drops any bytes still retained, and firn exits 0 even if they fail.
- rejected: an agent proposed a new exit status for a failed last write; the owner rejected it, because Redis 7.0.15 exits with status 0 after failed persistence at shutdown.
