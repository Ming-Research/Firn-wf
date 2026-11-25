# Intermittent socket read failures in the network cases

Item 271. The instruments are in
[`research/experiments/network-read-failures/`](../../experiments/network-read-failures/README.md).

## Question

A network case's test client sometimes fails to read its socket, and the same
commit passes when the job runs again. Which causes are there, is each in the
test harness or in firn, and what removes each?

## What CI's history shows

Read with `gh` on 2026-11-25: the last 400 runs of `check.yml` (2026-10-20 to
2026-11-25), every failed `network tests` job among them (88 logs), and the
two runs the item names. Three kinds of read failure appear.

| Kind | Occurrences | Where |
| --- | --- | --- |
| A read with a timeout fails with `Interrupted` (EINTR) | 2 | run 38429625079 attempt 1, `firn_applies_an_idle_limit_to_a_previously_unlimited_receive`; run 40070874864, `pubsub::active_subscription_families_exempt_idle_timeout_then_last_unsubscribe_closes` |
| `Connection reset by peer` at a case's first request | 8 | runs 38394872575 (2026-10-13, on main), 39521631992, 39522933590, 39904677982, 39939595879, 40093002331, 40853929178 and 40114551912 |
| A read ends at its 10 s bound (`WouldBlock`) while 14 MiB is being received | 2 | runs 39572426334 and 39927345767, both `pubsub::soft_output_limit_interval_restarts_after_the_backlog_drains`, both about 81 s into the case, which is the read of the second frame |

Eight more cases failed once each in the diagnostic runs that
[`test-stability.md`](../aof-rewrite/test-stability.md) lists. Those runs kept
only the line of each panic, not its message, so their kind cannot be
recovered.

Since the gate runs each test through `whitefoot-kit/ci/run-tests.py`
(2026-11-24) no network job has reached its tests and failed, so there is no
failure rate for the gate as it runs now.

## Connection reset: the port lease (settled before this investigation)

`lease_port` in `tests/support.rs` held a leased port with a listening
socket. A child that another test thread started while the listener was open
kept a copy of it until its exec. After the lease dropped its listener the
copy went on accepting: the case's first connection completed against the
copy and was reset when the child exec'd. Commit e3a1a36 holds the port with
a socket that is bound and never listens. Measured then on GitHub-hosted
ubuntu-24.04: with the listening lease 67 and 94 of 10000 connections were
not refused (runs 40112052978, 40112176185), with the bound one 0 of 10000
(run 40112186246). All eight resets are on commits without e3a1a36
(`git merge-base --is-ancestor`). Of the 400 runs, 365 are on commits without
it and 35 on commits with it; none of the 35 has a reset. What is open:
whether any reset still occurs with it.

## EINTR: hypothesis

No test sends the test process a signal, and the signal trace of run
40070874864 shows none sent to it but SIGCHLD, which that trace only counted.
The hypothesis, from the sources of Linux and glibc, has three parts:

1. The kernel discards a signal whose action is the default "ignore", as
   SIGCHLD's is, when it is generated, unless the thread it is sent to blocks
   it: a blocked signal is queued (`sig_ignored` in `kernel/signal.c`). A
   child's exit sends SIGCHLD to the thread that started the child. glibc
   blocks every signal in a thread for the length of `posix_spawn` (from
   before the `clone` until the new child has exec'd), of `pthread_create`
   and of `fork`. So a SIGCHLD is queued when a child of thread T exits while
   T is inside one of them, as when T starts `/bin/kill` to stop a server
   that is already exiting.
2. A queued signal sent to the process wakes one thread that does not block
   it (`complete_signal`): the process's current target, which is its first
   thread until that thread is found not wanting the signal. libtest's first
   thread blocks every signal while it starts a test's thread, so a SIGCHLD
   queued in that window goes to the oldest other thread, a test's, which
   then stays the target until it exits, when the next younger thread
   becomes it.
3. The woken thread dequeues the signal and drops it. A system call it was
   waiting in is restarted, except a socket call with a timeout
   (`SO_RCVTIMEO`, `SO_SNDTIMEO`), which fails with EINTR
   (`sock_intr_errno`). std's `read_exact` and `read_to_end` retry EINTR; a
   bare `read`, `peek` or `accept` reports it.

If this holds, blocking SIGCHLD in the test process's first thread before any
other thread exists removes the cause: every thread inherits the mask, so no
thread can be woken. Waiting for a child does not need the signal. A child
inherits the mask of the thread that starts it: std's source says, on both
its `posix_spawn` and its `fork` path, "Inherit the signal mask from this
process rather than resetting it" (read 2026-11-25 in the stable release's
`std/src/sys/process/unix/unix.rs`). So the servers and the documented shell
commands the cases run would start with SIGCHLD blocked, which is not how
they run outside the tests, unless each child is given the original mask
again before its exec.

### Comparisons and what would reject the hypothesis

`sigchld_eintr.rs`: one process, a spawner thread that starts `/bin/true`
children and a reader in 20 ms timed reads on an idle connection, 3 s a
configuration, traced with the kernel's signal tracepoints.

| Configuration | Prediction | Rejects the hypothesis if |
| --- | --- | --- |
| first thread reads, spawner does not wait | reads interrupted, SIGCHLD queued | no read is interrupted |
| first thread reads, spawner waits for each child | none or very few interrupted | as many as without waiting |
| first thread waits, another thread reads | none interrupted: the first thread dequeues every one | the reader is interrupted |
| first thread starts threads, another thread reads | reads interrupted | none is, with SIGCHLD queued |
| the same with SIGCHLD blocked in every thread | none interrupted, none dequeued, in every configuration | any read is interrupted |
| a child's mask after exec, parent blocking SIGCHLD | SIGCHLD blocked in the child too, by `posix_spawn` and by `fork` | (decides the fix's shape, not the hypothesis: an empty mask would mean no child needs its mask restored) |

In the suite (`run.sh suite`): the trace shows SIGCHLD queued for the test
process and dequeued by threads other than its first. With SIGCHLD blocked,
none is dequeued in the test process. The hypothesis is rejected for the
suite if no SIGCHLD is ever dequeued by a test's thread without the block, or
if one still is with it.

### Results

Measured on 2026-11-25 on GitHub's ubuntu-24.04 runner (4 processors, kernel
7.0.0-1014-aws, glibc 2.39, rustc 1.100.0), with no trace event lost in any
run.

`sigchld_eintr.rs`, 3 s a configuration (run 45611579505):

| First thread | Spawner | SIGCHLD | Children | SIGCHLD queued | Reads interrupted |
| --- | --- | --- | --- | --- | --- |
| reads | does not wait | open | 24293 | 20379 | 9812 |
| reads | waits for each child | open | 9386 | 0 | 0 |
| reads | does not wait, fork and exec | open | 14105 | 0 | 0 |
| waits | does not wait | open | 23664 | 19881 | 6484 |
| starts threads | does not wait | open | 18800 | 16570 | 7466 |
| starts threads | waits for each child | open | 9080 | 4 | 4 |
| starts threads | does not wait, fork and exec | open | 9759 | 0 | 0 |
| reads | does not wait | blocked | 25437 | 1 | 0 |
| starts threads | does not wait | blocked | 18801 | 1 | 0 |
| starts threads | does not wait, fork and exec | blocked | 9827 | 1 | 0 |

The hypothesis holds: a child's exit is queued when its parent thread blocks
signals, a queued SIGCHLD ends another thread's timed read with EINTR, and
with SIGCHLD blocked in every thread one signal stays pending for the life of
the process and no read is interrupted. Three results differ from what was
predicted or add to it:

- A waiting first thread does not protect the others (6484 reads interrupted,
  where none was predicted). A thread that has been woken for a signal and has
  not run yet does not want the next one (`wants_signal`), so a second SIGCHLD
  queued within that time goes to the next thread, which then stays the
  target.
- A child that exits at once is queued even when its parent waits for it
  (4 of 9080): it can exit after its exec and before its parent has restored
  its mask at the end of `posix_spawn`.
- std's `fork` path queued none, with or without waiting: the window is
  `posix_spawn`'s. It is no fix by itself, because a thread also blocks every
  signal while it starts a thread, as the harness does to drain a child's
  output just after starting the child.

A child inherits its parent's mask on both paths: with SIGCHLD blocked in the
parent, `/proc/PID/status` of the child after its exec shows `SigBlk:
0000000000010000`, by `posix_spawn` and by `fork`; with it open, zero.

The suite, `make test` pass after pass with the same trace:

| Test process | Run | Passes | Failed | SIGCHLD per pass: discarded | queued | dequeued |
| --- | --- | --- | --- | --- | --- | --- |
| as on main | 45611579505 | 7 | 0 | 807 to 819 | 2 to 7 (28 in all) | every one queued: 17 by the first thread, 11 by the thread that started the child |
| SIGCHLD blocked in every thread, children inheriting the block | 45611579570 | 6 | 0 | 0 | 1 for each test process, 918 to 952 found already pending | 0 |
| SIGCHLD blocked in every thread, the harness's children given the first mask again before their exec | 45631079729 | 7 | 0 | 0 | 1 for each test process, 806 to 822 found already pending | 0 |

On main, the 28 queued came from children that exit at once: `/bin/kill` 12,
`/bin/sh` 11, `redis-server` 4 and one `firn-test` that was killed. The 11
that the starting thread dequeued itself were dequeued 7 to 123 µs after they
were queued, which is that thread restoring its mask. So in every pass a
SIGCHLD is queued for the test process a few times and wakes a thread that
does not block it. The trace does not record which thread was woken, and no
pass had a read fail: at the rate CI's history shows, 2 in about 400 passes,
seeing one would need hundreds of passes. With the block no thread is left
that can be woken, and none dequeued one. Giving a child its mask again takes
a function that runs in the child before its exec, with which std starts the
child by `fork` instead of `posix_spawn`; a pass took 141 to 142 s with it,
as it took 140 to 142 s on main on the same kind of machine.

The comment above the trace step of `check.yml`, "SIGCHLD ... has no handler
in the test process and so cannot interrupt a read", is wrong.

## The 10 s read stall: hypotheses

Both failures stopped receiving the second 14 MiB frame, which firn had held
for 33 s behind a full socket, on a runner where every other case had already
ended. Each case failed 81.0 s and 81.1 s after it started: its read of the
second frame begins 70 s after the first publication, so the 10 s without a
byte began as soon as the client had read what its own socket held. Both ran
on GitHub's Azure machines with the runner image 20261012.334.1; the image's
kernel is not in the logs. firn's output owner sends through `std::net::send_once`
(`firn/server/promoted.wf`), which on this route is an `IORING_OP_SEND` on
Whitefoot's ring with one driver thread. The candidates:

- **TCP**: the server's socket still holds unsent bytes behind a closed
  window during the stall, so the kernel did not resume.
- **Whitefoot's runtime**: the socket has room, and a completion is unreaped,
  no send is submitted, or the driver sleeps.
- **firn**: the output owner is not sending because something else holds it.

`run.sh soft-limit` runs the case with `soft-limit-stall.patch`, which reads
the frame with one-second bounds and, for each second without a byte, prints
the server's sockets (`ss -tinm`), each of its threads' state and system
call, and its rings' queues. One stalled run tells the candidates apart. It
also prints how long each frame took and the longest wait within it for every
run. If no run stalls, the cause stays unknown, and the bound on its rate is
what the number of runs gives.

`run.sh stall-loop` repeats the step that stalled without the case's long
waits: publish 14 MiB to a subscriber that is not reading, wait 50 ms to 3 s
while the server's send waits for room, on every other cycle ask the server
`PUBSUB NUMSUB` on the publisher's connection as the case does before its
second read, then read the frame the same way. It separates two readings. If
the stall comes from how the runtime resumes a waiting send, the loop shows
it within thousands of rounds, and shows whether the question before the read
matters. If the loop never waits long and only the case with its 33 s wait
does, the length of the wait is part of the cause.

## The gate's failure rate

`run.sh suite` runs `make test`, the gate's network target, pass after pass
on the gate's kind of runner and says how each pass ended and what each
failing case said.

The 13 passes above all passed (140 s a pass on one runner, 153 s on the
other), as did the gate's own run of main that day (run 45610525110). CI's
history is the measure of the rate: the 400 runs hold 10 read failures, of
which 7 were the port lease's, which leaves 3 in about 400 passes, under 1%.
At that rate 100 passes without a failure have a probability of about 47%
with nothing fixed, so a loop of 100 passes cannot by itself show a cause
removed; what shows it is, for each cause, an observation that tells the
fixed state from the unfixed one. Every pass also runs
`leased_port_refuses_connections_while_a_concurrent_spawn_runs`, which
connects to 10000 leased ports and fails if one is not refused: 130000
connections in the 13 passes, none accepted.
