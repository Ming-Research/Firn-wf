#!/bin/bash
# Runs one consumer's own Redis tests, pinned to a release, against one server,
# for the consumers investigation (research/investigations/consumers/).
#
#   run.sh <consumer> <server>
#
# consumer: django, connect-redis, rate-limiter-flexible or sidekiq; server:
# redis, the redis-server on PATH, which must be Redis 7.2.16, with MONITOR
# recording every command into monitor.log from before the tests start, or
# firn, the executable FIRN names. The consumer's test output goes to test.log
# and its exit status to status, under $OUT/<consumer>-<server>/. The script
# exits 0 whatever the tests did; a failure to set the run up or to record it
# exits 1.
#
# A consumer that runs its tests file by file (sidekiq) also writes cases.tsv,
# one `CASE<TAB>STATUS` row for each case, in the format compare.py reads; see
# run_each_file below. FILE_LIMIT, in seconds, bounds each file (default 300).
# Sidekiq's run ends with a probe of its heartbeat, heartbeat.txt.
# The consumers workflow (.github/workflows/consumers.yml) calls it.
set -u
consumer=$1
server=$2
here=$(cd "$(dirname "$0")" && pwd)
out=${OUT:-/tmp/consumers}/$consumer-$server
src=$out/src
rm -rf "$out"
mkdir -p "$out" "$out/bin"

case $consumer in
    django)
        repo=https://github.com/django/django.git
        ref=6.1.1
        port=6379 ;;
    connect-redis)
        repo=https://github.com/tj/connect-redis.git
        ref=v10.0.0
        port=18543 ;;
    rate-limiter-flexible)
        repo=https://github.com/animir/node-rate-limiter-flexible.git
        ref=v11.2.1
        port=6379 ;;
    sidekiq)
        repo=https://github.com/sidekiq/sidekiq.git
        ref=v8.1.7
        port=6379 ;;
    *) echo "unknown consumer $consumer" >&2; exit 1 ;;
esac
case $server in
    redis)
        command -v redis-server >/dev/null || { echo "no redis-server" >&2; exit 1; }
        redis-server --version | grep -q 'v=7\.2\.16 ' ||
            { echo "the reference must be Redis 7.2.16: $(redis-server --version)" >&2; exit 1; } ;;
    firn) test -x "${FIRN:-}" || { echo "FIRN names no executable" >&2; exit 1; }
        FIRN=$(cd "$(dirname "$FIRN")" && pwd)/$(basename "$FIRN") ;;
    *) echo "unknown server $server" >&2; exit 1 ;;
esac
real_redis=$(command -v redis-server || true)

git clone --quiet --depth 1 --branch "$ref" "$repo" "$src" || exit 1
echo "$consumer $ref $(git -C "$src" rev-parse HEAD) against $server" | tee "$out/what.txt"

# The script starts the server under test on the consumer's port before the
# tests, and with Redis waits for MONITOR to answer OK, so that the record
# holds every command the tests send. connect-redis starts a redis-server of
# its own; the one on PATH is a stand-in that only waits, so the consumer's
# command finds the server already running and its stop ends the stand-in.
printf '#!/bin/sh\nexec sleep 3600\n' > "$out/bin/redis-server"
chmod +x "$out/bin/redis-server"
export PATH="$out/bin:$PATH"

started=
watching=
start_server() {
    # Both servers keep an append-only file, as a deployment would, each in
    # its own directory.
    mkdir -p "$out/data"
    if [ "$server" = redis ]; then
        "$real_redis" --port "$port" --save "" --appendonly yes --dir "$out/data" --daemonize no > "$out/server.log" 2>&1 &
    else
        (cd "$out/data" && exec "$FIRN" --port "$port" --appendonly yes) > "$out/server.log" 2>&1 &
    fi
    started=$!
    for i in $(seq 1 50); do
        redis-cli -p "$port" PING 2>/dev/null | grep -q PONG && break
        sleep 0.2
    done
    redis-cli -p "$port" PING 2>/dev/null | grep -q PONG ||
        { echo "the server did not answer on $port" >&2; cat "$out/server.log" >&2; exit 1; }
    if [ "$server" = redis ]; then
        # A restart appends to the same record, each MONITOR session starting
        # with its own OK.
        local oks
        oks=$(grep -c '^OK$' "$out/monitor.log" 2>/dev/null)
        redis-cli -p "$port" MONITOR >> "$out/monitor.log" 2>&1 &
        watching=$!
        for i in $(seq 1 50); do
            [ "$(grep -c '^OK$' "$out/monitor.log")" -gt "${oks:-0}" ] && return 0
            sleep 0.1
        done
        echo "MONITOR did not start: $(tail -n 3 "$out/monitor.log")" >&2
        exit 1
    fi
}
stop_server() {
    if [ -n "$watching" ]; then
        kill "$watching" 2>/dev/null
        wait "$watching" 2>/dev/null
        # A kill can leave a half line; the next session's OK starts a line.
        echo >> "$out/monitor.log"
    fi
    if [ -n "$started" ]; then
        kill "$started" 2>/dev/null
        for i in $(seq 1 100); do
            kill -0 "$started" 2>/dev/null || break
            sleep 0.1
        done
        kill -9 "$started" 2>/dev/null
        wait "$started" 2>/dev/null
    fi
    started=
    watching=
    true
}
# A server that starts on an empty directory, as the files of a consumer that
# runs them one by one each need.
restart_server() {
    stop_server
    rm -rf "$out/data"
    start_server
}

# run_each_file LIMIT COMMAND [ARG...] runs COMMAND ARG... FILE, once for each
# FILE of the files array, in a process of its own on a server started anew on
# an empty directory, so that a server a file crashes does not take the next
# files with it. It gives each process CASE_RECORD, the path of cases.tsv, and
# CASE_FILE, FILE; the consumer's reporter appends one row `FILE::NAME<TAB>
# STATUS` to the record for each case as it ends. A process that is still
# running after LIMIT seconds is killed and the record gets the row
# `FILE<TAB>unfinished`; a process that ends having recorded no case gets the
# same row. A case without a row did not pass. The consumer's output goes to
# test.log, each file under a line `==== FILE`; files.tsv gets one row `FILE
# <TAB>EXIT STATUS<TAB>SECONDS` for each. The function returns 1 when any
# process exited with a status other than 0 or was killed. The record's format
# is that of compare.py's description.
cases=$out/cases.tsv
run_each_file() {
    local limit=$1 file before rc began worst=0
    shift
    : > "$cases"
    : > "$out/test.log"
    : > "$out/files.tsv"
    export CASE_RECORD=$cases
    for file in "${files[@]}"; do
        restart_server
        export CASE_FILE=$file
        before=$(wc -l < "$cases")
        echo "==== $file" >> "$out/test.log"
        began=$SECONDS
        timeout -k 10 "$limit" "$@" "$file" >> "$out/test.log" 2>&1
        rc=$?
        echo "==== $file exited $rc" >> "$out/test.log"
        printf '%s\t%s\t%s\n' "$file" "$rc" $((SECONDS - began)) >> "$out/files.tsv"
        [ "$rc" -ne 0 ] && worst=1
        if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ] || [ "$(wc -l < "$cases")" -eq "$before" ]; then
            printf '%s\tunfinished\n' "$file" >> "$cases"
        fi
    done
    return $worst
}

# The probe of what Sidekiq's heartbeat does against this server: a real
# `sidekiq` process (bin/sidekiq), with one job that does nothing in its queue, runs for
# about 30 seconds, which spans three heartbeats (a beat every 10 seconds)
# and the flush of its metrics, which Sidekiq makes on a beat, with the
# BITFIELD command. The log goes to heartbeat.log and a summary to
# heartbeat.txt. The probe records what the server does; it fails on nothing.
heartbeat_probe() {
    local pid i registered
    restart_server
    {
        echo "enqueue one no-op job:"
        bundle exec ruby -e 'require "sidekiq"; Sidekiq::Client.push("class" => "HeartbeatNoop", "args" => [])' 2>&1
        echo "exit $?"
    } > "$out/heartbeat.log"
    bundle exec ruby bin/sidekiq -r "$here/sidekiq_heartbeat_job.rb" -q default -c 1 -t 5 >> "$out/heartbeat.log" 2>&1 &
    pid=$!
    sleep 22
    registered=$(redis-cli -p "$port" SCARD processes 2>&1)
    sleep 10
    kill -TERM "$pid" 2>/dev/null
    for i in $(seq 1 150); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    kill -0 "$pid" 2>/dev/null && { kill -9 "$pid" 2>/dev/null; echo "sidekiq had to be killed" >> "$out/heartbeat.log"; }
    wait "$pid" 2>/dev/null
    {
        echo "heartbeat probe against $server"
        echo "processes registered after 22 s (SCARD processes): $registered"
        echo "jobs processed (GET stat:processed): $(redis-cli -p "$port" GET stat:processed 2>&1 | head -n 1)"
        echo "jobs left in queue:default (LLEN): $(redis-cli -p "$port" LLEN queue:default 2>&1 | head -n 1)"
        echo "error lines in heartbeat.log: $(grep -c -w ERROR "$out/heartbeat.log")"
    } | tee "$out/heartbeat.txt"
}
trap stop_server EXIT

cd "$src" || exit 1
case $consumer in
    django)
        python3 -m venv "$out/venv" && . "$out/venv/bin/activate" &&
            pip install --quiet -e . redis > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        echo "client: redis-py $(python -c 'import redis; print(redis.__version__)')" | tee -a "$out/what.txt"
        # The server named in CACHES is all the configuration the suites
        # need; as the default cache it also backs the cache sessions.
        printf '%s\n' 'from test_sqlite import *  # noqa: F401,F403' '' 'CACHES = {' \
            '    "default": {' \
            '        "BACKEND": "django.core.cache.backends.redis.RedisCache",' \
            "        \"LOCATION\": \"redis://127.0.0.1:$port\"," \
            '    },' '}' > tests/test_consumers_redis.py
        start_server
        (cd tests && python runtests.py --settings=test_consumers_redis --parallel 1 \
            cache sessions_tests) > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
    connect-redis)
        # The tests import the built package, as the project's own CI
        # builds it first.
        { npm install --no-audit --no-fund && npm run build; } > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        echo "client: $(npm ls redis --depth=0 2>/dev/null | grep -o 'redis@[0-9.]*')" | tee -a "$out/what.txt"
        start_server
        npx vitest run > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
    rate-limiter-flexible)
        # The Redis tests need no native module; other stores' tests do,
        # and their builds are skipped.
        npm install --no-audit --no-fund --ignore-scripts > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        echo "clients: $(npm ls ioredis redis --depth=0 2>/dev/null | grep -o -E '(ioredis|redis)@[0-9.]*' | tr '\n' ' ')" | tee -a "$out/what.txt"
        start_server
        npx mocha --exit test/RateLimiterRedis.ioredis.test.js \
            test/RateLimiterRedis.redis.test.js \
            test/RateLimiterRedisNonAtomic.ioredis.test.js > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
    sidekiq)
        # The project's own CI runs the newest Ruby and Rails of its matrix
        # (Ruby 4.0, Rails 8.1) and keeps no lock file, so the gems are the
        # newest its Gemfile allows today. Its tests load only the default and
        # test gem groups, so the lint, terminal UI and load-test groups, whose
        # native gems the tests never use, are not installed.
        export RAILS_VERSION=8.1 BUNDLE_WITHOUT=development:tui:load_test BUNDLE_PATH="$out/bundle"
        bundle install --jobs 4 > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        echo "ruby: $(ruby -v)" | tee -a "$out/what.txt"
        echo "gems: $(bundle list 2>/dev/null | grep -o -E '(redis-client|railties|minitest|connection_pool|rack) \([0-9][^)]*\)' | tr '\n' ' ')" | tee -a "$out/what.txt"
        # The tests find their server in REDIS_URL, where helper.rb flushes
        # database 0 before each case; sharding_test.rb uses databases 5 and 6
        # besides. Minitest's SEED fixes the order of cases, the same on both
        # servers. Each file runs in a process of its own, as run_each_file
        # does for any consumer.
        export REDIS_URL="redis://127.0.0.1:$port/0" SEED=1
        files=()
        while IFS= read -r file; do files+=("$file"); done < <(find test -name '*_test.rb' | sort)
        echo "test files: ${#files[@]}, seed $SEED, at most ${FILE_LIMIT:-300} s each" | tee -a "$out/what.txt"
        run_each_file "${FILE_LIMIT:-300}" bundle exec ruby -Ilib -Itest -r "$here/minitest_record.rb"
        echo $? > "$out/status"
        heartbeat_probe ;;
esac
echo "$consumer against $server: tests exited $(cat "$out/status")"
if [ "$server" = redis ]; then
    sleep 1
    kill -0 "$watching" 2>/dev/null || { echo "MONITOR ended before the tests did" >&2; exit 1; }
    python3 "$here/profile.py" "$out/monitor.log" > "$out/profile.tsv" || exit 1
fi
exit 0
