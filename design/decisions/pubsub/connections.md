# Subscribed connections

How a connection changes owners at its first subscription, what a subscribed connection holds while it runs a command, how its output is charged, and how it ends.

### decision pubsub-connection-promoted-at-first-subscription
A connection keeps one receive, execute and send owner until its first successful subscription or actual blocking-list suspension, then moves once to separate owners.
- rejected: an agent proposed splitting every connection into separate owners when it is accepted; the owner rejected it, because connections that neither subscribe nor block keep their path at no extra cost, and only a subscriber must get pushes and see end of input while a command or a send waits.

### decision pubsub-promoted-command-holds-keyspace-through-reply
On a subscribed or blocked connection, a data command, a data-only EXEC and a declared-key script included, holds the whole keyspace, Meta and the event hub until its whole reply is committed.
- rejected: an agent proposed committing the reply after releasing the data hold; the owner rejected it, because a client that saw the command's effect could then publish a message that reaches the subscriber before the command's reply.
- rejected: an agent proposed reserving a reply position before taking the data hold; the owner rejected it, because a GET could then see a preceding SET and PUBLISH while its reserved reply still came before that publication.
- rejected: an agent proposed adding output commitment to every ordinary entry-handler interface; the owner rejected it, because the existing whole-map command parts already give that protected boundary.

### decision pubsub-promoted-path-chosen-at-compile-time
The shared admission code and evaluator are instantiated by a compile-time constant into an ordinary path and a subscribed path.
- rejected: an agent proposed one path that checks at run time whether the connection is subscribed; the owner rejected it, because connections that never subscribe must keep their original handlers and shared targets, with no runtime check or wrapper call.

### decision pubsub-promoted-info-replies-under-the-hold
On a subscribed connection, INFO takes its key-count, script-registry and server-state snapshots and commits its whole reply in one statement under the keyspace, Meta and hub hold.
- replaced: INFO releasing the data after its key-count snapshot and committing its reply later was on main; the owner changed it, because a later SET and PUBLISH transaction could then reach the subscriber before an INFO reply that still showed the empty database.

### decision pubsub-output-charged-by-reserved-capacity
A subscribed connection's output limit is charged the reserved capacity of each outstanding frame's prefix and body until the whole frame is released, and a shared body in full to every recipient.
- rejected: an agent proposed charging only the wire bytes queued; the owner rejected it, because a count of wire bytes can miss the spare capacity a frame keeps.
- rejected: an agent proposed releasing the charge when the dispatcher hands a frame on; the owner rejected it, because the charge must not be lost while the frame moves through the hub, the dispatcher, the inbox and the socket owner.

### decision pubsub-connection-failure-invalidates-before-waking
A receive or send failure, or removal for output pressure, invalidates the connection's membership before it wakes the owners; QUIT and a protocol error end the output after their reply.
- rejected: an agent proposed rolling back a completed command when the network fails; the owner rejected it, because cancellation, a command's accepted effects and the delivery of its reply are separate boundaries.
- rejected: an agent proposed waiting for a child to end before enabling its cleanup; the owner rejected it, because cancellation and a child's completion are separate boundaries.