# Instruments for the network cases' read failures

They serve the investigation
[`research/investigations/firn/network-read-failures.md`](../../investigations/firn/network-read-failures.md),
which holds the question, the predictions and the results.

| File | What it is |
| --- | --- |
| `sigchld_eintr.rs` | One process in which a thread starts `/bin/true` children while another waits in timed reads on an idle connection; it counts the reads that end interrupted, with SIGCHLD open or blocked in every thread. |
| `trace_summary.py` | Reads a kernel trace of SIGCHLD and says, for the processes named, how many were discarded, queued or already pending, and which threads dequeued one. |
| `soft-limit-stall.patch` | Changes `tests/pubsub.rs` for the experiment only: the soft-limit case reads its 14 MiB frames with one-second bounds, prints each frame's read time and longest wait, and prints the server's sockets, threads and rings for every second without a byte. It also adds the case `stall_loop`, which repeats the step that stalled (publish 14 MiB to a subscriber that is not reading, wait, read) without the long waits. |
| `soft_summary.py` | Summarises the runs of the patched case and the rounds of `stall_loop`. |
| `tcp_window_reopen.rs` | Pairs of loopback sockets with no server of ours: a writer writes a 14 MiB frame, the reader leaves it unread for the case's waits and then reads it as the case does, and says how long each read waited for a byte and what the kernel says of both sockets during a wait. |
| `run.sh` | Runs each experiment: `environment`, `mechanism`, `suite MINUTES`, `soft-limit MINUTES INTERVAL [RECEIVE_MAX]`, `stall-loop MINUTES PROCESSES WAITS [RECEIVE_MAX]`, `tcp-reopen MINUTES INTERVAL_MS WAITS [RECEIVE_MAX]`, `tcp-reopen-vm MINUTES KERNEL INTERVAL_MS WAITS [RECEIVE_MAX]`. RECEIVE_MAX sets the most the kernel may grow a TCP receive buffer to for the mode. `tcp-reopen-vm` runs `tcp_window_reopen.rs` as the only program of a QEMU machine that boots KERNEL: an Ubuntu kernel package by its name, which apt downloads, or `host` for the kernel the runner itself runs. |

## How a run is made

The experiments need the gate's kind of machine, GitHub's ubuntu-24.04 with
4 processors, and root for the kernel's tracing. The engine of wf-agents runs
a named test of the Makefile on a commit it is given, and runs
`aof-sync-fault` on that kind of machine (`fast.yml`, job `github`). So each
run is asked for on a commit that is not on any branch, whose Makefile gives
`aof-sync-fault` the experiment as its recipe, for example:

```make
aof-sync-fault: firn-test
	$(ROOT)/research/experiments/network-read-failures/run.sh environment
	$(ROOT)/research/experiments/network-read-failures/run.sh suite 19
```

The job's log holds what the experiment printed, and the job's notices repeat
the first and last of it, for when the log cannot be read. `run.sh` exits
with 0 when its instruments worked, whatever the cases did: a failing pass is
a result, which the log says.
