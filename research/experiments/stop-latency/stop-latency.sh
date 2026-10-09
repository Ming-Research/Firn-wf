#!/bin/bash
# Experiment only (branch exp/cancel-firn, never merged): the time from SIGTERM
# to firn's exit, with 50 connections busy or idle, for each image given.
# Usage: stop-latency.sh OUT name=binary ...
set -u
OUT=$1
shift
PORT=7511
TRIALS=${TRIALS:-10}
echo "image,load,trial,stop_ms" >"$OUT"
cleanup() { pkill -f "redis-benchmark -p $PORT" 2>/dev/null; pkill -f "idle-clients.py $PORT" 2>/dev/null; [ -n "${pid:-}" ] && kill -KILL "$pid" 2>/dev/null; }
trap cleanup EXIT
cat > /tmp/idle-clients.py <<'PY'
import socket, sys, time
port = int(sys.argv[1]); held = []
for _ in range(50):
    s = socket.create_connection(("127.0.0.1", port)); held.append(s)
time.sleep(3600)
PY
for spec in "$@"; do
    name=${spec%%=*}; binary=${spec#*=}
    for load in busy idle; do
        for trial in $(seq 1 "$TRIALS"); do
            taskset -c 2 "$binary" --port "$PORT" >/dev/null 2>&1 &
            pid=$!
            until redis-cli -p "$PORT" PING >/dev/null 2>&1; do sleep 0.05; done
            if [ "$load" = busy ]; then
                taskset -c 8-11 redis-benchmark -p "$PORT" -c 50 -n 100000000 -t set -P 1 -q >/dev/null 2>&1 &
            else
                taskset -c 8-11 python3 /tmp/idle-clients.py "$PORT" &
            fi
            load_pid=$!
            sleep 2
            start=$(date +%s%N)
            kill -TERM "$pid"
            wait "$pid" 2>/dev/null
            end=$(date +%s%N)
            kill -KILL "$load_pid" 2>/dev/null
            wait "$load_pid" 2>/dev/null
            pid=
            echo "$name,$load,$trial,$(( (end - start) / 1000000 ))" | tee -a "$OUT"
            sleep 0.5
        done
    done
done
