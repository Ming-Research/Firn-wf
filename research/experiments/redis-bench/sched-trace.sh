#!/usr/bin/env bash
# Temporary; called only by sched-trace.yml; remove both and sched-trace.py
# before any PR. Native perf summaries do not provide sleep-to-wake percentiles,
# so the Python companion reconstructs them from the retained tracepoints.
# Question: does runnable delay explain the limiter-script tail at 8 connections?
# Compare 8 and 1 connections, then 8 with each busy driver pinned to its own
# CPU, same binary, 10 s each, tracing ~2-7 s. Test whether >1 ms wake delays
# land on the waker CPU while the other driver runs there; independent pinning
# should reduce those delays if wake-affine placement explains the tail.
# Reject that explanation if 8 connections reproduce the tail while runnable
# delays stay short. This is diagnostic, not a performance attribution: tracing
# perturbs all runs and a single comparison cannot establish a regression or cause.
# redis-bench.sh cannot be sourced: lines 109-118 build at top level and its
# final block runs benchmarks. No mode exposes only startup or scheduler tracing.
set -euo pipefail
export LC_ALL=C
ROOT=$(pwd)
: "${OUT:?}" "${FIRN_REDIS_BENCH_RECORD:?}"
session_py="$ROOT/research/experiments/redis-bench/session.py"
summary_py="$ROOT/research/experiments/redis-bench/sched-trace.py"
client="$OUT/workload-target/release/firn-workload"

# redis-bench.yml's profile discovery, then PATH for distributions with no
# versioned linux-tools directory. Invoke perf directly, as on its 14900K path.
PERF=$(ls /usr/lib/linux-tools/*/perf /usr/lib/linux-tools-*/perf 2>/dev/null | head -1 || true)
PERF=${PERF:-$(command -v perf || true)}
if [[ -z "$PERF" ]]; then echo 'No perf available; scheduler trace cannot run.'; exit 1; fi
"$PERF" version

# Mirror redis-bench.sh:99-102, 704-706 and 197-199. On the 14900K:
# server CPUs 0,1; client CPUs 2..17; client caps its workers to connections.
total=$(nproc)
[[ "$total" -gt 2 ]]
SERVER_CPUS=0,1
CLIENT_THREADS=$((total - 2 < 16 ? total - 2 : 16))
CLIENT_CPUS=$(seq -s, 2 $((2 + CLIENT_THREADS - 1)))
PORT=$((10000 + $$ % 400 * 50))
echo "server CPUs=$SERVER_CPUS WF_DRIVERS=2; client CPUs=$CLIENT_CPUS --threads=$CLIENT_THREADS"
echo 'Trace all CPUs: remote sched_wakeup events execute on the waker CPU.'
echo 'Metrics use only TIDs enumerated under /proc/<firn-pid>/task.'
echo 'WF_DRIVERS requests two drivers; generic comm names do not prove individual thread roles.'
server= client_pid=
trap '[[ -z "$client_pid" ]] || kill "$client_pid" 2>/dev/null || true; [[ -z "$server" ]] || kill "$server" 2>/dev/null || true' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# One short CI probe selects the recorder before the measured workloads.
# No sudo/sysctl fallback: record the errors and fail if both paths are barred.
record() {
    local path=$1 duration=$2
    if [[ "$recorder" == sched ]]; then
        # sched record usually selects sched_waking (wake initiation). Also
        # retain sched_wakeup so our percentiles start at wake completion.
        "$PERF" sched record -a -e sched:sched_wakeup -o "$path" -- sleep "$duration"
    else
        "$PERF" record -a -R -c 1 -m 1024 -e sched:sched_switch -e sched:sched_wakeup \
            -e sched:sched_wakeup_new -o "$path" -- sleep "$duration"
    fi
}
recorder=sched
if ! record "$OUT/probe-sched.data" 0.1 > "$OUT/probe-sched.txt" 2>&1; then
    cat "$OUT/probe-sched.txt"
    recorder=tracepoints
    if ! record "$OUT/probe-tracepoints.data" 0.1 > "$OUT/probe-tracepoints.txt" 2>&1; then
        cat "$OUT/probe-tracepoints.txt"
        echo 'Scheduler trace unavailable with existing host permissions; no settings changed.'
        exit 1
    fi
fi
echo "recorder=$recorder"

snapshot() {
    echo "== /proc/$server/task ($1)"
    ls "/proc/$server/task"
    for task in /proc/"$server"/task/*; do
        tid=${task##*/}
        comm=$(cat "$task/comm")
        printf '%s\t%s\n' "$tid" "$comm" >> "$OUT/threads-$run.tsv"
        printf 'TID=%s comm=%s ' "$tid" "$comm"
        awk '/^Cpus_allowed_list:/ {print}' "$task/status"
    done
}

failed=0
for run in 8 1 8-pinned; do
    conns=${run%%-*}
    echo "== run=$run connections=$conns"
    PORT=$((PORT + 1))
    # No AOF: positional arguments PORT, request limit 0, AOF '-', idle 0.
    (python3 "$session_py" register "$FIRN_REDIS_BENCH_RECORD" parent
     WF_DRIVERS=2 exec taskset -c "$SERVER_CPUS" "$ROOT/build/firn-lto" "$PORT" 0 - 0) \
        > "$OUT/server-$run.txt" 2>&1 &
    server=$!
    # Mirror redis-bench.sh:211-220: wait for PONG, bounded to 20 seconds.
    tries=0
    until redis-cli -p "$PORT" PING 2>/dev/null | grep -q PONG; do
        kill -0 "$server"
        tries=$((tries + 1))
        if [[ "$tries" -gt 400 ]]; then echo 'firn never answered'; exit 1; fi
        sleep 0.05
    done
    : > "$OUT/threads-$run.tsv"
    snapshot before > "$OUT/threads-$run.txt"
    # Mirror redis-bench.sh:722-724. Connection::new (main.rs:238-255)
    # SCRIPT LOADs the unchanged limiter source before its measurement clock.
    taskset -c "$CLIENT_CPUS" "$client" --port "$PORT" \
        --threads "$CLIENT_THREADS" --connections "$conns" \
        --workload limiter-script --seconds 10 > "$OUT/client-$run.csv" \
        2> "$OUT/client-$run.txt" &
    client_pid=$!
    sleep 1
    kill -0 "$client_pid"
    # Rank by /proc stat utime+stime after one second of load, not comm names.
    python3 "$summary_py" --drivers "$server" > "$OUT/drivers-$run.tsv"
    echo 'Busiest two firn TIDs after 1 s (TID, comm, CPU ticks):'
    cat "$OUT/drivers-$run.tsv"
    if [[ "$run" == 8-pinned ]]; then
        cpu=0
        while IFS=$'\t' read -r tid comm ticks; do
            echo "Pinning firn TID=$tid comm=$comm CPU_ticks=$ticks to server CPU=$cpu"
            taskset -pc "$cpu" "$tid"
            cpu=$((cpu + 1))
        done < "$OUT/drivers-$run.tsv" > "$OUT/pinning-$run.txt" 2>&1
        cat "$OUT/pinning-$run.txt"
    fi
    sleep 1
    kill -0 "$client_pid"
    snapshot trace-start >> "$OUT/threads-$run.txt"
    record "$OUT/perf-$run.data" 5 > "$OUT/record-$run.txt" 2>&1 || failed=1
    # Still running after capture: rejects traces that fall after the load.
    kill -0 "$client_pid" || failed=1
    snapshot trace-end >> "$OUT/threads-$run.txt"
    wait "$client_pid" || failed=1
    client_pid=
    snapshot after >> "$OUT/threads-$run.txt"
    # Save the TGID for timehist's process filter before clearing server.
    firn_pid=$server
    kill "$server"
    wait "$server" || true
    server=
    cat "$OUT/threads-$run.txt" "$OUT/record-$run.txt" "$OUT/client-$run.txt"
    echo 'workload,connections,requests,seconds,rate,p50_ms,p99_ms'
    cat "$OUT/client-$run.csv"
    tids=$(cut -f1 "$OUT/threads-$run.tsv" | sort -nu | paste -sd, -)
    # Native per-thread summary; individual events remain in events-*.txt.
    if ! "$PERF" sched timehist -i "$OUT/perf-$run.data" -s -p "$firn_pid" --tid "$tids" \
        > "$OUT/timehist-$run.txt" 2>&1; then
        echo 'Native timehist unavailable; raw tracepoint percentiles follow.'
    fi
    if ! "$PERF" sched timehist -i "$OUT/perf-$run.data" -w -M -p "$firn_pid" --tid "$tids" \
        >> "$OUT/timehist-$run.txt" 2>&1; then
        echo 'Native timehist wakeup/migration detail unavailable; raw tracepoints follow.'
    fi
    # latency -p prevents merging generic thread names; filter by exact TID.
    if "$PERF" sched latency -i "$OUT/perf-$run.data" --sort max -p \
        > "$OUT/latency-all-$run.txt" 2>&1; then
        python3 "$summary_py" --latency "$OUT/threads-$run.tsv" \
            "$OUT/latency-all-$run.txt" > "$OUT/latency-$run.txt" || failed=1
    else
        cp "$OUT/latency-all-$run.txt" "$OUT/latency-$run.txt"
        echo 'Native latency unavailable; raw tracepoint percentiles follow.'
    fi
    # Keep diagnostics (including lost events) and raw data, never hide errors.
    # The header TID/CPU identify the wake event's context (waker), while the
    # payload target_cpu / CPU identifies the CPU the awakened task is queued on.
    if "$PERF" script -i "$OUT/perf-$run.data" --ns -F trace:comm,tid,cpu,time,event,trace \
        > "$OUT/events-$run.txt" 2> "$OUT/decode-$run.txt"; then
        python3 "$summary_py" "$OUT/threads-$run.tsv" "$OUT/events-$run.txt" \
            "$OUT/drivers-$run.tsv" > "$OUT/percentiles-$run.txt" 2>&1 || failed=1
    else
        failed=1
    fi
    cat "$OUT/timehist-$run.txt" "$OUT/latency-$run.txt" "$OUT/decode-$run.txt"
    [[ ! -f "$OUT/percentiles-$run.txt" ]] || cat "$OUT/percentiles-$run.txt"
    # perf warns about dropped events on stderr; such a trace is not evidence.
    if grep -Ei 'lost [1-9][0-9]*|[1-9][0-9]* (lost|chunks lost)|lost[^0-9]*[1-9][0-9]*' \
        "$OUT/record-$run.txt" "$OUT/decode-$run.txt" \
        "$OUT/timehist-$run.txt" "$OUT/latency-all-$run.txt"; then failed=1; fi
done
exit "$failed"
