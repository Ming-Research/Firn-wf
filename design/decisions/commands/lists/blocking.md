# Blocking list commands

How BLPOP, BRPOP, BRPOPLPUSH and BLMOVE wait for a list and are served: the holds a wait and a push take, what a pending request keeps, and how waits reach scripts and the append-only file.

### decision blocking-wait-registers-under-the-keyspace-hold
A failed list lookup and its waiter's FIFO registration share one whole-keyspace, Meta and event-hub hold, and a producer serves ready waiters after its whole command commits, still under that hold.
- rejected: an agent proposed waking each waiting connection's owner to race for the values; the owner rejected it, because rescanning the original keys, FIFO order and chains of moves must come before any competing consumer.

### decision blocking-list-creation-reads-a-waiter-bound
A push or move that would create a list reads under Meta a bound on the keys clients wait on, and only if it is not zero asks the event hub about that key; other list writes hold only their keys.
- rejected: an agent proposed holding the whole keyspace for every list producer; the owner rejected it, because LPUSH, RPUSH, LINSERT and LMOVE then ran at 0.31 of their rate at depth 16 (data: `research/investigations/firn/blocking-lists-performance.md`).
- rejected: an agent proposed that hold for every write that can create a list; the owner rejected it, because a push creating its list then ran at 0.313 at depth 16, though a creation where nobody waits wakes nobody (data: `research/investigations/firn/blocking-lists-performance.md`).
- rejected: an agent proposed a second statement taking the event hub for every creation; the owner rejected it, because with the file on a creating push ran at 0.90 at depth 16, against 1.00 for the bound (data: `research/investigations/firn/blocking-lists-performance.md`).
- rejected: an agent proposed not merging until pushes and the hand-off met the acceptance line; the owner rejected it, because four versions did not remove the loss and the hand-off gap lies in the runtime's listening lock (data: `research/investigations/firn/blocking-lists-performance.md`).

### decision blocking-producers-stage-their-effects
A push or move that creates a list where a client waits, RENAME, RENAMENX, COPY and a promoted connection's list writes stage their effects in a buffer and copy them into the log.
- rejected: an agent proposed an in-place path for a producer that serves a blocked client; the owner rejected it, because it needs separate parsing, replies, expiry records and rewrite capture to save one allocation and copy (data: `research/investigations/firn/blocking-lists-performance.md`).
- rejected: an agent proposed staging into a buffer the connection owns and reuses; the owner rejected it, because its lifetime and re-entry would have to be handled by EXEC, scripts and served waiters, which share one connection's state, to save one small allocation.

### decision blocking-pending-request-keeps-no-client
A pending request keeps owned arguments, protocol and identity, deadline, key memberships and scalar random and peak state; the suspended command keeps its Client, and service uses a temporary one.
- rejected: an agent proposed retaining a second Client through the whole suspension; the owner rejected it, because a pending request needs no reply workspace, session configuration or process meter, and a completed retry must hand the connection's random state back before its pipeline goes on.

### decision blocking-resumed-command-repeats-admission
Each resumed command repeats memory and persistence admission under the producer's hold, and with a finite maxmemory a service first waits out an active rewrite.
- rejected: an agent proposed skipping eviction on replay; the owner rejected it, because producer allocations can change admission.
- rejected: an agent proposed releasing the value-allocation boundary to wait for the rewrite; the owner rejected it, because rewrite cleanup cannot finish while the waiter holds Meta.

### decision blocking-timeout-converts-only-zero
Only the canonical literal 0, an infinite wait, is accepted as a blocking timeout; a finite timeout stays behind one incomplete conversion until Redis's extended-float parse can be reproduced.
- rejected: an agent proposed converting with an exact decimal ceiling or with binary64; the owner rejected it, because neither reproduces Redis's separately rounded extended parse and multiplication, so finite deadlines would silently differ.

### decision blocking-in-scripts-never-registers-a-waiter
In a script a blocking list command takes what is there or returns the nonblocking null, and list readiness stays private to an accepted whole-map attempt until it commits.
- rejected: an agent proposed serving waiters after each redis.call; the owner rejected it, because intermediate pushes may be consumed or deleted within the script, and an abandoned attempt must wake nobody.
