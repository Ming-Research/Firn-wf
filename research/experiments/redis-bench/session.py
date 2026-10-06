"""Records a measurement's session and stops what it leaves.

redis-bench.yml runs each measurement on the shared self-hosted machine in a
session of its own (setsid). `record PATH PID` writes the session's identity
before the measurement starts: the boot, the session's number, which is its
leader's process id, and the leader's start time. `stop PATH` stops the
processes of that session started within WINDOW of its leader, each signalled
through a pidfd opened on the very process found, first TERM and then KILL,
and removes the record once none is left; it exits 1 and keeps the record
when one outlives KILL. A process is told apart by its session and start time
rather than its name or environment, since other projects run the same
servers on this machine and Redis and Valkey overwrite the environment the
kernel shows when they set their process titles.

Linux keeps a session's number from being reused while any process of the
session lives, so a number naming live processes names this session's,
unless all of them ended and the number was reused by a new session: a live
leader with another start time shows that case, and the window bounds it
when that leader has ended too.
"""

import os
import signal
import sys
import time

# The measurement step stops after 110 minutes; a process of the session
# started later than this after its leader belongs to another one.
WINDOW_SECONDS = 3 * 3600


def boot_id():
    with open("/proc/sys/kernel/random/boot_id") as f:
        return f.read().strip()


def identity(pid):
    """A process's session and start time in clock ticks since boot, or None."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            text = f.read()
    except OSError:
        return None
    # The command name, in parentheses, may hold spaces; the fields after it
    # start at the state, the third.
    fields = text[text.rindex(")") + 2 :].split()
    return int(fields[3]), int(fields[19])


def command_line(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode(errors="replace").strip()
    except OSError:
        return "?"


def record(path, pid):
    found = identity(pid)
    if found is None or found[0] != pid:
        sys.exit(f"process {pid} does not lead a session")
    with open(path, "w") as f:
        f.write(f"{boot_id()} {pid} {found[1]}\n")


def stop(path):
    try:
        with open(path) as f:
            booted, session, start = f.read().split()
        session, start = int(session), int(start)
    except FileNotFoundError:
        return
    except (OSError, ValueError):
        print(f"{path} does not read as a record; removed")
        os.remove(path)
        return
    if booted != boot_id():
        print(f"{path} names a session of an earlier boot; removed")
        os.remove(path)
        return
    leader = identity(session)
    if leader is not None and leader[1] != start:
        print(f"session {session} is another one now; record removed")
        os.remove(path)
        return
    latest = start + WINDOW_SECONDS * os.sysconf("SC_CLK_TCK")

    def members():
        found = []
        for name in os.listdir("/proc"):
            if name.isdigit() and int(name) != os.getpid():
                pid = int(name)
                known = identity(pid)
                if known is not None and known[0] == session and start <= known[1] <= latest:
                    found.append((pid, known[1]))
        return found

    for sig, grace in ((signal.SIGTERM, 10), (signal.SIGKILL, 5)):
        left = members()
        if not left:
            break
        for pid, started in left:
            try:
                handle = os.pidfd_open(pid)
            except ProcessLookupError:
                continue
            try:
                # Read after opening: the handle names the process found only
                # if that process still holds the number.
                if identity(pid) == (session, started):
                    print(f"stopping {pid} ({command_line(pid)}) with {sig.name}")
                    try:
                        signal.pidfd_send_signal(handle, sig)
                    except ProcessLookupError:
                        pass
                    except PermissionError:
                        print(f"not permitted to signal {pid}")
            finally:
                os.close(handle)
        deadline = time.monotonic() + grace
        while members() and time.monotonic() < deadline:
            time.sleep(0.1)
    left = members()
    if left:
        named = ", ".join(f"{pid} ({command_line(pid)})" for pid, _ in left)
        sys.exit(f"session {session} kept {named} past KILL; {path} kept")
    os.remove(path)


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "record":
        record(sys.argv[2], int(sys.argv[3]))
    elif len(sys.argv) == 3 and sys.argv[1] == "stop":
        stop(sys.argv[2])
    else:
        sys.exit("usage: session.py record PATH PID | stop PATH")


if __name__ == "__main__":
    main()
