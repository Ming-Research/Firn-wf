#!/usr/bin/env bash
# Runs the Redis suite as N run.sh shards side by side, then merges their runs
# into DIR for ratchet.py check. The gate runs it through make redis-suite.
#
# usage: run-shards.sh --shards N --out DIR [run.sh options] firn FIRN_EXECUTABLE
#
# Shard I of N runs in DIR/shard-I, whose files keep run.sh's layout; DIR
# receives the merged tests.tsv, units.tsv, attempts.tsv, meta.txt and
# suite-units.txt. The merge fails unless the shards together ran every unit
# of the suite exactly once.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
shards= out=
rest=()
while [ $# -gt 0 ]; do
    case $1 in
        --shards) shards=${2:?}; shift 2 ;;
        --out) out=${2:?}; shift 2 ;;
        *) rest+=("$1"); shift ;;
    esac
done
if ! [[ $shards =~ ^[1-9][0-9]*$ ]] || [ -z "$out" ] || [ ${#rest[@]} -eq 0 ]; then
    sed -n '/^# usage:/p' "$0" | sed 's/^# //' >&2
    exit 2
fi

mkdir -p "$out"
rm -rf "$out"/shard-*
dirs=() pids=()
for ((i = 1; i <= shards; i++)); do
    dirs+=("$out/shard-$i")
    # The subshell reports run.sh's status, not the prefixing sed's.
    (
        set -o pipefail
        "$here/run.sh" --shard "$i/$shards" --out "$out/shard-$i" "${rest[@]}" 2>&1 \
            | sed -u "s|^|[shard $i/$shards] |"
    ) &
    pids+=($!)
done
status=0
for pid in "${pids[@]}"; do
    wait "$pid" || status=1
done
if [ "$status" != 0 ]; then
    echo "run-shards.sh: a shard's run.sh failed" >&2
    exit 1
fi
python3 -B "$here/ratchet.py" merge "${dirs[@]}" --out "$out"
