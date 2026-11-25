#!/usr/bin/env python3
"""Check the suite patches against the pristine pinned Redis suite.

run.sh calls this after extraction and before applying the patches. Native GNU
patch owns contextual matching; the temporary fixtures show that repeated,
missing and changed inputs fail, for the pub/sub barrier patch and for the
connection reset patch. For the reset patch two witnesses also run a small
unit against the pinned redis-server: a passing test must leave the shared
connection as it is, and a test after a failed one must see a fresh
connection; the same unit run on the pristine suite must fail its last test,
which shows the witness can detect the missing patch. Remove the pub/sub part
with pubsub-barriers.patch when the pinned suite itself waits for the
unsubscribe acknowledgments, and the reset part with connection-reset.patch
when the pinned suite restores the connection itself.
"""

import argparse
import os
from pathlib import Path
import random
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time


def copy_fixture(destination, sources):
    for relative, content in sources.items():
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)


def check_application(fixture, patch, name, must_succeed):
    result = subprocess.run(
        ["patch", "--batch", "--forward", "--fuzz=0", "-p1", "-d", str(fixture)],
        input=patch,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    if (result.returncode == 0) != must_succeed:
        expected = "success" if must_succeed else "a nonzero exit status"
        output = result.stdout.decode("utf-8", errors="replace").strip()
        raise ValueError(
            f"{name}: expected {expected}, got exit {result.returncode}\n{output}"
        )


def check_version(suite):
    version_file = suite / "src/version.h"
    versions = re.findall(
        r'^#define REDIS_VERSION "([^"]+)"$',
        version_file.read_text(encoding="utf-8"),
        flags=re.MULTILINE,
    )
    if versions != ["7.2.16"]:
        raise ValueError(f"{version_file}: expected Redis 7.2.16, found {versions!r}")


def check_pubsub(suite, patch_path):
    sources = {
        relative: (suite / relative).read_bytes()
        for relative in ["tests/unit/pubsub.tcl", "tests/unit/pubsubshard.tcl"]
    }
    title = b'    test "PUBLISH/PSUBSCRIBE after PUNSUBSCRIBE without arguments" {'
    if sources["tests/unit/pubsub.tcl"].count(title) != 1:
        raise ValueError("pristine pubsub.tcl must contain the exact PUNSUBSCRIBE test heading once")
    patch = patch_path.read_bytes()

    with tempfile.TemporaryDirectory(prefix="redis-pubsub-barriers-") as temporary:
        root = Path(temporary)
        pristine = root / "pristine"
        copy_fixture(pristine, sources)
        check_application(pristine, patch, "pristine pinned source", must_succeed=True)
        check_application(pristine, patch, "already patched source", must_succeed=False)

        missing = root / "missing"
        copy_fixture(missing, sources)
        (missing / "tests/unit/pubsubshard.tcl").unlink()
        check_application(missing, patch, "missing pubsubshard.tcl", must_succeed=False)

        changed = root / "changed"
        copy_fixture(changed, sources)
        altered = sources["tests/unit/pubsub.tcl"].replace(
            title, b'    test "Altered PUNSUBSCRIBE test heading" {', 1
        )
        (changed / "tests/unit/pubsub.tcl").write_bytes(altered)
        check_application(changed, patch, "changed PUNSUBSCRIBE context", must_succeed=False)

    print("pubsub barrier patch: pristine applies; repeated, missing and changed inputs fail")


WITNESS_UNIT = "unit/connection-reset-witness"

WITNESS_SOURCE = b"""start_server {tags {"witness"}} {
    test "witness passing test keeps the connection mode" {
        r hello 3
        r readraw 1
        r ping
    } {+PONG}

    test "witness next test sees the connection mode kept" {
        assert_equal {+PONG} [r ping]
        r readraw 0
        assert_equal 3 [dict get [r hello] proto]
    }

    test "witness failing test" {
        r hello 2
        r hmset witness-hash a 1 b 2
        r readraw 1
        r hgetall witness-hash
        assert_equal 1 2
    }

    test "witness test after a failure sees a fresh connection" {
        assert_equal PONG [r ping]
        assert_equal 3 [dict get [r hello] proto]
    }
}
"""

RESULT_LINE = re.compile(r"^\[(ok|err)\s*\]:\s*(witness [^(]*?)\s*(?:\(\d+ ms\)|in .*)?$")


def free_port(low, high, taken=()):
    """A loopback port in low..high-1 that nothing listens on, chosen at random."""
    for candidate in random.sample(range(low, high), 200):
        if candidate in taken:
            continue
        with socket.socket() as probe:
            try:
                probe.bind(("127.0.0.1", candidate))
            except OSError:
                continue
        return candidate
    raise ValueError(f"no free port in {low}-{high - 1}")


def start_reference_server(directory, port):
    version = subprocess.run(
        ["redis-server", "--version"], stdout=subprocess.PIPE, check=True
    ).stdout.decode("utf-8", errors="replace")
    if not re.search(r"\bv=7\.2\.16\b", version):
        raise ValueError(f"the witnesses need the pinned redis-server 7.2.16, found: {version.strip()}")
    server = subprocess.Popen(
        ["redis-server", "--port", str(port), "--bind", "127.0.0.1", "--save", "",
         "--appendonly", "no", "--dir", str(directory)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if server.poll() is not None:
            raise ValueError("the witnesses' redis-server exited at start")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return server
        except OSError:
            time.sleep(0.05)
    server.kill()
    server.wait()
    raise ValueError("the witnesses' redis-server did not listen within 20 s")


def run_witness_unit(suite_copy, runtest, port, base_port):
    (suite_copy / "tests/unit" / f"{Path(WITNESS_UNIT).name}.tcl").write_bytes(WITNESS_SOURCE)
    shutil.copy2(runtest, suite_copy / "runtest")
    result = subprocess.run(
        ["sh", "./runtest", "--host", "127.0.0.1", "--port", str(port), "--single", WITNESS_UNIT,
         "--baseport", str(base_port), "--portcount", "64",
         "--singledb", "--durable", "--clients", "1", "--timeout", "60"],
        cwd=suite_copy,
        env={**os.environ, "TERM": "dumb"},
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=180,
        check=False,
    )
    output = result.stdout.decode("utf-8", errors="replace")
    outcomes = {}
    for line in output.splitlines():
        found = RESULT_LINE.match(line.strip())
        if found:
            outcomes[found.group(2)] = found.group(1)
    return outcomes, output


def check_connection_reset(suite, patch_path, ports):
    source = suite / "tests/support/test.tcl"
    sources = {"tests/support/test.tcl": source.read_bytes()}
    anchor = b"    set test_start_time [clock milliseconds]\n"
    if sources["tests/support/test.tcl"].count(anchor) != 1:
        raise ValueError("pristine test.tcl must contain the test start time line once")
    patch = patch_path.read_bytes()

    with tempfile.TemporaryDirectory(prefix="redis-connection-reset-") as temporary:
        root = Path(temporary)
        pristine = root / "pristine"
        copy_fixture(pristine, sources)
        check_application(pristine, patch, "pristine pinned source", must_succeed=True)
        check_application(pristine, patch, "already patched source", must_succeed=False)

        missing = root / "missing"
        (missing / "tests/support").mkdir(parents=True)
        check_application(missing, patch, "missing test.tcl", must_succeed=False)

        changed = root / "changed"
        copy_fixture(changed, sources)
        altered = sources["tests/support/test.tcl"].replace(
            anchor, b"    set test_started_at [clock milliseconds]\n", 1
        )
        (changed / "tests/support/test.tcl").write_bytes(altered)
        check_application(changed, patch, "changed test start context", must_succeed=False)

        check_version(suite)
        unpatched = root / "unpatched"
        patched = root / "patched"
        for copy in (unpatched, patched):
            shutil.copytree(suite / "tests", copy / "tests")
        check_application(patched, patch, "pristine suite copy", must_succeed=True)

        # The suite's test server takes a free port in the 32 below the base
        # port. Shards run side by side, each with a range of ports of its
        # own, so the witnesses take their ports from the range run.sh gives.
        low, high = ports
        port = free_port(low, high)
        base_port = free_port(low + 32, high - 64, taken=(port,)) + 32
        server = start_reference_server(root, port)
        try:
            runtest = suite / "runtest"
            before, before_output = run_witness_unit(unpatched, runtest, port, base_port)
            after, after_output = run_witness_unit(patched, runtest, port, base_port)
        finally:
            server.terminate()
            server.wait(timeout=20)

    expected_after = {
        "witness passing test keeps the connection mode": "ok",
        "witness next test sees the connection mode kept": "ok",
        "witness failing test": "err",
        "witness test after a failure sees a fresh connection": "ok",
    }
    if after != expected_after:
        raise ValueError(
            f"patched suite: witness outcomes {after!r}, expected {expected_after!r}\n{after_output}"
        )
    if before.get("witness test after a failure sees a fresh connection") != "err":
        raise ValueError(
            "pristine suite: the witness after a failed test must fail without the patch, got "
            f"{before!r}\n{before_output}"
        )
    print(
        "connection reset patch: pristine applies; repeated, missing and changed inputs fail; "
        "a passing test keeps the connection and the test after a failure gets a fresh one, "
        "which the unpatched suite does not give"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ports", nargs=2, type=int, metavar=("LOW", "HIGH"), required=True,
        help="the loopback ports LOW to HIGH-1 this run may use",
    )
    parser.add_argument("suite", type=Path, help="pristine extracted Redis 7.2.16 directory")
    parser.add_argument("pubsub_patch", type=Path, help="pubsub-barriers.patch")
    parser.add_argument("reset_patch", type=Path, help="connection-reset.patch")
    arguments = parser.parse_args()

    check_version(arguments.suite)
    check_pubsub(arguments.suite, arguments.pubsub_patch)
    check_connection_reset(arguments.suite, arguments.reset_patch, arguments.ports)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"check-pubsub-barriers.py: {error}", file=sys.stderr)
        sys.exit(1)
