#!/bin/bash
# Runs one consumer's own Redis tests, pinned to a release, against one server,
# for the consumers investigation (research/investigations/consumers/).
#
#   run.sh <consumer> <server>
#
# consumer: django, connect-redis or rate-limiter-flexible; server: redis, the
# redis-server on PATH (Redis 7.0.15 in CI), with MONITOR recording every
# command into monitor.log, or firn, the executable FIRN names. The consumer's
# test output goes to test.log and its exit status to status, under
# $OUT/<consumer>-<server>/. The script exits 0 whatever the tests did; a
# failure to set the run up exits 1. The consumers workflow
# (.github/workflows/consumers.yml) calls it.
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
    redis) command -v redis-server >/dev/null || { echo "no redis-server" >&2; exit 1; } ;;
    firn) test -x "${FIRN:-}" || { echo "FIRN names no executable" >&2; exit 1; } ;;
    *) echo "unknown server $server" >&2; exit 1 ;;
esac
real_redis=$(command -v redis-server || true)

git clone --quiet --depth 1 --branch "$ref" "$repo" "$src" || exit 1
echo "$consumer $ref $(git -C "$src" rev-parse HEAD) against $server" | tee "$out/what.txt"

# A redis-server on PATH that starts the server under test on the port it is
# given, and records the commands it receives when that server is Redis.
# connect-redis starts its server this way; the others reach one the script
# starts through the same command.
cat > "$out/bin/redis-server" <<EOF
#!/bin/sh
port=6379
prev=
for word in "\$@"; do
    [ "\$prev" = --port ] && port=\$word
    prev=\$word
done
if [ "$server" = redis ]; then
    ( for i in 1 2 3 4 5 6 7 8 9 10; do
          redis-cli -p "\$port" PING >/dev/null 2>&1 && break
          sleep 0.2
      done
      exec redis-cli -p "\$port" MONITOR >> "$out/monitor.log" 2>&1 ) &
    exec "$real_redis" --port "\$port" --save "" --appendonly no --daemonize no
else
    exec "${FIRN:-}" --port "\$port"
fi
EOF
chmod +x "$out/bin/redis-server"
export PATH="$out/bin:$PATH"

started=
start_server() {
    redis-server --port "$port" > "$out/server.log" 2>&1 &
    started=$!
    for i in $(seq 1 50); do
        redis-cli -p "$port" PING 2>/dev/null | grep -q PONG && return 0
        sleep 0.2
    done
    echo "the server did not answer on $port" >&2
    cat "$out/server.log" >&2
    exit 1
}
stop_server() {
    [ -n "$started" ] && kill "$started" 2>/dev/null
    pkill -f "redis-cli -p $port MONITOR" 2>/dev/null
    true
}
trap stop_server EXIT

cd "$src" || exit 1
case $consumer in
    django)
        python3 -m venv "$out/venv" && . "$out/venv/bin/activate" &&
            pip install --quiet -e . redis > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        # The server named in CACHES is all the configuration the suites
        # need; as the default cache it also backs the cache sessions.
        cat > tests/test_consumers_redis.py <<EOF
from test_sqlite import *  # noqa: F401,F403

CACHES = {
    "default": {
        "BACKEND": "django.core.cache.backends.redis.RedisCache",
        "LOCATION": "redis://127.0.0.1:$port",
    },
}
EOF
        start_server
        (cd tests && python runtests.py --settings=test_consumers_redis --parallel 1 \
            cache sessions_tests) > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
    connect-redis)
        npm install --no-audit --no-fund > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        npx vitest run > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
    rate-limiter-flexible)
        npm install --no-audit --no-fund > "$out/setup.log" 2>&1 || { cat "$out/setup.log"; exit 1; }
        start_server
        npx mocha --exit test/RateLimiterRedis.ioredis.test.js \
            test/RateLimiterRedis.redis.test.js \
            test/RateLimiterRedisNonAtomic.ioredis.test.js > "$out/test.log" 2>&1
        echo $? > "$out/status" ;;
esac
echo "$consumer against $server: tests exited $(cat "$out/status")"
if [ "$server" = redis ]; then
    sleep 1
    python3 "$here/profile.py" "$out/monitor.log" > "$out/profile.tsv"
    echo "$(wc -l < "$out/monitor.log") commands recorded, $(($(wc -l < "$out/profile.tsv") - 1)) forms"
fi
exit 0
