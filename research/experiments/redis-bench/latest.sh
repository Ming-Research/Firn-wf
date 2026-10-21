#!/usr/bin/env bash
# The newest stable releases of the four competitors of redis-bench.sh (lines
# redis-latest, valkey-latest, valkey-io-latest, dragonfly-latest-N and
# garnet-latest-N), whatever the measuring host has installed and without
# installing anything on it: Redis and Valkey are built from their release
# archives as tests/redis-suite/reference.sh builds 7.2.16, Dragonfly and
# Garnet are extracted from the release's prebuilt x86-64 Linux archive. Garnet
# is a framework-dependent .NET 10 program and the measuring host has no .NET,
# so Microsoft's runtime archive is kept beside it and found through
# DOTNET_ROOT by a wrapper script, garnet-server, that latest.sh writes next to
# GarnetServer; nothing is installed system-wide. A newer release is adopted by
# moving the pins below, with the archive's SHA-256 (Redis: the redis-hashes
# table; Valkey: the valkey-hashes table; Dragonfly and Garnet: the digest
# GitHub shows for the release asset; the .NET runtime: its SHA-256 computed
# when the SHA-512 in Microsoft's releases.json for it was checked).
#
# usage: latest.sh [redis | valkey | dragonfly | garnet | versions]
#
#   redis      prints the path of the redis-server of the release, building it
#              from the archive when the cache lacks it
#   valkey     prints the path of the valkey-server of the release, built so
#   dragonfly  prints the path of the dragonfly executable of the release,
#              extracted from the archive when the cache lacks it
#   garnet     prints the path of the garnet-server wrapper of the release,
#              extracted so, with the .NET runtime it needs
#   versions   prints the four versions the pins name
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
valkey_version=9.1.2
valkey_sha256=19c23908e7d57e8d91ef85b41f5646307582f10f4f0fb999bbf89ed24ec9c983
valkey_url=https://github.com/valkey-io/valkey/archive/refs/tags/$valkey_version.tar.gz
dragonfly_version=2.0.2
dragonfly_sha256=eb9dfe654c69ab190cb3aa329ed3599ebe19c5e9a3a437bb7bc38ef52f665c88
dragonfly_url=https://github.com/dragonflydb/dragonfly/releases/download/v$dragonfly_version/dragonfly-x86_64.tar.gz
garnet_version=2.2.3
garnet_sha256=fda5b097b498edcf3a9d50be84c6da4276dd78219fd48153835f0db51bc80fdd
garnet_url=https://github.com/microsoft/garnet/releases/download/v$garnet_version/linux-x64-based.tar.xz
dotnet_version=10.0.13
dotnet_sha256=958c952691d2c3f8ce21f717f4e911c02b5da0e21ead21bd576368c454b79c77
dotnet_url=https://builds.dotnet.microsoft.com/dotnet/Runtime/$dotnet_version/dotnet-runtime-$dotnet_version-linux-x64.tar.gz

cache=${REDIS_BENCH_LATEST_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/firn-redis-bench-latest}
redis_archive=$cache/redis-$redis_version.tar.gz
redis_bin=$cache/redis-$redis_version-bin
valkey_archive=$cache/valkey-$valkey_version.tar.gz
valkey_bin=$cache/valkey-$valkey_version-bin
dragonfly_archive=$cache/dragonfly-$dragonfly_version-x86_64.tar.gz
dragonfly_dir=$cache/dragonfly-$dragonfly_version
garnet_archive=$cache/garnet-$garnet_version-linux-x64-based.tar.xz
garnet_dir=$cache/garnet-$garnet_version
dotnet_archive=$cache/dotnet-runtime-$dotnet_version-linux-x64.tar.gz
dotnet_dir=$cache/dotnet-$dotnet_version

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

# reports EXECUTABLE VERSION KIND: whether the executable reports the version.
reports() {
    local esc found
    esc=$(printf '\033')
    case $3 in
        redis) found=$("$1" --version 2> /dev/null | sed -n 's/^Redis server v=\([^ ]*\).*/\1/p') ;;
        valkey) found=$("$1" --version 2> /dev/null | sed -n 's/^Valkey server v=\([^ ]*\).*/\1/p') ;;
        dragonfly) found=$("$1" --version 2> /dev/null | sed -n "s/${esc}\[[0-9;]*m//g; s/^dragonfly v\([0-9][0-9.]*\).*/\1/p") ;;
        garnet) found=$("$1" --version 2> /dev/null | sed -n 's/^GarnetServer \([0-9.]*\).*/\1/p') ;;
    esac
    [ "$found" = "$2" ]
}

# The work directory is global: the exit trap removes it after the function.
work=

# build_server NAME VERSION ARCHIVE BIN: builds NAME-server from the source
# archive, as reference.sh builds Redis.
build_server() {
    local name=$1 version=$2 archive=$3 bin=$4 compiler src
    work=$(mktemp -d "$cache/build.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    tar -xzf "$archive" -C "$work"
    src=$(find "$work" -mindepth 1 -maxdepth 1 -type d)
    compiler=cc
    command -v cc > /dev/null || compiler=clang
    # The caller's make must not hand its job server or flags to this one.
    if ! env -u MAKEFLAGS -u MFLAGS -u MAKELEVEL sh -c \
        'make -C "$1" -j"$(nproc)" CC="$2" "$3"-server' \
        build "$src" "$compiler" "$name" > "$work/build.log" 2>&1; then
        tail -n 40 "$work/build.log" >&2
        echo "latest.sh: building $name $version failed" >&2
        exit 1
    fi
    reports "$src/src/$name-server" "$version" "$name" || {
        echo "latest.sh: the build of $name $version does not report $version" >&2
        exit 1
    }
    mkdir -p "$work/install"
    cp "$src/src/$name-server" "$work/install/$name-server"
    rm -rf "$bin"
    mv "$work/install" "$bin"
    rm -rf "$work"
    echo "latest.sh: built $name $version into $bin" >&2
}

# extract_binary NAME VERSION ARCHIVE DIR SUBDIR EXECUTABLE: extracts the
# prebuilt archive and keeps its SUBDIR (the top level when empty) as DIR, only
# when the EXECUTABLE in it reports the version.
extract_binary() {
    local name=$1 version=$2 archive=$3 dir=$4 inner=$5 exe=$6 root
    work=$(mktemp -d "$cache/extract.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    mkdir "$work/x"
    case $archive in
        *.tar.xz) tar -xJf "$archive" -C "$work/x" ;;
        *) tar -xzf "$archive" -C "$work/x" ;;
    esac
    root=$work/x${inner:+/$inner}
    reports "$root/$exe" "$version" "$name" || {
        echo "latest.sh: the $name archive does not report $version" >&2
        exit 1
    }
    rm -rf "$dir"
    mv "$root" "$dir"
    rm -rf "$work"
    echo "latest.sh: extracted $name $version into $dir" >&2
}

# extract_dotnet: Microsoft's .NET runtime, kept only when it lists the pinned
# version of Microsoft.NETCore.App.
extract_dotnet() {
    work=$(mktemp -d "$cache/extract.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    mkdir "$work/x"
    tar -xzf "$dotnet_archive" -C "$work/x"
    "$work/x/dotnet" --list-runtimes 2> /dev/null | grep -q "^Microsoft.NETCore.App $dotnet_version " || {
        echo "latest.sh: the .NET archive does not list runtime $dotnet_version" >&2
        exit 1
    }
    rm -rf "$dotnet_dir"
    mv "$work/x" "$dotnet_dir"
    rm -rf "$work"
    echo "latest.sh: extracted .NET runtime $dotnet_version into $dotnet_dir" >&2
}

# extract_garnet: the release, and beside it the wrapper that runs GarnetServer
# with DOTNET_ROOT at the runtime; kept only when the wrapper reports the
# version, and otherwise showing what GarnetServer said.
extract_garnet() {
    work=$(mktemp -d "$cache/extract.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    mkdir "$work/x"
    tar -xJf "$garnet_archive" -C "$work/x"
    rm -rf "$garnet_dir"
    mv "$work/x/net10.0" "$garnet_dir"
    rm -rf "$work"
    printf '#!/bin/sh\nDOTNET_ROOT=%s exec %s "$@"\n' "'$dotnet_dir'" "'$garnet_dir/GarnetServer'" > "$garnet_dir/garnet-server"
    chmod +x "$garnet_dir/garnet-server"
    reports "$garnet_dir/garnet-server" "$garnet_version" garnet || {
        echo "latest.sh: Garnet $garnet_version does not report its version; it says:" >&2
        "$garnet_dir/garnet-server" --version 2>&1 | head -8 >&2 || true
        rm -rf "$garnet_dir"
        exit 1
    }
    echo "latest.sh: extracted Garnet $garnet_version into $garnet_dir" >&2
}

case ${1:-} in
    versions)
        echo "redis $redis_version"
        echo "valkey $valkey_version"
        echo "dragonfly $dragonfly_version"
        echo "garnet $garnet_version" ;;
    redis)
        if ! reports "$redis_bin/redis-server" "$redis_version" redis; then
            fetch "$redis_url" "$redis_archive" "$redis_sha256"
            build_server redis "$redis_version" "$redis_archive" "$redis_bin"
        fi
        echo "$redis_bin/redis-server" ;;
    valkey)
        if ! reports "$valkey_bin/valkey-server" "$valkey_version" valkey; then
            fetch "$valkey_url" "$valkey_archive" "$valkey_sha256"
            build_server valkey "$valkey_version" "$valkey_archive" "$valkey_bin"
        fi
        echo "$valkey_bin/valkey-server" ;;
    dragonfly)
        if ! reports "$dragonfly_dir/dragonfly-x86_64" "$dragonfly_version" dragonfly; then
            fetch "$dragonfly_url" "$dragonfly_archive" "$dragonfly_sha256"
            extract_binary dragonfly "$dragonfly_version" "$dragonfly_archive" "$dragonfly_dir" "" dragonfly-x86_64
        fi
        echo "$dragonfly_dir/dragonfly-x86_64" ;;
    garnet)
        if ! reports "$garnet_dir/garnet-server" "$garnet_version" garnet; then
            fetch "$dotnet_url" "$dotnet_archive" "$dotnet_sha256"
            [ -x "$dotnet_dir/dotnet" ] || extract_dotnet
            fetch "$garnet_url" "$garnet_archive" "$garnet_sha256"
            extract_garnet
        fi
        echo "$garnet_dir/garnet-server" ;;
    *)
        sed -n '/^# usage:/,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
        exit 2 ;;
esac
