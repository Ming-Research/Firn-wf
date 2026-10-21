#!/usr/bin/env python3
"""Tables from the CSV files of a redis-bench run, for the records and the
acceptance rows.

  summary.py pubsub DIR [--firn LINE] [--twin LINE]
      pubsub.csv and pubsub-limited.csv of a workloads run with the pub/sub
      workloads: per workload and pipeline depth, each line's median rate and
      p99 over the passes, firn's multiple of the fastest competitor, and
      whether the depth's criteria hold. A cell is client-limited when the
      median client work fraction of firn's line or of the fastest competitor's
      is above 0.90, and then asks only that firn is not behind.
  summary.py ratios COMPARE.csv --base NAME --head NAME [--twin NAME]
      compare.csv of a compare run: per cpus, test and depth, the median over
      passes of head's rate over base's, with the 95% interval of the mean log
      ratio over the passes (Student's t); the twin, an image measured again
      as head, gives the run's noise as its own ratio to head.
  summary.py selftest

Only the standard library, since the hosts that run it have no more.
"""
import argparse
import csv
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

# Two-sided 95% critical values of Student's t by degrees of freedom.
T95 = {1: 12.706, 2: 4.303, 3: 3.182, 4: 2.776, 5: 2.571, 6: 2.447, 7: 2.365,
       8: 2.306, 9: 2.262, 10: 2.228, 11: 2.201, 12: 2.179, 13: 2.160,
       14: 2.145, 15: 2.131, 16: 2.120, 17: 2.110, 18: 2.101, 19: 2.093,
       20: 2.086, 25: 2.060, 30: 2.042}


def t95(df):
    if df >= 31:
        return 1.96
    while df not in T95:
        df -= 1
    return T95[df]


def rows(path):
    with open(path, newline="") as handle:
        return list(csv.DictReader(handle))


def interval(ratios):
    """The mean log ratio's 95% interval, as ratios, from at least two."""
    logs = [math.log(r) for r in ratios]
    mean = statistics.fmean(logs)
    half = t95(len(logs) - 1) * statistics.stdev(logs) / math.sqrt(len(logs))
    return math.exp(mean - half), math.exp(mean + half)


def verdict(low, high):
    if high < 1:
        return "slower"
    if low > 1:
        return "faster"
    return "no detected change"


def ratios(args):
    by_pass = defaultdict(dict)
    for r in rows(args.compare):
        key = (int(r["cpus"]), r["test"], int(r["depth"]))
        by_pass[key].setdefault(r["line"], {})[int(r["pass"])] = float(r["rps"])
    print("cpus,test,depth,passes,head/base median,head/base 95% interval,verdict,"
          "twin/head median,twin/head 95% interval,twin verdict")
    for key in sorted(by_pass):
        lines = by_pass[key]
        if args.base not in lines or args.head not in lines:
            continue
        shared = sorted(set(lines[args.base]) & set(lines[args.head]))
        if len(shared) < 2:
            print(",".join(map(str, key)) + f",{len(shared)},too few passes")
            continue
        head = [lines[args.head][p] / lines[args.base][p] for p in shared]
        low, high = interval(head)
        out = [*map(str, key), str(len(shared)), f"{statistics.median(head):.4f}",
               f"{low:.4f}-{high:.4f}", verdict(low, high)]
        if args.twin in lines:
            twin = [lines[args.twin][p] / lines[args.head][p] for p in shared]
            tlow, thigh = interval(twin)
            out += [f"{statistics.median(twin):.4f}", f"{tlow:.4f}-{thigh:.4f}",
                    verdict(tlow, thigh)]
        print(",".join(out))


def is_firn(line):
    return line.startswith("firn-") or line.startswith("image-")


def median_or_nan(values):
    return statistics.median(values) if values else float("nan")


def pubsub(args):
    directory = Path(args.dir)
    rates = defaultdict(lambda: defaultdict(list))
    p99s = defaultdict(lambda: defaultdict(list))
    work = defaultdict(lambda: defaultdict(list))
    for r in rows(directory / "pubsub.csv"):
        key = (int(r["cpus"]), r["workload"], int(r["connections"]), int(r["pipeline"]))
        rates[key][r["line"]].append(float(r["rate"]))
        p99s[key][r["line"]].append(float(r["p99_ms"]))
    for r in rows(directory / "pubsub-limited.csv"):
        key = (int(r["cpus"]), r["workload"], int(r["connections"]), int(r["pipeline"]))
        work[key][r["line"]].append(float(r["client_work_fraction"]))
    failures = 0
    for key in sorted(rates):
        cpus, workload, subscribers, depth = key
        lines = rates[key]
        median = {line: statistics.median(v) for line, v in lines.items()}
        firn = args.firn or next((l for l in sorted(lines)
                                  if is_firn(l) and not l.endswith("-twin")), None)
        competitors = [l for l in lines if not is_firn(l)]
        print(f"== {cpus} server CPUs, {workload}, {subscribers} subscribers, "
              f"pipeline {depth}, passes {max(len(v) for v in lines.values())}")
        print(f"{'line':<22}{'rate (msg/s)':>14}{'p99 (ms)':>10}{'client work':>13}")
        for line in sorted(lines):
            print(f"{line:<22}{median[line]:>14.0f}{statistics.median(p99s[key][line]):>10.3f}"
                  f"{median_or_nan(work[key][line]):>13.3f}")
        if firn not in lines or not competitors:
            print("no firn line or no competitor in this cell\n")
            continue
        best = max(competitors, key=lambda l: median[l])
        limited = any(median_or_nan(work[key][l]) > 0.90 for l in (firn, best))
        multiple = median[firn] / median[best]
        need = 1.0 if depth == 1 or limited else 1.4
        best_p99 = min(statistics.median(p99s[key][l]) for l in competitors)
        firn_p99 = statistics.median(p99s[key][firn])
        rate_ok, p99_ok = multiple >= need, firn_p99 <= best_p99
        failures += (not rate_ok) + (not p99_ok)
        print(f"{firn} / {best} (the fastest competitor): {multiple:.3f}, needs at least "
              f"{need:.1f}{' (client-limited)' if limited else ''}: {'ok' if rate_ok else 'BELOW'}")
        print(f"p99 {firn_p99:.3f} ms against the best competitor's {best_p99:.3f} ms: "
              f"{'ok' if p99_ok else 'HIGHER'}")
        twin = args.twin or f"{firn}-twin"
        if twin in lines:
            print(f"twin {twin}: {median[twin] / median[firn]:.4f} of {firn}, "
                  f"a gap of {abs(median[twin] / median[firn] - 1) * 100:.2f}%")
        print()
    print(f"{failures} criteria not met")


def selftest():
    # t values
    assert t95(1) == 12.706 and t95(5) == 2.571 and t95(21) == 2.086 and t95(100) == 1.96
    # interval: identical ratios have a zero-width interval around themselves
    low, high = interval([1.1, 1.1, 1.1])
    assert abs(low - 1.1) < 1e-9 and abs(high - 1.1) < 1e-9
    # a consistent 10% slowdown is a regression, noise around 1 is no change
    assert verdict(*interval([0.90, 0.91, 0.89, 0.90])) == "slower"
    assert verdict(*interval([1.01, 0.99, 1.02, 0.98])) == "no detected change"
    assert verdict(*interval([1.10, 1.11, 1.09, 1.10])) == "faster"
    import io, tempfile
    from contextlib import redirect_stdout
    with tempfile.TemporaryDirectory() as tmp:
        compare = Path(tmp) / "compare.csv"
        lines = ["line,pass,cpus,test,depth,requests,rps,server_cpu_us_per_request,p50_ms,p99_ms"]
        for p, (b, h, t) in enumerate([(100, 90, 90), (100, 91, 91), (100, 89, 89), (100, 90, 90)], 1):
            for name, rate in (("before", b), ("after", h), ("after-twin", t)):
                lines.append(f"{name},{p},2,set,16,1000,{rate},1.0,0.1,0.2")
        compare.write_text("\n".join(lines) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            ratios(argparse.Namespace(compare=str(compare), base="before", head="after", twin="after-twin"))
        row = out.getvalue().splitlines()[1].split(",")
        assert row[:4] == ["2", "set", "16", "4"] and row[4] == "0.9000" and row[6] == "slower", row
        assert row[7] == "1.0000" and row[9] == "no detected change", row
        header = "line,pass,cpus,workload,connections,pipeline,published,delivered,seconds,rate,p50_ms,p99_ms"
        data = [header]
        limited = ["line,pass,cpus,workload,connections,pipeline,client_busy_mean,client_work_fraction,client_limited"]
        for p in (1, 2, 3):
            for line, rate, p99, fraction in (("reference", 100, 3.0, 0.5), ("valkey", 80, 2.5, 0.5),
                                              ("firn-2", 150, 2.0, 0.5), ("firn-2-twin", 148, 2.0, 0.5)):
                data.append(f"{line},{p},2,pubsub-channel,50,16,10,500,5,{rate},1.0,{p99}")
                limited.append(f"{line},{p},2,pubsub-channel,50,16,0.9,{fraction},0")
        (Path(tmp) / "pubsub.csv").write_text("\n".join(data) + "\n")
        (Path(tmp) / "pubsub-limited.csv").write_text("\n".join(limited) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            pubsub(argparse.Namespace(dir=tmp, firn=None, twin=None))
        text = out.getvalue()
        assert "firn-2 / reference (the fastest competitor): 1.500, needs at least 1.4: ok" in text, text
        assert text.rstrip().endswith("0 criteria not met"), text
        # 1.5 against 1.4 fails once firn drops to 130, and p99 once it passes 3.0
        data = [header]
        for p in (1, 2, 3):
            for line, rate, p99 in (("reference", 100, 3.0), ("firn-2", 130, 3.5)):
                data.append(f"{line},{p},2,pubsub-channel,50,16,10,500,5,{rate},1.0,{p99}")
        (Path(tmp) / "pubsub.csv").write_text("\n".join(data) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            pubsub(argparse.Namespace(dir=tmp, firn=None, twin=None))
        text = out.getvalue()
        assert "BELOW" in text and "HIGHER" in text and text.rstrip().endswith("2 criteria not met"), text
    print("summary selftest: passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("pubsub")
    p.add_argument("dir")
    p.add_argument("--firn")
    p.add_argument("--twin")
    p = sub.add_parser("ratios")
    p.add_argument("compare")
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    p.add_argument("--twin")
    sub.add_parser("selftest")
    args = parser.parse_args()
    {"pubsub": pubsub, "ratios": ratios}.get(args.command, lambda _: selftest())(args)


if __name__ == "__main__":
    sys.exit(main())
