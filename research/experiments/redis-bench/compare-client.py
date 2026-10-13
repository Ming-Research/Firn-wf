#!/usr/bin/env python3
"""Pure allocation, line validation and Redis 7.2.16 CSV logic for compare.

The shell instrument owns process execution. This helper never starts a server
or benchmark; its selftest is run by the instrument's placement-selftest in CI.
"""

import csv
import io
import math
from pathlib import Path
import re
import sys
import tempfile


LINES = {"reference", "reference-io", "valkey", "valkey-io", "dragonfly", "garnet"}
HEADER = ["test", "rps", "avg_latency_ms", "min_latency_ms", "p50_latency_ms",
          "p95_latency_ms", "p99_latency_ms", "max_latency_ms"]


def selection(images, lines, reference):
    names = []
    for pair in images.split():
        name, separator, path = pair.partition("=")
        if not separator or not path or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", name):
            raise ValueError(f"invalid IMAGES entry: {pair!r}")
        if name in names:
            raise ValueError(f"duplicate image name: {name}")
        if lines.strip() and name in LINES:
            raise ValueError(f"image name collides with competitor: {name}")
        names.append(name)
    competitors = []
    probe = False
    for line in lines.split():
        if line == "probe":
            if probe:
                raise ValueError("duplicate probe token")
            probe = True
        elif line not in LINES:
            raise ValueError(f"unknown COMPARE_LINES token: {line}")
        elif line in competitors:
            raise ValueError(f"duplicate competitor: {line}")
        else:
            competitors.append(line)
    if reference == "true" and "reference" not in competitors:
        competitors.append("reference")
    if "reference" in names and "reference" in competitors:
        raise ValueError("image name collides with reference")
    if not names and not competitors:
        raise ValueError("compare needs at least one image or competitor")
    return names, competitors, probe


def allocation(cpus, requests):
    # placement_for_count already expands and validates CPU lists.
    numbers = cpus.split(",")
    if not numbers or any(not x.isdecimal() for x in numbers) or len(set(numbers)) != len(numbers):
        raise ValueError("allocation requires distinct, expanded client CPUs")
    if not 1 <= len(numbers) <= 50:
        raise ValueError("50 connections require between 1 and 50 client processes")
    if requests < 50:
        raise ValueError("compare requests must be at least 50")
    connections = [50 // len(numbers) + (i < 50 % len(numbers)) for i in range(len(numbers))]
    # Prefix differences preserve the exact total and apportion requests to
    # connections, so the four-connection processes do not finish too early.
    result = []
    previous = 0
    cumulative = 0
    for i, (cpu, count) in enumerate(zip(numbers, connections)):
        cumulative += count
        end = requests * cumulative // 50
        result.append((i, cpu, count, end - previous))
        previous = end
    return result


def latency(path, test):
    with Path(path).open(newline="") as stream:
        rows = list(csv.reader(stream, strict=True))
    if len(rows) != 2 or rows[0] != HEADER or len(rows[1]) != len(HEADER):
        raise ValueError(f"{path}: expected one Redis 7.2.16 benchmark CSV row and header")
    row = rows[1]
    if test.startswith("lrange_"):
        expected = f"LRANGE mylist 0 {int(test.split('_')[1]) - 1}"
    elif test == "mset":
        expected = "MSET (10 keys)"
    else:
        expected = test.upper()
    if row[0] != expected:
        raise ValueError(f"{path}: expected {expected!r}, got {row[0]!r}")
    try:
        values = [float(x) for x in row[1:]]
    except ValueError as error:
        raise ValueError(f"{path}: nonnumeric benchmark CSV") from error
    if any(not math.isfinite(x) or x < 0 for x in values) or values[0] <= 0:
        raise ValueError(f"{path}: invalid benchmark CSV values")
    if not values[2] <= values[3] <= values[4] <= values[5] <= values[6]:
        raise ValueError(f"{path}: unordered latency percentiles")
    return values[3], values[5]


def aggregate(paths, test):
    if not paths:
        raise ValueError("no benchmark CSV files")
    values = [latency(path, test) for path in paths]
    return (max(x[0] for x in values), max(x[1] for x in values)), values


def selftest():
    def rejected(function, *args):
        try:
            function(*args)
        except (ValueError, csv.Error):
            return
        raise AssertionError(f"accepted invalid input: {args!r}")

    rows = allocation(",".join(map(str, range(16, 32))), 1000003)
    assert [row[2] for row in rows] == [4, 4] + [3] * 14
    assert sum(row[2] for row in rows) == 50
    assert sum(row[3] for row in rows) == 1000003
    assert rows[0][3] == 80000 and rows[-1][3] == 60001
    for count in (1, 2, 3, 16, 50):
        for requests in (50, 51, 99999):
            rows = allocation(",".join(map(str, range(count))), requests)
            assert sum(row[2] for row in rows) == 50
            assert sum(row[3] for row in rows) == requests
            assert all(row[3] > 0 for row in rows)
    rejected(allocation, "1,1", 100)
    rejected(allocation, "1-16", 100)
    rejected(allocation, "1", 49)
    rejected(allocation, ",".join(map(str, range(51))), 100)
    assert selection("firn=/a firn-twin=/a", "reference valkey probe", "true") == (
        ["firn", "firn-twin"], ["reference", "valkey"], True)
    assert selection("old=/a", "", "true") == (["old"], ["reference"], False)
    for images, lines in (("x=/a x=/b", ""), ("valkey=/a", "probe"),
                          ("x=/a", "valkey valkey"), ("x=/a", "bad"),
                          ("x=/a", "probe probe"), ("bad", "")):
        rejected(selection, images, lines, "false")
    rejected(selection, "reference=/a", "", "true")
    with tempfile.TemporaryDirectory(prefix="compare-client-selftest-") as directory:
        path = Path(directory) / "client.csv"
        def write(title="SET", p99="7", extra=""):
            stream = io.StringIO()
            writer = csv.writer(stream)
            writer.writerow(HEADER)
            writer.writerow([title, "1234", "2", "1", "3", "4", p99, "9"])
            path.write_text(stream.getvalue() + extra)
        write()
        assert latency(path, "set") == (3, 7)
        second = Path(directory) / "second.csv"
        second.write_text(path.read_text().replace(',7,', ',8,'))
        assert aggregate([path, second], "set")[0] == (3, 8)
        rejected(aggregate, [], "set")
        write("LRANGE mylist 0 599")
        assert latency(path, "lrange_600") == (3, 7)
        write("MSET (10 keys)")
        assert latency(path, "mset") == (3, 7)
        rejected(latency, path, "get")
        for bad in ("nan", "inf", "bad", "-1", "2", "10"):
            write(p99=bad)
            rejected(latency, path, "set")
        write(extra='"SET","1"\n')
        rejected(latency, path, "set")
        path.write_text("")
        rejected(latency, path, "set")
    print("compare-client-selftest: passed")


def main():
    mode, *args = sys.argv[1:]
    if mode == "selection":
        images, competitors, probe = selection(*args)
        print("true" if probe else "false")
        print(" ".join(images))
        print(" ".join(competitors))
    elif mode == "allocation":
        for row in allocation(args[0], int(args[1])):
            print(" ".join(map(str, row)))
    elif mode == "csv":
        test, *paths = args
        quantiles, values = aggregate(paths, test)
        print(f"{quantiles[0]:.3f},{quantiles[1]:.3f}")
        for i, (p50, p99) in enumerate(values):
            print(f"{i},{p50:.3f},{p99:.3f}")
    elif mode == "selftest":
        selftest()
    else:
        raise ValueError(f"unknown mode: {mode}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, csv.Error) as error:
        print(f"compare-client: {error}", file=sys.stderr)
        sys.exit(1)
