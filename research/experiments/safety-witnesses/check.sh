#!/bin/sh
# Python's JSON parser checks structured compiler diagnostics; grep cannot
# distinguish a source rejection from a quoted rule in an unrelated failure.
set -eu
exec "${PY:-python3}" - "$0" "$@" <<'PY'
import argparse
import collections
import csv
import hashlib
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
    require(case["source"] in record.get("source", ""),
            "diagnostic does not mark the intended faulty operation")
    return record


def acceptance(result, output):
    require(result.returncode == 0, "corrected program failed to compile")
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
                             ("category", "Internal"), ("source", "return unit;"),
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
    print("harness self-test: wrong acceptance, wrong diagnostic, tool failure and missing input/output detected", flush=True)


def inventory(root):
    with (root / "cases.tsv").open(newline="") as stream:
        cases = list(csv.DictReader(stream, delimiter="\t"))
    require(cases and set(cases[0]) == {"case", "class", "rule", "kind", "source"},
            "cases.tsv has the wrong columns")
    names = [case["case"] for case in cases]
    require(len(names) == len(set(names)), "duplicate witness in cases.tsv")
    require(set(names) == {path.name for path in root.iterdir() if path.is_dir()},
            "witness directories and cases.tsv differ")
    counts = collections.Counter(case["class"] for case in cases)
    require(len(cases) >= 10 and set(counts) == {"use-after-free", "bounds", "data-race", "deadlock"}
            and min(counts.values()) >= 2, "need at least ten cases and two in each class")
    for case in cases:
        require(all(case.values()), f"incomplete expectation: {case}")
        for variant in ("rejected", "accepted"):
            source = root / case["case"] / f"{variant}.wf"
            content = source_bytes(source)
            require(20 <= len(content.splitlines()) <= 80, f"expected 20–80 lines: {source}")
        require(case["source"] in (root / case["case"] / "rejected.wf").read_text(),
                f"missing expected faulty operation: {case['case']}")
    return cases


def compile_one(compiler, source, output, logs):
    # Delete a previous executable so a stale artifact cannot satisfy acceptance.
    output.unlink(missing_ok=True)
    command = [str(compiler), "--diagnostic-format", "json", str(source), "-o", str(output)]
    start = time.monotonic()
    try:
        result = subprocess.run(command, text=True, capture_output=True, timeout=120)
    except subprocess.TimeoutExpired as error:
        # TimeoutExpired can carry bytes even when text=True was requested.
        for suffix, captured in ((".stdout", error.stdout), (".stderr", error.stderr)):
            text = captured.decode("utf-8", errors="replace") if isinstance(captured, bytes) else captured
            logs.with_suffix(suffix).write_text(text or "")
        raise
    elapsed = time.monotonic() - start
    logs.with_suffix(".stdout").write_text(result.stdout)
    logs.with_suffix(".stderr").write_text(result.stderr)
    print(f"{source.parent.name}/{source.name}: exit={result.returncode}, {elapsed:.2f}s", flush=True)
    return result


def run(root, compiler, out):
    cases = inventory(root)
    require(compiler.is_file() and os.access(compiler, os.X_OK), "compiler is missing or not executable")
    out.mkdir(parents=True, exist_ok=True)
    repo = root.parents[2]
    evidence = {"pin": (repo / "whitefoot.pin").read_text().strip(),
                "compiler_sha256": hashlib.sha256(compiler.read_bytes()).hexdigest(),
                "cases": []}
    try:
        for case in cases:
            name = case["case"]
            destination = out / name
            destination.mkdir(exist_ok=True)
            rejected = root / name / "rejected.wf"
            accepted = root / name / "accepted.wf"
            row = dict(case, status="incomplete", rejected_sha256=hashlib.sha256(source_bytes(rejected)).hexdigest(),
                       accepted_sha256=hashlib.sha256(source_bytes(accepted)).hexdigest())
            evidence["cases"].append(row)
            bad = compile_one(compiler, rejected, destination / "rejected", destination / "rejected")
            row["rejected_exit"] = bad.returncode
            diagnostic = rejection(bad, case, rejected)
            row["diagnostic"] = diagnostic
            good = compile_one(compiler, accepted, destination / "accepted", destination / "accepted")
            row["accepted_exit"] = good.returncode
            acceptance(good, destination / "accepted")
            row["status"] = "pass"
            print(f"PASS {name}: {case['rule']} {case['kind']}; corrected executable built", flush=True)
    finally:
        (out / "results.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"safety witnesses: {len(cases)} pairs passed; evidence in {out}", flush=True)


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
