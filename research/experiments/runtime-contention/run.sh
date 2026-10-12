#!/usr/bin/env bash
# Builds the runtime-contention witnesses (README.md beside this file) with
# the compiler release whitefoot.pin names and runs each one on each CPU list,
# with as many runtime drivers as the list has CPUs.
#
#   run.sh --out DIR
#
# Settings, from the environment:
#   CONTEXTS   contexts each witness starts (default 64)
#   MILLIS     milliseconds each run lasts (default 5000)
#   PASSES     passes over every list and witness (default 1)
#   WITNESSES  witnesses to run, in order (default "base private shared read read_param write heap")
#   CPU_LISTS  taskset CPU lists (default "0,2 0,2,4,6 0,2,4,6,8,10,12,14 0-15")
#
# It writes DIR/results.csv with one row per run,
#   witness,cpus,drivers,N,ops,seconds,ops_per_s,user_frac,sys_frac,pass
# where user_frac and sys_frac are the shares of the listed CPUs' time that
# /proc/stat counts as user (user and nice) and system (system, irq and
# softirq) between just before and just after the run, and seconds is the
# witness's own elapsed time from before its first spawn to after its last
# join. Each run's standard output and its standard error, with the
# per-driver counts WF_SCHED_REPORT=2 prints at exit, are kept in DIR/runs/.
# Linux only: it needs taskset and /proc/stat.
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

CONTEXTS=${CONTEXTS:-64}
MILLIS=${MILLIS:-5000}
PASSES=${PASSES:-1}
WITNESSES=${WITNESSES:-"base private shared read read_param write heap"}
CPU_LISTS=${CPU_LISTS:-"0,2 0,2,4,6 0,2,4,6,8,10,12,14 0-15"}

mkdir -p "$out/bin" "$out/runs"

# The pinned compiler, as the Makefile's own targets use it.
make -s -C "$root" compiler
release=$(sed -n -E 's/^release = (wf-(exp-)?[0-9a-f]{12})$/\1/p' "$root/whitefoot.pin")
whitefootc=${WHITEFOOTC:-$root/build/whitefoot/$release/whitefootc}

for witness in $WITNESSES; do
    "$whitefootc" --full-lto --graph "$here/modules.wfg" --entry "w_$witness" -o "$out/bin/w_$witness"
done

# Each atomic statement whose handle is not a read-only reference parameter
# counts the object's handles up with a call of wf__shared_share at its start.
# w_read and w_read_param differ only there, so w_read's program should hold
# exactly one such call more; the counts are recorded, not enforced.
share_calls() {
    "$whitefootc" --emit-llvm --graph "$here/modules.wfg" --entry "w_$1" -o "$out/bin/w_$1.ll" ||
        { echo unavailable; return 0; }
    grep -c 'call void @wf__shared_share(' "$out/bin/w_$1.ll" || true
}
share_record="wf__shared_share calls: read=$(share_calls read) read_param=$(share_calls read_param)"

{
    echo "kernel: $(uname -srvmo)"
    echo "cpus: $(nproc)"
    echo "model: $(awk -F': ' '/^model name/ {print $2; exit}' /proc/cpuinfo)"
    echo "load: $(cat /proc/loadavg)"
    echo "firn: $(git -C "$root" rev-parse HEAD)"
    echo "whitefoot: $release"
    echo "contexts: $CONTEXTS  millis: $MILLIS  passes: $PASSES"
    echo "witnesses: $WITNESSES"
    echo "cpu lists: $CPU_LISTS"
    echo "$share_record"
} | tee "$out/host.txt"

# The CPU numbers a taskset list names, one per line: "0-3,8" gives 0 1 2 3 8.
expand_list() {
    echo "$1" | tr ',' '\n' | while IFS=- read -r first last; do
        seq "$first" "${last:-$first}"
    done
}

# The listed CPUs' user, system and total jiffies, summed, from /proc/stat.
cpu_times() {
    awk -v cpus="$(expand_list "$1" | tr '\n' ' ')" '
        BEGIN { n = split(cpus, list, " "); for (i = 1; i <= n; i++) want["cpu" list[i]] = 1 }
        $1 in want {
            user += $2 + $3; sys += $4 + $7 + $8
            for (f = 2; f <= 9; f++) total += $f
        }
        END { print user, sys, total }' /proc/stat
}

csv="$out/results.csv"
echo "witness,cpus,drivers,N,ops,seconds,ops_per_s,user_frac,sys_frac,pass" > "$csv"
failed=0
for pass in $(seq 1 "$PASSES"); do
    for list in $CPU_LISTS; do
        drivers=$(expand_list "$list" | wc -l | tr -d ' ')
        for witness in $WITNESSES; do
            name="$witness-$drivers-p$pass"
            before=$(cpu_times "$list")
            status=0
            WF_DRIVERS=$drivers WF_SCHED_REPORT=2 timeout $((MILLIS / 1000 + 120)) \
                taskset -c "$list" "$out/bin/w_$witness" "$CONTEXTS" "$MILLIS" \
                > "$out/runs/$name.out" 2> "$out/runs/$name.err" || status=$?
            after=$(cpu_times "$list")
            line=$(cat "$out/runs/$name.out")
            echo "$name: $line (status $status)"
            seen=$(echo "$line" | sed -n 's/.* drivers=\([0-9]*\) .*/\1/p')
            if [ "$status" -ne 0 ] || [ "$seen" != "$drivers" ]; then
                echo "$name: status $status, drivers $seen where $drivers were set" >&2
                failed=1
                continue
            fi
            echo "$line" | awk -v witness="$witness" -v list="$list" -v drivers="$drivers" \
                -v pass="$pass" -v before="$before" -v after="$after" '
                {
                    for (i = 1; i <= NF; i++) { split($i, kv, "="); v[kv[1]] = kv[2] }
                    split(before, b, " "); split(after, a, " ")
                    total = a[3] - b[3]
                    seconds = v["ns"] / 1e9
                    rate = v["ops_per_s"]
                    if (rate == "?") rate = sprintf("%.0f", v["ops"] / seconds)
                    user = 0; sys = 0
                    if (total > 0) { user = (a[1] - b[1]) / total; sys = (a[2] - b[2]) / total }
                    printf "%s,\"%s\",%s,%s,%s,%.6f,%s,%.4f,%.4f,%s\n", witness, list, drivers,
                        v["contexts"], v["ops"], seconds, rate, user, sys, pass
                }' >> "$csv"
        done
    done
done
cat "$csv"
exit "$failed"
