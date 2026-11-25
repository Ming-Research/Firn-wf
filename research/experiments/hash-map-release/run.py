"""Builds the map program of this directory with two Whitefoot releases and times
the two builds against each other, interleaved, on one CPU.

usage: run.py --root ROOT --out OUT --old NAME=COMPILER,LANGUAGE --new NAME=COMPILER,LANGUAGE
              [--operations N] [--reps N] [--cpu CPU]

The program holds the 100,000 fields of redis-benchmark's HSET and HGET tests in
the HashMap<Key, Bytes> that firn's pkg::bytes module defines (its Key, Bytes and
KeyHash are copied from ROOT, not rewritten). Three entries are built by each
compiler with full link-time optimization, as firn is measured: fill builds the
map, replace builds it and then puts a random present field N times (HSET on an
existing field), lookup builds it and then looks up a random present field N times
(HGET). The cost of an operation is the entry's cost minus fill's, over N. The old
build is also run from a second copy (the twin), whose ratio to the old is noise
by construction. Each repetition runs every entry once for each of the three
executables, in an order that rotates, so that a drift of the machine reaches all
alike.
"""
import argparse
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

ENTRIES = ("fill", "replace", "lookup")
# Two-sided 95% Student t, by degrees of freedom 1 to 30.
T95 = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228,
       2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086,
       2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048, 2.045, 2.042]


def interval(logs):
    """Mean of the logs and its 95% interval, as ratios."""
    n = len(logs)
    mean = sum(logs) / n
    if n < 2:
        return math.exp(mean), math.nan, math.nan
    spread = math.sqrt(sum((x - mean) ** 2 for x in logs) / (n - 1))
    t = T95[n - 2] if n - 1 <= 30 else 1.96 + 2.4 / (n - 1)
    half = t * spread / math.sqrt(n)
    return math.exp(mean), math.exp(mean - half), math.exp(mean + half)


# A run is read for an operation only if its twin's interval lies within these
# bounds and holds 1.000: a smaller effect than the twin's spread cannot be seen.
RESOLUTION = (0.985, 1.015)


def verdict(new, twin):
    """The reading of a run for one operation from the (mean, low, high) ratios of the new build and of the twin."""
    if not (RESOLUTION[0] <= twin[1] <= 1.0 <= twin[2] <= RESOLUTION[1]):
        return "unresolved"
    if new[1] > 1.0:
        return "slower"
    if new[2] < 1.0:
        return "faster"
    return "no difference seen"


def selftest():
    quiet = (1.0, 0.995, 1.005)
    assert verdict((1.04, 1.03, 1.05), quiet) == "slower"
    assert verdict((0.96, 0.95, 0.97), quiet) == "faster"
    assert verdict((1.003, 0.995, 1.010), quiet) == "no difference seen"
    assert verdict((1.0187, 0.6986, 1.4852), (1.0039, 0.9507, 1.0600)) == "unresolved"
    assert verdict((1.04, 1.03, 1.05), (1.02, 1.015, 1.025)) == "unresolved"
    assert verdict((1.04, 1.03, 1.05), (1.0, 0.98, 1.02)) == "unresolved"
    mean, low, high = interval([math.log(1.04)] * 5)
    assert abs(mean - 1.04) < 1e-9 and abs(low - 1.04) < 1e-9 and abs(high - 1.04) < 1e-9
    mean, low, high = interval([math.log(x) for x in (1.00, 1.02, 0.98, 1.01, 0.99)])
    assert low < 1.0 < high
    mean, low, high = interval([math.log(x) for x in (1.03, 1.04, 1.05, 1.04, 1.03)])
    assert low > 1.0
    assert parse_stat("123,,cycles:u,1000,100.00\n456,,instructions:u,1000,100.00\n") == {"cycles": 123, "instructions": 456}
    assert parse_stat("<not counted>,,cycles:u,0,0.00\n") == {}
    assert parse_stat("9,,cpu_core/cycles/u,1,100.00\n<not supported>,,cpu_atom/cycles/u,0,0.00\n") == {"cycles": 9}
    print("selftest ok")


def parse_stat(text):
    """The counts of perf stat -x, output by event name, without the modifier; an event not counted is left out."""
    out = {}
    for line in text.splitlines():
        parts = line.split(",")
        if len(parts) < 3 or not parts[0].isdigit():
            continue
        name = parts[2].split("/")[1] if "/" in parts[2] else parts[2]
        out[name.split(":")[0]] = int(parts[0])
    return out


def run(command, **kwargs):
    return subprocess.run(command, check=False, text=True, capture_output=True, **kwargs)


def assemble(root, out, operations):
    src = os.path.join(out, "src")
    shutil.rmtree(src, ignore_errors=True)
    here = os.path.dirname(os.path.abspath(__file__))
    shutil.copytree(os.path.join(root, "firn", "bytes"), os.path.join(src, "bytes"))
    shutil.copytree(os.path.join(here, "repro"), os.path.join(src, "repro"))
    shutil.copy(os.path.join(here, "modules.wfg"), os.path.join(src, "modules.wfg"))
    path = os.path.join(src, "repro", "repro.wf")
    text = open(path).read()
    text, count = re.subn(r"const operations: u64 = \d+_u64;", f"const operations: u64 = {operations}_u64;", text)
    assert count == 1
    open(path, "w").write(text)
    return src


def build(src, out, label, compiler, language):
    env = dict(os.environ, WHITEFOOT_LANGUAGE=language)
    binaries = {}
    for entry in ENTRIES:
        binary = os.path.join(out, "bin", f"{label}-{entry}")
        os.makedirs(os.path.dirname(binary), exist_ok=True)
        started = time.time()
        done = run([compiler, "--full-lto", "--graph", os.path.join(src, "modules.wfg"), "--entry", entry, "-o", binary], env=env)
        if done.returncode != 0:
            print(f"build of {label} {entry} failed ({done.returncode}):\n{done.stdout[-3000:]}\n{done.stderr[-3000:]}")
            sys.exit(2)
        print(f"built {label} {entry} in {time.time() - started:.0f}s")
        binaries[entry] = binary
    return binaries


def stat_command(perf, cpu, path, command):
    return ["taskset", "-c", str(cpu), perf, "stat", "-x,", "-e", "cycles:u,instructions:u", "-o", path] + command


def perf_usable(cpu):
    perf = shutil.which("perf")
    if not perf:
        print("perf: not found")
        return None
    with tempfile.TemporaryDirectory() as scratch:
        path = os.path.join(scratch, "stat")
        probe = run(stat_command(perf, cpu, path, ["true"]))
        counts = parse_stat(open(path).read()) if os.path.exists(path) else {}
    print(f"perf: {perf} probe exit {probe.returncode} counts {counts} {probe.stderr.strip()[-200:]}")
    return perf if "cycles" in counts and "instructions" in counts else None


def measure(binary, cpu, perf):
    with tempfile.TemporaryDirectory() as scratch:
        path = os.path.join(scratch, "stat")
        command = stat_command(perf, cpu, path, [binary]) if perf else ["taskset", "-c", str(cpu), binary]
        begun = time.perf_counter_ns()
        done = run(command)
        wall = time.perf_counter_ns() - begun
        counts = parse_stat(open(path).read()) if perf and os.path.exists(path) else {}
    if done.returncode != 0:
        print(f"{binary} exited {done.returncode}: {done.stderr[-500:]}")
        sys.exit(3)
    if perf and not ("cycles" in counts and "instructions" in counts):
        print(f"{binary}: perf stat counted {counts}")
        sys.exit(3)
    counts["wall"] = wall
    return counts


def busy_totals():
    with open("/proc/stat") as handle:
        rows = [line.split() for line in handle if re.match(r"cpu\d+ ", line)]
    return {int(r[0][3:]): (sum(int(x) for x in r[1:9]), int(r[4]) + int(r[5])) for r in rows}


def other_busy(before, after, cpu):
    total = idle = 0
    for number in before:
        if number == cpu:
            continue
        total += after[number][0] - before[number][0]
        idle += after[number][1] - before[number][1]
    return 1 - idle / total if total else math.nan


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "selftest":
        selftest()
        return
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--old", required=True)
    parser.add_argument("--new", required=True)
    parser.add_argument("--operations", type=int, default=20000000)
    parser.add_argument("--reps", type=int, default=9)
    parser.add_argument("--cpu", type=int, default=4)
    args = parser.parse_args()
    labels = {}
    for role, spec in (("old", args.old), ("new", args.new)):
        name, paths = spec.split("=", 1)
        compiler, language = paths.split(",", 1)
        labels[role] = (name, compiler, language)
    os.makedirs(args.out, exist_ok=True)
    src = assemble(args.root, args.out, args.operations)
    bins = {}
    for role in ("old", "new"):
        name, compiler, language = labels[role]
        bins[role] = build(src, args.out, role, compiler, language)
    bins["twin"] = {}
    for entry in ENTRIES:
        twin = os.path.join(args.out, "bin", f"twin-{entry}")
        shutil.copy(bins["old"][entry], twin)
        bins["twin"][entry] = twin
    perf = perf_usable(args.cpu)
    print(f"releases old={labels['old'][0]} new={labels['new'][0]} operations={args.operations} reps={args.reps} cpu={args.cpu} counters={'perf cycles,instructions (user)' if perf else 'none: wall time only'}")
    for entry in ENTRIES:
        if open(bins["twin"][entry], "rb").read() != open(bins["old"][entry], "rb").read():
            print("the twin is not a copy of the old build")
            sys.exit(2)
    results = {}
    started = busy_totals()
    for rep in range(args.reps):
        for entry in ENTRIES:
            order = ["old", "new", "twin"]
            shift = (rep + ENTRIES.index(entry)) % 3
            order = order[shift:] + order[:shift]
            for role in order:
                results[(rep, entry, role)] = measure(bins[role][entry], args.cpu, perf)
    busy = other_busy(started, busy_totals(), args.cpu)
    print(f"the CPUs but {args.cpu} were {busy * 100:.1f}% busy over the measurement; loadavg {open('/proc/loadavg').read().split()[0]}")
    metrics = ["cycles", "instructions"] if perf else []
    metrics.append("wall")
    for entry in ("replace", "lookup"):
        for metric in metrics:
            cost = {}
            for role in ("old", "new", "twin"):
                base = sorted(results[(rep, "fill", role)][metric] for rep in range(args.reps))[args.reps // 2]
                cost[role] = [(results[(rep, entry, role)][metric] - base) / args.operations for rep in range(args.reps)]
            ratios = {}
            for role in ("new", "twin"):
                ratios[role] = interval([math.log(cost[role][rep] / cost["old"][rep]) for rep in range(args.reps)])
            unit = "ns" if metric == "wall" else metric
            median = sorted(cost["old"])[args.reps // 2]
            slower = sum(1 for rep in range(args.reps) if cost["new"][rep] > cost["old"][rep])
            print(f"{entry:7} {metric:12} old {median:9.2f} {unit}/op  new/old {ratios['new'][0]:.4f} [{ratios['new'][1]:.4f}, {ratios['new'][2]:.4f}]  twin/old {ratios['twin'][0]:.4f} [{ratios['twin'][1]:.4f}, {ratios['twin'][2]:.4f}]  new slower in {slower}/{args.reps}  {verdict(ratios['new'], ratios['twin'])}")
    for entry in ("fill",):
        for metric in metrics:
            values = {role: sorted(results[(rep, entry, role)][metric] for rep in range(args.reps))[args.reps // 2] for role in ("old", "new", "twin")}
            print(f"{entry:7} {metric:12} median per run: old {values['old']} new {values['new']} twin {values['twin']}")
    print("per-repetition cycles/op (replace, lookup) old new twin:" if perf else "per-repetition ns/op (replace, lookup) old new twin:")
    key = "cycles" if perf else "wall"
    for rep in range(args.reps):
        row = []
        for entry in ("replace", "lookup"):
            row.append(" ".join(f"{(results[(rep, entry, role)][key] - results[(rep, 'fill', role)][key]) / args.operations:7.2f}" for role in ("old", "new", "twin")))
        print(f"  {rep}: " + " | ".join(row))


if __name__ == "__main__":
    main()
