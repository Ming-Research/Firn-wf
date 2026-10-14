#!/usr/bin/env python3
"""Check exception-safe cleanup of the pinned Redis WATCH/list fixtures.

run.sh calls this before applying blocked-client-cleanup.patch. GNU patch
checks contextual applicability; a network-free Tcl stub exercises the actual
patched fixture bodies and deliberate missing-cleanup/error-forwarding faults.
Native patch alone cannot establish those control-flow properties. Remove this
instrument with the patch when the upstream fixtures own equivalent cleanup.
"""

import argparse
from pathlib import Path
import re
import subprocess
import sys
import tempfile


RELATIVE = "tests/unit/type/list.tcl"
TITLES = (
    "PUSH resulting from BRPOPLPUSH affect WATCH",
    "BRPOPLPUSH does not affect WATCH while still blocked",
)


def copy_fixture(destination, content):
    target = destination / RELATIVE
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(content)


def check_application(fixture, patch, name, must_succeed):
    result = subprocess.run(
        ["patch", "--batch", "--forward", "--fuzz=0", "-p1", "-d", str(fixture)],
        input=patch, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
    )
    if (result.returncode == 0) != must_succeed:
        output = result.stdout.decode("utf-8", errors="replace").strip()
        raise ValueError(f"{name}: unexpected patch exit {result.returncode}\n{output}")


def fixtures(source):
    found = []
    for title in TITLES:
        heading = '    test "' + title + '" {\n'
        if source.count(heading) != 1:
            raise ValueError(f"expected fixture heading exactly once: {title}")
        pattern = re.escape(heading) + r"(.*?)\n    \} (\{\}|\{somevalue\})"
        match = re.search(pattern, source, flags=re.DOTALL)
        if match is None:
            raise ValueError(f"missing complete fixture: {title}")
        found.append((title, match.group(1), match.group(2)))
    return found


def check_original_body(original, patched):
    for (title, before, expected), (after_title, after, after_expected) in zip(
        fixtures(original), fixtures(patched)
    ):
        if (title, expected) != (after_title, after_expected):
            raise ValueError(f"changed fixture name or expected result: {title}")
        capture = re.search(
            r"        set test_outcome \[catch \{\n(.*?)\n        \} test_result test_options\]",
            after, flags=re.DOTALL,
        )
        if capture is None:
            raise ValueError(f"missing captured original body: {title}")
        body = "\n".join(line[4:] for line in capture.group(1).splitlines())
        baseline = "        set blocked_before [s blocked_clients]\n"
        if not body.startswith(baseline):
            raise ValueError(f"missing blocked-client entry count: {title}")
        body = body[len(baseline):]
        before = "\n".join(
            line for line in before.splitlines()
            if line not in ("        $blocked_client close", "        $watching_client close")
        )
        if body != before:
            raise ValueError(f"commands or successful result changed inside fixture: {title}")


TCL_STUB = r'''
set fixtures [dict create]
proc test {name body expected} {
    dict set ::fixtures $name [list $body $expected]
}
proc redis_deferring_client {} {
    incr ::next_client
    if {$::creation_failure == $::next_client} {
        return -code error -errorcode {FIXTURE CREATE} -errorinfo {original creation error} {creation refused by fixture}
    }
    set index $::next_client
    set name fixture_client_$index
    interp alias {} $name {} fixture_client $index
    lappend ::created $index
    return $name
}
proc fixture_client {index method args} {
    if {$method eq "close"} {
        lappend ::closed $index
        if {[lsearch -exact $::close_failures $index] >= 0} {
            return -code error -errorcode [list FIXTURE CLOSE $index] -errorinfo {original close error} {close refused by fixture}
        }
        return {}
    }
    if {$method eq "read"} {
        incr ::reads
        if {$::watch_failure && $::reads == 1} {
            return -code error -errorcode {FIXTURE WATCH REFUSED} -errorinfo {original WATCH refusal} {WATCH refused by fixture}
        }
        if {$::reads == 4} {return $::expected_result}
        return OK
    }
    return {}
}
proc r {args} {return 0}
proc s {field} {
    if {$field ne "blocked_clients" || $::created ne {}} {
        error {entry blocked-client count must precede client creation}
    }
    return $::baseline
}
proc wait_for_blocked_client {} {return {}}
proc wait_for_blocked_clients_count {count} {
    lappend ::barrier_calls $count
    if {$::closed ne $::created || $count != $::baseline} {
        error {cleanup count barrier must follow both closes and restore the entry count}
    }
    if {$::barrier_failure} {
        return -code error -errorcode {FIXTURE CLEANUP BARRIER} -errorinfo {original barrier error} {barrier refused by fixture}
    }
}
proc check_case {name kind} {
    foreach index {1 2} {catch {rename fixture_client_$index {}}}
    set ::next_client 0
    set ::created {}
    set ::closed {}
    set ::reads 0
    set ::baseline 3
    set ::barrier_calls {}
    set ::watch_failure [expr {$kind eq "watch-error" || $kind eq "body-and-close-errors" || $kind eq "body-and-barrier-errors"}]
    set ::creation_failure 0
    set ::close_failures {}
    if {$kind eq "creation-first-error"} {set ::creation_failure 1}
    if {$kind eq "creation-second-error"} {set ::creation_failure 2}
    if {$kind eq "close-first-error"} {set ::close_failures {1}}
    if {$kind eq "close-second-error"} {set ::close_failures {2}}
    if {$kind eq "body-and-close-errors"} {set ::close_failures {1 2}}
    set ::barrier_failure [expr {$kind eq "barrier-error" || $kind eq "body-and-barrier-errors" || $::close_failures ne {}}]
    set body [lindex [dict get $::fixtures $name] 0]
    set ::expected_result [lindex [dict get $::fixtures $name] 1]
    set status [catch {uplevel #0 $body} actual options]
    if {$::closed ne $::created} {
        error "$name/$kind: every created client must close exactly once in creation order; created $::created, closed $::closed"
    }
    if {$::barrier_calls ne [list $::baseline]} {
        error "$name/$kind: entry blocked-client count must be restored before returning"
    }
    if {$kind eq "success"} {
        if {$status != 0 || $actual ne $::expected_result} {
            error "$name/$kind: successful result changed: status $status, value $actual"
        }
        return
    }
    if {$::watch_failure} {
        set wanted {FIXTURE WATCH REFUSED}
        set message {WATCH refused by fixture}
        set marker {original WATCH refusal}
        set original_options $::test_options
    } elseif {$::creation_failure != 0} {
        set wanted {FIXTURE CREATE}
        set message {creation refused by fixture}
        set marker {original creation error}
        set original_options $::test_options
    } elseif {$kind eq "close-first-error"} {
        set wanted {FIXTURE CLOSE 1}
        set message {close refused by fixture}
        set marker {original close error}
        set original_options $::blocked_options
    } elseif {$kind eq "close-second-error"} {
        set wanted {FIXTURE CLOSE 2}
        set message {close refused by fixture}
        set marker {original close error}
        set original_options $::watching_options
    } else {
        set wanted {FIXTURE CLEANUP BARRIER}
        set message {barrier refused by fixture}
        set marker {original barrier error}
        set original_options $::barrier_options
    }
    if {$status != 1 || $actual ne $message} {
        error "$name/$kind: original error status/message changed"
    }
    foreach key {-code -level -errorcode} {
        if {[dict get $options $key] ne [dict get $original_options $key]} {
            error "$name/$kind: original $key was not propagated"
        }
    }
    if {[dict get $options -errorcode] ne $wanted} {
        error "$name/$kind: original errorcode changed"
    }
    set info [dict get $options -errorinfo]
    if {[string first $marker $info] != 0 || [string first [dict get $original_options -errorinfo] $info] != 0} {
        error "$name/$kind: original errorinfo was not retained"
    }
}
proc require_fixtures {} {
    set expected [list {PUSH resulting from BRPOPLPUSH affect WATCH} {BRPOPLPUSH does not affect WATCH while still blocked}]
    if {[lsort [dict keys $::fixtures]] ne [lsort $expected]} {
        error {expected exactly the two named WATCH fixtures}
    }
}
proc run_all {} {
    require_fixtures
    foreach name [dict keys $::fixtures] {
        foreach kind {success watch-error close-first-error close-second-error body-and-close-errors creation-first-error creation-second-error barrier-error body-and-barrier-errors} {
            check_case $name $kind
        }
    }
}
proc run_watch_error {} {
    require_fixtures
    foreach name [dict keys $::fixtures] {check_case $name watch-error}
}
'''


def check_behavior(source, name, must_succeed, watch_only=False, expected_failure=None):
    fixture_source = "\n".join(
        'test "' + title + '" {\n' + body + "\n} " + expected
        for title, body, expected in fixtures(source)
    )
    run = "run_watch_error" if watch_only else "run_all"
    script = TCL_STUB + fixture_source + "\nif {[catch {" + run + r'''} message options]} {
    puts stderr $message
    exit 1
}
exit 0
'''
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", prefix="redis-blocked-client-stub-", suffix=".tcl"
    ) as fixture_script:
        fixture_script.write(script)
        fixture_script.flush()
        result = subprocess.run(
            ["tclsh", fixture_script.name], text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False,
        )
    if (result.returncode == 0) != must_succeed:
        raise ValueError(f"{name}: unexpected Tcl exit {result.returncode}\n{result.stdout.strip()}")
    if expected_failure is not None:
        if expected_failure not in result.stdout:
            raise ValueError(f"{name}: failed for the wrong reason\n{result.stdout.strip()}")
        print(f"{name}: detected {expected_failure}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", type=Path, help="pristine extracted Redis 7.2.16 directory")
    parser.add_argument("patch", type=Path, help="blocked-client-cleanup.patch")
    arguments = parser.parse_args()
    versions = re.findall(
        r'^#define REDIS_VERSION "([^"]+)"$',
        (arguments.suite / "src/version.h").read_text(encoding="utf-8"), flags=re.MULTILINE,
    )
    if versions != ["7.2.16"]:
        raise ValueError(f"expected Redis 7.2.16, found {versions!r}")
    source = (arguments.suite / RELATIVE).read_bytes()
    patch = arguments.patch.read_bytes()
    with tempfile.TemporaryDirectory(prefix="redis-blocked-client-cleanup-") as temporary:
        root = Path(temporary)
        pristine = root / "pristine"
        copy_fixture(pristine, source)
        check_application(pristine, patch, "pristine source", True)
        patched = (pristine / RELATIVE).read_text(encoding="utf-8")
        check_original_body(source.decode("utf-8"), patched)
        check_behavior(patched, "patched fixture control flow", True)
        check_behavior(
            source.decode("utf-8"), "original WATCH error must expose missing cleanup", False,
            watch_only=True, expected_failure="every created client must close exactly once",
        )
        check_application(pristine, patch, "already patched source", False)
        missing = root / "missing"
        copy_fixture(missing, source)
        (missing / RELATIVE).unlink()
        check_application(missing, patch, "missing list.tcl", False)
        for index, title in enumerate(TITLES):
            changed = root / f"changed-{index}"
            heading = ('    test "' + title + '" {').encode("utf-8")
            copy_fixture(changed, source.replace(heading, b'    test "Altered WATCH fixture" {', 1))
            check_application(changed, patch, "changed fixture context", False)
        for client in ("blocked", "watching"):
            close = f"set {client}_outcome [catch {{${client}_client close}} {client}_result {client}_options]"
            if patched.count(close) != 2:
                raise ValueError(f"expected two explicit {client} client closes")
            mutant = patched.replace(close, f"set {client}_outcome 0", 1)
            check_behavior(
                mutant, f"missing {client} cleanup must be detected", False,
                expected_failure="every created client must close exactly once",
            )
        forwarding = "return -options $test_options $test_result"
        if patched.count(forwarding) != 2:
            raise ValueError("expected two original-error forwarding sites")
        mutant = patched.replace(forwarding, "error $test_result", 1)
        check_behavior(
            mutant, "lost original error options must be detected", False,
            expected_failure="original -errorcode was not propagated",
        )
        barrier = "set barrier_outcome [catch {wait_for_blocked_clients_count $blocked_before} barrier_result barrier_options]"
        if patched.count(barrier) != 2:
            raise ValueError("expected two cleanup count barriers")
        mutant = patched.replace(barrier, "set barrier_outcome 0", 1)
        check_behavior(
            mutant, "missing cleanup barrier must be detected", False,
            expected_failure="entry blocked-client count must be restored before returning",
        )
    print("blocked client cleanup: original bodies/results preserved; both clients close and restore the entry count; errors retain precedence/options; original and injected faults detected")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        print(f"check-blocked-client-cleanup.py: {error}", file=sys.stderr)
        sys.exit(1)
