"""Records a measurement's session and stops what it leaves.

redis-bench.yml runs each measurement on the shared self-hosted machine in a
session of its own (setsid), and other projects' runners and servers share
the machine. A process is named by its id and its start time, which together
name one process within a boot; its name or environment would not do, since
other projects run the same servers and Redis and Valkey overwrite the
environment the kernel shows when they set their process titles.

`record PATH PID` writes the session's first line before the measurement
starts: the boot, the session's number, which is its leader's process id, and
the leader's start time. `register PATH parent` adds a line naming the
caller's parent: redis-bench.sh starts each server in a subshell that
registers itself so and then becomes the server by exec, so that a server is
named before it runs. `stop PATH` stops what the session left:

- while its leader runs, the leader and every process of its session, found
  by a scan between two checks that the leader is the recorded one, since
  Linux keeps a session's number from being reused while any process of the
  session lives;
- once the leader has ended, only the servers the record names that still
  run, since the session's number may then name another session.

Each process is signalled through a pidfd opened on the process found, TERM
and then KILL. A record from an earlier boot names nothing left. `stop`
removes the record once nothing it names runs, and exits 1, keeping it, when
a process outlives KILL or the record does not read, so that no measurement
starts on a host it cannot account for. A process of the session that is
not a registered server and outlives its leader, such as a client, carries
the runner's tracking variable, by which the runner stops it when the job
ends.
"""

import os
import signal
import sys
import time


def boot_id():
    with open("/proc/sys/kernel/random/boot_id") as f:
        return f.read().strip()


def identity(pid):
    """A live process's session and start time in clock ticks since boot, or
    None for a process that has ended, a zombie included, since it holds
    nothing and takes no signal."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            text = f.read()
    except OSError:
        return None
    # The command name, in parentheses, may hold spaces and parentheses; the
    # fields after its last parenthesis start at the state, the third.
    fields = text[text.rindex(")") + 2 :].split()
    if fields[0] == "Z":
        return None
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
    try:
        with open(path, "w") as f:
            f.write(f"{boot_id()} {pid} {found[1]}\n")
    except OSError as error:
        sys.exit(f"cannot record session {pid} in {path}: {error.strerror}")


def register(path, pid):
    if pid == "parent":
        pid = os.getppid()
    pid = int(pid)
    found = identity(pid)
    if found is None:
        return
    try:
        with open(path, "a") as f:
            f.write(f"{pid} {found[1]}\n")
    except OSError as error:
        sys.exit(f"cannot register server {pid} in {path}: {error.strerror}")


def read_record(path):
    """The record's boot, session, leader start and registered servers, or
    None when it does not read."""
    try:
        with open(path) as f:
            lines = f.read().splitlines()
        booted, session, start = lines[0].split()
        servers = []
        for line in lines[1:]:
            pid, started = line.split()
            servers.append((int(pid), int(started)))
        return booted, int(session), int(start), servers
    except (OSError, ValueError, IndexError):
        return None


def stop(path):
    if not os.path.exists(path):
        return
    read = read_record(path)
    if read is None:
        sys.exit(f"{path} does not read as a session record; it is kept, and "
                 "a person must stop what it names and remove it")
    booted, session, start, servers = read
    if booted != boot_id():
        print(f"{path} names a session of an earlier boot; removed")
        os.remove(path)
        return

    # Every process shown to be the session's, by its registration or by a
    # scan its leader was running across, stays named by its identity through
    # the rounds, so that one which outlives TERM after its leader has gone
    # still gets KILL. The record is read again each round, for a server
    # registered while the session was being stopped.
    named = {}

    def targets():
        again = read_record(path)
        for pid, started in (again[3] if again is not None else servers):
            named.setdefault(pid, (session, started))
        if identity(session) == (session, start):
            found = {}
            for name in os.listdir("/proc"):
                if name.isdigit() and int(name) != os.getpid():
                    pid = int(name)
                    known = identity(pid)
                    if known is not None and known[0] == session:
                        found[pid] = known
            if identity(session) == (session, start):
                named.update(found)
        return [(pid, known) for pid, known in named.items()
                if identity(pid) == known]

    for sig, grace in ((signal.SIGTERM, 10), (signal.SIGKILL, 5)):
        left = targets()
        if not left:
            break
        for pid, known in left:
            try:
                handle = os.pidfd_open(pid)
            except ProcessLookupError:
                continue
            try:
                # Read after opening: the handle names the process found only
                # if that process still holds the number.
                if identity(pid) == known:
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
        while targets() and time.monotonic() < deadline:
            time.sleep(0.1)
    left = targets()
    if left:
        named = ", ".join(f"{pid} ({command_line(pid)})" for pid, _ in left)
        sys.exit(f"session {session} kept {named} past KILL; {path} kept")
    os.remove(path)


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "record":
        record(sys.argv[2], int(sys.argv[3]))
    elif len(sys.argv) == 4 and sys.argv[1] == "register":
        register(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 3 and sys.argv[1] == "stop":
        stop(sys.argv[2])
    else:
        sys.exit("usage: session.py record PATH PID | register PATH PID|parent | stop PATH")


if __name__ == "__main__":
    main()
