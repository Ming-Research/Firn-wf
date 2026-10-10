#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
release=$(sed -n 's/^release = //p' "$ROOT/whitefoot.pin")
[ -n "$release" ] || { echo 'whitefoot.pin has no release' >&2; exit 1; }
WFC=${WFC:-"$ROOT/build/whitefoot/$release/whitefootc"}
OUT=${OUT:-"$ROOT/build/guard-fairness"}
K=${K:-1000}
PASSES=${PASSES:-2}
CPUS=${CPUS:-2,4}
case $K in ''|*[!0-9]*) echo 'K must be an integer in 1..20000' >&2; exit 1 ;; esac
[ "$K" -ge 1 ] && [ "$K" -le 20000 ] || { echo 'K must be in 1..20000' >&2; exit 1; }
case $PASSES in ''|*[!0-9]*) echo 'PASSES must be a positive integer' >&2; exit 1 ;; esac
[ "$PASSES" -ge 1 ] || { echo 'PASSES must be positive' >&2; exit 1; }
[ -x "$WFC" ] || { echo "missing pinned compiler: $WFC; run make compiler in CI" >&2; exit 1; }
mkdir -p "$OUT"
"$WFC" --full-lto "$ROOT/research/experiments/guard-fairness/probe.wf" -o "$OUT/probe"
printf 'revision,%s,release,%s\n' "$(git -C "$ROOT" rev-parse HEAD)" "$release"
printf 'compiler,%s\n' "$WFC"
uname -srvmo
printf 'pilot,N=2,K=100,WF_DRIVERS=2,cpus=%s\n' "$CPUS"
WF_DRIVERS=2 taskset -c "$CPUS" "$OUT/probe" 2 100
pass=1
while [ "$pass" -le "$PASSES" ]; do
    order='2 8 50'
    if [ $((pass % 2)) -eq 0 ]; then order='50 8 2'; fi
    for n in $order; do
        printf 'pass,%s,N,%s,K,%s,WF_DRIVERS=2,cpus=%s\n' "$pass" "$n" "$K" "$CPUS"
        WF_DRIVERS=2 taskset -c "$CPUS" "$OUT/probe" "$n" "$K"
    done
    pass=$((pass + 1))
done
