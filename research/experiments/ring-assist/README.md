# Ring-assist witness

A minimal Whitefoot server, independent of firn's code, that shows whether a
receive the kernel has already completed waits for a computing context on the
same driver while another driver is idle.

## Question

With two drivers (`WF_DRIVERS=2`), one context computes for about two seconds
without waiting and another context's receive completes 100 ms into that
computation. Does the second context answer within 100 ms, or only after the
computation ends?

At the pinned release a started context is made ready on its starter's
driver, an idle driver takes only ready contexts from another, and a ring
completion that needs the submitting thread stays with the driver that owns
the ring: an idle driver's borrow pass handles due sleeps and records already
published, and the monitor's handoff to a spare thread fires only on an
overdue deadline. A receive with no deadline parked on the computing driver
therefore waits for the whole computation.

## Program and driver

`ring.wf` listens on 127.0.0.1 at the port its argument names, accepts
exactly two connections and spawns one context for each, as firn spawns
`serve`. A connection whose first line is `go N` runs N steps of a 64-bit
linear congruential sequence, the runtime-contention witnesses' `step`, with
no I/O, atomic statement or sleep, then sends the result and a newline and
closes. The other connection sends back each line it receives until the peer
finishes sending.

`driver.py` starts the program for each run with `WF_DRIVERS=2` and
`WF_SCHED_REPORT=2`, connects two sockets, the first then the second, waits
100 ms so both contexts park in their receives, sends `go N` on one, waits
100 ms and sends `ping` on the other. Odd runs send `go` on the first socket
and even runs on the second, so both accept orders get the computation.
N comes from a calibration at the start: the go connection alone with a
growing N until one run lasts at least 250 ms, scaled to `TARGET_MS` (2000
by default); `STEPS` fixes N instead.

Which driver each context parks on is the runtime's choice, not the
program's: both start on the accepting root's driver, and the idle driver may
take one while it is ready. Sharing is therefore likely, not forced; each
run's `WF_SCHED_REPORT=2` counts in `runs/run-N.err` show where contexts ran.

## Expectation and acceptance

- Under the gap: whenever the two contexts share a driver, the echo arrives
  only after the result, so `echo_ms` is close to `compute_ms` minus 100 and
  `echo_before_compute` is 0; this is expected in roughly half the runs or
  more.
- With the gap closed: every run's echo arrives within 100 ms of its ping.

The acceptance condition is the second: `driver.py`, and so the workflow
step, exits 1 when any run's `echo_ms` exceeds 100 (`LIMIT_MS`), and 2 when a
run broke before it could be judged.

## Reading results.csv

One row per run, written as the run ends:

- `run`: the run's number; odd runs computed on the first-connected socket.
- `echo_ms`: milliseconds from sending `ping` to receiving its echo.
- `compute_ms`: milliseconds from sending `go N` to receiving the result.
- `echo_before_compute`: 1 when the echo arrived before the result.

An empty field means that answer never came; `driver.err` and the run's
files under `runs/` say why.

## Files

- `modules.wfg`, `module.wfm`, `ring.wf`: the program, entry `ring_assist`.
- `driver.py`: the client driver; its docstring lists its settings.
- `run.sh --out DIR`: builds the program with `--full-lto` and the compiler
  release `whitefoot.pin` names, records `host.txt` and runs `driver.py`.
- `run.env`: the settings of the temporary workflow
  `.github/workflows/ring-assist.yml` (`runner`, github or 14900k, and
  `runs`); a push that changes it starts the workflow, which uploads the
  CSV, `driver.err` and `runs/`.

## Purpose and lifetime

The witness serves the Whitefoot request for this runtime gap. When that
request closes, this directory is removed, or kept as its acceptance test if
Whitefoot adopts it as one.
