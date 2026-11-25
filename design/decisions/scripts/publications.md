# Script publications

What a script attempt does when it publishes or reads the pub/sub registries, and when its publications reach subscribers.

### decision script-publish-retries-in-whole-map-mode
A declared-key attempt that reaches PUBLISH, SPUBLISH or PUBSUB introspection retries in whole-map mode before it reads the registry, and every whole-map attempt holds the event hub from its start.
- rejected: an agent proposed taking the event hub inside the running attempt when it first publishes; the owner rejected it, because a declared-key attempt can undo its earlier writes, while a whole-map attempt has no general way to undo them before a late, wider hold.
- rejected: an agent proposed restarting a whole-map attempt that has written when it reaches a publication; the owner rejected it, because a whole-map attempt has no general way to undo the writes it already made.

### decision script-publications-staged-until-final-reply
A script's publications pick their recipients and counts under the attempt's hub hold, but are delivered only once the script's final result or error reply is formed.
- rejected: an agent proposed sending each publication when its redis.call runs; the owner rejected it, because an attempt that is abandoned and run again must publish nothing.
- rejected: an agent proposed dropping a script's publications when Lua raises an error; the owner rejected it, because a Lua error keeps the effects before it, so the subscriber gets the message and the caller the error, as in Redis.