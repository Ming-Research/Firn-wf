#!/usr/bin/env bash
# Runs the test suite of Redis 7.2.16 against one server in the suite's
# external mode, one unit at a time, then summarizes the outcome with
# summarize.py. research/experiments/redis-compat/README.md describes the
# method and the historical results.
#
# usage: run.sh --out DIR [options] redis [SERVER_ARGUMENT ...]
#        run.sh --out DIR [options] firn FIRN_EXECUTABLE
#        run.sh --out DIR [options] external HOST PORT
#
#   redis     starts the reference redis-server, Redis 7.2.16 built by
#             reference.sh, anew for every unit on a free port, without
#             persistence and with any further arguments given
#   firn      starts the given firn executable anew for every unit on a free
#             port, as `firn --port PORT --firn-clients 0 --appendonly no --timeout 0`
#   external  runs every unit against a server that is already running
#
# options:
#   --out DIR        run directory, created if missing; it receives a copy of
#                    the suite, every unit's output, tests.tsv, units.tsv,
#                    summary.txt, suite-units.txt (every unit the suite lists)
#                    and shard-units.txt (the units this run was to run)
#   --units "U ..."  units to run, by default every unit the suite lists
#   --shard I/N      runs only shard I of N (1 <= I <= N) of those units, as
#                    ratchet.py assign splits them by the times in
#                    unit-seconds.tsv, and takes its ports from the I-th of N
#                    equal parts of the range below the ephemeral ports, so
#                    that shards run side by side, each with an --out of its
#                    own (run-shards.sh does this and merges them)
#   --timeout SEC    seconds a test may go without progress before the suite
#                    stops the unit, default 120
#   --retries N      times a unit is run again past a test that timed out,
#                    default 5
#   --reference TSV  tests.tsv of an earlier run, for the not-reached column
#   --known-hangs TSV  tests already seen making no progress, as rows of unit,
#                    test name and the evidence; each unit skips its rows from
#                    the start instead of waiting out the timeout for each
#   --tolerant       lets the calls the suite's framework makes at the start
#                    and end of every block (FLUSHALL, FUNCTION FLUSH, and for
#                    the block's overrides CONFIG GET, CONFIG SET and the INFO
#                    that waits for an append-only rewrite) fail without
#                    stopping the unit: a diagnostic of what lies past them,
#                    not a measure of compatibility
#   -- ARG ...       further runtest arguments, such as --tags -needs:debug
#
# The suite's sources are the Redis 7.2.16 release archive, and redis-server,
# redis-cli and redis-benchmark are built from that archive: reference.sh
# fetches the archive, checks its SHA-256, builds the tools and caches both.
# They stand first on PATH for this script, the suite and the servers it
# starts. REDIS_COMPAT_CACHE names the cache directory,
# ${XDG_CACHE_HOME:-$HOME/.cache}/whitefoot-redis-compat by default.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
build_reference=$here/reference.sh
version=$("$build_reference" version)
sha256=$("$build_reference" sha256)
# Seconds a whole unit may take, a backstop behind the suite's own timeout.
unit_limit=3600

usage() {
    sed -n '/^# usage:/,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2
    exit 2
}

out= units= timeout=120 retries=5 reference= tolerant=0 known= shard=
while [ $# -gt 0 ]; do
    case $1 in
        --out) out=${2:?}; shift 2 ;;
        --units) units=${2:?}; shift 2 ;;
        --shard) shard=${2:?}; shift 2 ;;
        --timeout) timeout=${2:?}; shift 2 ;;
        --retries) retries=${2:?}; shift 2 ;;
        --reference) reference=$(realpath "${2:?}"); shift 2 ;;
        --known-hangs) known=$(realpath "${2:?}"); shift 2 ;;
        --tolerant) tolerant=1; shift ;;
        -*) usage ;;
        *) break ;;
    esac
done
[ -n "$out" ] && [ $# -ge 1 ] || usage
shard_index=1 shard_count=1
if [ -n "$shard" ]; then
    [[ $shard =~ ^([1-9][0-9]*)/([1-9][0-9]*)$ ]] || usage
    shard_index=${BASH_REMATCH[1]} shard_count=${BASH_REMATCH[2]}
    [ "$shard_index" -le "$shard_count" ] || usage
fi
target=$1
shift
host=127.0.0.1 port= firn=
# The comparator runs as the suite's own servers do in one respect besides
# the port and persistence: tests/assets/default.conf enables DEBUG, which
# Redis 7 refuses by default.
redis_args=(--bind 127.0.0.1 --save '' --appendonly no --enable-debug-command yes)
case $target in
    redis)
        while [ $# -gt 0 ] && [ "$1" != -- ]; do
            redis_args+=("$1")
            shift
        done ;;
    firn)
        [ $# -ge 1 ] && [ -x "$1" ] || usage
        firn=$(realpath "$1")
        shift ;;
    external)
        [ $# -ge 2 ] || usage
        host=$1 port=$2
        shift 2 ;;
    *) usage ;;
esac
extra=()
if [ $# -gt 0 ]; then
    [ "$1" = -- ] || usage
    shift
    extra=("$@")
fi

archive=$("$build_reference" archive)
PATH=$("$build_reference" bin):$PATH
if [ "$target" = redis ]; then
    installed=$(redis-server --version | sed -n 's/^Redis server v=\([^ ]*\).*/\1/p')
    if [ "$installed" != "$version" ]; then
        echo "run.sh: redis-server is $installed, the suite is $version" >&2
        exit 1
    fi
fi

mkdir -p "$out"
out=$(cd "$out" && pwd)
suite=$out/redis-$version
rm -rf "$suite" "$out/logs" "$out/servers" "$out/hung" "$out/data"
tar -xzf "$archive" -C "$out"
# Redis's no-argument unsubscribe helpers read no acknowledgments. Preserve
# those wire commands, but make their tests wait for all known memberships
# before another connection publishes or the next test starts. The same
# contextual patch applies to every target; stale or missing hunks stop the run.
python3 -B "$here/check-pubsub-barriers.py" "$suite" "$here/pubsub-barriers.patch"
patch --batch --forward --fuzz=0 -p1 -d "$suite" < "$here/pubsub-barriers.patch"
# Tests that run the suite's own client tools look for them in src/, as in a
# built source tree; the 7.2.16 tools built from the same archive stand in
# for them.
for tool in redis-cli redis-benchmark; do
    ln -s "$(command -v "$tool")" "$suite/src/$tool"
done
if [ "$tolerant" = 1 ]; then
    # The six calls of run_external_server_test become calls that may fail;
    # the count checks that the suite still makes them as expected.
    framework=$suite/tests/support/server.tcl
    sed -i -E \
        -e 's/^    (r flushall|r function flush)$/    catch {\1}/' \
        -e 's/^        (r config set \$param \$val)$/        catch {\1}/' \
        -e 's/^        (dict set saved_config \$param \[lindex \[r config get \$param\] 1\])$/        catch {\1}/' \
        -e 's/^            (waitForBgrewriteaof r)$/            catch {\1}/' \
        "$framework"
    tolerated=$(grep -c -E '^ +catch \{(r flushall|r function flush|r config set \$param \$val|dict set saved_config |waitForBgrewriteaof r)' "$framework" || true)
    if [ "$tolerated" != 6 ]; then
        echo "run.sh: --tolerant found $tolerated of the framework's 6 calls" >&2
        exit 1
    fi
fi
mkdir -p "$out/logs" "$out/servers" "$out/hung" "$out/data"
cd "$suite"
./runtest --list-tests > "$out/suite-units.txt"
[ -n "$units" ] || units=$(cat "$out/suite-units.txt")
if [ -n "$shard" ]; then
    units=$(printf '%s\n' $units | python3 -B "$here/ratchet.py" assign --costs "$here/unit-seconds.tsv" --shard "$shard")
fi
printf '%s\n' $units > "$out/shard-units.txt"

flags=(--singledb --ignore-encoding --ignore-digest --durable --timeout "$timeout")
{
    echo "suite: redis-$version.tar.gz sha256 $sha256"
    echo "suite adjustment: pubsub-barriers.patch waits for and checks all known no-argument unsubscribe acknowledgments"
    echo "target: $target"
    case $target in
        redis) echo "server: $(redis-server --version)"
               echo "server arguments: --port PORT$(printf ' %q' "${redis_args[@]}")" ;;
        firn) echo "server: firn sha256 $(sha256sum < "$firn" | cut -d' ' -f1)"
              echo "server arguments: PORT 0 - 0" ;;
        external) echo "server: $host:$port" ;;
    esac
    echo "runtest arguments: --host $host --port PORT --single UNIT --baseport FREE --portcount 1000 ${flags[*]} --skipfile HUNG ${extra[*]}"
    echo "shard: ${shard:-none}"
    if [ "$tolerant" = 1 ]; then
        echo "start_server framework: tolerant (FLUSHALL, FUNCTION FLUSH and the overrides' CONFIG GET, CONFIG SET and INFO may fail)"
    else
        echo "start_server framework: as released"
    fi
    echo "retries: $retries"
    echo "client: $(redis-cli --version), tclsh $(echo 'puts [info patchlevel]' | tclsh)"
    echo "host: $(uname -srm), $(nproc) processors"
    echo "started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$out/meta.txt"
printf 'unit\tattempt\truntest_exit\tserver\n' > "$out/attempts.tsv"

# Ports come from below the kernel's range of ephemeral ports: an outgoing
# connection, the suite's own among them, can hold a port in that range
# without listening on it, and a listener then cannot bind the port.
ephemeral_low=32768
if [ -r /proc/sys/net/ipv4/ip_local_port_range ]; then
    read -r ephemeral_low _ < /proc/sys/net/ipv4/ip_local_port_range
fi
if [ "$ephemeral_low" -le 11000 ]; then
    echo "run.sh: the ephemeral ports start at $ephemeral_low, leaving none below them" >&2
    exit 1
fi

# A shard takes its ports from its own part of the range below the ephemeral
# ports, so shards running side by side start their servers on ports of
# their own. The suite's port search also binds port+10000 for an instant as
# its cluster-bus probe, which can land in another shard's part; a collision
# there makes the search skip the port or start_server retry.
port_low=$((10000 + (shard_index - 1) * (ephemeral_low - 10000) / shard_count))
port_high=$((10000 + shard_index * (ephemeral_low - 10000) / shard_count))
if [ $((port_high - port_low)) -lt 3000 ]; then
    echo "run.sh: shard $shard_index of $shard_count has only ports $port_low to $((port_high - 1))" >&2
    exit 1
fi

# Prints a loopback port from this run's range that nothing listens on and
# that leaves the given number of ports above it in the range.
free_port() {
    local candidate
    for candidate in $(shuf -i "$port_low-$((port_high - 1 - ${1:-0}))" -n 100); do
        if ! (exec 3<> "/dev/tcp/127.0.0.1/$candidate") 2> /dev/null; then
            echo "$candidate"
            return 0
        fi
    done
    return 1
}

server_pid=
# Starts the server under test on a free port, setting port and server_pid.
start_server() {
    local log=$1 try deadline
    for try in 1 2 3; do
        # Every server starts in an empty directory: a test that saves an RDB
        # file or turns on the append-only file would otherwise hand what it
        # saved, functions included, to the next unit's server.
        rm -rf "$out/data"
        mkdir -p "$out/data"
        port=$(free_port)
        case $target in
            redis) redis-server --port "$port" --dir "$out/data" "${redis_args[@]}" > "$log" 2>&1 & ;;
            firn) (cd "$out/data" && exec "$firn" --port "$port" --firn-clients 0 --appendonly no --timeout 0) > "$log" 2>&1 & ;;
        esac
        server_pid=$!
        # The server has 10 seconds to answer PING; redis-cli waits for a
        # reply without limit, so every probe has its own.
        deadline=$((SECONDS + 10))
        while [ "$SECONDS" -lt "$deadline" ]; do
            kill -0 "$server_pid" 2> /dev/null || break
            if [ "$(timeout 2 redis-cli -h 127.0.0.1 -p "$port" ping 2> /dev/null)" = PONG ]; then
                return 0
            fi
            sleep 0.1
        done
        stop_server
    done
    echo "run.sh: the server did not start, see $log" >&2
    exit 1
}

# Stops the server under test and sets state to "running" when it was still
# running, or to "exited STATUS" as wait reports it when it had stopped
# itself. It runs in this shell, never in a subshell, so that wait can reap
# the server.
stop_server() {
    local status=0 i
    if kill -0 "$server_pid" 2> /dev/null; then
        kill -TERM "$server_pid" 2> /dev/null || true
        for i in $(seq 50); do
            kill -0 "$server_pid" 2> /dev/null || break
            sleep 0.1
        done
        kill -KILL "$server_pid" 2> /dev/null || true
        wait "$server_pid" 2> /dev/null || true
        state=running
    else
        wait "$server_pid" 2> /dev/null || status=$?
        state="exited $status"
    fi
    server_pid=
}

unit_pid=
cleanup() {
    if [ -n "$unit_pid" ]; then kill -TERM -- "-$unit_pid" 2> /dev/null || true; fi
    if [ -n "$server_pid" ]; then kill -KILL "$server_pid" 2> /dev/null || true; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for unit in $units; do
    hung=$out/hung/$unit.txt
    mkdir -p "$(dirname "$hung")" "$(dirname "$out/logs/$unit")" "$(dirname "$out/servers/$unit")"
    : > "$hung"
    if [ -n "$known" ]; then
        awk -F '\t' -v unit="$unit" '$1 == unit { print $2 }' "$known" >> "$hung"
    fi
    attempt=1
    starts=1
    while true; do
        log=$out/logs/$unit.$attempt.log
        started=$SECONDS
        [ "$target" = external ] || start_server "$out/servers/$unit.$attempt.log"
        # The suite listens for its own test client on the port below its
        # base port, and its check for a busy port misses one that another
        # process holds on 127.0.0.1, so the port is chosen here. The ports
        # the suite's own servers take, 1000 from its base port, stay within
        # this run's range.
        control=$(free_port 1100)
        # timeout runs the suite in a process group of its own, which the
        # cleanup above stops whole if this script is stopped.
        rc=0
        TERM=dumb timeout -k 10 "$unit_limit" ./runtest --host "$host" --port "$port" \
            --single "$unit" --baseport "$((control + 32))" --portcount 1000 "${flags[@]}" --skipfile "$hung" \
            "${extra[@]}" > "$log" 2>&1 &
        unit_pid=$!
        wait "$unit_pid" || rc=$?
        unit_pid=
        if [ "$target" = external ]; then
            if [ "$(timeout 2 redis-cli -h "$host" -p "$port" ping 2> /dev/null)" = PONG ]; then
                state=running
            else
                state="not answering"
            fi
        else
            stop_server
        fi
        printf '%s\t%s\t%s\t%s\n' "$unit" "$attempt" "$rc" "$state" >> "$out/attempts.tsv"
        printf '%s, attempt %s: runtest exit %s, server %s, %s ok, %s err, %s s\n' \
            "$unit" "$attempt" "$rc" "$state" "$(grep -c '^\[ok\]' "$log" || true)" \
            "$(grep -c '^\[err\]' "$log" || true)" "$((SECONDS - started))"
        # The suite did not start when its test client never reported ready,
        # as when its own port was taken; such an attempt is made again.
        if ! grep -q '^\[ready\]: ' "$log" && [ "$starts" -lt 3 ]; then
            starts=$((starts + 1))
            continue
        fi
        # A test that made no progress for the timeout is named in the
        # suite's report of its client; the unit runs again without it.
        stuck=$(sed -n 's/^sock[^ ]* => (IN PROGRESS) //p' "$log" | head -n 1)
        if grep -q '^\[TIMEOUT\]:' "$log" && [ -n "$stuck" ] && [ "$attempt" -le "$retries" ]; then
            printf '%s\n' "$stuck" >> "$hung"
            attempt=$((attempt + 1))
            starts=1
            continue
        fi
        break
    done
done

python3 "$here/summarize.py" "$out" ${reference:+--reference "$reference"} | tee "$out/summary.txt"
