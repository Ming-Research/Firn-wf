import pathlib, shutil, sys
root = pathlib.Path(sys.argv[1]); out = pathlib.Path(sys.argv[2])
call = "        pkg::protocol::push_arg(args: &args, start: event.start, length: event.length);\n"
variants = {
  "v0_unchanged": lambda s: s,
  "w1_no_held_writeback": lambda s: s.replace("    set reader^.held = held;\n    set reader^.position", "    set reader^.position"),
  "w2_no_position_write": lambda s: s.replace("    set reader^.position = position;\n    if event.state", "    if event.state"),
  "w3_cursor_clamped": lambda s: s.replace("    set reader^.cursor = next;\n", "    set reader^.cursor = imin(next, held);\n"),
  "w4_position_first": lambda s: s.replace("    set reader^.cursor = next;\n    set reader^.held = held;\n    set reader^.position = position;\n", "    set reader^.position = position;\n    set reader^.cursor = next;\n    set reader^.held = held;\n"),
  "w5_reassert_after_writes": lambda s: s.replace("    set reader^.position = position;\n    if event.state", "    set reader^.position = position;\n    set reader^.cursor = imin(reader^.cursor, reader^.held);\n    if event.state"),
  "w6_no_position_local": lambda s: s.replace("    let position = reader^.syntax.position;\n", "").replace("    set reader^.position = position;\n", ""),
}
for name, f in variants.items():
    d = out / name
    shutil.copytree(root / "firn", d / "firn")
    for entry in root.resolve().iterdir():
        if entry.name not in ("firn", ".git", "build"):
            (d / entry.name).symlink_to(entry)
    p = d / "firn/persistence/replay.wf"
    s = p.read_text(); t = f(s)
    if name != "v0_unchanged" and t == s:
        print(name, "PATCH_DID_NOT_APPLY"); continue
    p.write_text(t); print(name, "ok")
