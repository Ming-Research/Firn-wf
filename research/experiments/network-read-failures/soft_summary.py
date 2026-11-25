#!/usr/bin/env python3
"""Summarises the runs that run.sh's `soft-limit` and `stall-loop` modes made.

    soft_summary.py DIRECTORY
    soft_summary.py --loop DIRECTORY

DIRECTORY holds, for each run N of the soft-limit case, soft-N.log (the case's output with soft-limit-stall.patch
applied) and soft-N.status (its exit status). It prints how many runs there were and how many failed, the spread of the
time each 14 MiB frame took to read and of the longest wait for a byte within it, every run that waited a second or
more, with what the case printed about the server during the wait, and every failed run's output.

With --loop it reads loop-N.log and loop-N.status, of the patch's stall_loop case, and prints the same spreads for its
rounds, grouped by how long the server's send had waited and by whether the server was asked a question before the
read.
"""

import glob
import os
import re
import statistics
import sys

BODY = re.compile(r"^BODY bytes=(\d+) ms=(\d+) longest_wait_ms=(\d+)")
SHOWN = 400


def spread(values):
    if not values:
        return "none"
    ordered = sorted(values)
    at = lambda share: ordered[min(len(ordered) - 1, int(share * len(ordered)))]
    return (
        f"n={len(ordered)} min={ordered[0]} median={statistics.median(ordered):.0f} p90={at(0.90)} p99={at(0.99)} "
        f"max={ordered[-1]}"
    )


LOOP = re.compile(r"^LOOP round=(\d+) wait_ms=(\d+) asked=(true|false)")


def read_runs(directory, prefix):
    runs = {}
    for path in glob.glob(os.path.join(directory, f"{prefix}-*.log")):
        number = int(re.search(r"-(\d+)\.log$", path).group(1))
        with open(path, encoding="utf-8", errors="replace") as log:
            lines = log.read().splitlines()
        try:
            with open(path[: -len(".log")] + ".status", encoding="utf-8") as status:
                code = int(status.read().strip() or -1)
        except (OSError, ValueError):
            code = -1
        runs[number] = (code, lines)
    return runs


def show(runs, numbers):
    shown = 0
    for number in numbers:
        code, lines = runs[number]
        print(f"---- run {number}, exit status {code}, {len(lines)} lines ----")
        for index, line in enumerate(lines):
            if shown >= SHOWN * 6:
                print("(no more shown)")
                return
            near_stall = any(other.startswith("STALL") for other in lines[index : index + 3])
            if line.startswith(("STALL", "DUMP", "STEP", "SAMPLE", "thread ", "test ")) or "panicked" in line or near_stall:
                print(line[:1500])
                shown += 1


def loop(directory):
    runs = read_runs(directory, "loop")
    failed = sorted(number for number, (code, _) in runs.items() if code != 0)
    groups = {}
    stalled = []
    rounds = 0
    for number, (code, lines) in sorted(runs.items()):
        pending = None
        for line in lines:
            found = LOOP.match(line)
            if found:
                pending = (int(found.group(2)), found.group(3) == "true")
                continue
            body = BODY.match(line)
            if body and pending is not None:
                took, longest = groups.setdefault(pending, ([], []))
                took.append(int(body.group(2)))
                longest.append(int(body.group(3)))
                rounds += 1
                pending = None
        if any(line.startswith("STALL ") for line in lines):
            stalled.append(number)
    print(f"stall-loop: processes={len(runs)} failed={len(failed)} {failed} rounds={rounds}")
    for (wait, asked), (took, longest) in sorted(groups.items()):
        slow = sum(1 for value in longest if value >= 200)
        print(f"stall-loop: wait {wait} ms, asked={asked}: read ms: {spread(took)}")
        print(f"stall-loop: wait {wait} ms, asked={asked}: longest wait for a byte ms: {spread(longest)}; 200 ms or more: {slow}")
    print(f"stall-loop: processes with a wait of a second or more for a byte: {len(stalled)} {stalled}")
    show(runs, sorted(set(stalled) | set(failed)))
    return 0


def main():
    if sys.argv[1] == "--loop":
        return loop(sys.argv[2])
    directory = sys.argv[1]
    runs = {}
    for path in glob.glob(os.path.join(directory, "soft-*.log")):
        number = int(re.search(r"soft-(\d+)\.log$", path).group(1))
        with open(path, encoding="utf-8", errors="replace") as log:
            lines = log.read().splitlines()
        try:
            with open(path[: -len(".log")] + ".status", encoding="utf-8") as status:
                code = int(status.read().strip() or -1)
        except (OSError, ValueError):
            code = -1
        runs[number] = (code, lines)

    failed = sorted(number for number, (code, _) in runs.items() if code != 0)
    frames = {1: ([], []), 2: ([], [])}
    stalled = []
    for number, (code, lines) in sorted(runs.items()):
        bodies = [BODY.match(line) for line in lines]
        bodies = [found for found in bodies if found]
        for index, found in enumerate(bodies[:2], start=1):
            frames[index][0].append(int(found.group(2)))
            frames[index][1].append(int(found.group(3)))
        if any(line.startswith("STALL ") for line in lines):
            stalled.append(number)

    print(f"soft-limit: runs={len(runs)} failed={len(failed)} {failed[:40]}")
    for index in (1, 2):
        print(f"soft-limit: frame {index} read ms: {spread(frames[index][0])}")
        print(f"soft-limit: frame {index} longest wait for a byte ms: {spread(frames[index][1])}")
    print(f"soft-limit: runs that waited a second or more for a byte: {len(stalled)} {stalled[:40]}")

    shown = 0
    for number in sorted(set(stalled) | set(failed)):
        code, lines = runs[number]
        print(f"soft-limit: ---- run {number}, exit status {code}, {len(lines)} lines ----")
        for line in lines:
            if shown >= SHOWN * 6:
                print("soft-limit: (no more shown)")
                return 0
            if line.startswith(("STALL", "DUMP", "BODY", "STEP", "SAMPLE", "thread ", "test ")) or "panicked" in line:
                print(line[:1500])
                shown += 1

    sample = [number for number, (_, lines) in sorted(runs.items()) if any(line.startswith("SAMPLE") for line in lines)]
    for number in sample[:1]:
        print(f"soft-limit: ---- run {number}: the sample of a waiting send ----")
        for line in runs[number][1]:
            if line.startswith(("SAMPLE", "DUMP", "BODY", "STEP")):
                print(line[:1500])
    return 0


if __name__ == "__main__":
    sys.exit(main())
