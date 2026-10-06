#!/bin/bash
# Runs one consumer's own Redis tests, pinned to a release, against one server,
# for the consumers investigation (research/investigations/consumers/).
#
#   run.sh <consumer> <server>
#
# consumer: django, connect-redis or rate-limiter-flexible; server: redis, the
# redis-server on PATH, which must be Redis 7.0.15, with MONITOR recording
# every command into monitor.log from before the tests start, or firn, the
# executable FIRN names. The consumer's test output goes to test.log and its
# exit status to status, under $OUT/<consumer>-<server>/. The script exits 0
# whatever the tests did; a failure to set the run up or to record it exits 1.
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
    *) echo "unknown consumer $consumer" >&2; exit 1 ;;
esac
case $server in
    redis)
        command -v redis-server >/dev/null || { echo "no redis-server" >&2; exit 1; }
        redis-server --version | grep -q 'v=7\.0\.15 ' ||
            { echo "the reference must be Redis 7.0.15: $(redis-server --version)" >&2; exit 1; } ;;
    firn) test -x "${FIRN:-}" || { echo "FIRN names no executable" >&2; exit 1; } ;;
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
    if [ "$server" = redis ]; then
        "$real_redis" --port "$port" --save "" --appendonly no --daemonize no > "$out/server.log" 2>&1 &
    else
        "$FIRN" --port "$port" > "$out/server.log" 2>&1 &
    fi
    started=$!
    for i in $(seq 1 50); do
        redis-cli -p "$port" PING 2>/dev/null | grep -q PONG && break
        sleep 0.2
    done
    redis-cli -p "$port" PING 2>/dev/null | grep -q PONG ||
        { echo "the server did not answer on $port" >&2; cat "$out/server.log" >&2; exit 1; }
    if [ "$server" = redis ]; then
        redis-cli -p "$port" MONITOR > "$out/monitor.log" 2>&1 &
        watching=$!
        for i in $(seq 1 50); do
            [ "$(head -n 1 "$out/monitor.log" 2>/dev/null)" = OK ] && return 0
            sleep 0.1
        done
        echo "MONITOR did not start: $(cat "$out/monitor.log")" >&2
        exit 1
    fi
}
stop_server() {
    [ -n "$watching" ] && kill "$watching" 2>/dev/null
    [ -n "$started" ] && kill "$started" 2>/dev/null
    true
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
esac
echo "$consumer against $server: tests exited $(cat "$out/status")"
if [ "$server" = redis ]; then
    sleep 1
    kill -0 "$watching" 2>/dev/null || { echo "MONITOR ended before the tests did" >&2; exit 1; }
    python3 "$here/profile.py" "$out/monitor.log" > "$out/profile.tsv" || exit 1
fi
exit 0
