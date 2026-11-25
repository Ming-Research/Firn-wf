#!/bin/sh
# Python's JSON parser checks structured compiler diagnostics; grep cannot
# distinguish a source rejection from a quoted rule in an unrelated failure.
set -eu
exec "${PY:-python3}" - "$0" "$@" <<'PY'
import argparse
import collections
import contextlib
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


class Failure(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Failure(message)


def source_bytes(path):
    require(path.is_file(), f"missing witness: {path}")
    return path.read_bytes()


def rejection(result, case, source):
    require(result.returncode > 0, "faulty program compiled or compiler was killed")
    try:
        record = json.loads(result.stderr)
    except (ValueError, TypeError) as error:
        raise Failure("expected one structured source diagnostic") from error
    require(isinstance(record, dict), "diagnostic is not an object")
    require(record.get("category") == "Source", "failure is not a source rejection")
    require(record.get("rule") == case["rule"], "wrong diagnostic rule")
    require(record.get("kind") == case["kind"], "wrong diagnostic kind")
    at = record.get("at", {})
    require(isinstance(at, dict) and at.get("file") == str(source),
            "diagnostic does not name the rejected witness")
    marked_source = record.get("source", "")
    require(isinstance(marked_source, str) and case["source"] in marked_source,
            "diagnostic does not mark the intended faulty operation")
    return record


def acceptance(result, output):
    require(result.returncode == 0, f"corrected program failed to compile (exit={result.returncode})")
    require(output.is_file() and os.access(output, os.X_OK),
            "compiler reported success without producing an executable")


def self_test():
    # These injected results exercise the same verdict functions as real runs.
    # They are harness tests, never evidence that Whitefoot rejects a program.
    with tempfile.TemporaryDirectory(prefix="safety-witness-harness-") as scratch:
        root = Path(scratch)
        source = root / "rejected.wf"
        source.write_text("set buffer[index] = value;\n")
        case = {"rule": "OP-4", "kind": "UndischargedBoundsObligation",
                "source": "set buffer[index] = value;"}
        record = {"category": "Source", "rule": case["rule"],
                  "kind": case["kind"], "at": {"file": str(source)},
                  "source": case["source"]}

        def result(code=1, diagnostic=None):
            return subprocess.CompletedProcess([], code, "",
                                               json.dumps(record if diagnostic is None else diagnostic))

        def refuses(label, operation):
            try:
                operation()
            except Failure:
                return
            raise Failure(f"harness self-test accepted {label}")

        rejection(result(), case, source)
        refuses("faulty program accepted", lambda: rejection(result(0), case, source))
        refuses("compiler signal", lambda: rejection(result(-9), case, source))
        for field, wrong in (("rule", "GRAM-4"), ("kind", "UnrelatedError"),
                             ("category", "Internal"), ("source", "return unit;"), ("source", None),
                             ("at", {"file": "unrelated.wf"})):
            changed = dict(record, **{field: wrong})
            refuses(f"wrong {field}", lambda: rejection(result(diagnostic=changed), case, source))
        refuses("unstructured tool failure", lambda: rejection(
            subprocess.CompletedProcess([], 1, "", "compiler unavailable"), case, source))
        refuses("missing source", lambda: source_bytes(root / "missing.wf"))
        output = root / "accepted"
        refuses("missing executable", lambda: acceptance(result(0), output))
        output.write_text("#!/bin/sh\nexit 0\n")
        output.chmod(0o700)
        acceptance(result(0), output)
        refuses("corrected program rejected", lambda: acceptance(result(1), output))
        # Exercise traversal and reporting with injected outcomes, without a
        # compiler: failures must not suppress either variant or later cases.
        names = ("accepted-fails", "wrong-rejection", "timed-out", "spawn-error",
                 "missing-source", "short-source", "missing-directory", "later-pass")
        cases = [dict(case, case=name, **{"class": "bounds"}) for name in names]
        for name in names:
            if name == "missing-directory":
                continue
            directory = root / name
            directory.mkdir()
            for variant in ("rejected", "accepted"):
                (directory / f"{variant}.wf").write_text(case["source"] + "\n" * 20)
        (root / "missing-source" / "rejected.wf").unlink()
        (root / "short-source" / "rejected.wf").write_text(case["source"] + "\n")
        calls = []

        def injected(command, **options):
            source = Path(command[3])
            calls.append((source.parent.name, source.stem))
            if source.parent.name == "timed-out" and source.stem == "rejected":
                raise subprocess.TimeoutExpired(command, 120, output=b"timeout stdout", stderr=b"timeout stderr")
            if source.parent.name == "spawn-error" and source.stem == "rejected":
                raise OSError("injected process startup failure")
            if source.stem == "rejected":
                diagnostic = dict(record, at={"file": str(source)})
                if source.parent.name == "wrong-rejection":
                    diagnostic["rule"] = "GRAM-4"
                return subprocess.CompletedProcess(command, 1, "rejected stdout", json.dumps(diagnostic))
            if source.parent.name == "accepted-fails":
                return subprocess.CompletedProcess(command, 1, "accepted failure stdout", "accepted compiler diagnostic")
            executable = Path(command[-1])
            executable.write_text("#!/bin/sh\nexit 0\n")
            executable.chmod(0o700)
            return subprocess.CompletedProcess(command, 0, "", "")

        captured = io.StringIO()
        evidence = {"cases": []}
        destination = root / "results"
        destination.mkdir()
        with contextlib.redirect_stderr(captured), contextlib.redirect_stdout(io.StringIO()):
            evidence["cases"] = check_cases(cases, root, root / "fake-compiler", destination, injected)
        for name in names:
            if name != "missing-directory":
                require((name, "accepted") in calls, f"harness skipped correction after failure: {name}")
        missing = next(row for row in evidence["cases"] if row["case"] == "missing-directory")
        require(missing["rejected"]["status"] == missing["accepted"]["status"] == "fail",
                "missing directory did not record both failed attempts")
        require(calls[-2:] == [("later-pass", "rejected"), ("later-pass", "accepted")],
                "harness skipped later witnesses")
        for expected in ("accepted failure stdout", "accepted compiler diagnostic", "rejected stdout",
                         "timeout stdout", "timeout stderr", "injected process startup failure"):
            require(expected in captured.getvalue(), f"harness omitted failure output: {expected}")
        for suffix, expected in (("stdout", "timeout stdout"), ("stderr", "timeout stderr")):
            require((destination / "timed-out" / f"rejected.{suffix}").read_text() == expected,
                    f"harness did not retain timeout {suffix}")
        require(evidence["cases"][-1]["status"] == "pass", "later valid pair did not pass")
        require(sum(row["status"] == "fail" for row in evidence["cases"]) == 7,
                "harness lost aggregated failures")
        summary = root / "summary.md"
        write_report(evidence, destination, summary)
        saved = json.loads((destination / "results.json").read_text())
        require(saved["failed"] == 7 and all(row["failures"] for row in saved["cases"][:-1]),
                "JSON evidence omitted failure reasons")
        for expected in ("accepted-fails", "wrong diagnostic rule", "timed-out", "missing witness", "20–80 lines"):
            require(expected in summary.read_text(), f"CI summary omitted failure: {expected}")
    print("harness self-test: precise verdicts, failure output, timeout output, continued attempts and aggregated evidence checked", flush=True)


def inventory(root):
    with (root / "cases.tsv").open(newline="") as stream:
        cases = list(csv.DictReader(stream, delimiter="\t"))
    require(cases and set(cases[0]) == {"case", "class", "rule", "kind", "source"},
            "cases.tsv has the wrong columns")
    names = [case["case"] for case in cases]
    require(len(names) == len(set(names)), "duplicate witness in cases.tsv")
    directories = {path.name for path in root.iterdir() if path.is_dir()}
    require(directories <= set(names), "witness directory is not listed in cases.tsv")
    # Missing directories and files fail their individual variants below; they
    # must not prevent the remaining manifest entries from being checked.
    counts = collections.Counter(case["class"] for case in cases)
    require(len(cases) >= 10 and set(counts) == {"use-after-free", "bounds", "data-race", "deadlock"}
            and min(counts.values()) >= 2, "need at least ten cases and two in each class")
    for case in cases:
        require(all(case.values()), f"incomplete expectation: {case}")
    return cases


def compile_one(compiler, source, output, invoke):
    # Delete a previous executable so a stale artifact cannot satisfy acceptance.
    output.unlink(missing_ok=True)
    command = [str(compiler), "--diagnostic-format", "json", str(source), "-o", str(output)]
    return invoke(command, text=True, errors="replace", capture_output=True, timeout=120)


def captured_text(value):
    # TimeoutExpired can carry bytes even when text=True was requested.
    return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value or ""


def check_cases(cases, root, compiler, out, invoke=subprocess.run):
    rows = []
    for case in cases:
        name = case["case"]
        destination = out / name
        row = dict(case, status="incomplete", failures=[])
        rows.append(row)
        for variant in ("rejected", "accepted"):
            source = root / name / f"{variant}.wf"
            output = destination / variant
            start = time.monotonic()
            stdout = stderr = ""
            attempt = {"status": "pass"}
            row[variant] = attempt
            try:
                destination.mkdir(exist_ok=True)
                content = source_bytes(source)
                row[f"{variant}_sha256"] = hashlib.sha256(content).hexdigest()
                require(20 <= len(content.splitlines()) <= 80, f"expected 20–80 lines: {source}")
                if variant == "rejected":
                    require(case["source"] in content.decode("utf-8"),
                            f"missing expected faulty operation: {name}")
                result = compile_one(compiler, source, output, invoke)
                row[f"{variant}_exit"] = result.returncode
                stdout, stderr = result.stdout, result.stderr
                if variant == "rejected":
                    row["diagnostic"] = rejection(result, case, source)
                else:
                    acceptance(result, output)
            except (Failure, OSError, UnicodeError, subprocess.TimeoutExpired) as error:
                attempt["status"] = "fail"
                attempt["reason"] = str(error)
                if isinstance(error, subprocess.TimeoutExpired):
                    attempt["reason"] = f"compiler timed out after {error.timeout}s"
                    stdout = captured_text(error.stdout)
                    stderr = captured_text(error.stderr)
            attempt["elapsed_seconds"] = round(time.monotonic() - start, 3)
            try:
                output.with_suffix(".stdout").write_text(stdout)
                output.with_suffix(".stderr").write_text(stderr)
            except OSError as error:
                attempt["status"] = "fail"
                earlier = attempt.get("reason")
                attempt["reason"] = f"{earlier}; " if earlier else ""
                attempt["reason"] += f"cannot save compiler output: {error}"
            print(f"{name}/{variant}.wf: exit={row.get(f'{variant}_exit', 'unavailable')}, "
                  f"{attempt['elapsed_seconds']:.3f}s", flush=True)
            if attempt["status"] == "fail":
                row["failures"].append(f"{variant}: {attempt['reason']}")
                print(f"FAIL {name}/{variant}.wf: {attempt['reason']}", file=sys.stderr, flush=True)
                for label, captured in (("stdout", stdout), ("stderr", stderr)):
                    print(f"--- {name}/{variant}.wf {label} ---\n{captured or '(empty)'}",
                          file=sys.stderr, flush=True)
        row["status"] = "fail" if row["failures"] else "pass"
        if row["status"] == "pass":
            print(f"PASS {name}: {case['rule']} {case['kind']}; corrected executable built", flush=True)
    return rows


def write_report(evidence, out, summary=None):
    rows = evidence["cases"]
    evidence["passed"] = sum(row["status"] == "pass" for row in rows)
    evidence["failed"] = len(rows) - evidence["passed"]
    (out / "results.json").write_text(json.dumps(evidence, indent=2) + "\n")
    if summary:
        lines = ["## Safety witnesses", "", f"{evidence['passed']} pairs passed; {evidence['failed']} pairs failed.", "",
                 "| Witness | Rejected | Accepted | Failure reasons |", "| --- | --- | --- | --- |"]
        for row in rows:
            reason = "; ".join(row["failures"]).replace("|", "\\|").replace("\n", " ")
            lines.append(f"| {row['case']} | {row['rejected']['status']} | {row['accepted']['status']} | {reason} |")
        with Path(summary).open("a") as stream:
            stream.write("\n".join(lines) + "\n")


def run(root, compiler, out):
    cases = inventory(root)
    require(compiler.is_file() and os.access(compiler, os.X_OK), "compiler is missing or not executable")
    out.mkdir(parents=True, exist_ok=True)
    (out / "results.json").unlink(missing_ok=True)
    repo = root.parents[2]
    evidence = {"pin": (repo / "whitefoot.pin").read_text().strip(),
                "compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(),
                "cases": []}
    evidence["cases"] = check_cases(cases, root, compiler, out)
    write_report(evidence, out, os.environ.get("GITHUB_STEP_SUMMARY"))
    print(f"safety witnesses: {evidence['passed']} pairs passed, {evidence['failed']} failed; evidence in {out}", flush=True)
    require(evidence["failed"] == 0, f"{evidence['failed']} witness pairs failed; see all reasons in {out / 'results.json'}")


root = Path(sys.argv.pop(1)).resolve().parent
parser = argparse.ArgumentParser(description="Compile safety witnesses in CI using the pinned Whitefoot compiler")
parser.add_argument("--self-test", action="store_true", help="only test the diagnostic harness; no compiler invoked")
parser.add_argument("--compiler", type=Path)
parser.add_argument("--out", type=Path)
args = parser.parse_args()
try:
    self_test()
    if not args.self_test:
        require(args.compiler is not None and args.out is not None, "--compiler and --out are required")
        run(root, args.compiler.resolve(), args.out.resolve())
except (Failure, OSError, subprocess.TimeoutExpired) as error:
    print(f"safety-witnesses: FAIL: {error}", file=sys.stderr)
    sys.exit(1)
PY
