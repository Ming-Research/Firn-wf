# Version/provenance recording for redis-bench.sh's opt-in competitor lines.
# Sourced by the instrument; sourcing performs no commands. The reference
# archive pin belongs to tests/redis-suite/reference.sh. The current competitor
# installer resolves release tags and asset digests dynamically, then removes
# the archives without an install receipt: its executable hashes cannot stand
# in for missing immutable archive pins. Probes expose that gap; full runs fail.

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
                cv_name=valkey; cv_bin=valkey-server; cv_expected=9.1.2 ;;
            dragonfly)
                cv_name=dragonfly; cv_bin=${DRAGONFLY:-dragonfly}; cv_expected=2.0.2 ;;
            garnet)
                cv_name=garnet; cv_bin=${GARNET:-garnet-server}; cv_expected=2.2.1 ;;
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
        cv_path=$(command -v "$cv_bin" 2>/dev/null) || cv_path=
        if [ -z "$cv_path" ] || [ ! -f "$cv_path" ] || [ ! -x "$cv_path" ]; then
            printf 'server-version,%s,expected=%s,executable=%s,version_status=missing,version_support=%s,archive_sha256=unavailable,archive_hash_status=unverified,executable_origin=missing,archive_provenance=unverified\n' \
                "$cv_name" "$cv_expected" "$cv_bin" "$cv_version_support" >>"$OUT/host.txt"
            case ${COMPARE_PROBE:-false}:$cv_name in
                true:dragonfly|true:garnet) ;;
                *)
                    echo "versions: $cv_name has no executable '$cv_bin'" >&2
                    cv_failed=1 ;;
            esac
            continue
        fi
        cv_archive_sha=unavailable
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
        {
            printf 'server-version,%s,expected=%s,executable=%s,version_status=%s,version_support=%s,version_command_exit=%s,executable_sha256=%s,archive_sha256=%s,archive_hash_status=%s,executable_origin=%s,reference_executable=%s,archive_provenance=%s\n' \
                "$cv_name" "$cv_expected" "$cv_path" "$cv_version_status" "$cv_version_support" "$cv_exit" \
                "$cv_executable_sha" "$cv_archive_sha" "$cv_archive_hash_status" \
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
    echo 'versions-selftest: passed'
)
