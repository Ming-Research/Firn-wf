#!/usr/bin/env python3
"""Record a fixed reduction ladder, without treating rejection as acceptance.

The temporary select-db-fnparam-ladder workflow downloads and verifies releases
through Whitefoot-kit before calling this instrument. Exit zero means that the
comparison was recorded; it says nothing about firn's gate or witness acceptance.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys


SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]*\Z")
CONTROLS = {"ordinal-before", "ordinal-after", "direct-control", "accepted-baseline",
            "specialized-full-raw", "caller-view-control"}
HEADER = r'(?m)^(?:[^\r\n"]+:\d+:\d+:[ \t]*)?[ \t]*error'
FN9 = re.compile(HEADER + r"\[FN-9\]:[ \t]*UndischargedPostcondition[ \t]*$")
ERROR = re.compile(HEADER + r"\[[^\]\n]+\]:")
CALLBACK_RETURN = re.compile(r"(?m)^[ \t]*source:[ \t]*return[ \t]+apply[ \t]*\(")
ADAPTER = re.compile(
    r"(?m)^[ \t]*concrete_function:[ \t]*(?:[A-Za-z_][A-Za-z0-9_]*::)*"
    r"(?:forward|command_held_entries_run_due)(?=::<|<|[ \t]|$)"
)
LENGTH_CONTRACT = re.compile(
    r"(?m)^[ \t]*postcondition:[^\n]*\b([A-Za-z_][A-Za-z0-9_]*)\s*\^\s*\.\s*len\s*==\s*"
    r"entry\s*\(\s*\1\s*\)\s*\^\s*\.\s*len\b"
)
INFRASTRUCTURE = re.compile(
    r"(?im)^\s*(?:error:\s*)?(?:usage:|unknown (?:option|argument)|"
    r"unrecognized (?:option|argument)|unexpected argument|invalid argument)|"
    r"(?i:no such file or directory|cannot (?:read|open|load) (?:file|graph|module))"
)


def load_cases(path, chain_steps=16):
    """Reject corrupt metadata and paths escaping the instrument's directory."""
    root = path.resolve().parent
    cases = json.loads(path.read_text())
    if not isinstance(cases, list) or not cases:
        raise ValueError("cases.json must be a nonempty list")
    ids = set()
    chain_numbers = []
    controls = set()
    for case in cases:
        if not isinstance(case, dict) or set(case) - {"id", "kind", "path", "delta", "chain"}:
            raise ValueError("each case must contain only id, kind, path, delta and optional chain")
        for key in ("id", "kind", "path", "delta"):
            if not isinstance(case.get(key), str) or not case[key].strip():
                raise ValueError(f"case has missing or invalid {key}: {case!r}")
        if not SAFE_ID.fullmatch(case["id"]) or case["id"] in ids:
            raise ValueError(f"unsafe or duplicate case id: {case['id']}")
        ids.add(case["id"])
        if case["kind"] not in ("bundle", "graph"):
            raise ValueError(f"invalid case kind: {case['kind']}")
        if not isinstance(case.get("chain", True), bool):
            raise ValueError(f"invalid chain flag: {case['id']}")
        relative = Path(case["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"case path must remain under the ladder: {case['path']}")
        source = (root / relative).resolve()
        if not source.is_relative_to(root) or not source.is_file():
            raise ValueError(f"missing or escaping case input: {case['path']}")
        expected_suffix = ".wf" if case["kind"] == "bundle" else ".wfg"
        if source.suffix != expected_suffix:
            raise ValueError(f"wrong input suffix for {case['kind']}: {case['path']}")
        if case["kind"] == "graph":
            # This ladder uses native, directory-local pkg modules. Check their
            # inputs before running either compiler; missing bodies are not a
            # compiler rejection to compare.
            modules = re.findall(r"(?m)^\s*pkg::([A-Za-z0-9_:]+)\s*:", source.read_text())
            if not modules:
                raise ValueError(f"graph declares no local modules: {case['path']}")
            for module in modules:
                directory = source.parent.joinpath(*module.split("::"))
                inputs = [directory / "module.wfm", *directory.glob("*.wf")]
                if len(inputs) < 2 or any(not item.is_file() or not item.resolve().is_relative_to(root)
                                          for item in inputs):
                    raise ValueError(f"missing or escaping graph module input: pkg::{module}")
        if case.get("chain", True):
            match = re.match(r"^(\d{2})(?:-|_|$)", case["id"])
            if not match:
                raise ValueError(f"chain id must start with its two-digit step: {case['id']}")
            chain_numbers.append(int(match[1]))
        else:
            controls.add(case["id"])
    if chain_numbers != list(range(chain_steps)):
        raise ValueError(f"chain steps must occur exactly once in order, from 00 through {chain_steps - 1:02d}")
    if controls != CONTROLS:
        raise ValueError(f"control cases must be exactly: {', '.join(sorted(CONTROLS))}")
    return cases, root


def load_compiler(tag, commit, directory):
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or tag != f"wf-{commit[:12]}":
        raise ValueError(f"invalid formal release identity: {tag} {commit}")
    directory = Path(directory).resolve()
    manifest_path = directory / "whitefoot-release.json"
    manifest = json.loads(manifest_path.read_text())
    if not isinstance(manifest, dict) or manifest.get("tag") != tag or manifest.get("commit") != commit:
        raise ValueError(f"manifest must describe exact tag {tag} and full commit {commit}")
    binary = directory / "whitefootc"
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"missing executable compiler: {binary}")
    return {"tag": tag, "commit": commit, "binary": str(binary), "manifest": manifest}


def command_for(compiler, case, root):
    source = str(root / case["path"])
    if case["kind"] == "bundle":
        return [compiler["binary"], "--check", source]
    return [compiler["binary"], "--graph", source, "--check-modules"]


def matched_adapter_fn9(diagnostic):
    # Match all evidence inside the same error block, never across unrelated
    # failures. An FN-9 label alone is not evidence for this forwarding gap.
    starts = list(ERROR.finditer(diagnostic))
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(diagnostic)
        block = diagnostic[start.start():end]
        if FN9.search(block.splitlines()[0]) and all(pattern.search(block) for pattern in (ADAPTER, CALLBACK_RETURN, LENGTH_CONTRACT)):
            return True
    return False


def first_line(diagnostic):
    lines = [line.strip() for line in diagnostic.splitlines() if line.strip()]
    return next((line for line in lines if ERROR.search(line)), lines[0] if lines else "(no diagnostic)")


def run_case(compiler, case, root, output, timeout):
    argv = command_for(compiler, case, root)
    timed_out = False
    try:
        result = subprocess.run(argv, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=timeout, check=False)
        diagnostic = result.stdout.decode("utf-8", errors="replace")
        code = result.returncode
    except subprocess.TimeoutExpired as error:
        diagnostic = (error.stdout or b"").decode("utf-8", errors="replace")
        diagnostic += f"\nTIMED OUT after {timeout:g} seconds; exit code 124 is the runner's timeout marker.\n"
        code, timed_out = 124, True
    diagnostic_path = output / "diagnostics" / compiler["tag"] / f"{case['id']}.txt"
    diagnostic_path.parent.mkdir(parents=True, exist_ok=True)
    diagnostic_path.write_text(diagnostic)
    matched = code != 0 and not timed_out and matched_adapter_fn9(diagnostic)
    return {
        "step": case["id"], "compiler": compiler["tag"], "commit": compiler["commit"],
        "kind": case["kind"], "path": case["path"], "delta": case["delta"],
        "chain": case.get("chain", True), "argv": argv, "exit_code": code,
        "timed_out": timed_out, "first_diagnostic_line": first_line(diagnostic),
        "adapter_fn9": matched,
        "infrastructure_error": not timed_out and bool(INFRASTRUCTURE.search(diagnostic)),
        "diagnostic": str(diagnostic_path.relative_to(output)),
    }


def summarize(rows, compilers):
    summaries = []
    for compiler in compilers:
        selected = [row for row in rows if row["compiler"] == compiler["tag"]]
        chain = [row for row in selected if row["chain"]]
        passing = [row["step"] for row in chain if row["exit_code"] == 0]
        failing = [row["step"] for row in chain if row["adapter_fn9"]]
        summaries.append({
            "compiler": compiler["tag"],
            "first_passing_chain_step": passing[0] if passing else None,
            "last_failing_adapter_fn9_chain_step": failing[-1] if failing else None,
            "failing_adapter_fn9_controls": [row["step"] for row in selected
                                             if not row["chain"] and row["adapter_fn9"]],
            "failures_outside_adapter_fn9": [row["step"] for row in selected
                                             if row["exit_code"] != 0 and not row["adapter_fn9"]],
        })
    return summaries


def markdown(rows, summaries):
    def escape(value):
        return str(value).replace("|", "\\|").replace("\n", " ").replace("\r", " ")
    lines = ["| Step | Compiler | Exit code | First diagnostic line |",
             "| --- | --- | ---: | --- |"]
    lines.extend(f"| {escape(row['step'])} | {row['compiler']} | {row['exit_code']} | "
                 f"{escape(row['first_diagnostic_line'])} |" for row in rows)
    lines.extend(["", "Zero means accepted by --check; 124 is a runner timeout marker. "
                  "Other exit codes are the compiler's actual return codes.", ""])
    for summary in summaries:
        lines.append(f"- {summary['compiler']}: first passing chain step: "
                     f"{summary['first_passing_chain_step'] or 'none'}; last failing chain step "
                     f"with the adapter FN-9: {summary['last_failing_adapter_fn9_chain_step'] or 'none'}; "
                     f"controls failing with that diagnostic: {', '.join(summary['failing_adapter_fn9_controls']) or 'none'}; "
                     f"failures outside that diagnostic: {', '.join(summary['failures_outside_adapter_fn9']) or 'none'}.")
    lines.extend(["", "The last matching failure is the most reduced observed failure in this "
                  "ordered ladder, not proof of a globally minimal witness. Passing and failing "
                  "steps need not be monotonic. Compiler rejection is experiment data; recording "
                  "success is not firn gate success.", ""])
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--compiler", nargs=3, action="append", required=True,
                        metavar=("TAG", "FULL_COMMIT", "DIRECTORY"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--chain-steps", type=int, default=16,
                        help="exact expected count, so a missing final reduction cannot go unnoticed")
    args = parser.parse_args(argv)
    try:
        if not 0 < args.timeout <= 120:
            raise ValueError("timeout must be greater than zero and no more than 120 seconds")
        if not 1 <= args.chain_steps <= 100:
            raise ValueError("chain-steps must be between 1 and 100")
        cases, root = load_cases(args.cases, args.chain_steps)
        compilers = [load_compiler(*specification) for specification in args.compiler]
        if len(compilers) != 2 or len({compiler['tag'] for compiler in compilers}) != 2:
            raise ValueError("comparison requires exactly two distinct compiler releases")
        args.output.mkdir(parents=True, exist_ok=True)
        (args.output / "provenance.json").write_text(json.dumps(compilers, indent=2) + "\n")
        print("Compiler provenance (kit checksum-verified release manifests):", flush=True)
        print(json.dumps(compilers, indent=2), flush=True)
        rows = []
        for case in cases:
            for compiler in compilers:
                rows.append(run_case(compiler, case, root, args.output, args.timeout))
        summaries = summarize(rows, compilers)
        (args.output / "rows.json").write_text(json.dumps(rows, indent=2) + "\n")
        (args.output / "summary.json").write_text(json.dumps(summaries, indent=2) + "\n")
        table = markdown(rows, summaries)
        (args.output / "table.md").write_text(table)
        print(table, flush=True)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
                stream.write(table)
        if any(row["infrastructure_error"] for row in rows):
            raise ValueError("compiler reported invalid arguments or missing inputs; see recorded diagnostics")
    except (OSError, ValueError, TypeError) as error:
        print(f"Experiment infrastructure error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
