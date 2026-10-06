#!/usr/bin/env python3
"""Check Redis suite passes, or record stable and unstable passes across runs.

Names are the literal, already cleaned names in summarize.py's tests.tsv.
Some tests share a name: a (unit, name) passes only if all its rows pass.
The recording workflow calls `record`; make redis-suite calls `check` and
`--self-test`. Keep this comparison here while the Redis suite gates firn;
remove it and its baseline files if that gate is retired.
"""

import argparse
import collections
import contextlib
import io
from pathlib import Path
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


def read_list(path, columns):
    tests = set()
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
            tests.add(key)
    return tests


def passed_tests(outcomes):
    return {key for key, results in outcomes.items() if all(r == "passed" for r in results)}


def check(run, passing, unstable):
    outcomes = read_run(run)
    required = read_list(passing, 2)
    excluded = read_list(unstable, 3)
    if required & excluded:
        raise ValueError("passing.tsv and unstable.tsv overlap: " + repr(sorted(required & excluded)))
    passed = passed_tests(outcomes)
    missing = required - passed
    for unit, name in sorted(missing):
        result = ", ".join(outcomes.get((unit, name), ["missing from run"]))
        print(f"REGRESSION\t{unit}\t{name}\t{result}")
    for unit, name in sorted(passed - required - excluded):
        print(f"CANDIDATE\t{unit}\t{name}\tadd to passing.tsv in the same PR")
    print(f"ratchet: {len(required) - len(missing)}/{len(required)} required tests passed; "
          f"{len(excluded)} unstable tests excluded")
    return 1 if missing else 0


def record(runs, out):
    counts = collections.Counter()
    for run in runs:
        counts.update(passed_tests(read_run(run)))
    out.mkdir(parents=True, exist_ok=True)
    with (out / "passing.tsv").open("w", encoding="utf-8") as passing, \
            (out / "unstable.tsv").open("w", encoding="utf-8") as unstable:
        passing.write("# unit\ttest name; passed in every recorded run\n")
        unstable.write("# unit\ttest name\treason\n")
        for (unit, name), count in sorted(counts.items()):
            if count == len(runs):
                passing.write(f"{unit}\t{name}\n")
            else:
                unstable.write(f"{unit}\t{name}\tpassed in {count} of {len(runs)} recorded runs\n")
    print(f"recorded {len(runs)} runs: {sum(c == len(runs) for c in counts.values())} stable, "
          f"{sum(c < len(runs) for c in counts.values())} unstable")
    return 0


def self_test():
    """Small independent TSV fixtures; never starts a server or the suite."""
    with tempfile.TemporaryDirectory(prefix="firn-ratchet-", dir="/tmp") as directory:
        root = Path(directory)
        run, passing, unstable = (root / name for name in ("tests.tsv", "passing.tsv", "unstable.tsv"))
        passing.write_text("unit/a\tkept\n", encoding="utf-8")
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
        passing.write_text("unit/a\tkept\nunit/b\tmissing\n", encoding="utf-8")
        case("every regression named", "unit/a\tfailed\t\t\tkept\n", 1,
             includes=("REGRESSION\tunit/a\tkept", "REGRESSION\tunit/b\tmissing"))
        passing.write_text("# empty bootstrap list\n", encoding="utf-8")
        case("empty list passes and reports candidates", kept, 0,
             includes=("CANDIDATE\tunit/a\tkept", "0/0 required tests passed"))
        case("event rows ignored", "unit/a\tblock-error\tother\terror\t-\n", 0,
             excludes=("REGRESSION", "CANDIDATE"))

        fixture(run, kept + "unit/a\tpassed\t\t\tflaky\nunit/a\tfailed\t\t\tnever\n")
        second = root / "second.tsv"
        fixture(second, kept + kept + "unit/a\tfailed\t\t\tflaky\nunit/b\tpassed\t\t\tonce\n")
        record([run, second], root / "recorded")
        assert (root / "recorded/passing.tsv").read_text() == (
            "# unit\ttest name; passed in every recorded run\nunit/a\tkept\n")
        assert (root / "recorded/unstable.tsv").read_text() == (
            "# unit\ttest name\treason\n"
            "unit/a\tflaky\tpassed in 1 of 2 recorded runs\n"
            "unit/b\tonce\tpassed in 1 of 2 recorded runs\n")
        print("PASS: recording intersection, unstable reasons, missing tests, deduplication and sorting")

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
        for content in ("unit/a\n", "unit/a\tkept\nunit/a\tkept\n", "unit/a\tflaky\n"):
            passing.write_text(content, encoding="utf-8")
            invalid("malformed or overlapping list rejected", lambda: check(run, passing, unstable))
    return 0


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
    args = parser.parse_args()
    try:
        if args.self_test:
            return self_test()
        if args.command == "check":
            return check(args.run, args.passing, args.unstable)
        if args.command == "record":
            return record(args.runs, args.out)
        parser.error("choose check, record or --self-test")
    except (OSError, ValueError) as error:
        print(f"ratchet: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
