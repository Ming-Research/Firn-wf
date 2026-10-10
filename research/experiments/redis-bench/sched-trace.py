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
from bisect import bisect_right
from collections import Counter, defaultdict
from decimal import Decimal
from pathlib import Path


def field(payload, name, default=None):
    match = re.search(rf"(?:^|\s){name}=(.*?)(?=\s+\w+=|$)", payload)
    if match:
        return match[1].strip()
    return default


def wake_task(payload):
    pid = field(payload, "pid")
    if pid is not None:
        return int(pid)
    match = re.match(r".*:(\d+)\s+\[\d+\]", payload)
    if not match:
        raise ValueError(f"missing pid: {payload}")
    return int(match[1])


def switch_tasks(payload):
    # perf 7's pretty tracepoint format, or the named-field kernel format.
    match = re.fullmatch(r"(.*):(\d+)\s+\[\d+\]\s+(\S+)\s+==>\s+"
                         r"(.*):(\d+)\s+\[\d+\]\s*", payload)
    if match:
        prev_comm, prev, state, next_comm, nex = match.groups()
    else:
        prev, nex = field(payload, "prev_pid"), field(payload, "next_pid")
        if prev is None or nex is None:
            raise ValueError(f"missing switch PIDs: {payload}")
        prev_comm = field(payload, "prev_comm", "unknown")
        next_comm = field(payload, "next_comm", "unknown")
        # prev_state may end in the pretty format's switch delimiter.
        state = field(payload, "prev_state", "").split()
        state = state[0] if state else ""
    return int(prev), prev_comm, state, int(nex), next_comm


def busiest_threads(pid):
    tasks = []
    for task in (Path("/proc") / str(pid) / "task").iterdir():
        try:
            # comm (field 2) may contain spaces and parentheses; fields after
            # its final ')' start at state (3), so utime/stime (14/15) are 11/12.
            stat = (task / "stat").read_text()
        except FileNotFoundError:
            continue  # A short-lived task can end during enumeration.
        end_comm = stat.rindex(")")
        fields = stat[end_comm + 1:].split()
        ticks = int(fields[11]) + int(fields[12])
        tasks.append((ticks, int(task.name), stat[stat.index("(") + 1:end_comm]))
    tasks.sort(key=lambda task: (-task[0], task[1]))
    if len(tasks) < 2:
        raise ValueError("fewer than two firn tasks after 1 s of load")
    return tasks[:2]


def cpu_occupants(switches, trace_start, trace_end):
    intervals, ends = {}, {}
    for cpu, rows in switches.items():
        spans = []
        start, expected = trace_start, None
        for stamp, prev, prev_comm, nex, next_comm in rows:
            # The first switch's prev describes the initial occupant. Later
            # disagreement indicates missing switches; do not invent a task.
            task = (prev, prev_comm) if expected is None or expected[0] == prev else (None, "unknown")
            spans.append((start, stamp, *task))
            start, expected = stamp, (nex, next_comm)
        spans.append((start, trace_end, *expected))
        intervals[cpu] = spans
        ends[cpu] = [span[1] for span in spans]
    return intervals, ends


def occupants_during(cpu, start, end, intervals, ends):
    if cpu not in intervals:
        return [(None, "unknown", (end - start) / 1_000_000)]
    occupied = defaultdict(int)
    for index in range(bisect_right(ends[cpu], start), len(intervals[cpu])):
        left, right, tid, comm = intervals[cpu][index]
        if left >= end:
            break
        duration = min(right, end) - max(left, start)
        if duration > 0:
            occupied[tid, comm] += duration
    return [(tid, comm, duration / 1_000_000) for (tid, comm), duration in occupied.items()]


def analyze(lines, tids, drivers=None):
    drivers = set(tids) if drivers is None else set(drivers)
    events = []
    pattern = re.compile(r"^(.*?)\b(\d+\.\d+):\s+sched:(sched_\w+):\s+(.*)$")
    for line in lines:
        match = pattern.match(line)
        if not match:
            if line.strip():
                raise ValueError(f"unrecognized perf script row: {line.rstrip()}")
            continue
        prefix, stamp, event, payload = match.groups()
        context = re.search(r"(?:^|\s)(\d+)\s+\[(\d+)\]\s*$", prefix)
        waker, cpu = map(int, context.groups()) if context else (None, None)
        events.append((int(Decimal(stamp) * 1_000_000_000), event, payload, cpu, waker))
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
    switches = defaultdict(list)
    long_delays = []

    def sample(tid, metric, start, end):
        if end < start:
            raise ValueError("negative interval")
        samples[tid, metric].append((end - start) / 1_000_000)

    for stamp, event, payload, cpu, waker in events:
        if event in (wake, "sched_wakeup_new"):
            tid = wake_task(payload)
            if tid not in tids or field(payload, "success") == "0":
                continue
            counts["wake_events"] += 1
            if tid in asleep:
                sample(tid, "sleep_to_wake", asleep.pop(tid), stamp)
            else:
                counts["wake_without_sleep_endpoint"] += 1
            # A duplicate wake must not replace the first runnable timestamp.
            ready.setdefault(tid, stamp)
            target = field(payload, "target_cpu")
            if target is None:
                match = re.search(r"\bCPU:(\d+)", payload)
                target = match[1] if match else None
            awakened.setdefault(tid, (stamp, int(target) if target is not None else None, cpu, waker))
        elif event == "sched_switch":
            prev, prev_comm, state, nex, next_comm = switch_tasks(payload)
            if cpu is not None:
                switches[cpu].append((stamp, prev, prev_comm, nex, next_comm))
            if prev in tids:
                out[prev] = stamp
                if not state:
                    raise ValueError(f"missing prev_state for firn: {payload}")
                if state.startswith("R"):
                    ready[prev] = stamp
                else:
                    asleep[prev] = stamp
                    ready.pop(prev, None)
                awakened.pop(prev, None)
            if nex in tids:
                counts["switch_ins"] += 1
                for metric, pending in (("wait_time_off_cpu", out),
                                        ("sch_delay", ready)):
                    if nex in pending:
                        sample(nex, metric, pending.pop(nex), stamp)
                    else:
                        counts["missing_" + metric] += 1
                if nex in awakened:
                    start, target, waker_cpu, waker_tid = awakened.pop(nex)
                    sample(nex, "wake_to_run", start, stamp)
                    if nex in drivers and stamp - start > 1_000_000:
                        long_delays.append({"tid": nex, "start": start, "end": stamp,
                                            "target_cpu": target, "waker_cpu": waker_cpu,
                                            "waker_tid": waker_tid, "run_cpu": cpu})
                asleep.pop(nex, None)
    counts["unfinished_sleep"] = len(asleep)
    counts["unfinished_runnable"] = len(ready)
    if not counts["switch_ins"] or not any(k[1] == "wake_to_run" for k in samples):
        raise ValueError("no firn switch-ins or matched wake-to-run pairs; trace incomplete")
    intervals, ends = cpu_occupants(switches, events[0][0], events[-1][0])
    for delay in long_delays:
        delay["occupants"] = occupants_during(delay["target_cpu"], delay["start"], delay["end"], intervals, ends)
    return samples, counts, wake, long_delays


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
    values, counts, wake, delays = analyze(rows, {7, 8})
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
    partial, counts, wake, delays = analyze(rows[1:], {7})
    assert not partial.get((7, "sleep_to_wake"))
    assert partial[7, "wait_time_off_cpu"] == [4.0]
    assert counts["missing_wait_time_off_cpu"] == 1
    alternate = [r.replace("sched_wakeup:", "sched_waking:") for r in rows]
    assert analyze(alternate, {7})[2] == "sched_waking"
    # Actual perf 7 payload shapes from the native run, plus the new header.
    # Woken CPU 0 is occupied by driver 8 for 1 ms and worker 9 for 2 ms.
    pretty = [
        "firn-lto 7 [000] 1.000000000: sched:sched_switch: firn-lto:7 [120] S ==> firn-lto:8 [120]",
        "migration/0 18 [000] 1.001000000: sched:sched_wakeup: migration/0:18 [0] CPU:000",
        "firn-lto 8 [000] 1.002000000: sched:sched_wakeup: firn-lto:7 [120] CPU:000",
        "firn-lto 8 [000] 1.003000000: sched:sched_switch: firn-lto:8 [120] R+ ==> worker:9 [120]",
        "worker 9 [000] 1.005000000: sched:sched_switch: worker:9 [120] S ==> firn-lto:7 [120]",
    ]
    values, counts, wake, delays = analyze(pretty, {7, 8}, {7, 8})
    assert values[7, "wake_to_run"] == [3.0] and counts["wake_events"] == 1
    assert delays == [{"tid": 7, "start": 1_002_000_000, "end": 1_005_000_000,
                       "target_cpu": 0, "waker_cpu": 0, "waker_tid": 8, "run_cpu": 0,
                       "occupants": [(8, "firn-lto", 1.0), (9, "worker", 2.0)]}]
    named = [
        "firn-lto 7 [000] 1.000000000: sched:sched_switch: prev_comm=firn-lto prev_pid=7 prev_prio=120 prev_state=S ==> next_comm=firn-lto next_pid=8 next_prio=120",
        "firn-lto 8 [001] 1.002000000: sched:sched_wakeup: comm=firn-lto pid=7 prio=120 target_cpu=0",
        "firn-lto 8 [000] 1.003000000: sched:sched_switch: prev_comm=firn-lto prev_pid=8 prev_state=R+ next_comm=worker next_pid=9",
        "worker 9 [000] 1.005000000: sched:sched_switch: prev_comm=worker prev_pid=9 prev_state=S next_comm=firn-lto next_pid=7",
    ]
    named_delays = analyze(reversed(named), {7, 8}, {7})[3]
    assert named_delays[0] == dict(delays[0], waker_cpu=1)
    assert switch_tasks("a: name:7 [120] R ==> b name:8 [120]") == (7, "a: name", "R", 8, "b name")
    # Old time-only output still yields percentiles, with context unavailable.
    legacy = [re.sub(r"^.*?\[(\d+)\]\s+(?=1\.)", "", row) for row in pretty]
    legacy_values, _, _, legacy_delays = analyze(legacy, {7, 8}, {7})
    assert legacy_values == values
    assert legacy_delays[0]["waker_cpu"] is None
    assert legacy_delays[0]["occupants"] == [(None, "unknown", 3.0)]
    assert analyze(pretty, {7, 8}, {8})[3] == []  # Only selected drivers.
    boundary = [row.replace("1.005000000", "1.003000000") for row in pretty]
    assert analyze(boundary, {7, 8}, {7})[3] == []  # Strictly greater than 1 ms.
    duplicate = pretty[:3] + [pretty[2].replace("1.002000000", "1.002500000")] + pretty[3:]
    assert analyze(duplicate, {7, 8}, {7})[3] == delays  # Keep first wake.
    failed_wake = named[:1] + [named[1] + " success=0"] + named[1:]
    assert analyze(failed_wake, {7, 8}, {7})[1]["wake_events"] == 1
    assert occupants_during(0, 0, 3_000_000, *cpu_occupants(
        {0: [(1_000_000, 8, "driver", 9, "worker"),
             (2_000_000, 10, "other", 7, "driver")]}, 0, 3_000_000)) == [
                 (8, "driver", 1.0), (None, "unknown", 1.0), (7, "driver", 1.0)]
    print("scheduler summary synthetic checks passed")


def print_long_delays(delays, threads, drivers):
    print("Driver wake-to-run delays > 1 ms; CPUs are wake target, wake-event context, and actual switch-in.")
    print("Wake-event context can be an interrupt; its TID alone does not prove that task initiated the wake.")
    print("Occupants are sched_switch intervals on the original wake target CPU, even if the driver later migrates.")
    print("Missing CPU headers/switches are unknown; these are complete wake/switch-in pairs only.")
    print("TID\twake_s\tdelay_ms\twoken_CPU\twaker_CPU\twaker_TID\trun_CPU\toccupants(TID:comm=ms)")
    same_counts, occupant_counts, occupant_ms = Counter(), Counter(), Counter()
    for delay in delays:
        target, waker_cpu = delay["target_cpu"], delay["waker_cpu"]
        same = "unknown" if target is None or waker_cpu is None else ("yes" if target == waker_cpu else "no")
        same_counts[same] += 1
        occupied = []
        for tid, comm, duration in delay["occupants"]:
            role = "unknown" if tid is None else ("idle" if tid == 0 else
                   "driver" if tid in drivers else "firn-other" if tid in threads else "other")
            key = same, tid, comm, role
            occupant_counts[key] += 1
            occupant_ms[key] += duration
            occupied.append(f"{tid if tid is not None else 'NA'}:{comm}={duration:.6f}")
        context = [delay[name] for name in ("target_cpu", "waker_cpu", "waker_tid", "run_cpu")]
        print(f"{delay['tid']}\t{Decimal(delay['start']) / 1_000_000_000:.9f}\t"
              f"{(delay['end'] - delay['start']) / 1_000_000:.6f}\t" +
              "\t".join("NA" if value is None else str(value) for value in context) +
              "\t" + "; ".join(occupied))
    print("woken_CPU==waker_CPU\tlong_wakes")
    for same in ("yes", "no", "unknown"):
        print(f"{same}\t{same_counts[same]}")
    print("Each occupant is counted once per long wake; a wake can have several occupants, so row counts can exceed wakes.")
    print("woken_CPU==waker_CPU\toccupant_TID\tcomm\trole\tlong_wakes\toccupied_ms")
    for key in sorted(occupant_counts, key=lambda key: (key[0], -occupant_counts[key], str(key[1:]))):
        same, tid, comm, role = key
        print(f"{same}\t{tid if tid is not None else 'NA'}\t{comm}\t{role}\t"
              f"{occupant_counts[key]}\t{occupant_ms[key]:.6f}")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if sys.argv[1] == "--drivers":
        for ticks, tid, comm in busiest_threads(int(sys.argv[2])):
            print(f"{tid}\t{comm}\t{ticks}")
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
    drivers = set(threads)
    if not latency and len(args) > 2:
        with open(args[2]) as source:
            drivers = {int(row.split("\t", 1)[0]) for row in source}
        if len(drivers) != 2 or not drivers.issubset(threads):
            raise ValueError("driver inventory must name two inventoried firn TIDs")
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
        samples, counts, wake, delays = analyze(source, threads, drivers)
    print("Selected driver TIDs:", ",".join(map(str, sorted(drivers))))
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
    print_long_delays(delays, threads, drivers)


if __name__ == "__main__":
    main()
