import pathlib, shutil, sys
root = pathlib.Path(sys.argv[1]); out = pathlib.Path(sys.argv[2])
call = "        pkg::protocol::push_arg(args: &args, start: event.start, length: event.length);\n"
probe = """
fn scanner_probe(syntax: &AofSyntax, input: &[u8], start: u64, end: u64) -> next: u64 reads(input), writes(syntax) contract {
  requires syntax^.line.len < 1023_u64;
  requires start <= end;
  requires end <= input^.len;
  ensures next <= end;
} {
  let event = aof_syntax_next(syntax: syntax, input: input, start: start, end: end, eof: False());
  return event.next;
}
"""
def scan(old, new):
    return lambda s, f="scanner": (s.replace(old, new) if f == "scanner" else s)
variants = {
  "v0_unchanged": {},
  "x1_probe_caller": {"replay": lambda s: s + probe},
  "x2_no_linelen_ensures": {"scanner": lambda s: s.replace("  ensures syntax^.line.len < 1023_u64;\n  ensures event.next >= start;", "  ensures event.next >= start;")},
  "x3_no_lower_ensures": {"scanner": lambda s: s.replace("  ensures event.next >= start;\n", "")},
  "x4_no_upper_ensures": {"scanner": lambda s: s.replace("  ensures event.next <= end;\n} {\n  doc \"Consume one memory chunk", "} {\n  doc \"Consume one memory chunk")},
}
files = {"replay": "firn/persistence/replay.wf", "scanner": "firn/persistence/scanner.wf"}
for name, edits in variants.items():
    d = out / name
    shutil.copytree(root / "firn", d / "firn")
    for entry in root.resolve().iterdir():
        if entry.name not in ("firn", ".git", "build"):
            (d / entry.name).symlink_to(entry)
    ok = True
    for key, f in edits.items():
        p = d / files[key]
        s = p.read_text(); t = f(s)
        if t == s:
            ok = False
        p.write_text(t)
    print(name, "ok" if ok else "PATCH_DID_NOT_APPLY")
