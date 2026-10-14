# Version/provenance recording for redis-bench.sh's opt-in competitor lines.
# Sourced by the instrument; sourcing performs no commands. The reference
# archive pin belongs to tests/redis-suite/reference.sh. The current competitor
# installer writes a JSON receipt binding archive pins to installed executable
# hashes. Dragonfly/Garnet pins are GitHub's published asset digests; the CI
# installer independently hashes downloaded bytes before issuing a receipt.
# Valkey's source pin is the digest published in the immutable Homebrew formula
# b4bfd41d027c1161d55ceeb5246a4bb02f619392, not a locally computed digest:
# https://github.com/Homebrew/homebrew-core/blob/b4bfd41d027c1161d55ceeb5246a4bb02f619392/Formula/v/valkey.rb#L4-L5
# Installation in CI checks those source bytes before building. An executable
# hash cannot replace an archive pin; missing provenance fails full runs.

# Match a complete formal-release token, not a prefix (2.0.20), prerelease
# (2.0.2-rc1), or build suffix (2.0.2+local). Accept complete v/v= forms
# printed by Redis/Valkey and a plain token when another server provides it.
# Dragonfly v2.0.2 dfly_main.cc constructs colored kGitTag-kGitSha output:
# https://github.com/dragonflydb/dragonfly/blob/v2.0.2/src/server/dfly_main.cc#L994-L997
# The exact Git SHA generation is not yet verified. Do not invent a permitted
# suffix: a probe records the real output as mismatched until that format is
# established. Garnet v2.2.1's --version parser is also still unverified: its
# entry point only passes args into GarnetServer, without handling --version:
# https://github.com/microsoft/garnet/blob/v2.2.1/main/GarnetServer/Program.cs#L12-L23
version_release_matches() {
    printf '%s\n' "$2" | awk -v release="$1" '
        {
            count = split($0, token, /[[:space:]=:,()]+/)
            for (i = 1; i <= count; i++) {
                if (token[i] == release || token[i] == "v" release)
                    found = 1
            }
        }
        END { exit !found }
    '
}

# Compare canonical paths supplied by realpath at runtime. Merely fetching the
# pinned archive does not establish that redis-server on PATH is its build.
# Keep this part pure so the wrong executable and failed resolution have cases
# without fetching/building the reference or touching a real installation.
reference_origin_matches() {
    case $1 in /*) ;; *) return 1 ;; esac
    case $2 in /*) ;; *) return 1 ;; esac
    [ "$1" = "$2" ]
}

# A probe records unsupported version/provenance without certifying it. A full
# run requires both checks; an invalid probe flag never weakens the guard.
compare_version_acceptable() {
    case $1 in
        true) return 0 ;;
        false) [ "$2" = matched ] && [ "$3" = verified ] ;;
        *) return 1 ;;
    esac
}

# Version, archive URL and immutable SHA-256, one field per line. These pins
# must agree with redis-bench.yml's install-competitors job. "unavailable"
# is an explicit blocker, never a digest accepted from a receipt or environment.
competitor_pin() {
    case $1 in
        dragonfly)
            printf '%s\n' '2.0.2' \
                'https://github.com/dragonflydb/dragonfly/releases/download/v2.0.2/dragonfly-x86_64.tar.gz' \
                'eb9dfe654c69ab190cb3aa329ed3599ebe19c5e9a3a437bb7bc38ef52f665c88' ;;
        garnet)
            printf '%s\n' '2.2.1' \
                'https://github.com/microsoft/garnet/releases/download/v2.2.1/linux-x64-based.tar.xz' \
                '605d91a6ba4c3f4f416b74e676e945b983cf73fdd28e8cad569aa84e699d3903' ;;
        valkey)
            printf '%s\n' '9.1.2' \
                'https://github.com/valkey-io/valkey/archive/refs/tags/9.1.2.tar.gz' \
                '19c23908e7d57e8d91ef85b41f5646307582f10f4f0fb999bbf89ed24ec9c983' ;;
        *) return 1 ;;
    esac
}

# The shell has no JSON parser. Python parses receipts as data and rejects
# duplicate keys, and the shared validator
# below checks the immutable pins and actual paths/hashes. --selftest supplies
# fake data to that same validator; it performs no installation or file reads.
competitor_receipt_check() {
    python3 - "$@" <<'PY'
import copy
import hashlib
import json
import posixpath
import re
import shlex
import sys
from pathlib import Path


def digest(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def canonical(value):
    return isinstance(value, str) and value.startswith("/") and posixpath.normpath(value) == value


MANAGED_KINDS = ("dotnet_wrapper", "dotnet_apphost_wrapper")


def wrapper_text(runtime, payload, kind="dotnet_wrapper"):
    # A DLL wrapper execs dotnet with a DLL argument. A framework-dependent
    # apphost wrapper execs the apphost itself, with DOTNET_ROOT locating the
    # runtime. Both forms bind the same three files in the installation receipt.
    command = shlex.quote(payload)
    if kind == "dotnet_wrapper":
        command = shlex.quote(runtime) + " " + command
    return ("#!/bin/sh\nexport DOTNET_ROOT=" + shlex.quote(str(Path(runtime).parent)) +
            "\nexec " + command + ' "$@"\n')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate-json-key")
        result[key] = value
    return result


def validate_receipt(receipt, expected, actual):
    version, archive_url, archive_sha = expected
    if not digest(archive_sha):
        return "archive-pin-unavailable"
    if not isinstance(receipt, dict):
        return "receipt-not-object"
    for field, wanted in (("version", version), ("archive_url", archive_url),
                          ("archive_sha256", archive_sha)):
        if receipt.get(field) != wanted:
            return field + "-mismatch"
    kind = receipt.get("executable_kind")
    if kind != "native" and kind not in MANAGED_KINDS:
        return "executable-kind-invalid"
    for part in ("executable", "payload", "runtime"):
        path_field, sha_field = part + "_path", part + "_sha256"
        required = part == "executable" or kind in MANAGED_KINDS
        if not required and path_field not in receipt and sha_field not in receipt:
            continue
        if path_field not in receipt or sha_field not in receipt:
            return part + "-pair-missing"
        if not canonical(receipt[path_field]):
            return part + "-path-not-canonical"
        if not digest(receipt[sha_field]):
            return part + "-digest-invalid"
        if actual.get(path_field) != receipt[path_field]:
            return part + "-path-mismatch"
        if actual.get(sha_field) != receipt[sha_field]:
            return part + "-digest-mismatch"
    if kind in MANAGED_KINDS:
        wanted = wrapper_text(receipt["runtime_path"], receipt["payload_path"], kind)
        if actual.get("wrapper_text") != wanted:
            return "wrapper-command-mismatch"
    return "verified"


def file_facts(path):
    resolved = Path(path).resolve(strict=True)
    checksum = hashlib.sha256()
    with resolved.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return str(resolved), checksum.hexdigest()


def selftest():
    expected = ("2.2.1", "https://example.invalid/garnet-2.2.1.tar.xz", "a" * 64)
    receipt = {"version": expected[0], "archive_url": expected[1],
               "archive_sha256": expected[2], "executable_kind": "native",
               "executable_path": "/install/GarnetServer", "executable_sha256": "b" * 64}
    actual = {"executable_path": receipt["executable_path"],
              "executable_sha256": receipt["executable_sha256"]}

    def check(wanted, candidate=receipt, facts=actual, pins=expected):
        got = validate_receipt(candidate, pins, facts)
        if got != wanted:
            raise ValueError("receipt-selftest: expected " + wanted + ", got " + got)

    check("verified")
    for field, replacement in (("version", "2.2.10"), ("archive_url", "wrong-url"),
                               ("archive_sha256", "c" * 64)):
        changed = dict(receipt, **{field: replacement})
        check(field + "-mismatch", changed)
    for missing_pin in ("unavailable", "A" * 64, "a" * 63):
        check("archive-pin-unavailable", pins=(expected[0], expected[1], missing_pin))
    check("receipt-not-object", candidate=[])
    check("executable-kind-invalid", candidate=dict(receipt, executable_kind="other"))
    missing_executable_hash = copy.copy(receipt)
    del missing_executable_hash["executable_sha256"]
    check("executable-pair-missing", candidate=missing_executable_hash)
    check("executable-path-not-canonical", candidate=dict(receipt, executable_path="GarnetServer"))
    check("executable-path-not-canonical", candidate=dict(receipt, executable_path="/install/../GarnetServer"))
    check("executable-digest-invalid", candidate=dict(receipt, executable_sha256="bad"))
    check("executable-path-mismatch", facts=dict(actual, executable_path="/other/GarnetServer"))
    check("executable-digest-mismatch", facts=dict(actual, executable_sha256="c" * 64))
    check("payload-pair-missing", candidate=dict(receipt, payload_path="/install/GarnetServer.dll"))
    optional = dict(receipt, payload_path="/install/GarnetServer.dll", payload_sha256="d" * 64)
    optional_actual = dict(actual, payload_path=optional["payload_path"], payload_sha256="d" * 64)
    check("verified", optional, optional_actual)
    check("payload-digest-mismatch", optional, dict(optional_actual, payload_sha256="e" * 64))
    wrapper = dict(optional, executable_kind="dotnet_wrapper", runtime_path="/dotnet/dotnet",
                   runtime_sha256="e" * 64)
    wrapper_actual = dict(optional_actual, runtime_path=wrapper["runtime_path"], runtime_sha256="e" * 64,
                          wrapper_text=wrapper_text(wrapper["runtime_path"], wrapper["payload_path"]))
    check("verified", wrapper, wrapper_actual)
    check("payload-pair-missing", dict(receipt, executable_kind="dotnet_wrapper"))
    missing_runtime = copy.copy(wrapper)
    del missing_runtime["runtime_sha256"]
    check("runtime-pair-missing", missing_runtime, wrapper_actual)
    check("runtime-path-mismatch", wrapper, dict(wrapper_actual, runtime_path="/other/dotnet"))
    check("runtime-digest-mismatch", wrapper, dict(wrapper_actual, runtime_sha256="f" * 64))
    check("wrapper-command-mismatch", wrapper, dict(wrapper_actual, wrapper_text='exec /other/dotnet "$@"\n'))
    apphost = dict(wrapper, executable_kind="dotnet_apphost_wrapper", payload_path="/install/net8.0/GarnetServer")
    apphost_actual = dict(wrapper_actual, payload_path=apphost["payload_path"],
                          wrapper_text='#!/bin/sh\nexport DOTNET_ROOT=/dotnet\nexec /install/net8.0/GarnetServer "$@"\n')
    check("verified", apphost, apphost_actual)
    check("payload-pair-missing", dict(receipt, executable_kind="dotnet_apphost_wrapper"))
    check("payload-path-mismatch", apphost, dict(apphost_actual, payload_path="/install/net10.0/GarnetServer"))
    check("payload-digest-mismatch", apphost, dict(apphost_actual, payload_sha256="f" * 64))
    apphost_missing_runtime = dict(apphost)
    del apphost_missing_runtime["runtime_sha256"]
    check("runtime-pair-missing", apphost_missing_runtime, apphost_actual)
    check("runtime-path-mismatch", apphost, dict(apphost_actual, runtime_path="/other/dotnet"))
    check("runtime-digest-mismatch", apphost, dict(apphost_actual, runtime_sha256="f" * 64))
    check("wrapper-command-mismatch", apphost,
          dict(apphost_actual, wrapper_text='#!/bin/sh\nexport DOTNET_ROOT=/dotnet\nexec /dotnet/dotnet /install/net8.0/GarnetServer "$@"\n'))
    try:
        json.loads('{"version":"2.2.1","version":"other"}', object_pairs_hook=unique_object)
    except ValueError as error:
        if str(error) != "duplicate-json-key":
            raise
    else:
        raise ValueError("receipt-selftest: accepted duplicate JSON keys")


try:
    if sys.argv[1:] == ["--selftest"]:
        selftest()
        print("receipt-selftest: passed")
    else:
        receipt_path, version, archive_url, archive_sha, executable = sys.argv[1:]
        # A receipt cannot make an absent hardcoded immutable pin acceptable.
        if not digest(archive_sha):
            print("archive-pin-unavailable")
            sys.exit(1)
        with Path(receipt_path).open(encoding="utf-8") as source:
            receipt = json.load(source, object_pairs_hook=unique_object)
        actual = {}
        actual["executable_path"], actual["executable_sha256"] = file_facts(executable)
        if isinstance(receipt, dict):
            for part in ("payload", "runtime"):
                value = receipt.get(part + "_path")
                if canonical(value):
                    actual[part + "_path"], actual[part + "_sha256"] = file_facts(value)
            if receipt.get("executable_kind") in MANAGED_KINDS:
                actual["wrapper_text"] = Path(actual["executable_path"]).read_bytes().decode("utf-8")
        reason = validate_receipt(receipt, (version, archive_url, archive_sha), actual)
        print(reason)
        sys.exit(0 if reason == "verified" else 1)
except FileNotFoundError:
    print("receipt-or-installed-file-missing")
    sys.exit(1)
except (OSError, ValueError, TypeError) as error:
    print("receipt-invalid:" + type(error).__name__ + ":" + str(error).replace("\n", " "))
    sys.exit(1)
PY
}

# Selected plain competitor names, including reference-io/valkey-io. Each
# executable is recorded once even when both of its configurations are chosen.
# ROOT, OUT, DRAGONFLY/GARNET and COMPARE_PROBE come from redis-bench.sh.
# Complete the inventory before returning a failure, so host.txt explains all
# installed lines rather than only the first mismatch. --version is bounded:
# an executable that treats it as an ordinary server start cannot hang a run.
compare_versions() (
    cv_failed=0
    cv_seen=' '
    for cv_line do
        case $cv_line in
            reference|reference-io)
                cv_name=reference; cv_bin=redis-server; cv_expected=7.2.16 ;;
            valkey|valkey-io)
                cv_name=valkey; cv_bin=valkey-server ;;
            dragonfly)
                cv_name=dragonfly; cv_bin=${DRAGONFLY:-dragonfly} ;;
            garnet)
                cv_name=garnet; cv_bin=${GARNET:-garnet-server} ;;
            *)
                echo "versions: unsupported competitor '$cv_line'" >&2
                cv_failed=1
                continue ;;
        esac
        case $cv_name in
            dragonfly) cv_version_support=build-suffix-unverified ;;
            garnet) cv_version_support=version-command-unverified ;;
            *) cv_version_support=formal-release-token ;;
        esac
        case $cv_seen in *" $cv_name "*) continue ;; esac
        cv_seen="$cv_seen$cv_name "
        cv_archive_url=unavailable
        cv_archive_sha=unavailable
        cv_receipt_path=not-applicable
        cv_receipt_status=not-applicable
        if [ "$cv_name" != reference ]; then
            cv_pins=$(competitor_pin "$cv_name") || return 1
            cv_expected=$(printf '%s\n' "$cv_pins" | sed -n '1p')
            cv_archive_url=$(printf '%s\n' "$cv_pins" | sed -n '2p')
            cv_archive_sha=$(printf '%s\n' "$cv_pins" | sed -n '3p')
            cv_receipt_path=$HOME/.local/opt/$cv_name-$cv_expected/firn-install-receipt
        fi
        cv_path=$(command -v "$cv_bin" 2>/dev/null) || cv_path=
        if [ -z "$cv_path" ] || [ ! -f "$cv_path" ] || [ ! -x "$cv_path" ]; then
            printf 'server-version,%s,expected=%s,executable=%s,version_status=missing,version_support=%s,archive_url=%s,archive_sha256=%s,receipt_path=%s,receipt_check=executable-missing,archive_hash_status=unverified,executable_origin=missing,archive_provenance=unverified\n' \
                "$cv_name" "$cv_expected" "$cv_bin" "$cv_version_support" \
                "$cv_archive_url" "$cv_archive_sha" "$cv_receipt_path" >>"$OUT/host.txt"
            case ${COMPARE_PROBE:-false}:$cv_name in
                true:dragonfly|true:garnet) ;;
                *)
                    echo "versions: $cv_name has no executable '$cv_bin'" >&2
                    cv_failed=1 ;;
            esac
            continue
        fi
        cv_archive_hash_status=unverified
        cv_executable_origin=unverified
        cv_archive_status=unverified
        cv_reference_executable=unavailable
        if [ "$cv_name" = reference ]; then
            if cv_archive_sha=$(bash "$ROOT/tests/redis-suite/reference.sh" sha256) && \
                bash "$ROOT/tests/redis-suite/reference.sh" archive >/dev/null; then
                cv_archive_hash_status=verified
            else
                cv_archive_hash_status=check-failed
            fi
            # bin is invoked only at measurement runtime in CI. The workflow
            # normally built this already; obtain its actual location rather
            # than inferring that PATH points at the reference build. Do this
            # before reading version/hash in case bin repairs a stale cache.
            if cv_reference_bin=$(bash "$ROOT/tests/redis-suite/reference.sh" bin); then
                cv_selected_realpath=$(realpath -- "$cv_path" 2>/dev/null) || cv_selected_realpath=
                cv_reference_executable=$(realpath -- "$cv_reference_bin/redis-server" 2>/dev/null) || cv_reference_executable=
                if reference_origin_matches "$cv_selected_realpath" "$cv_reference_executable"; then
                    cv_executable_origin=matched-reference-build
                elif [ -n "$cv_selected_realpath" ] && [ -n "$cv_reference_executable" ]; then
                    cv_executable_origin=path-mismatch
                else
                    cv_executable_origin=check-failed
                fi
            else
                cv_executable_origin=check-failed
            fi
            if [ "$cv_archive_hash_status" = verified ] && \
                [ "$cv_executable_origin" = matched-reference-build ]; then
                cv_archive_status=verified
            fi
        fi
        if cv_output=$(timeout --signal=TERM --kill-after=2s 5s "$cv_path" --version </dev/null 2>&1); then
            cv_exit=0
        else
            cv_exit=$?
        fi
        cv_version_status=command-failed
        if [ "$cv_exit" -eq 0 ]; then
            if version_release_matches "$cv_expected" "$cv_output"; then
                cv_version_status=matched
            else
                cv_version_status=mismatched
            fi
        fi
        cv_executable_sha=$(sha256sum "$cv_path" 2>/dev/null | awk '{print $1}')
        [ -n "$cv_executable_sha" ] || cv_executable_sha=unavailable
        if [ "$cv_name" != reference ]; then
            if cv_receipt_status=$(competitor_receipt_check "$cv_receipt_path" "$cv_expected" \
                "$cv_archive_url" "$cv_archive_sha" "$cv_path"); then
                cv_archive_hash_status=matched-install-receipt
                cv_executable_origin=matched-install-receipt
                cv_archive_status=verified
            else
                [ -n "$cv_receipt_status" ] || cv_receipt_status=check-failed
            fi
        fi
        {
            printf 'server-version,%s,expected=%s,executable=%s,version_status=%s,version_support=%s,version_command_exit=%s,executable_sha256=%s,archive_url=%s,archive_sha256=%s,receipt_path=%s,receipt_check=%s,archive_hash_status=%s,executable_origin=%s,reference_executable=%s,archive_provenance=%s\n' \
                "$cv_name" "$cv_expected" "$cv_path" "$cv_version_status" "$cv_version_support" "$cv_exit" \
                "$cv_executable_sha" "$cv_archive_url" "$cv_archive_sha" "$cv_receipt_path" "$cv_receipt_status" "$cv_archive_hash_status" \
                "$cv_executable_origin" "$cv_reference_executable" "$cv_archive_status"
            printf 'server-version-output-begin,%s\n%s\nserver-version-output-end,%s\n' \
                "$cv_name" "$cv_output" "$cv_name"
        } >>"$OUT/host.txt"
        if ! compare_version_acceptable "${COMPARE_PROBE:-false}" "$cv_version_status" "$cv_archive_status"; then
            echo "versions: $cv_name expected $cv_expected; version=$cv_version_status archive=$cv_archive_hash_status origin=$cv_executable_origin provenance=$cv_archive_status (see host.txt)" >&2
            cv_failed=1
        fi
    done
    [ "$cv_failed" -eq 0 ]
)

# Pure fake-output/guard cases, run by placement-selftest in CI. No server or
# source archive is fetched, started or hashed by these cases.
versions_selftest() (
    versions_expect_match() {
        if ! version_release_matches "$1" "$2"; then
            echo "versions-selftest: rejected release '$2'" >&2
            exit 1
        fi
    }
    versions_expect_reject() {
        if version_release_matches "$1" "$2"; then
            echo "versions-selftest: accepted non-release '$2'" >&2
            exit 1
        fi
    }
    versions_expect_match 7.2.16 'Redis server v=7.2.16 sha=00000000:0 malloc=jemalloc'
    versions_expect_match 9.1.2 'Valkey server v=9.1.2 sha=00000000:0'
    versions_expect_match 2.0.2 'dragonfly version v2.0.2'
    versions_expect_match 2.2.1 'GarnetServer 2.2.1'
    versions_expect_match 2.2.1 'GarnetServer version: 2.2.1 (release)'
    versions_expect_reject 2.0.2 'dragonfly version v2.0.20'
    versions_expect_reject 2.0.2 'dragonfly version v2.0.2-rc1'
    versions_expect_reject 2.0.2 'dragonfly version v2.0.2+local'
    # A SHA suffix remains rejected until its exact pinned build format is
    # established; it must not accidentally reopen prerelease acceptance.
    versions_expect_reject 2.0.2 'dragonfly v2.0.2-01234567'
    versions_expect_reject 2.0.2 'dragonfly version v2.0.2.1'
    versions_expect_reject 2.0.2 'dragonfly version prefix2.0.2'
    versions_expect_reject 2.2.1 'GarnetServer 2.1.9'
    reference_origin_matches /cache/reference/bin/redis-server /cache/reference/bin/redis-server || exit 1
    if reference_origin_matches /usr/bin/redis-server /cache/reference/bin/redis-server || \
        reference_origin_matches '' /cache/reference/bin/redis-server || \
        reference_origin_matches /cache/reference/bin/redis-server '' || \
        reference_origin_matches redis-server redis-server; then
        echo 'versions-selftest: accepted a wrong or unresolved reference executable' >&2
        exit 1
    fi
    compare_version_acceptable true mismatched unverified || exit 1
    compare_version_acceptable true command-failed unverified || exit 1
    compare_version_acceptable false matched verified || exit 1
    if compare_version_acceptable false matched unverified || \
        compare_version_acceptable false mismatched verified || \
        compare_version_acceptable false command-failed verified || \
        compare_version_acceptable invalid matched verified; then
        echo 'versions-selftest: version/provenance guard accepted unverified full run' >&2
        exit 1
    fi
    competitor_receipt_check --selftest || exit 1
    echo 'versions-selftest: passed'
)
