"""Compare the cases a consumer's tests passed on Redis 7.2.16 and on firn.

    python3 compare.py --redis REDIS_CASES [--redis REDIS_CASES ...]
        --firn FIRN_CASES [--unstable LIST] [--unstable-found FILE]
        [--redis-what WHAT --firn-what WHAT] [--title TEXT] > summary.md
    python3 compare.py --self-test

It knows no consumer. Its inputs are the per-case records run.sh writes
(cases.tsv), whose format every consumer's reporter shares:

    CASE<TAB>STATUS

CASE names the case, led by the test file it belongs to and without a tab or
a newline, as `FILE::NAME`; STATUS is pass, fail, error, skip or unfinished. A
file whose process ran out of time or recorded no case gets the row
`FILE<TAB>unfinished`; a row `FILE::NAME<TAB>unfinished` is a case that did not
end. A case that has no row did not pass. A case with several rows passed when
all its rows did. Lines that start with `#` and blank lines
are ignored.

The unstable list, which the output also writes in this format, is:

    CASE<TAB>REASON

with the header `# case<TAB>reason`, as in tests/redis-suite/unstable.tsv, and
the reason `passed in K of N recorded runs` for a case that passed in some of
the Redis runs and not all. A case in the list, or one that the Redis runs
disagree on, is left out of the comparison and named apart.

Redis's stable passes are the cases that passed in every Redis run. The output
is Markdown for the workflow's job summary: how many cases both servers
passed, only Redis passed and only firn passed, and each case's name.
--redis-what and --firn-what name the two cells' what.txt files, whose lines
after the first (the versions of the tools and gems the run installed) the
comparison reports when they differ. The exit status is 0 when the comparison
was made, and 1 when an input is missing or malformed, or no Redis run has a
case row.
"""

import argparse
import sys
import tempfile
from pathlib import Path

STATUSES = ("pass", "fail", "error", "skip", "unfinished")


class RecordError(Exception):
    pass


def read_record(path):
    """The record's cases and unfinished files: ({case: [status]}, [file])."""
    cases = {}
    unfinished = []
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        raise RecordError(f"{path}: {error.strerror}") from error
    for number, line in enumerate(text.split("\n"), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2 or not fields[0]:
            raise RecordError(f"{path}:{number}: expected CASE<TAB>STATUS, got {line!r}")
        case, status = fields
        if status not in STATUSES:
            raise RecordError(f"{path}:{number}: unknown status {status!r}")
        if status == "unfinished" and "::" not in case:
            unfinished.append(case)
        else:
            cases.setdefault(case, []).append(status)
    return cases, unfinished


def read_unstable(path):
    """The unstable list's cases and reasons: {case: reason}."""
    listed = {}
    try:
        text = Path(path).read_text(encoding="utf-8")
    except OSError as error:
        raise RecordError(f"{path}: {error.strerror}") from error
    for number, line in enumerate(text.split("\n"), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2 or not all(fields):
            raise RecordError(f"{path}:{number}: expected CASE<TAB>REASON, got {line!r}")
        if fields[0] in listed:
            raise RecordError(f"{path}:{number}: {fields[0]!r} is listed twice")
        listed[fields[0]] = fields[1]
    return listed


def passed(statuses):
    return all(status == "pass" for status in statuses)


def environment(path):
    """The what.txt lines after the first, which names the cell itself."""
    try:
        lines = Path(path).read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise RecordError(f"{path}: {error.strerror}") from error
    return lines[1:]


def compare(redis_runs, firn, listed):
    """Sort the cases into the sets the report names.

    redis_runs is a list of (cases, unfinished) from the Redis runs; firn is
    one (cases, unfinished); listed is the checked-in unstable list.
    """
    names = set(firn[0])
    for cases, _ in redis_runs:
        names.update(cases)
    found = {}
    for name in sorted(names):
        results = [passed(cases[name]) if name in cases else False for cases, _ in redis_runs]
        if any(results) and not all(results):
            found[name] = f"passed in {sum(results)} of {len(results)} recorded runs"
    excluded = set(listed) | set(found)
    redis_pass = {name for name in names - excluded
                  if all(name in cases and passed(cases[name]) for cases, _ in redis_runs)}
    firn_pass = {name for name in names - excluded
                 if name in firn[0] and passed(firn[0][name])}
    return {
        "both": sorted(redis_pass & firn_pass),
        "only_redis": sorted(redis_pass - firn_pass),
        "only_firn": sorted(firn_pass - redis_pass),
        "neither": sorted(names - excluded - redis_pass - firn_pass),
        "unstable": {name: listed.get(name) or found[name] for name in sorted(excluded & names)},
        "found": {name: reason for name, reason in found.items() if name not in listed},
    }


def tally(cases):
    counts = {status: 0 for status in STATUSES}
    for statuses in cases.values():
        counts["pass" if passed(statuses) else next(s for s in statuses if s != "pass")] += 1
    return counts


def describe(cases, unfinished_files):
    """One line: the cases by status, then the files that did not finish."""
    counts = tally(cases)
    parts = [f"{counts[status]} {status}" for status in STATUSES if status != "unfinished"]
    parts.append(f"{counts['unfinished']} unfinished")
    return f"{len(cases)} cases, " + ", ".join(parts) + f"; {len(unfinished_files)} unfinished files"


def names_block(title, names):
    if not names:
        return [f"{title}: none", ""]
    return [f"<details><summary>{title}: {len(names)}</summary>", "", "```text", *names, "```", "</details>", ""]


def report(title, redis_runs, firn, sets, redis_what=None, firn_what=None):
    out = [f"## {title}", ""]
    out.append("| | cases |")
    out.append("|---|---|")
    out.append(f"| both pass | {len(sets['both'])} |")
    out.append(f"| only Redis passes | {len(sets['only_redis'])} |")
    out.append(f"| only firn passes | {len(sets['only_firn'])} |")
    out.append(f"| neither passes | {len(sets['neither'])} |")
    out.append(f"| unstable, left out of the comparison | {len(sets['unstable'])} |")
    out.append("")
    for index, (cases, unfinished) in enumerate(redis_runs, 1):
        out.append(f"Redis run {index}: {describe(cases, unfinished)}")
    out.append(f"firn: {describe(*firn)}")
    out.append("")
    if redis_what is not None and firn_what is not None and redis_what != firn_what:
        out.append("**The two cells installed different versions**, so a difference may come from them:")
        out.append("")
        out.append("```text")
        out.extend(f"redis: {line}" for line in redis_what)
        out.extend(f"firn:  {line}" for line in firn_what)
        out.append("```")
        out.append("")
    for title_, names in (("Unfinished files on Redis", sorted({f for _, u in redis_runs for f in u})),
                          ("Unfinished files on firn", sorted(firn[1]))):
        out.extend(names_block(title_, names))
    out.extend(names_block("Both pass", sets["both"]))
    out.extend(names_block("Only Redis passes", sets["only_redis"]))
    out.extend(names_block("Only firn passes", sets["only_firn"]))
    out.extend(names_block("Neither passes", sets["neither"]))
    out.extend(names_block("Unstable on Redis, left out",
                           [f"{case}\t{reason}" for case, reason in sets["unstable"].items()]))
    return "\n".join(out)


def write_unstable(path, found):
    with open(path, "w", encoding="utf-8") as out:
        out.write("# case\treason\n")
        for case, reason in found.items():
            out.write(f"{case}\t{reason}\n")


def run(args):
    redis_runs = [read_record(path) for path in args.redis]
    firn = read_record(args.firn)
    if not any(cases for cases, _ in redis_runs):
        raise RecordError("no Redis run has a case row")
    listed = read_unstable(args.unstable) if args.unstable else {}
    sets = compare(redis_runs, firn, listed)
    redis_what = firn_what = None
    if args.redis_what and args.firn_what:
        redis_what, firn_what = environment(args.redis_what), environment(args.firn_what)
    print(report(args.title, redis_runs, firn, sets, redis_what, firn_what))
    if args.unstable_found:
        write_unstable(args.unstable_found, sets["found"])


def self_test():
    """Check each set and each rejection with records whose answer is known."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)

        def write(name, text):
            (root / name).write_text(text, encoding="utf-8")
            return root / name

        redis1 = write("redis1.tsv", "f::a\tpass\nf::b\tpass\nf::c\tfail\nf::d\tpass\nf::e\tskip\ng::x\tpass\ng::y\tpass\nh\tunfinished\n")
        redis2 = write("redis2.tsv", "f::a\tpass\nf::b\tpass\nf::c\tfail\nf::d\tfail\nf::e\tskip\ng::x\tpass\ng::y\tpass\nh\tunfinished\n")
        # g::q began and did not end: a case row whose status is unfinished.
        firn = write("firn.tsv", "f::a\tpass\nf::b\terror\nf::c\tpass\nf::d\tpass\nf::e\tskip\ng::z\tpass\n# comment\n\ng::q\tunfinished\ng\tunfinished\n")
        # g::x and g::y have no firn row, so firn did not pass them.
        unstable = write("unstable.tsv", "# case\treason\ng::y\tpassed in 1 of 3 recorded runs\n")
        sets = compare([read_record(redis1), read_record(redis2)], read_record(firn), read_unstable(unstable))
        assert sets["both"] == ["f::a"], sets
        # f::b errors on firn; g::x is in a file firn did not finish.
        assert sets["only_redis"] == ["f::b", "g::x"], sets
        # f::c fails on Redis and passes on firn; g::z is only on firn.
        assert sets["only_firn"] == ["f::c", "g::z"], sets
        # f::e is skipped on both.
        assert sets["neither"] == ["f::e", "g::q"], sets
        # f::d passed in one Redis run of two; g::y is on the list.
        assert sets["unstable"] == {"f::d": "passed in 1 of 2 recorded runs",
                                    "g::y": "passed in 1 of 3 recorded runs"}, sets
        assert sets["found"] == {"f::d": "passed in 1 of 2 recorded runs"}, sets
        # A single Redis run has nothing to disagree with.
        single = compare([read_record(redis1)], read_record(firn), {})
        assert single["both"] == ["f::a", "f::d"] and single["unstable"] == {}, single
        # A case with several rows passes only when every row does.
        rows = write("rows.tsv", "f::a\tpass\nf::a\tfail\nf::b\tpass\nf::b\tpass\n")
        assert passed(read_record(rows)[0]["f::a"]) is False
        assert passed(read_record(rows)[0]["f::b"]) is True
        assert read_record(firn)[1] == ["g"], read_record(firn)

        text = report("t", [read_record(redis1), read_record(redis2)], read_record(firn), sets,
                      ["ruby 4.0", "gems: a (1)"], ["ruby 4.0", "gems: a (2)"])
        assert "| both pass | 1 |" in text and "| only Redis passes | 2 |" in text and "| only firn passes | 2 |" in text, text
        assert "different versions" in text and "firn:  gems: a (2)" in text, text
        # firn's g::q is a case that did not end, and its file g did not finish.
        assert "1 unfinished; 1 unfinished files" in text, text
        same = report("t", [read_record(redis1)], read_record(firn), single, ["x"], ["x"])
        assert "different versions" not in same

        found_path = root / "found.tsv"
        write_unstable(found_path, sets["found"])
        assert found_path.read_text() == "# case\treason\nf::d\tpassed in 1 of 2 recorded runs\n"
        assert read_unstable(found_path) == sets["found"]

        # Input that cannot be read as a record is refused, not guessed at.
        refused = [
            ("three columns", write("bad1.tsv", "f::a\tpass\textra\n")),
            ("unknown status", write("bad2.tsv", "f::a\tpassed\n")),
            ("no tab", write("bad3.tsv", "f::a pass\n")),
            ("missing file", root / "absent.tsv"),
        ]
        for label, path in refused:
            try:
                read_record(path)
            except RecordError:
                continue
            raise AssertionError(f"{label} was accepted")
        for label, text_ in (("one column", "f::a\n"), ("listed twice", "f::a\tr\nf::a\tr\n")):
            try:
                read_unstable(write("bad.list", text_))
            except RecordError:
                continue
            raise AssertionError(f"unstable list: {label} was accepted")
        empty = write("empty.tsv", "# nothing ran\n")
        arguments = argparse.Namespace(redis=[str(empty)], firn=str(firn), unstable=None,
                                       unstable_found=None, redis_what=None, firn_what=None, title="t")
        try:
            run(arguments)
        except RecordError:
            pass
        else:
            raise AssertionError("a Redis run without a case row was accepted")
    print("PASS: sets, stability, versions, unfinished files and refused input")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--redis", action="append", default=[], help="a Redis run's cases.tsv; repeat for more runs")
    parser.add_argument("--firn", help="firn's cases.tsv")
    parser.add_argument("--unstable", help="the checked-in unstable list")
    parser.add_argument("--unstable-found", help="write the cases the Redis runs disagree on, not yet in the list, here")
    parser.add_argument("--redis-what")
    parser.add_argument("--firn-what")
    parser.add_argument("--title", default="Redis 7.2.16 against firn")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.redis or not args.firn:
        parser.error("--redis and --firn are required")
    try:
        run(args)
    except RecordError as error:
        print(f"compare.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
