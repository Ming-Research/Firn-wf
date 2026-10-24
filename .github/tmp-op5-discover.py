#!/usr/bin/env python3
"""Temporary (firn-pin-wf-main): list every OP-5 site of a graph.

The compiler names one site per function body and stops there, so a body's
next redundant condition shows only after the first is repaired. This script
applies the compiler's own repair (delete the decided test and its dead
branch) to a disposable CI workspace, one reported site per module per round,
until nothing is reported, and prints each repair with its context. The
repairs are read by a person before they enter the repository.

usage: tmp-op5-discover.py ROUNDS WFC GRAPH_DIR MODE [MODULE...]
  MODE modules: run `--check-modules`; MODE module: run `--check-module
  pkg::MODULE` for each MODULE.
"""
import re
import subprocess
import sys
from pathlib import Path

HEAD = re.compile(
    r"^(?:[\w:]+: rejected: )?(\S+\.wf):(\d+):(\d+): error\[OP-5\]: RedundantCondition$"
)
OTHER = re.compile(r"error\[(?!OP-5\])[A-Z0-9-]+\]")


def sites(output):
    lines = output.splitlines()
    found = []
    for n, line in enumerate(lines):
        m = HEAD.match(line)
        if not m:
            continue
        decided = None
        for k in range(n + 1, min(n + 8, len(lines))):
            d = re.match(r"^  decided: (True|False)$", lines[k])
            if d:
                decided = d.group(1) == "True"
                break
        facts = next((l.strip() for l in lines[n + 1 : n + 8] if l.startswith("  facts:")), "")
        found.append((m.group(1), int(m.group(2)), int(m.group(3)), decided, facts))
    return found


def skip_string(text, i):
    """i is at an opening quote; return the index after the closing quote."""
    i += 1
    while text[i] != '"':
        i += 2 if text[i] == "\\" else 1
    return i + 1


def match_brace(text, i):
    """i is at `{`; return the index of its `}`."""
    depth = 0
    while True:
        c = text[i]
        if c == '"':
            i = skip_string(text, i)
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1


def skip_space(text, i):
    while text[i] in " \t\n":
        i += 1
    return i


def keyword_at(text, i, word):
    return text.startswith(word, i) and not (text[i + len(word)].isalnum() or text[i + len(word)] == "_")


def unwrap(text, open_i, close_i, indent):
    """The statements of a block, moved out one level (two columns)."""
    body = text[open_i + 1 : close_i]
    rows = body.split("\n")
    while rows and not rows[0].strip():
        rows.pop(0)
    while rows and not rows[-1].strip():
        rows.pop()
    out = []
    for row in rows:
        out.append(row[2:] if row.startswith("  ") else row.lstrip())
    return "\n".join(out)


def repair(text, line, col, decided):
    """Return (new_text, old_fragment, new_fragment), or None if the form is not a plain statement."""
    starts = [0]
    for n, c in enumerate(text):
        if c == "\n":
            starts.append(n + 1)
    cond = starts[line - 1] + col - 1
    j = cond - 1
    while text[j] in " \t\n":
        j -= 1
    if text[j - 1 : j + 1] != "if":
        return None
    if_start = j - 1
    k = if_start - 1
    while text[k] in " \t\n":
        k -= 1
    else_kw = text[k - 3 : k + 1] == "else"
    if not else_kw and text[k] not in "{};":
        return None
    then_open = text.index("{", cond)
    then_close = match_brace(text, then_open)
    # the tail after the then block
    n = skip_space(text, then_close + 1)
    tail = None  # ("block", open, close) or ("if", if_index)
    end = then_close + 1
    if keyword_at(text, n, "else"):
        m = skip_space(text, n + 4)
        if text[m] == "{":
            c = match_brace(text, m)
            tail = ("block", m, c)
            end = c + 1
        elif keyword_at(text, m, "if"):
            # the chain's end: walk the links
            p = m
            while True:
                o = text.index("{", p)
                c = match_brace(text, o)
                q = skip_space(text, c + 1)
                if keyword_at(text, q, "else"):
                    r = skip_space(text, q + 4)
                    if text[r] == "{":
                        c2 = match_brace(text, r)
                        end = c2 + 1
                        break
                    p = r
                    continue
                end = c + 1
                break
            tail = ("if", m)
    old = text[if_start:end]
    indent = starts[line - 1]
    if decided:
        if else_kw:
            # `else if c {A} ...` becomes `else {A}`
            new = text[then_open:then_close + 1]
            return text[:if_start] + new + text[end:], old, new
        new = unwrap(text, then_open, then_close, 0)
        new = new.lstrip(" ") if new else new
        return text[:if_start] + new + text[end:], old, new
    if else_kw:
        if tail is None:
            # drop `else if c {A}` together with the space before `else`
            q = k - 3
            while text[q - 1] in " \t\n":
                q -= 1
            return text[:q] + text[end:], text[q:end], ""
        if tail[0] == "block":
            return text[:if_start] + text[tail[1]:end] + text[end:], old, text[tail[1]:end]
        return text[:if_start] + text[tail[1]:end] + text[end:], old, text[tail[1]:end]
    if tail is None:
        # the whole statement goes, with its line
        ls = text.rfind("\n", 0, if_start) + 1
        le = text.find("\n", end) + 1
        return text[:ls] + text[le:], old, ""
    if tail[0] == "block":
        new = unwrap(text, tail[1], tail[2], 0).lstrip(" ")
        return text[:if_start] + new + text[end:], old, new
    return text[:if_start] + text[tail[1]:end] + text[end:], old, text[tail[1]:end]


def main():
    rounds, wfc, graph_dir, mode = int(sys.argv[1]), sys.argv[2], Path(sys.argv[3]), sys.argv[4]
    modules = sys.argv[5:]
    commands = (
        [[wfc, "--graph", "modules.wfg", "--check-modules"]]
        if mode == "modules"
        else [[wfc, "--graph", "modules.wfg", "--check-module", f"pkg::{m}"] for m in modules]
    )
    applied = []
    manual = []
    for round_number in range(1, rounds + 1):
        todo = {}
        other = []
        for command in commands:
            result = subprocess.run(command, cwd=graph_dir, capture_output=True, text=True)
            output = result.stdout + result.stderr
            if result.returncode != 0 and not sites(output):
                other.append(output)
            for path, line, col, decided, facts in sites(output):
                todo.setdefault((graph_dir / path).resolve(), {})[(line, col)] = (decided, facts)
        print(f"== round {round_number}: {sum(len(v) for v in todo.values())} site(s), "
              f"{len(other)} command(s) rejected without OP-5", flush=True)
        for text in other:
            kinds = sorted(set(re.findall(r"error\[[A-Z0-9-]+\]", text)))
            print(f"   rejected for other reasons: {kinds}")
            print(text[:1500])
        progress = False
        for path, found in todo.items():
            # bottom first, so earlier line numbers stay valid
            for (line, col), (decided, facts) in sorted(found.items(), reverse=True):
                text = path.read_text()
                done = None if decided is None else repair(text, line, col, decided)
                where = f"{path.relative_to(Path.cwd())}:{line}:{col}"
                if done is None:
                    manual.append(where)
                    print(f"MANUAL {where} decided={decided} (not a plain if statement)")
                    continue
                new_text, old, new = done
                path.write_text(new_text)
                progress = True
                applied.append(where)
                print(f"REPAIRED {where} decided={decided}\n  {facts[:300]}\n--- old\n{old}\n--- new\n{new}\n---", flush=True)
        if not todo or not progress:
            break
    print(f"repaired {len(applied)} site(s); manual {len(manual)}: {manual}")
    for command in commands:
        result = subprocess.run(command, cwd=graph_dir, capture_output=True, text=True)
        print(f"final {' '.join(command[-2:])}: exit {result.returncode}")
        if result.returncode != 0:
            print((result.stdout + result.stderr)[:3000])


if __name__ == "__main__":
    main()
