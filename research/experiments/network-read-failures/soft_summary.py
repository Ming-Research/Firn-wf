#!/usr/bin/env python3
"""Summarises the runs of the soft-limit case that run.sh's `soft-limit` mode made.

    soft_summary.py DIRECTORY

DIRECTORY holds, for each run N, soft-N.log (the case's output with soft-limit-stall.patch applied) and soft-N.status
(its exit status). It prints how many runs there were and how many failed, the spread of the time each 14 MiB frame
took to read and of the longest wait for a byte within it, every run that waited a second or more, with what the case
printed about the server during the wait, and every failed run's output.
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


def main():
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
