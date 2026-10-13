import pathlib, shutil, sys
root = pathlib.Path(sys.argv[1]); out = pathlib.Path(sys.argv[2])
call = "        pkg::protocol::push_arg(args: &args, start: event.start, length: event.length);\n"
variants = {
  "v0_unchanged": lambda s: s,
  "v1_no_call": lambda s: s.replace(call, ""),
  "v2_const_args": lambda s: s.replace(call, call.replace("event.start", "0_u64").replace("event.length", "0_u64")),
  "v3_no_retain_if": lambda s: s.replace("      if retain {\n" + call + "      }\n", call),
  "v4_call_reasserts": lambda s: s.replace(call, call + "        set reader^.cursor = imin(reader^.cursor, reader^.held);\n"),
  "v5_args_unused_box": lambda s: s.replace(call, "        let other = box_slots_new::<Span>(capacity: 0_u64);\n        pkg::protocol::push_arg(args: &other, start: event.start, length: event.length);\n"),
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
