Decision: firn's EXEC runs a transaction's queued commands in order by their command parts, in one atomic statement holding every key and the keyspace's metadata, with the time frozen at the instant EXEC began and their file records wrapped in MULTI and EXEC when they are more than one command, because Redis 7.0.15 lets no other client's command come between a transaction's commands, freezes the time of commands nested in EXEC, and propagates a transaction so, and the parts are the implementation scripts share with the network path ([command-parts](command-parts.md)), instead of running each queued command through the network path in a statement of its own.

Decision: A command a transaction cannot run by its parts is refused when it is sent, with firn's error that it does not know the command or does not yet run it in transactions, so that EXEC aborts the transaction, as Redis aborts one after an unknown command, because firn names its commands only in the dispatcher's rows and a transaction that queued such a command could only run part of itself, instead of queueing every command the dispatcher knows and failing at EXEC.

Decision: A connection's transaction is kept apart from its client record and passed with it to each command, because EXEC reads the queued commands while their replies are written into the client record, instead of keeping the queue inside the client record.

Rejected:
- Queueing every known command and running those without parts outside the statement: rejected because such a transaction is no longer atomic, which is what the selected consumers rely on.
- WATCH and UNWATCH now: rejected because no selected consumer sends them; WATCH inside a transaction is refused as Redis refuses it.
