#!/bin/sh
# Experiment only (branch exp/halo-gc-stats): the counted heap's growth per
# command for minimal commands and scripts, read from INFO scriptprobe's
# heap_in_use before and after a run of each. Usage: leak-probe.sh FIRN OUT
set -eu
FIRN=$1
OUT=$2
PORT=7411
N=${LEAK_N:-200000}
echo "variant,warm_heap,heap_after_n,heap_after_2n,bytes_per_command" >"$OUT"
heap() { redis-cli -p "$PORT" INFO scriptprobe | tr -d '\r' | sed -n 's/^heap_in_use://p'; }
probe() {
    name=$1
    shift
    "$FIRN" --port "$PORT" >"/tmp/firn-$name.log" 2>&1 &
    pid=$!
    tries=0
    until redis-cli -p "$PORT" PING >/dev/null 2>&1; do
        tries=$((tries + 1))
        [ "$tries" -lt 100 ] || { echo "firn did not start for $name" >&2; exit 1; }
        sleep 0.1
    done
    redis-benchmark -p "$PORT" -c 8 -n 50000 -r 100000 "$@" >/dev/null
    h0=$(heap)
    redis-benchmark -p "$PORT" -c 8 -n "$N" -r 100000 "$@" >/dev/null
    h1=$(heap)
    redis-benchmark -p "$PORT" -c 8 -n "$N" -r 100000 "$@" >/dev/null
    h2=$(heap)
    per=$(python3 -c "print(round(($h2 - $h1) / $N, 1))")
    echo "$name,$h0,$h1,$h2,$per" | tee -a "$OUT"
    kill "$pid"
    wait "$pid" 2>/dev/null || true
}
probe set SET key:__rand_int__ v
probe incrby INCRBY key:__rand_int__ 1
probe eval-return-1 EVAL "return 1" 0
probe eval-local-table EVAL "local t = {1, 2, 3} return 1" 0
probe eval-return-table EVAL "return {1, 2}" 0
probe eval-concat EVAL "local s = ARGV[1] .. 'x' return 1" 0 abc
probe eval-call-get EVAL "return redis.call('GET', KEYS[1])" 1 key:__rand_int__
probe eval-call-incrby EVAL "return redis.call('INCRBY', KEYS[1], 1)" 1 key:__rand_int__
probe eval-call-set-ex-nx EVAL "return redis.call('SET', KEYS[1], 0, 'EX', 100, 'NX')" 1 key:__rand_int__
probe eval-call-pttl EVAL "return redis.call('PTTL', KEYS[1])" 1 key:__rand_int__
