#!/usr/bin/env bash
# Builds the ring-assist witness (README.md beside this file) with the
# compiler release whitefoot.pin names and runs its client driver.
#
#   run.sh --out DIR
#
# RUNS, STEPS, TARGET_MS and LIMIT_MS in the environment reach driver.py,
# which documents them. It writes DIR/host.txt, DIR/results.csv (one row per
# run, written as each run ends, so a failing run still leaves its rows) and
# each run's standard output and error, with the per-driver counts
# WF_SCHED_REPORT=2 prints at exit, in DIR/runs/. Its status is driver.py's:
# 1 when an echo came late, 2 when a run broke.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)

out=
while [ $# -gt 0 ]; do
    case $1 in
        --out) out=$2; shift 2 ;;
        *) echo "usage: run.sh --out DIR" >&2; exit 2 ;;
    esac
done
[ -n "$out" ] || { echo "usage: run.sh --out DIR" >&2; exit 2; }

mkdir -p "$out/bin" "$out/runs"

# The pinned compiler, as the Makefile's own targets use it.
make -s -C "$root" compiler
release=$(sed -n -E 's/^release = (wf-(exp-)?[0-9a-f]{12})$/\1/p' "$root/whitefoot.pin")
whitefootc=${WHITEFOOTC:-$root/build/whitefoot/$release/whitefootc}

"$whitefootc" --full-lto --graph "$here/modules.wfg" --entry ring_assist -o "$out/bin/ring_assist"

{
    echo "kernel: $(uname -srvmo)"
    echo "cpus: $(nproc)"
    echo "model: $(awk -F': ' '/^model name/ {print $2; exit}' /proc/cpuinfo)"
    echo "load: $(cat /proc/loadavg)"
    echo "firn: $(git -C "$root" rev-parse HEAD)"
    echo "whitefoot: $release"
    echo "runs: ${RUNS:-10}  steps: ${STEPS:-calibrated}  target_ms: ${TARGET_MS:-2000}  limit_ms: ${LIMIT_MS:-100}"
} | tee "$out/host.txt"

status=0
python3 "$here/driver.py" "$out/bin/ring_assist" "$out" 2> "$out/driver.err" || status=$?
cat "$out/driver.err" >&2
exit "$status"
