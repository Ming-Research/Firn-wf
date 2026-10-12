#!/usr/bin/env bash
# The reference server of firn (AGENTS.md): Redis 7.2.16, built from the
# official release archive. Ubuntu's package is 7.0.15, so every place that
# needs a reference redis-server, redis-cli or redis-benchmark gets them here:
# run.sh (the suite's sources, its client tools and the comparator), the
# consumers and redis-bench workflows, and the experiments they run.
#
# usage: reference.sh [bin | archive | version | sha256]
#
#   bin      (default) prints the directory that holds redis-server,
#            redis-cli and redis-benchmark of the reference, building it
#            from the archive when the cache lacks it
#   archive  prints the path of the release archive, fetched when missing
#            and checked against the SHA-256 pin below on every call
#   version  prints the version the pin names
#   sha256   prints the archive's SHA-256 pin
#
# Everything else goes to standard error. REDIS_COMPAT_CACHE names the
# directory that keeps the archive and the build,
# ${XDG_CACHE_HOME:-$HOME/.cache}/whitefoot-redis-compat by default. The
# build is made beside its place and moved there whole, so an interrupted
# build is never taken for a finished one, and it is used only when all three
# tools report the pinned version.
set -euo pipefail

version=7.2.16
sha256=960a8ec15e34ff40e57ff16837b26b33bd81f2da6d24497bb63de532a323a18e
url=https://download.redis.io/releases/redis-$version.tar.gz
cache=${REDIS_COMPAT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/whitefoot-redis-compat}
archive=$cache/redis-$version.tar.gz
bin=$cache/redis-$version-bin

# Whether the directory holds the three tools, each of the pinned version.
complete() {
    [ "$("$1/redis-server" --version 2> /dev/null | sed -n 's/^Redis server v=\([^ ]*\).*/\1/p')" = "$version" ] &&
        [ "$("$1/redis-cli" --version 2> /dev/null | sed -n 's/^redis-cli \([^ ]*\).*/\1/p')" = "$version" ] &&
        [ "$("$1/redis-benchmark" --version 2> /dev/null | sed -n 's/^redis-benchmark \([^ ]*\).*/\1/p')" = "$version" ]
}

fetch_archive() {
    mkdir -p "$cache"
    if [ ! -f "$archive" ]; then
        # Callers started together may all find the archive missing; each
        # fetches into a name of its own and the last rename wins.
        local part
        part=$(mktemp "$archive.part.XXXXXX")
        curl -fsSL -o "$part" "$url"
        mv -f "$part" "$archive"
    fi
    if ! echo "$sha256  $archive" | sha256sum --check --status; then
        echo "reference.sh: $archive does not have the SHA-256 $sha256" >&2
        exit 1
    fi
}

# The build directory is global: the exit trap removes it after this returns.
work=
build() {
    local compiler
    work=$(mktemp -d "$cache/build.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    tar -xzf "$archive" -C "$work"
    compiler=cc
    command -v cc > /dev/null || compiler=clang
    # The suite's make must not hand its job server or flags to this one.
    if ! env -u MAKEFLAGS -u MFLAGS -u MAKELEVEL sh -c \
        'make -C "$1" -j"$(nproc)" CC="$2" && make -C "$1" CC="$2" PREFIX="$3" install' \
        build "$work/redis-$version" "$compiler" "$work/install" > "$work/build.log" 2>&1; then
        tail -n 40 "$work/build.log" >&2
        echo "reference.sh: building Redis $version failed" >&2
        exit 1
    fi
    complete "$work/install/bin" || {
        echo "reference.sh: the build of Redis $version does not report $version" >&2
        exit 1
    }
    rm -rf "$bin"
    mv "$work/install/bin" "$bin"
    echo "reference.sh: built Redis $version into $bin" >&2
}

case ${1:-bin} in
    version) echo "$version" ;;
    sha256) echo "$sha256" ;;
    archive)
        fetch_archive
        echo "$archive" ;;
    bin)
        if ! complete "$bin"; then
            fetch_archive
            build
        fi
        echo "$bin" ;;
    *)
        sed -n '/^# usage:/,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
        exit 2 ;;
esac
