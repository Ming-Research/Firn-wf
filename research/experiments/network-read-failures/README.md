# Instruments for the network cases' read failures

They serve the investigation
[`research/investigations/firn/network-read-failures.md`](../../investigations/firn/network-read-failures.md),
which holds the question, the predictions and the results.

| File | What it is |
| --- | --- |
| `sigchld_eintr.rs` | One process in which a thread starts `/bin/true` children while another waits in timed reads on an idle connection; it counts the reads that end interrupted, with SIGCHLD open or blocked in every thread. |
| `trace_summary.py` | Reads a kernel trace of SIGCHLD and says, for the processes named, how many were discarded, queued or already pending, and which threads dequeued one. |
| `soft-limit-stall.patch` | Changes `tests/pubsub.rs` for the experiment only: the soft-limit case reads its 14 MiB frames with one-second bounds, prints each frame's read time and longest wait, and prints the server's sockets, threads and rings for every second without a byte. |
| `soft_summary.py` | Summarises the runs of the patched case. |
| `run.sh` | Runs each experiment: `environment`, `mechanism`, `suite MINUTES`, `soft-limit MINUTES INTERVAL`. |

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

The job's log holds what the experiment printed. `run.sh` exits with 0 when
its instruments worked, whatever the cases did: a failing pass is a result,
which the log says.
