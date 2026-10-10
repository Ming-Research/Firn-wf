"""Temporary companion to sched-trace.yml; remove before any PR.

perf script supplies timestamped tracepoint payloads, independent of comm names.
Native timehist wait is switch-out to switch-in, NOT sleep to wake:
https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/tools/perf/Documentation/perf-sched.txt
Only complete pairs inside the capture are sampled; missing endpoints are counted.
No zero is invented for a censored interval. Preemption is runnable, not sleep.
"""

import math
import re
import sys
from collections import Counter, defaultdict
from decimal import Decimal


def analyze(lines, tids):
    events = []
    pattern = re.compile(r"^\s*(\d+\.\d+):\s+sched:(sched_\w+):\s+(.*)$")
    for line in lines:
        match = pattern.match(line)
        if not match:
            if line.strip():
                raise ValueError(f"unrecognized perf script row: {line.rstrip()}")
            continue
        stamp, event, payload = match.groups()
        events.append((int(Decimal(stamp) * 1_000_000_000), event, payload))
    # perf script normally orders events; sort explicitly across CPU buffers.
    events.sort(key=lambda event: event[0])
    if not events:
        raise ValueError("no scheduler events decoded")
    # perf sched record on some kernels records waking, others wakeup; never
    # count both. Prefer completed wakeup when present. Waking is initiation,
    # so its delay also contains the wakeup path, clearly labelled below.
    wake = "sched_wakeup" if any(e[1] == "sched_wakeup" for e in events) else "sched_waking"
    samples = defaultdict(list)
    out, asleep, ready, awakened = {}, {}, {}, {}
    counts = Counter()

    def field(payload, name):
        match = re.search(rf"(?:^|\s){name}=(\S+)", payload)
        if not match:
            raise ValueError(f"missing {name}: {payload}")
        return match[1]

    def sample(tid, metric, start, end):
        if end < start:
            raise ValueError("negative interval")
        samples[tid, metric].append((end - start) / 1_000_000)

    for stamp, event, payload in events:
        if event in (wake, "sched_wakeup_new"):
            tid = int(field(payload, "pid"))
            if tid not in tids or "success=0" in payload:
                continue
            counts["wake_events"] += 1
            if tid in asleep:
                sample(tid, "sleep_to_wake", asleep.pop(tid), stamp)
            else:
                counts["wake_without_sleep_endpoint"] += 1
            # A duplicate wake must not replace the first runnable timestamp.
            ready.setdefault(tid, stamp)
            awakened.setdefault(tid, stamp)
        elif event == "sched_switch":
            prev = int(field(payload, "prev_pid"))
            nex = int(field(payload, "next_pid"))
            if prev in tids:
                out[prev] = stamp
                if field(payload, "prev_state").startswith("R"):
                    ready[prev] = stamp
                else:
                    asleep[prev] = stamp
                    ready.pop(prev, None)
                awakened.pop(prev, None)
            if nex in tids:
                counts["switch_ins"] += 1
                for metric, pending in (("wait_time_off_cpu", out),
                                        ("sch_delay", ready),
                                        ("wake_to_run", awakened)):
                    if nex in pending:
                        sample(nex, metric, pending.pop(nex), stamp)
                    elif metric != "wake_to_run":
                        counts["missing_" + metric] += 1
                asleep.pop(nex, None)
    counts["unfinished_sleep"] = len(asleep)
    counts["unfinished_runnable"] = len(ready)
    if not counts["switch_ins"] or not any(k[1] == "wake_to_run" for k in samples):
        raise ValueError("no firn switch-ins or matched wake-to-run pairs; trace incomplete")
    return samples, counts, wake


def self_test():
    # Hand-derived oracle: sleep 2 ms, wake delay 3 ms, off-CPU 5 ms;
    # then a 4 ms preemption with no new wake or sleep sample.
    rows = [
        "1.000000000: sched:sched_switch: prev_pid=7 prev_state=S next_pid=0",
        "1.002000000: sched:sched_wakeup: comm=generic pid=7 target_cpu=0",
        "1.005000000: sched:sched_switch: prev_pid=0 prev_state=R next_pid=7",
        "1.006000000: sched:sched_switch: prev_pid=7 prev_state=R+ next_pid=0",
        "1.010000000: sched:sched_switch: prev_pid=0 prev_state=R next_pid=7",
        "1.011000000: sched:sched_switch: prev_pid=7 prev_state=S next_pid=0",
    ]
    values, counts, wake = analyze(rows, {7, 8})
    assert values == {(7, "sleep_to_wake"): [2.0], (7, "wait_time_off_cpu"): [5.0, 4.0],
                      (7, "sch_delay"): [3.0, 4.0], (7, "wake_to_run"): [3.0]}
    assert wake == "sched_wakeup" and counts["unfinished_sleep"] == 1
    for bad in ([], rows[:1], ["bad input"], [rows[1].replace("pid=7", "tid=7")]):
        try:
            analyze(bad, {7})
        except ValueError:
            pass
        else:
            raise AssertionError("missing or malformed trace was accepted")
    partial, counts, wake = analyze(rows[1:], {7})
    assert not partial.get((7, "sleep_to_wake"))
    assert partial[7, "wait_time_off_cpu"] == [4.0]
    assert counts["missing_wait_time_off_cpu"] == 1
    alternate = [r.replace("sched_wakeup:", "sched_waking:") for r in rows]
    assert analyze(alternate, {7})[2] == "sched_waking"
    print("scheduler summary synthetic checks passed")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    latency = sys.argv[1] == "--latency"
    args = sys.argv[2:] if latency else sys.argv[1:]
    threads = {}
    with open(args[0]) as source:
        for row in source:
            tid, comm = row.rstrip("\n").split("\t", 1)
            threads[int(tid)] = comm
    if not threads:
        raise ValueError("empty firn thread inventory")
    with open(args[1]) as source:
        if latency:
            print("perf sched latency --sort max -p; only inventoried firn TIDs")
            matched = 0
            for line in source:
                match = re.search(r":(\d+)\s+\|", line)
                if match and int(match[1]) in threads:
                    print(line, end="")
                    matched += 1
                elif "Task" in line or re.match(r"^\s*-{5,}", line):
                    print(line, end="")
            if not matched:
                raise ValueError("native latency summary contained no firn TIDs")
            return
        samples, counts, wake = analyze(source, threads)
    print(f"Wake endpoint: {wake}; times in milliseconds; nearest-rank percentiles.")
    print("Native perf may use sched_waking (initiation); these samples prefer sched_wakeup (completion).")
    print("sch_delay: runnable to running, including preemptions; wake_to_run: wakes only.")
    print("wait_time_off_cpu: switch-out to switch-in (perf timehist's wait time).")
    print("sleep_to_wake: blocking switch-out to wake endpoint (excludes runnable delay).")
    print("Boundary/missing pairs are excluded, not zero-filled; counters:", dict(counts))
    print("TID\tcomm\tmetric\tn\tp50_ms\tp90_ms\tp99_ms\tmax_ms")
    for tid, comm in sorted(threads.items()):
        for metric in ("sch_delay", "wake_to_run", "sleep_to_wake", "wait_time_off_cpu"):
            values = sorted(samples.get((tid, metric), []))
            if not values:
                print(f"{tid}\t{comm}\t{metric}\t0\tNA\tNA\tNA\tNA")
                continue
            percentiles = [values[math.ceil(len(values) * p / 100) - 1] for p in (50, 90, 99)]
            print(f"{tid}\t{comm}\t{metric}\t{len(values)}\t" +
                  "\t".join(f"{n:.6f}" for n in (*percentiles, values[-1])))


if __name__ == "__main__":
    main()
