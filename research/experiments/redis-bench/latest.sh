#!/usr/bin/env bash
# The newest stable releases of the two competitors whose installed copy on the
# self-hosted machine can be older than the release (redis-bench.sh, lines
# redis-latest and garnet): Redis, built from the official release archive as
# tests/redis-suite/reference.sh builds 7.2.16, and Garnet, from the release's
# prebuilt x86-64 Linux archive. Valkey and Dragonfly are the host's own and
# their versions stand in each run's host.txt. A newer release is adopted by
# moving the pins below, with the archive's SHA-256.
#
# usage: latest.sh [redis | garnet | versions]
#
#   redis     prints the path of the redis-server of the release, building it
#             from the archive when the cache lacks it
#   garnet    prints the path of the GarnetServer of the release, extracted
#             from the archive when the cache lacks it
#   versions  prints the two versions the pins name
#
# Everything else goes to standard error. REDIS_BENCH_LATEST_CACHE names the
# directory that keeps the archives and the builds,
# ${XDG_CACHE_HOME:-$HOME/.cache}/firn-redis-bench-latest by default. A build
# is made beside its place and moved there whole, and is used only when it
# reports the pinned version.
set -euo pipefail

redis_version=8.10.2
redis_sha256=b9ffee226b5eecdba98a679260dad764b2a4ebd90dce4ad5ac9e9f3eef9c02b3
redis_url=https://download.redis.io/releases/redis-$redis_version.tar.gz
garnet_version=2.2.3
garnet_sha256=fda5b097b498edcf3a9d50be84c6da4276dd78219fd48153835f0db51bc80fdd
garnet_url=https://github.com/microsoft/garnet/releases/download/v$garnet_version/linux-x64-based.tar.xz

cache=${REDIS_BENCH_LATEST_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/firn-redis-bench-latest}
redis_archive=$cache/redis-$redis_version.tar.gz
redis_bin=$cache/redis-$redis_version-bin
garnet_archive=$cache/garnet-$garnet_version-linux-x64-based.tar.xz
garnet_dir=$cache/garnet-$garnet_version

# fetch URL ARCHIVE SHA256: the archive, fetched when missing and checked on
# every call.
fetch() {
    mkdir -p "$cache"
    if [ ! -f "$2" ]; then
        # Callers started together may all find the archive missing; each
        # fetches into a name of its own and the last rename wins.
        local part
        part=$(mktemp "$2.part.XXXXXX")
        curl -fsSL -o "$part" "$1"
        mv -f "$part" "$2"
    fi
    if ! echo "$3  $2" | sha256sum --check --status; then
        echo "latest.sh: $2 does not have the SHA-256 $3" >&2
        exit 1
    fi
}

redis_reports() {
    [ "$("$1" --version 2> /dev/null | sed -n 's/^Redis server v=\([^ ]*\).*/\1/p')" = "$redis_version" ]
}

garnet_reports() {
    [ "$("$1" --version 2> /dev/null | sed -n 's/^GarnetServer \([0-9.]*\).*/\1/p')" = "$garnet_version" ]
}

# The build directory is global: the exit trap removes it after this returns.
work=
build_redis() {
    local compiler
    work=$(mktemp -d "$cache/build.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    tar -xzf "$redis_archive" -C "$work"
    compiler=cc
    command -v cc > /dev/null || compiler=clang
    # The caller's make must not hand its job server or flags to this one.
    if ! env -u MAKEFLAGS -u MFLAGS -u MAKELEVEL sh -c \
        'make -C "$1" -j"$(nproc)" CC="$2" redis-server' \
        build "$work/redis-$redis_version" "$compiler" > "$work/build.log" 2>&1; then
        tail -n 40 "$work/build.log" >&2
        echo "latest.sh: building Redis $redis_version failed" >&2
        exit 1
    fi
    redis_reports "$work/redis-$redis_version/src/redis-server" || {
        echo "latest.sh: the build of Redis $redis_version does not report $redis_version" >&2
        exit 1
    }
    mkdir -p "$work/install"
    cp "$work/redis-$redis_version/src/redis-server" "$work/install/redis-server"
    rm -rf "$redis_bin"
    mv "$work/install" "$redis_bin"
    echo "latest.sh: built Redis $redis_version into $redis_bin" >&2
}

extract_garnet() {
    work=$(mktemp -d "$cache/extract.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    tar -xJf "$garnet_archive" -C "$work"
    garnet_reports "$work/net10.0/GarnetServer" || {
        echo "latest.sh: the Garnet archive does not report $garnet_version" >&2
        exit 1
    }
    rm -rf "$garnet_dir"
    mv "$work/net10.0" "$garnet_dir"
    echo "latest.sh: extracted Garnet $garnet_version into $garnet_dir" >&2
}

case ${1:-} in
    versions)
        echo "redis $redis_version"
        echo "garnet $garnet_version" ;;
    redis)
        if ! redis_reports "$redis_bin/redis-server"; then
            fetch "$redis_url" "$redis_archive" "$redis_sha256"
            build_redis
        fi
        echo "$redis_bin/redis-server" ;;
    garnet)
        if ! garnet_reports "$garnet_dir/GarnetServer"; then
            fetch "$garnet_url" "$garnet_archive" "$garnet_sha256"
            extract_garnet
        fi
        echo "$garnet_dir/GarnetServer" ;;
    *)
        sed -n '/^# usage:/,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
        exit 2 ;;
esac
