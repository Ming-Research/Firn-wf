#!/usr/bin/env python3
"""Check Redis suite passes, record stable and unstable passes across runs, or split and merge shards.

Names are the literal, already cleaned names in summarize.py's tests.tsv.
Some tests share a name: a (unit, name) passes only if all its rows pass,
and passing.tsv records how many rows passed, so a run in which one of them
is missing, as when its unit stops early, is a regression too.
The recording workflow calls `record`; make redis-suite calls `check` and
`--self-test`. Keep this comparison here while the Redis suite gates firn;
remove it and its baseline files if that gate is retired.

A run may be split into shards, each a run.sh run of its own. run.sh calls
`assign` to pick its units, and run-shards.sh calls `merge`, which refuses
shards that do not together run each unit of the suite exactly once.
"""

import argparse
import collections
import contextlib
import io
import math
from pathlib import Path
import re
import sys
import tempfile

from summarize import EVENT_OUTCOMES, TEST_OUTCOMES


def read_run(path):
    """Read literal TSV, not CSV: quote characters in test names are data."""
    outcomes = collections.defaultdict(list)
    with path.open(encoding="utf-8") as source:
        if source.readline().rstrip("\n") != "unit\toutcome\tcause\tdetail\ttest":
            raise ValueError(f"{path}: expected summarize.py's tests.tsv header")
        for number, line in enumerate(source, 2):
            cells = line.rstrip("\n").split("\t")
            if len(cells) != 5:
                raise ValueError(f"{path}:{number}: expected five tab-separated columns")
            unit, outcome, _, _, name = cells
            if not unit or not name or outcome not in TEST_OUTCOMES + EVENT_OUTCOMES:
                raise ValueError(f"{path}:{number}: invalid unit, test or outcome")
            if name != "-":
                outcomes[(unit, name)].append(outcome)
    return outcomes


# A test's passes must be seen in this many recorded runs before it is
# required: one run cannot tell a stable pass from a flaky one.
MINIMUM_RUNS = 3


def read_list(path, columns):
    """Read passing.tsv (unit, name, rows) or unstable.tsv (unit, name, reason)."""
    tests = {}
    with path.open(encoding="utf-8") as source:
        for number, line in enumerate(source, 1):
            if line.startswith("#") or not line.strip():
                continue
            cells = line.rstrip("\n").split("\t")
            if len(cells) != columns or any(not cell for cell in cells) or cells[1] == "-":
                raise ValueError(f"{path}:{number}: expected {columns} nonempty columns")
            key = tuple(cells[:2])
            if key in tests:
                raise ValueError(f"{path}:{number}: duplicate test {key!r}")
            tests[key] = cells[2]
    return tests


def read_passing(path):
    """passing.tsv's third column is how many rows of the test must pass."""
    required = {}
    for key, rows in read_list(path, 3).items():
        if not rows.isdigit() or int(rows) < 1:
            raise ValueError(f"{path}: {key!r}: rows must be a positive integer, not {rows!r}")
        required[key] = int(rows)
    return required


def passed_tests(outcomes):
    """Each test whose rows all passed, with how many rows it had."""
    return {key: len(results) for key, results in outcomes.items()
            if all(r == "passed" for r in results)}


def check(run, passing, unstable):
    outcomes = read_run(run)
    required = read_passing(passing)
    excluded = set(read_list(unstable, 3))
    overlap = set(required) & excluded
    if overlap:
        raise ValueError("passing.tsv and unstable.tsv overlap: " + repr(sorted(overlap)))
    passed = passed_tests(outcomes)
    missing = {key for key, rows in required.items() if passed.get(key, 0) < rows}
    for unit, name in sorted(missing):
        results = outcomes.get((unit, name))
        if not results:
            result = "missing from run"
        elif all(r == "passed" for r in results):
            result = f"{len(results)} of {required[(unit, name)]} passing rows ran"
        else:
            result = ", ".join(results)
        print(f"REGRESSION\t{unit}\t{name}\t{result}")
    for unit, name in sorted(set(passed) - set(required) - excluded):
        print(f"CANDIDATE\t{unit}\t{name}\tadd to passing.tsv in the same PR")
    print(f"ratchet: {len(required) - len(missing)}/{len(required)} required tests passed; "
          f"{len(excluded)} unstable tests excluded")
    return 1 if missing else 0


def record(runs, out):
    if len(runs) < MINIMUM_RUNS:
        raise ValueError(f"recording needs at least {MINIMUM_RUNS} runs, not {len(runs)}")
    rows = collections.defaultdict(list)
    for run in runs:
        for key, count in passed_tests(read_run(run)).items():
            rows[key].append(count)
    out.mkdir(parents=True, exist_ok=True)
    stable = unstable_count = 0
    with (out / "passing.tsv").open("w", encoding="utf-8") as passing, \
            (out / "unstable.tsv").open("w", encoding="utf-8") as unstable:
        passing.write("# unit\ttest name\trows; all its rows passed in every recorded run\n")
        unstable.write("# unit\ttest name\treason\n")
        for (unit, name), counts in sorted(rows.items()):
            if len(counts) < len(runs):
                reason = f"passed in {len(counts)} of {len(runs)} recorded runs"
            elif len(set(counts)) > 1:
                reason = f"passed with {', '.join(map(str, counts))} rows in {len(runs)} recorded runs"
            else:
                passing.write(f"{unit}\t{name}\t{counts[0]}\n")
                stable += 1
                continue
            unstable.write(f"{unit}\t{name}\t{reason}\n")
            unstable_count += 1
    print(f"recorded {len(runs)} runs: {stable} stable, {unstable_count} unstable")
    return 0


# A unit missing from unit-seconds.tsv counts this many seconds: the mean of
# the units that file leaves out in the run it was taken from.
UNLISTED_UNIT_SECONDS = 0.3


def parse_shard(spec):
    """Shard I of N, written I/N with 1 <= I <= N; returns (I - 1, N)."""
    match = re.fullmatch(r"([1-9][0-9]*)/([1-9][0-9]*)", spec)
    if not match or int(match[1]) > int(match[2]):
        raise ValueError(f"shard {spec!r}: expected I/N with 1 <= I <= N")
    return int(match[1]) - 1, int(match[2])


def read_costs(path):
    """unit-seconds.tsv: a unit and the seconds it takes, one row each."""
    costs = {}
    with path.open(encoding="utf-8") as source:
        for number, line in enumerate(source, 1):
            if line.startswith("#") or not line.strip():
                continue
            cells = line.rstrip("\n").split("\t")
            if len(cells) != 2 or not cells[0] or cells[0] in costs:
                raise ValueError(f"{path}:{number}: expected a unit not listed before, then its seconds")
            try:
                seconds = float(cells[1])
            except ValueError:
                seconds = math.nan
            if not math.isfinite(seconds) or seconds <= 0:
                raise ValueError(f"{path}:{number}: seconds must be a positive number, not {cells[1]!r}")
            costs[cells[0]] = seconds
    return costs


def assign(units, costs, count):
    """The shard, 0-based, that each unit runs in: heaviest first, each to the
    least loaded shard, the lowest index among equals, so the same units and
    costs always split the same way and no unit lands in two shards."""
    if len(set(units)) != len(units):
        raise ValueError("a unit is listed twice")
    loads = [0.0] * count
    shard_of = {}
    for unit in sorted(units, key=lambda u: (-costs.get(u, UNLISTED_UNIT_SECONDS), u)):
        target = min(range(count), key=lambda s: (loads[s], s))
        shard_of[unit] = target
        loads[target] += costs.get(unit, UNLISTED_UNIT_SECONDS)
    return shard_of


def read_unit_list(path, allow_empty=False):
    units = [line for line in path.read_text(encoding="utf-8").split("\n") if line]
    if len(set(units)) != len(units):
        raise ValueError(f"{path}: a unit is listed twice")
    if not units and not allow_empty:
        raise ValueError(f"{path}: no units")
    return units


def read_table(path, header=None):
    """A TSV with a header line: (header, rows); a given header must match."""
    lines = path.read_text(encoding="utf-8").split("\n")
    if not lines[0] or (header is not None and lines[0] != header):
        raise ValueError(f"{path}: expected the header {header!r}")
    return lines[0], [line for line in lines[1:] if line]


def merge(shards, out):
    """Combine the run directories of shards into out, after checking that
    they list the same suite and that its units each ran in exactly one."""
    if not shards:
        raise ValueError("merging needs at least one shard")
    suite = read_unit_list(shards[0] / "suite-units.txt")
    owner = {}
    tables = collections.defaultdict(list)
    meta = [f"sharded run: {len(shards)} shards, merged by ratchet.py"]
    for shard in shards:
        if read_unit_list(shard / "suite-units.txt") != suite:
            raise ValueError(f"{shard} lists a different suite than {shards[0]}")
        assigned = read_unit_list(shard / "shard-units.txt", allow_empty=True)
        for unit in assigned:
            if unit in owner:
                raise ValueError(f"unit {unit} runs in both {owner[unit]} and {shard}")
            owner[unit] = shard
        for name in ("attempts.tsv", "units.tsv", "tests.tsv"):
            header, rows = read_table(shard / name, tables[name][0][0] if tables[name] else None)
            tables[name].append((header, rows))
            ran = {row.split("\t", 1)[0] for row in rows}
            if name == "units.tsv" or name == "attempts.tsv":
                if ran != set(assigned):
                    raise ValueError(f"{shard}: {name} covers {sorted(ran ^ set(assigned))} "
                                     f"differently than the units assigned to it")
            elif not ran <= set(assigned):
                raise ValueError(f"{shard}: tests.tsv has rows of units not assigned to it: "
                                 f"{sorted(ran - set(assigned))}")
        read_run(shard / "tests.tsv")
        meta += ["", f"== {shard.name} ==", (shard / "meta.txt").read_text(encoding="utf-8").rstrip("\n")]
    missing = [unit for unit in suite if unit not in owner]
    if missing:
        raise ValueError(f"units of the suite that no shard ran: {missing}")
    extra = [unit for unit in owner if unit not in set(suite)]
    if extra:
        raise ValueError(f"units that are not in the suite: {extra}")
    out.mkdir(parents=True, exist_ok=True)
    for name, parts in tables.items():
        with (out / name).open("w", encoding="utf-8") as merged:
            merged.write(parts[0][0] + "\n")
            for _, rows in parts:
                merged.writelines(row + "\n" for row in rows)
    (out / "suite-units.txt").write_text("".join(unit + "\n" for unit in suite), encoding="utf-8")
    (out / "meta.txt").write_text("\n".join(meta) + "\n", encoding="utf-8")
    print(f"merged {len(shards)} shards: {len(suite)} units, "
          f"{sum(len(rows) for _, rows in tables['tests.tsv'])} test rows")
    return 0


def self_test():
    """Small independent TSV fixtures; never starts a server or the suite."""
    with tempfile.TemporaryDirectory(prefix="firn-ratchet-", dir="/tmp") as directory:
        root = Path(directory)
        run, passing, unstable = (root / name for name in ("tests.tsv", "passing.tsv", "unstable.tsv"))
        passing.write_text("unit/a\tkept\t1\n", encoding="utf-8")
        unstable.write_text("unit/a\tflaky\tpassed in 1 of 2 recorded runs\n", encoding="utf-8")

        def fixture(path, rows):
            path.write_text("unit\toutcome\tcause\tdetail\ttest\n" + rows, encoding="utf-8")

        def case(label, rows, expected_status, includes=(), excludes=()):
            fixture(run, rows)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                status = check(run, passing, unstable)
            assert status == expected_status, (label, status, output.getvalue())
            for text in includes:
                assert text in output.getvalue(), (label, text, output.getvalue())
            for text in excludes:
                assert text not in output.getvalue(), (label, text, output.getvalue())
            print(f"PASS: {label}")

        kept = "unit/a\tpassed\t\t\tkept\n"
        case("all listed pass", kept, 0, excludes=("REGRESSION", "CANDIDATE"))
        for outcome in TEST_OUTCOMES:
            if outcome != "passed":
                case(f"listed {outcome}", f"unit/a\t{outcome}\t\t\tkept\n", 1,
                     includes=(f"REGRESSION\tunit/a\tkept\t{outcome}",))
        case("listed missing", "unit/b\tpassed\t\t\tkept\n", 1,
             includes=("REGRESSION\tunit/a\tkept\tmissing from run",))
        case("new passing candidate", kept + 'unit/a\tpassed\t\t\t"new"\n', 0,
             includes=('CANDIDATE\tunit/a\t"new"\tadd to passing.tsv',))
        for outcome in ("passed", "failed"):
            case(f"unstable {outcome} ignored", kept + f"unit/a\t{outcome}\t\t\tflaky\n", 0,
                 excludes=("REGRESSION", "CANDIDATE"))
        case("mixed duplicate fails", kept + "unit/a\tfailed\t\t\tkept\n", 1,
             includes=("REGRESSION\tunit/a\tkept\tpassed, failed",))
        case("passing duplicates pass", kept + kept, 0, excludes=("REGRESSION",))
        passing.write_text("unit/a\tkept\t2\n", encoding="utf-8")
        case("both recorded rows pass", kept + kept, 0, excludes=("REGRESSION",))
        case("a missing duplicate row fails", kept, 1,
             includes=("REGRESSION\tunit/a\tkept\t1 of 2 passing rows ran",))
        case("an extra passing row passes", kept + kept + kept, 0, excludes=("REGRESSION",))
        passing.write_text("unit/a\tkept\t1\nunit/b\tmissing\t1\n", encoding="utf-8")
        case("every regression named", "unit/a\tfailed\t\t\tkept\n", 1,
             includes=("REGRESSION\tunit/a\tkept", "REGRESSION\tunit/b\tmissing"))
        passing.write_text("# empty bootstrap list\n", encoding="utf-8")
        case("empty list passes and reports candidates", kept, 0,
             includes=("CANDIDATE\tunit/a\tkept", "0/0 required tests passed"))
        case("event rows ignored", "unit/a\tblock-error\tother\terror\t-\n", 0,
             excludes=("REGRESSION", "CANDIDATE"))

        dup = "unit/a\tpassed\t\t\tdup\n"
        fixture(run, kept + dup + dup + "unit/a\tpassed\t\t\tflaky\nunit/a\tfailed\t\t\tnever\n")
        second = root / "second.tsv"
        fixture(second, kept + dup + "unit/a\tfailed\t\t\tflaky\nunit/b\tpassed\t\t\tonce\n")
        third = root / "third.tsv"
        fixture(third, kept + dup + "unit/a\tfailed\t\t\tflaky\n")
        record([run, second, third], root / "recorded")
        assert (root / "recorded/passing.tsv").read_text() == (
            "# unit\ttest name\trows; all its rows passed in every recorded run\nunit/a\tkept\t1\n")
        assert (root / "recorded/unstable.tsv").read_text() == (
            "# unit\ttest name\treason\n"
            "unit/a\tdup\tpassed with 2, 1, 1 rows in 3 recorded runs\n"
            "unit/a\tflaky\tpassed in 1 of 3 recorded runs\n"
            "unit/b\tonce\tpassed in 1 of 3 recorded runs\n")
        print("PASS: recording intersection, row counts, unstable reasons, missing tests and sorting")
        invalid_runs = root / "too-few"
        try:
            record([run, second], invalid_runs)
        except ValueError:
            print("PASS: recording refuses fewer than three runs")
        else:
            raise AssertionError("recording accepted two runs")

        def invalid(label, action):
            try:
                action()
            except ValueError:
                print(f"PASS: {label}")
            else:
                raise AssertionError(f"{label}: invalid input accepted")

        for content in ("", "unit\ttest\n", "unit\toutcome\tcause\tdetail\ttest\nunit/a\tpassed\n",
                        "unit\toutcome\tcause\tdetail\ttest\nunit/a\tunknown\t\t\tkept\n"):
            run.write_text(content, encoding="utf-8")
            invalid("malformed run rejected", lambda: read_run(run))
        fixture(run, kept)
        for content in ("unit/a\n", "unit/a\tkept\n", "unit/a\tkept\t0\n", "unit/a\tkept\tx\n",
                        "unit/a\tkept\t1\nunit/a\tkept\t1\n", "unit/a\tflaky\t1\n"):
            passing.write_text(content, encoding="utf-8")
            invalid("malformed or overlapping list rejected", lambda: check(run, passing, unstable))
        shard_self_test(root, passing, unstable)
    return 0


def shard_self_test(root, passing, unstable):
    """Splitting units into shards and merging the shards' runs."""

    def rejects(label, text, action):
        try:
            action()
        except ValueError as error:
            assert text in str(error), (label, text, str(error))
            print(f"PASS: {label}")
        else:
            raise AssertionError(f"{label}: invalid input accepted")

    costs = {"unit/heavy": 100.0, "unit/mid": 30.0, "unit/small": 20.0}
    units = ["unit/a", "unit/small", "unit/heavy", "unit/b", "unit/mid", "unit/c"]
    split = assign(units, costs, 2)
    assert split == assign(units, costs, 2), "the split is not deterministic"
    assert split["unit/heavy"] == 0 and [u for u in units if split[u] == 0] == ["unit/heavy"], split
    assert sorted(split) == sorted(units) and set(split.values()) == {0, 1}, split
    assert set(assign(units, costs, 1).values()) == {0}
    print("PASS: the heaviest unit runs alone in the first shard, every unit in exactly one shard")
    assert [parse_shard(s) for s in ("1/1", "2/3")] == [(0, 1), (1, 3)]
    for spec in ("0/2", "3/2", "1/0", "1", "a/2", "1/2/3", " 1/2"):
        rejects(f"shard {spec!r} rejected", "expected I/N", lambda: parse_shard(spec))
    rejects("a unit listed twice cannot be split", "listed twice",
            lambda: assign(["unit/a", "unit/a"], costs, 2))
    table = root / "unit-seconds.tsv"
    table.write_text("# unit\tseconds\nunit/a\t1.5\n", encoding="utf-8")
    assert read_costs(table) == {"unit/a": 1.5}
    for content in ("unit/a\n", "unit/a\t0\n", "unit/a\t-1\n", "unit/a\tx\n", "unit/a\tnan\n",
                    "unit/a\t1\nunit/a\t2\n", "unit/a\t1\t2\n"):
        table.write_text(content, encoding="utf-8")
        rejects("malformed costs rejected", "unit-seconds.tsv", lambda: read_costs(table))

    header = "unit\toutcome\tcause\tdetail\ttest\n"
    suite = ["unit/a", "unit/b", "unit/c"]

    def shard_dir(name, assigned, listed=suite, ran=None, rows=None):
        directory = root / name
        directory.mkdir()
        (directory / "suite-units.txt").write_text("".join(u + "\n" for u in listed), encoding="utf-8")
        (directory / "shard-units.txt").write_text("".join(u + "\n" for u in assigned), encoding="utf-8")
        ran = assigned if ran is None else ran
        (directory / "attempts.tsv").write_text(
            "unit\tattempt\truntest_exit\tserver\n" + "".join(f"{u}\t1\t0\trunning\n" for u in ran),
            encoding="utf-8")
        (directory / "units.tsv").write_text(
            "unit\tpassed\n" + "".join(f"{u}\t1\n" for u in ran), encoding="utf-8")
        rows = [f"{u}\tpassed\t\t\tkept\n" for u in ran] if rows is None else rows
        (directory / "tests.tsv").write_text(header + "".join(rows), encoding="utf-8")
        (directory / "meta.txt").write_text(f"shard {name}\n", encoding="utf-8")
        return directory

    def merged(label, shards, text):
        out = root / f"merged-{label}"
        rejects(label, text, lambda: merge(shards, out))

    one = shard_dir("ok-1", ["unit/a"])
    two = shard_dir("ok-2", ["unit/b", "unit/c"])
    with contextlib.redirect_stdout(io.StringIO()):
        merge([one, two], root / "merged")
    assert (root / "merged/tests.tsv").read_text() == (
        header + "unit/a\tpassed\t\t\tkept\nunit/b\tpassed\t\t\tkept\nunit/c\tpassed\t\t\tkept\n")
    assert (root / "merged/suite-units.txt").read_text() == "unit/a\nunit/b\nunit/c\n"
    assert "== ok-2 ==" in (root / "merged/meta.txt").read_text()
    with contextlib.redirect_stdout(io.StringIO()):
        passing.write_text("unit/a\tkept\t1\nunit/c\tkept\t1\n", encoding="utf-8")
        assert check(root / "merged/tests.tsv", passing, unstable) == 0
        failing = shard_dir("ok-3", ["unit/b", "unit/c"], rows=["unit/b\tpassed\t\t\tkept\n"])
        merge([one, failing], root / "merged-failing")
        assert check(root / "merged-failing/tests.tsv", passing, unstable) == 1
    print("PASS: shards merge into one run that check applies to, a regression in either shard fails it")

    merged("a unit in two shards rejected", [one, shard_dir("dup", ["unit/a", "unit/b", "unit/c"])],
           "unit unit/a runs in both")
    merged("a unit no shard ran rejected", [one, shard_dir("gap", ["unit/b"])],
           "no shard ran: ['unit/c']")
    merged("a shard left out rejected", [two], "no shard ran: ['unit/a']")
    merged("a unit outside the suite rejected", [one, shard_dir("extra", ["unit/b", "unit/c", "unit/d"])],
           "not in the suite: ['unit/d']")
    merged("shards of different suites rejected",
           [one, shard_dir("other", ["unit/b", "unit/c"], listed=["unit/a", "unit/b"])],
           "lists a different suite")
    merged("an assigned unit that did not run rejected",
           [one, shard_dir("idle", ["unit/b", "unit/c"], ran=["unit/b"])], "attempts.tsv covers ['unit/c']")
    merged("a unit run without assignment rejected",
           [one, shard_dir("stray", ["unit/b", "unit/c"], ran=["unit/b", "unit/c", "unit/a"])],
           "attempts.tsv covers ['unit/a']")
    merged("test rows of an unassigned unit rejected",
           [one, shard_dir("rows", ["unit/b", "unit/c"], rows=["unit/a\tpassed\t\t\tkept\n"])],
           "tests.tsv has rows of units not assigned")
    merged("an empty suite list rejected", [shard_dir("empty", [], listed=[])], "no units")
    merged("no shards rejected", [], "at least one shard")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--self-test", action="store_true")
    commands = parser.add_subparsers(dest="command")
    checker = commands.add_parser("check", help="check a run against the committed ratchet")
    checker.add_argument("run", type=Path)
    checker.add_argument("--passing", type=Path, required=True)
    checker.add_argument("--unstable", type=Path, required=True)
    recorder = commands.add_parser("record", help="generate lists from repeated CI runs")
    recorder.add_argument("runs", nargs="+", type=Path)
    recorder.add_argument("--out", type=Path, required=True)
    assigner = commands.add_parser("assign", help="print the units of one shard, reading the suite's units from stdin")
    assigner.add_argument("--costs", type=Path, required=True)
    assigner.add_argument("--shard", required=True)
    merger = commands.add_parser("merge", help="combine the run directories of shards")
    merger.add_argument("shards", nargs="+", type=Path)
    merger.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.self_test:
            return self_test()
        if args.command == "check":
            return check(args.run, args.passing, args.unstable)
        if args.command == "record":
            return record(args.runs, args.out)
        if args.command == "assign":
            index, count = parse_shard(args.shard)
            units = sys.stdin.read().split()
            shard_of = assign(units, read_costs(args.costs), count)
            print("".join(unit + "\n" for unit in units if shard_of[unit] == index), end="")
            return 0
        if args.command == "merge":
            return merge(args.shards, args.out)
        parser.error("choose check, record, assign, merge or --self-test")
    except (OSError, ValueError) as error:
        print(f"ratchet: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
