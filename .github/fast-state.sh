#!/usr/bin/env bash
# Keeps firn's build state on the 14900k per work, for fast.yml and
# fast-state.yml. Every runner of that machine runs as one user with one home,
# so a work's tree is kept in that home, not in the runner's own checkout: a
# work's next run, on whichever runner, builds again only what the work
# changed since its last run.
#
# usage: fast-state.sh enter NAME COMMIT   take NAME's tree and bring it to COMMIT
#        fast-state.sh leave               let go of the tree this job took
#        fast-state.sh sweep HOURS         remove every tree but main's not used for HOURS
#
# A tree is $HOME/fast-state/Firn-wf/NAME/src, a clone of the repository with
# its submodules, holding build/ (the compiler's download, firn's builds and
# their whitefootc caches, the reference Redis) and tests/target/ (the network
# cases' Rust build). NAME is the engine's work, `manual` for a run asked by
# hand, or `main`, which fast-state.yml keeps built at the tip of main. A
# work's first run copies main's tree, with its times, so that it builds only
# what the work changed from main.
#
# git rewrites only the files that differ between the tree's last commit and
# COMMIT, so their times say to make and cargo what to build again, as in a
# developer's own tree.
#
# One job at a time holds a tree: `enter` takes an exclusive flock on
# NAME.lock beside it and keeps it, through a process left running, until
# `leave` or the end of the job, whose orphans the runner stops. A copy from
# main's tree takes main.lock shared; fast-state.yml takes it exclusive while
# it builds. `enter` prints the variables of the job: FIRN_SRC, the tree, and
# REDIS_COMPAT_CACHE.
set -euo pipefail

root=${FAST_STATE_ROOT:-$HOME/fast-state/Firn-wf}
repo=${FAST_STATE_REPO:?set FAST_STATE_REPO to the URL of the repository}
# How long a job waits for another run of the same work, in seconds.
wait_seconds=${FAST_STATE_WAIT:-900}
held=${RUNNER_TEMP:?}/fast-state-lock

say() { echo "fast-state: $*" >&2; }

take() {
    local lock=$1
    rm -f "$held.ok" "$held.pid"
    # flock keeps the lock while its command runs; -o keeps the descriptor
    # from the command, so stopping the command lets the lock go.
    nohup flock -x -o -w "$wait_seconds" "$lock" sh -c ': > "$1"; exec sleep 86400' sh "$held.ok" \
        < /dev/null > /dev/null 2>&1 &
    echo $! > "$held.pid"
    while [ ! -e "$held.ok" ]; do
        if ! kill -0 "$(cat "$held.pid")" 2> /dev/null; then
            say "another run held $lock for more than $wait_seconds s"
            exit 1
        fi
        sleep 0.2
    done
}

enter() {
    local name=$1 commit=$2 dir
    case $name in
        '' | *[!A-Za-z0-9_-]*) say "a tree's name is letters, digits, - or _, not '$name'"; exit 2 ;;
    esac
    dir=$root/$name
    mkdir -p "$root"
    take "$dir.lock"
    if [ ! -d "$dir/src/.git" ]; then
        rm -rf "$dir"
        mkdir -p "$dir"
        if [ "$name" != main ] && [ -d "$root/main/src/.git" ]; then
            say "copying main's tree into $name's"
            flock -s -w "$wait_seconds" "$root/main.lock" cp -a --reflink=auto "$root/main/." "$dir/"
        else
            say "starting $name's tree anew"
            git init -q "$dir/src"
        fi
    fi
    touch "$dir/used"
    cd "$dir/src"
    if git remote get-url origin > /dev/null 2>&1; then
        git remote set-url origin "$repo"
    else
        git remote add origin "$repo"
    fi
    git fetch -q --no-tags --prune origin +refs/heads/main:refs/remotes/origin/main
    git fetch -q --no-tags origin "$commit"
    git checkout -q --force --detach FETCH_HEAD
    git submodule sync -q --recursive
    git submodule update -q --init --recursive --force
    # What the builds make stays; whatever else the last run left goes.
    git clean -ffdxq -e /build -e /tests/target
    git submodule foreach --quiet --recursive git clean -ffdxq
    say "$name's tree is at $(git log --oneline -1)"
    echo "FIRN_SRC=$dir/src"
    echo "REDIS_COMPAT_CACHE=$dir/src/build/redis-compat-cache"
}

leave() {
    if [ -e "$held.pid" ]; then
        pid=$(cat "$held.pid")
        pkill -TERM -P "$pid" 2> /dev/null || true
        kill -TERM "$pid" 2> /dev/null || true
        rm -f "$held.pid" "$held.ok"
    fi
}

sweep() {
    local hours=$1 dir name
    [[ $hours =~ ^[1-9][0-9]*$ ]] || { say "HOURS is a whole number, not '$hours'"; exit 2; }
    [ -d "$root" ] || return 0
    for dir in "$root"/*/; do
        dir=${dir%/}
        name=${dir##*/}
        [ "$name" = main ] && continue
        if [ -n "$(find "$dir/used" -maxdepth 0 -mmin "-$((hours * 60))" 2> /dev/null)" ]; then
            continue
        fi
        # A tree a run holds is left for the next sweep.
        if flock -n "$dir.lock" rm -rf "$dir"; then
            say "removed $name's tree, unused for $hours hours"
        else
            say "$name's tree is in use; it stays"
        fi
    done
}

case ${1:-} in
    enter) [ $# -eq 3 ] || { sed -n 's/^# usage: /usage: /p; s/^#        /       /p' "$0" | head -3 >&2; exit 2; }
           enter "$2" "$3" ;;
    leave) leave ;;
    sweep) [ $# -eq 2 ] || exit 2; sweep "$2" ;;
    *) sed -n 's/^# usage: /usage: /p; s/^#        /       /p' "$0" | head -3 >&2; exit 2 ;;
esac
