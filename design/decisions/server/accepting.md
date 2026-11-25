# Accepting connections

What firn's accept loop does when accept fails: whether it keeps serving, how it paces the retry, and how it reports the error.

### decision accept-errors-keep-the-listener-open
An accept error leaves the listener open and accepting goes on, and only the orderly stop, a recorded shutdown or the test client count ends the loop.
- replaced: closing the listener on any accept error, then exiting 0 once clients left, was on main; the owner changed it, because one error such as running out of descriptors then silently ended service, where Redis 7.2.16 logs it and keeps serving.

### decision accept-errors-pause-100-ms
Every accept error except cancellation and ConnectionAborted waits 100 ms on the orderly-stop watch before accept is tried again.
- rejected: an agent proposed retrying at once on resource exhaustion; the owner rejected it, because Linux can return EMFILE before it checks for a pending peer, so an exhausted listener with no peer would spin the CPU.
- rejected: an agent proposed pausing only for resource exhaustion and retrying other errors at once; the owner rejected it, because errors such as EBADF or ENOTSOCK can also repeat with no readiness event, and would spin too.

### decision accept-error-diagnostics-share-one-limiter
All accept errors share one diagnostic limiter: the first is written at once, later ones at most once a second with the count suppressed since the last.
- rejected: an agent proposed writing every accept failure; the owner rejected it, because descriptor exhaustion must stay visible without filling stderr at an unbounded rate.

### decision accept-error-diagnostics-are-bounded
Each accept-error diagnostic is at most 512 bytes and gets one 100 ms deadline across partial writes and the orderly-stop watch, and an error or no progress drops the rest.
- rejected: an agent proposed blocking until every diagnostic is written; the owner rejected it, because a blocked or failing stderr must not hold up accepting or shutdown.
