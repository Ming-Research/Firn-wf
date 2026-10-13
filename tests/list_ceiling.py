#!/usr/bin/env python3
"""Assemble the real list command bodies with an eight-element test ceiling.

Whitefoot's graph loads all direct sources in a module directory and has no
source-overlay option. Build-directory copies let a test entry call private
command bodies without exporting a production test API. Within production
implementation records, only lists.wf's ceiling alias changes; the generated
graph and command interface register the test entry. The Makefile compiles
and runs the assembled entry in CI.
"""

import argparse
import os
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    out = args.out.resolve()
    # Never permit cleanup outside this target's disposable build directory.
    if out != root / "build" / "list-ceiling":
        parser.error("--out must name this checkout's build/list-ceiling")
    if out.exists():
        shutil.rmtree(out)
    program = out / "firn"
    shutil.copytree(root / "firn", program)
    lists = program / "commands" / "lists.wf"
    source = lists.read_text()
    old = "alias ceiling = pkg::store::ceiling;"
    if source.splitlines().count(old) != 1:
        raise ValueError("expected exactly one list ceiling alias line")
    if "pkg::store::ceiling" in source.replace(old, "", 1):
        raise ValueError("store ceiling referenced outside the list ceiling alias")
    lists.write_text(source.replace(old, "alias ceiling = pkg::list_limit::ceiling;"))
    limit = program / "list_limit"
    limit.mkdir()
    (limit / "module.wfm").write_text("public const ceiling: u64 = 8_u64;\n")
    shutil.copyfile(root / "tests" / "list_ceiling.wf", program / "commands" / "list_ceiling.wf")
    interface = program / "commands" / "module.wfm"
    with interface.open("a") as stream:
        stream.write('\npublic fn list_ceiling_main(inputs: std::process::Inputs) -> status: std::process::ExitStatus pure waits doc "Checks list ceiling admission using direct eight-element fixtures.";\n')
    graph = program / "modules.wfg"
    source = graph.read_text()
    for name in ("halo", "fpconv"):
        location = root / "deps" / "halo-wf" / "lib" / name
        source = source.replace(f'"../deps/halo-wf/lib/{name}"', f'"{os.path.relpath(location, program)}"')
    source = source.replace("pkg::commands: [", "pkg::list_limit: [];\npkg::commands: [pkg::list_limit, std::fs, ")
    source += "\nentry list_ceiling = pkg::commands::list_ceiling_main;\n"
    graph.write_text(source)


if __name__ == "__main__":
    main()
