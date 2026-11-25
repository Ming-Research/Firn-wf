# Transactions

How firn queues a transaction's commands under MULTI and runs them at EXEC.

### decision exec-runs-queue-in-one-statement
EXEC runs the queued commands in order, a CLIENT subcommand by its shared body, in one atomic statement holding every key, Meta, the client table and, if needed, the event hub, with one time for all.
- rejected: an agent proposed running each queued command through the network path in a statement of its own; the owner rejected it, because Redis lets no other client's command come between a transaction's commands.
- rejected: an agent proposed queueing every known command and running those without parts outside the statement; the owner rejected it, because such a transaction is no longer atomic, which is what the selected consumers rely on.

### decision transaction-admission-before-queueing
Admission identifies a command and checks its arity before authentication and eviction, and in a transaction queues a name firn does not run, so that EXEC refuses the whole transaction.
- rejected: an agent proposed refusing, when sent, a command without parts or a name firn does not identify; the owner rejected it, because a client that stops at the error never reaches EXEC and stays in the transaction, which turned 94 passing tests of Redis's own suite into failures.

### decision transaction-queue-limit
A transaction queues at most 1 GiB of arguments, and as many arguments and commands, and a command past the limit is refused when sent, so that EXEC aborts the transaction.
- rejected: an agent proposed queueing past the limit; the owner rejected it, because the queue's spaces grow no further than 2^30 elements, so a command that does not fit whole would lose part of its arguments.

### decision transaction-kept-apart-from-client
A connection's transaction is kept apart from its client record and passed with it to each command.
- rejected: an agent proposed keeping the queue inside the client record; the owner rejected it, because EXEC reads the queued commands while it writes their replies into the client record.

### decision transaction-persistence-refusal
A write or PING refused for a saved file error dirties an open MULTI, and EXEC discards a queue holding a WRITE command with EXECABORT and the MISCONF text, checked again under its Meta hold.
- rejected: an agent proposed running the writes of a queue that was accepted while the file was healthy; the owner rejected it, because Redis 7.0.15 checks the saved error again at EXEC and discards a queue that writes.
- rejected: an agent proposed refusing a read-only EXEC as well; the owner rejected it, because Redis 7.0.15 still runs a queue that does not write.

### decision transactions-have-no-watch
firn has no WATCH or UNWATCH: WATCH outside a transaction is answered as an unknown command, and inside one it is refused as Redis refuses it.
- rejected: an agent proposed WATCH and UNWATCH now; the owner rejected it, because no selected consumer sends them.

### decision exec-runs-client-subcommands-in-the-queue
EXEC runs a queued CLIENT subcommand through the same non-waiting body as the network path, inside its statement, and passes the queue's length for the multi field.
- rejected: an agent proposed refusing a transaction that queues a CLIENT subcommand with EXECABORT; the owner rejected it, because Redis 7.2.16's CLIENT descriptors omit NO_MULTI and execCommand runs each queued command, so a refusal loses replies and connection changes Redis applies.

### decision exec-publishes-connection-facts-before-releasing
EXEC publishes its connection's changed facts to the client table before its atomic statement ends.
- rejected: an agent proposed publishing them afterward in the network loop; the owner rejected it, because another connection could then see the transaction's new keys with the connection's old name or library facts.
