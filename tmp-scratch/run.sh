#!/bin/bash
# Temporary check for board item firn-ops-pin-followup: compiles the server log
# and the open-files adjustment alone in a scratch graph with the newer
# release, runs them and compares the results with date(1) and Redis 7.2.16.
# Deleted before the pull request is ready.
set -eu
compiler=build/whitefoot/wf-63b7cbb03820/whitefootc
out=build/scratch
rm -rf "$out"
mkdir -p "$out"
cp -r firn/serverlog firn/fdlimit tmp-scratch/scratch "$out/"
cat > "$out/modules.wfg" <<'GRAPH'
pkg::serverlog: [std::io, std::fs, std::time, std::collections::deque];
pkg::fdlimit: [pkg::serverlog, std::io, std::process];
pkg::scratch: [pkg::serverlog, pkg::fdlimit, std::io, std::fs, std::process, std::time];

entry scratch = pkg::scratch::main;
GRAPH
"$compiler" --par --par-call-grain off --graph "$out/modules.wfg" --entry scratch --cache "$out/cache" -o "$out/scratch-bin"
redis_bin=$(tests/redis-suite/reference.sh)
hard=$(ulimit -Hn)
echo "hard limit of this shell: $hard"

# One run of the scratch program under a soft and a hard limit; the pid it
# should print goes to $out/pid.
run_scratch() {
  ( ulimit -Hn "$1"; ulimit -Sn "$2"; export TZ=Asia/Kolkata
    exec bash -c 'echo $$ > build/scratch/pid; exec build/scratch/scratch-bin' ) > "$out/out.txt" || echo "scratch exited with $?"
}

# The message of a Redis log line, without its process id, time and level mark.
strip() { sed -E 's/^[0-9]+:[A-Z] [0-9]{2} [A-Za-z]{3} [0-9]{4} [0-9:.]{12} [.*#-] //'; }

# Redis' log lines about the limit when started under a soft and a hard limit.
run_redis() {
  ( ulimit -Hn "$1"; ulimit -Sn "$2"
    exec timeout 3 "$redis_bin/redis-server" --port 26379 --maxclients "$3" --save "" --appendonly no --logfile "" ) > "$out/redis.txt" 2>&1 || true
  grep -E "open files|maxclients of|ulimit" "$out/redis.txt" | strip > "$out/redis-fd.txt" || true
}

# The fd lines of the scratch program, in order, and its result lines.
firn_fd() { grep -E "open files|maxclients of|ulimit" "$out/out.txt" | strip; }

echo "=== calendar and level filter, against date(1)"
start=$(date +%s)
run_scratch "$hard" 1024
end=$(date +%s)
expected() {
  local ms=$1 off=$2 seconds
  seconds=$((ms / 1000 + off))
  printf '4242:M %s.%03d # case\n' "$(date -u -d "@$seconds" '+%d %b %Y %H:%M:%S')" $((ms % 1000))
}
{
  expected 0 0
  expected 951782400000 0
  expected 1709164799999 0
  expected 1709164799999 19800
  expected 4102444800000 -18000
  expected 1751241600123 3600
  expected 1234567890004 -3600
} > "$out/want-fixed.txt"
grep -E '^4242:M' "$out/out.txt" > "$out/got-fixed.txt"
diff -u "$out/want-fixed.txt" "$out/got-fixed.txt"
echo "calendar lines match"

pid=$(cat "$out/pid")
live=$(grep -E -- '-line$' "$out/out.txt")
echo "$live"
test "$(echo "$live" | wc -l)" = 2
echo "$live" | head -1 | grep -E "^$pid:M [0-9]{2} [A-Z][a-z]{2} [0-9]{4} [0-9:.]{12} \* notice-line$"
echo "$live" | tail -1 | grep -E "^$pid:M [0-9]{2} [A-Z][a-z]{2} [0-9]{4} [0-9:.]{12} # warning-line$"
stamp=$(echo "$live" | head -1 | sed -E 's/^[0-9]+:M ([0-9]{2} [A-Za-z]{3} [0-9]{4} [0-9:]{8})\..*/\1/')
seen=$(TZ=Asia/Kolkata date -d "$stamp" +%s)
echo "start $start seen $seen end $end"
test "$seen" -ge $((start - 1))
test "$seen" -le $((end + 1))
echo "live lines match: pid, level filter and local time"

echo "=== soft limit raised, against Redis 7.2.16"
run_redis "$hard" 1024 5000
cat "$out/redis-fd.txt"
run_scratch "$hard" 1024
firn_fd | head -1 > "$out/firn-fd.txt"
diff -u "$out/redis-fd.txt" "$out/firn-fd.txt"
grep -E '^[0-9]+:M .* scratch: maxclients=' "$out/out.txt" | strip
grep -q 'scratch: maxclients=5000 fatal=0 soft=5032 hard=' "$out/out.txt"
echo "raise matches"

echo "=== hard limit below the request, against Redis 7.2.16"
run_redis 2000 1024 5000
cat "$out/redis-fd.txt"
run_scratch 2000 1024
firn_fd | head -3 > "$out/firn-fd.txt"
diff -u "$out/redis-fd.txt" "$out/firn-fd.txt"
grep -q 'scratch: maxclients=1960 fatal=0 soft=1992 hard=2000' "$out/out.txt"
firn_fd | sed -n 4,6p | grep -E "maxclients of 1990 requiring at least 2022|Current maximum open files is 1992. maxclients has been reduced to 1960"
echo "reduction matches"

echo "=== limit too low to start, against Redis 7.2.16"
run_redis 30 30 5000
cat "$out/redis-fd.txt"
run_scratch 30 30
firn_fd | head -1 > "$out/firn-fd.txt"
diff -u "$out/redis-fd.txt" "$out/firn-fd.txt"
grep -q 'scratch: maxclients=0 fatal=1' "$out/out.txt"
echo "fatal matches"
