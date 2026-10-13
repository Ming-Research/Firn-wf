import pathlib
import shutil
import sys

root = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])

# These are compiler diagnostics, never runtime correctness fixes. All variants
# start from the same sources. a1/a2 test canary checking; a3/a4 isolate the
# scanner's next <= end summary under identical caller and implicit-exit masks.
# b0/b1 compare against a1, asking for facts before its masks so an earlier
# loop-header failure cannot hide the assertions. c1..c9 each compare to v0.
# New copy bounds (c3) or swap boundary obligations (c5) can fail earlier; such
# diagnostics are inconclusive about the original scanner/window proof loss.
CALL = "    let event = aof_syntax_next(syntax: &reader^.syntax, input: input, start: cursor, end: held, eof: reader^.eof);\n"
INPUT = "    let input = &reader^.bytes.inner[0_u64..held];\n"
FILL_MATCH = """      match aof_fill(store: store, factory: factory, file: file, reader: reader, clock: clock, rewriting: rewriting) {
        Ok(value: filled) => {
        }
        Err(error: reason) => {
          if reason != 0_u8 {
            return Err<AofRecord, u8>(error: reason);
          }
        }
      }
"""
FILL_BLOCK = """    if reader^.eof {
    } else if reader^.cursor == reader^.held {
""" + FILL_MATCH + "    }\n"
HEADER = """    invariant window: reader^.cursor <= reader^.held,
    invariant capacity: reader^.held <= reader^.bytes.inner.len,
    invariant header_room: reader^.syntax.line.len < 1023_u64,
    invariant probe_bound: reader^.syntax.probe <= 5_u64
"""
CANARY = """
fn diagnostic_canary() -> value: u64 pure contract {
  ensures value == 1_u64;
} {
  return 0_u64;
}
"""
# Both probes retain the scanner call and clamp only its implicit TYPE-11
# written-reference exit obligation. Their explicit next <= end proofs differ
# only in the returned value; neither clamp supplies the desired next fact.
PROBE = """
fn scanner_probe(syntax: &AofSyntax, input: &[u8], start: u64, end: u64) -> next: u64 reads(input), writes(syntax) contract {
  requires syntax^.line.len < 1023_u64;
  requires start <= end;
  requires end <= input^.len;
  ensures next <= end;
} {
  let ended = False();
  let event = aof_syntax_next(syntax: syntax, input: input, start: start, end: end, eof: ended);
  set syntax^.probe = imin(syntax^.probe, 5_u64);
  return event.next;
}
"""


def replace_exact(source, old, new, label, count=1):
    found = source.count(old)
    if found != count:
        raise ValueError(f"{label}: expected {count} occurrence(s), found {found}")
    return source.replace(old, new)


def edit_function(source, start_name, end_name, edit):
    start = f"fn {start_name}("
    end = f"fn {end_name}("
    for marker in (start, end):
        if source.count(marker) != 1:
            raise ValueError(f"function boundary {marker!r} must occur exactly once")
    before, rest = source.split(start)
    body, after = rest.split(end)
    return before + edit(start + body) + end + after


def record_edit(edit):
    return lambda source: edit_function(source, "aof_record", "aof_read_payload", edit)


def record_replace(old, new, label):
    return record_edit(lambda body: replace_exact(body, old, new, label))


def append_function(source, addition, name):
    if source.count(f"fn {name}(") != 0:
        raise ValueError(f"diagnostic function {name} already exists")
    if not source.endswith("\n") or source.endswith("\n\n"):
        raise ValueError("source must end with exactly one LF before appending")
    return source + addition


def masked_record(body):
    body = replace_exact(body, "    set reader^.cursor = next;\n",
                         "    set reader^.cursor = imin(next, held);\n", "mask cursor")
    return replace_exact(body, "    set reader^.position = position;\n",
                         "    set reader^.position = position;\n"
                         "    set reader^.syntax.line = slots_new::<u8, 1023>();\n"
                         "    set reader^.syntax.probe = imin(reader^.syntax.probe, 5_u64);\n",
                         "mask reader syntax")


def masked_boundary(source):
    return edit_function(
        source, "aof_boundary_appended", "aof_rw_boundary",
        lambda body: replace_exact(
            body, "    set consumed = event.next;\n",
            "    set consumed = imin(event.next, length);\n"
            "    set syntax.line = slots_new::<u8, 1023>();\n"
            "    set syntax.probe = imin(syntax.probe, 5_u64);\n",
            "mask append boundary"))


def masked_replay(addition=None, name=None):
    def apply(source):
        result = record_edit(masked_record)(source)
        return append_function(result, addition, name) if addition is not None else result
    return apply


# c6 drops the only accesses to store/factory/file/clock. Narrowing its effect
# row is compelled by EFF-2's unused-effect rule, not a second experimental knob.
def no_fill(body):
    body = replace_exact(body, FILL_BLOCK, "", "remove initial fill block")
    return replace_exact(
        body, "reads(store), writes(factory), writes(file), writes(reader), writes(clock) waits {\n",
        "writes(reader) waits {\n", "remove unused fill effects")


mask = {"replay": masked_replay(), "rewrite-boundary": masked_boundary}
variants = {
    "v0_unchanged": {},
    "a0_false_canary": {"replay": lambda source: append_function(source, CANARY, "diagnostic_canary")},
    "a1_masked_control": mask,
    "a2_masked_canary": {"replay": masked_replay(CANARY, "diagnostic_canary"), "rewrite-boundary": masked_boundary},
    "a3_masked_probe": {"replay": masked_replay(PROBE, "scanner_probe"), "rewrite-boundary": masked_boundary},
    "a4_masked_probe_control": {
        "replay": masked_replay(replace_exact(PROBE, "  return event.next;\n", "  return end;\n", "probe control return"), "scanner_probe"),
        "rewrite-boundary": masked_boundary,
    },
    "b0_assert_next_at_call": {
        "replay": lambda source: record_replace(CALL, CALL + "    invariant scanner_next: event.next <= held;\n", "assert next at call")(masked_replay()(source)),
        "rewrite-boundary": masked_boundary,
    },
    "b1_assert_header_at_call": {
        "replay": lambda source: record_replace(CALL, CALL + "    invariant scanner_header: reader^.syntax.line.len < 1023_u64;\n", "assert header at call")(masked_replay()(source)),
        "rewrite-boundary": masked_boundary,
    },
    "c1_eof_local": {"replay": record_replace(CALL, "    let ended = reader^.eof;\n" + replace_exact(CALL, "eof: reader^.eof", "eof: ended", "local EOF actual"), "local EOF binding")},
    "c2_inline_input": {"replay": record_edit(lambda body: replace_exact(
        replace_exact(body, INPUT, "", "remove input binding"), CALL,
        replace_exact(CALL, "input: input", "input: &reader^.bytes.inner[0_u64..held]", "inline input actual"), "inline input call"))},
    "c3_detached_input": {"replay": record_replace(INPUT,
        "    let diagnostic_bytes = box_array_filled::<u8>(count: held, value: 0_u8);\n"
        "    for (copy_index in 0_u64..held) {\n"
        "      set diagnostic_bytes.inner[copy_index] = reader^.bytes.inner[copy_index];\n"
        "    }\n"
        "    let input = &diagnostic_bytes.inner[0_u64..held];\n", "detach input backing")},
    "c4_syntax_alias": {"replay": record_replace(CALL, "    let syntax_ref = &reader^.syntax;\n" + replace_exact(CALL, "syntax: &reader^.syntax", "syntax: syntax_ref", "alias syntax actual"), "alias syntax binding")},
    "c5_detached_syntax": {"replay": record_replace(CALL,
        "    let local_syntax = aof_syntax_new(position: 0_u64, valid: 0_u64);\n"
        "    swap(first: &local_syntax, second: &reader^.syntax);\n" +
        replace_exact(CALL, "syntax: &reader^.syntax", "syntax: &local_syntax", "detached syntax actual") +
        "    swap(first: &local_syntax, second: &reader^.syntax);\n", "detach syntax backing")},
    "c6_no_fill": {"replay": record_edit(no_fill)},
    "c7_no_fill_match": {"replay": record_replace(FILL_MATCH,
        "      let fill_outcome = aof_fill(store: store, factory: factory, file: file, reader: reader, clock: clock, rewriting: rewriting);\n", "remove fill outcome join")},
    "c8_reverse_header": {"replay": record_replace(HEADER,
        "    invariant probe_bound: reader^.syntax.probe <= 5_u64,\n"
        "    invariant header_room: reader^.syntax.line.len < 1023_u64,\n"
        "    invariant capacity: reader^.held <= reader^.bytes.inner.len,\n"
        "    invariant window: reader^.cursor <= reader^.held\n", "reverse invariant order")},
    # An earlier TYPE-11 call-precondition error makes c9 inconclusive about
    # whether the later window fact is lost; inspect the diagnostic location.
    "c9_no_probe_header": {"replay": record_replace(
        "    invariant header_room: reader^.syntax.line.len < 1023_u64,\n"
        "    invariant probe_bound: reader^.syntax.probe <= 5_u64\n",
        "    invariant header_room: reader^.syntax.line.len < 1023_u64\n", "remove probe loop clause")},
}
files = {
    "replay": "firn/persistence/replay.wf",
    "scanner": "firn/persistence/scanner.wf",
    "rewrite-boundary": "firn/persistence/rewrite-boundary.wf",
}
sources = {key: (root / path).read_text() for key, path in files.items()}
# Check every patch before creating any variant, so a source mismatch aborts
# the experiment instead of leaving silently unmodified/partially patched cases.
prepared = {}
for name, edits in variants.items():
    prepared[name] = {}
    for key, edit in edits.items():
        changed = edit(sources[key])
        if changed == sources[key]:
            raise ValueError(f"{name}/{key}: patch did not change the source")
        if not changed.endswith("\n") or changed.endswith("\n\n"):
            raise ValueError(f"{name}/{key}: generated source must end with exactly one LF")
        prepared[name][key] = changed

for name, edits in prepared.items():
    directory = out / name
    shutil.copytree(root / "firn", directory / "firn")
    for entry in root.resolve().iterdir():
        if entry.name not in ("firn", ".git", "build"):
            (directory / entry.name).symlink_to(entry)
    for key, source in edits.items():
        (directory / files[key]).write_text(source)
    print(name, "ok")
