#!/usr/bin/env python3
"""Check the pub/sub barrier patch against the pristine pinned Redis suite.

run.sh calls this after extraction and before applying the patch. Native GNU
patch owns contextual matching; the temporary fixtures show that repeated,
missing and changed inputs fail. Remove this check with pubsub-barriers.patch
when the pinned suite itself waits for the unsubscribe acknowledgments.
"""

import argparse
from pathlib import Path
import re
import subprocess
import sys
import tempfile


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", type=Path, help="pristine extracted Redis 7.2.16 directory")
    parser.add_argument("patch", type=Path, help="pubsub-barriers.patch")
    arguments = parser.parse_args()

    version_file = arguments.suite / "src/version.h"
    versions = re.findall(
        r'^#define REDIS_VERSION "([^"]+)"$',
        version_file.read_text(encoding="utf-8"),
        flags=re.MULTILINE,
    )
    if versions != ["7.2.16"]:
        raise ValueError(f"{version_file}: expected Redis 7.2.16, found {versions!r}")

    sources = {
        relative: (arguments.suite / relative).read_bytes()
        for relative in ["tests/unit/pubsub.tcl", "tests/unit/pubsubshard.tcl"]
    }
    title = b'    test "PUBLISH/PSUBSCRIBE after PUNSUBSCRIBE without arguments" {'
    if sources["tests/unit/pubsub.tcl"].count(title) != 1:
        raise ValueError("pristine pubsub.tcl must contain the exact PUNSUBSCRIBE test heading once")
    patch = arguments.patch.read_bytes()

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


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        print(f"check-pubsub-barriers.py: {error}", file=sys.stderr)
        sys.exit(1)
