#!/usr/bin/env python3
"""Client driver for the ring-assist witness (README.md beside this file).

    driver.py BINARY OUT_DIR

Settings, from the environment:
  RUNS       runs to make (default 10)
  STEPS      the step count N each run sends; when unset it is calibrated
  TARGET_MS  the compute time calibration aims N at (default 2000)
  LIMIT_MS   the longest echo delay a run may show (default 100)

Each run starts BINARY on a free port with WF_DRIVERS=2, connects two sockets,
the first then the second, waits 100 ms, sends "go N" on one of them, waits
100 ms and sends "ping" on the other, and records when each answer arrives.
Odd runs send "go" on the first socket and even runs on the second. It writes
OUT_DIR/results.csv, one row per run as it ends,
    run,echo_ms,compute_ms,echo_before_compute
and each run's standard output and error in OUT_DIR/runs/. It exits 1 when
any run's echo came more than LIMIT_MS after the ping, and 2 when a run broke
(no answer, a closed socket or a nonzero exit) before that could be judged.
"""
import os
import selectors
import socket
import subprocess
import sys
import time

RUNS = int(os.environ.get("RUNS", "10"))
TARGET_MS = float(os.environ.get("TARGET_MS", "2000"))
LIMIT_MS = float(os.environ.get("LIMIT_MS", "100"))
SETTLE_S = 0.1
GAP_S = 0.1


def log(message):
    print(message, file=sys.stderr, flush=True)


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def connect(port, deadline):
    while True:
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=1)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            return sock
        except (ConnectionRefusedError, socket.timeout):
            if time.monotonic() > deadline:
                raise
            time.sleep(0.01)


class Server:
    """One run of the witness program, with its two client sockets."""

    def __init__(self, binary, out, name):
        self.port = free_port()
        env = dict(os.environ, WF_DRIVERS="2", WF_SCHED_REPORT="2")
        self.stdout = open(os.path.join(out, "runs", name + ".out"), "wb")
        self.stderr = open(os.path.join(out, "runs", name + ".err"), "wb")
        self.proc = subprocess.Popen([binary, str(self.port)], env=env,
                                     stdout=self.stdout, stderr=self.stderr)
        deadline = time.monotonic() + 10
        self.first = connect(self.port, deadline)
        self.second = connect(self.port, deadline)
        time.sleep(SETTLE_S)

    def finish(self, timeout):
        """Closes both sockets and returns the program's exit status, or None
        when it had not exited within timeout seconds."""
        for sock in (self.first, self.second):
            try:
                sock.shutdown(socket.SHUT_WR)
            except OSError:
                pass
        for sock in (self.first, self.second):
            sock.settimeout(timeout)
            try:
                while sock.recv(4096):
                    pass
            except OSError:
                pass
            sock.close()
        try:
            status = self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
            status = None
        self.stdout.close()
        self.stderr.close()
        return status


def await_lines(waits, timeout):
    """waits maps a socket to the bytes that end its answer; returns the
    monotonic nanoseconds each answer ended at, None for one that did not."""
    selector = selectors.DefaultSelector()
    held = {}
    arrived = {}
    for sock in waits:
        sock.setblocking(False)
        selector.register(sock, selectors.EVENT_READ)
        held[sock] = b""
    end = time.monotonic() + timeout
    while len(arrived) < len(waits):
        left = end - time.monotonic()
        if left <= 0:
            break
        for key, _ in selector.select(left):
            sock = key.fileobj
            try:
                data = sock.recv(4096)
            except BlockingIOError:
                continue
            now = time.monotonic_ns()
            if not data:
                selector.unregister(sock)
                arrived.setdefault(sock, None)
                continue
            held[sock] += data
            if sock not in arrived and waits[sock] in held[sock]:
                arrived[sock] = now
    selector.close()
    for sock in waits:
        sock.setblocking(True)
    return {sock: arrived.get(sock) for sock in waits}


def calibrate(binary, out):
    """Times the go connection alone, with the other closed at once, until a
    run lasts at least 250 ms, and scales its step count to TARGET_MS."""
    steps = 50_000_000
    for attempt in range(1, 8):
        server = Server(binary, out, "calibrate-%d" % attempt)
        server.second.shutdown(socket.SHUT_WR)
        sent = time.monotonic_ns()
        server.first.sendall(b"go %d\n" % steps)
        done = await_lines({server.first: b"\n"}, 120)[server.first]
        status = server.finish(10)
        if done is None or status != 0:
            log("calibration run %d broke: answer %s, status %s" % (attempt, done, status))
            sys.exit(2)
        elapsed_ms = (done - sent) / 1e6
        log("calibration: %d steps in %.1f ms" % (steps, elapsed_ms))
        if elapsed_ms >= 250:
            return max(1, int(steps * TARGET_MS / elapsed_ms))
        steps *= 4
    log("calibration never reached 250 ms")
    sys.exit(2)


def main():
    if len(sys.argv) != 3:
        log("usage: driver.py BINARY OUT_DIR")
        sys.exit(2)
    binary, out = sys.argv[1], sys.argv[2]
    os.makedirs(os.path.join(out, "runs"), exist_ok=True)
    steps = int(os.environ["STEPS"]) if os.environ.get("STEPS") else calibrate(binary, out)
    log("steps: %d  runs: %d  limit_ms: %g" % (steps, RUNS, LIMIT_MS))
    late = False
    broken = False
    csv = open(os.path.join(out, "results.csv"), "w")
    header = "run,echo_ms,compute_ms,echo_before_compute"
    print(header, file=csv, flush=True)
    print(header, flush=True)
    for run in range(1, RUNS + 1):
        server = Server(binary, out, "run-%d" % run)
        if run % 2 == 1:
            compute, echo = server.first, server.second
        else:
            compute, echo = server.second, server.first
        go_sent = time.monotonic_ns()
        compute.sendall(b"go %d\n" % steps)
        time.sleep(GAP_S)
        ping_sent = time.monotonic_ns()
        echo.sendall(b"ping\n")
        arrived = await_lines({compute: b"\n", echo: b"ping\n"}, 10 * TARGET_MS / 1000 + 30)
        status = server.finish(10)
        echo_at, compute_at = arrived[echo], arrived[compute]
        echo_ms = "" if echo_at is None else "%.1f" % ((echo_at - ping_sent) / 1e6)
        compute_ms = "" if compute_at is None else "%.1f" % ((compute_at - go_sent) / 1e6)
        before = ""
        if echo_at is not None and compute_at is not None:
            before = "1" if echo_at < compute_at else "0"
        row = "%d,%s,%s,%s" % (run, echo_ms, compute_ms, before)
        print(row, file=csv, flush=True)
        print(row, flush=True)
        if echo_at is None or compute_at is None or status != 0:
            log("run %d broke: echo %s, compute %s, status %s" % (run, echo_at, compute_at, status))
            broken = True
        elif (echo_at - ping_sent) / 1e6 > LIMIT_MS:
            late = True
    csv.close()
    if broken:
        sys.exit(2)
    if late:
        log("an echo came more than %g ms after its ping" % LIMIT_MS)
        sys.exit(1)


if __name__ == "__main__":
    main()
