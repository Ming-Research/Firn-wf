#!/usr/bin/env bash
# Execute the quickstart's commands, then check the resulting persistence.
set -euo pipefail

if [[ $# != 2 || ( $1 != tarball && $1 != image ) ]]; then
    printf 'Usage: %s tarball ARCHIVE.tar.gz | image DOCKER_IMAGE\n' "$0" >&2
    exit 2
fi
mode=$1
input=$2
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
doc=${QUICKSTART_DOC:-$repo/docs/quickstart.md}
scratch=$(mktemp -d)
http_pid=
image_name="firn-quickstart-$(basename "$scratch" | tr '[:upper:]' '[:lower:]')-$$"
probe_name="$image_name-inspect"
cleanup() {
    result=$?
    trap - EXIT
    if [[ $result != 0 ]]; then
        printf 'Quickstart %s failed (exit %s).\n' "$mode" "$result" >&2
        for log in "$scratch"/http.log "$scratch"/output.log "$scratch"/trace.log "$scratch"/work/*.log; do
            if [[ -f $log ]]; then
                printf '\n%s\n' "$(basename "$log")" >&2
                tail -n 80 "$log" >&2
            fi
        done
    fi
    if [[ -n $http_pid ]]; then
        kill "$http_pid" 2>/dev/null || true
        wait "$http_pid" 2>/dev/null || true
    fi
    if [[ $mode == image ]]; then
        docker rm -f "$image_name" "$probe_name" >/dev/null 2>&1 || true
        docker volume rm "$image_name-data" >/dev/null 2>&1 || true
    fi
    rm -rf "$scratch"
    return "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Reject incomplete or malformed command blocks before running any commands.
# All blocks are checked, including those belonging to the other path.
python3 - "$doc" "$mode" "$scratch/run.sh" <<'PY'
import pathlib
import sys

doc, mode, target = sys.argv[1:]
expected = ['common', 'tarball', 'image', 'common', 'tarball', 'image',
            'common', 'tarball', 'image', 'common']
blocks = []
active = None
body = []
for number, line in enumerate(pathlib.Path(doc).read_text().splitlines(), 1):
    if line.startswith('```'):
        if active is not None:
            if line != '```':
                raise SystemExit(f'{doc}:{number}: malformed closing fence')
            if not any(part.strip() for part in body):
                raise SystemExit(f'{doc}:{number}: empty quickstart block')
            blocks.append((active, '\n'.join(body)))
            active, body = None, []
        else:
            prefix = '```sh quickstart-'
            if not line.startswith(prefix) or line[len(prefix):] not in {'common', 'tarball', 'image'}:
                raise SystemExit(f'{doc}:{number}: every command fence must have a valid quickstart tag')
            active = line[len(prefix):]
    elif active is not None:
        body.append(line)
if active is not None:
    raise SystemExit(f'{doc}: unclosed quickstart block')
if [kind for kind, _ in blocks] != expected:
    raise SystemExit(f'{doc}: missing, extra, or reordered quickstart blocks')
pathlib.Path(target).write_text('\n\n'.join(body for kind, body in blocks
                                          if kind in {'common', mode}) + '\n')
PY

export QUICKSTART_WORKDIR="$scratch/work"
export FIRN_QUICKSTART_NAME="$image_name"
if [[ $mode == tarball ]]; then
    input=$(python3 - "$input" <<'PY'
import pathlib, sys
print(pathlib.Path(sys.argv[1]).resolve(strict=True))
PY
)
    archive=$(basename "$input")
    if [[ ! $archive =~ ^firn-([A-Za-z0-9][A-Za-z0-9._-]*)-linux-x86_64\.tar\.gz$ ]]; then
        printf 'Unexpected release archive name: %s\n' "$archive" >&2
        exit 2
    fi
    export FIRN_TAG=${BASH_REMATCH[1]}
    [[ -f $input.sha256 ]]
    python3 -u - "$(dirname "$input")" "$scratch/http.port" >"$scratch/http.log" 2>&1 <<'PY' &
import functools
import http.server
import pathlib
import sys

handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=sys.argv[1])
with http.server.HTTPServer(('127.0.0.1', 0), handler) as server:
    pathlib.Path(sys.argv[2]).write_text(str(server.server_port))
    server.serve_forever()
PY
    http_pid=$!
    for attempt in {1..50}; do
        [[ ! -s $scratch/http.port ]] || break
        kill -0 "$http_pid"
        sleep 0.1
    done
    [[ -s $scratch/http.port ]]
    export FIRN_RELEASE_URL="http://127.0.0.1:$(cat "$scratch/http.port")"
else
    export FIRN_IMAGE=$input
    docker image inspect "$FIRN_IMAGE" >"$scratch/image.json"
    python3 - "$scratch/image.json" <<'PY'
import json, sys

image = json.load(open(sys.argv[1]))[0]
config = image['Config']
user = config.get('User', '').split(':')[0]
if not user or user in {'0', 'root'}:
    raise SystemExit('The quickstart image must run as a nonroot user')
if config.get('WorkingDir') != '/data':
    raise SystemExit('The quickstart image must work in /data')
argv = (config.get('Entrypoint') or []) + (config.get('Cmd') or [])
for option, value in [('--appendonly', 'yes'), ('--bind', '0.0.0.0')]:
    pairs = list(zip(argv, argv[1:]))
    if (option, value) not in pairs:
        raise SystemExit(f'The image default command must include {option} {value}')
if image.get('Os') != 'linux' or image.get('Architecture') != 'amd64':
    raise SystemExit('The quickstart release image must be linux/amd64')
PY
fi

# One child shell executes exactly the selected Markdown bodies. GNU timeout
# bounds the child and its process group, including a background firn process.
timeout --kill-after=5s 180s bash -euo pipefail -c \
    'PS4="+quickstart:\${LINENO}: "; set -x; source "$1"' bash "$scratch/run.sh" \
    >"$scratch/output.log" 2>"$scratch/trace.log"

# Independently verify the final stop, allowing the prior connections' TIME_WAIT
# without allowing another listener on the quickstart's address and port.
python3 - <<'PY'
import socket

try:
    with socket.socket() as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(('127.0.0.1', 6379))
        listener.listen(1)
except OSError as error:
    raise SystemExit(f'After the quickstart stopped, 127.0.0.1:6379 could not bind and listen: {error}')
PY

# Check execution evidence as well as state: dropping a read/assertion from
# the document must not pass merely because the AOF still contains the SET.
python3 - "$scratch/trace.log" "$mode" <<'PY'
import pathlib, re, sys

trace = pathlib.Path(sys.argv[1]).read_text()
mode = sys.argv[2]
def require(pattern, count, description):
    found = len(re.findall(pattern, trace, re.MULTILINE))
    if found != count:
        raise SystemExit(f'Quickstart did not execute {description}: expected {count}, found {found}')
require(r'redis-cli .* --raw SET firn:quickstart survives-restart$', 1, 'one SET')
require(r'redis-cli .* --raw GET firn:quickstart$', 2, 'GET before and after restart')
require(r': assert_port_free$', 3, 'port probes before startup and after both stops')
require(r'\[\[ OK == OK \]\]$', 1, 'the OK assertion')
require(r'\[\[ survives-restart == survives-restart \]\]$', 2, 'both value assertions')
if not re.search(r'\[\[ PONG == PONG \]\]$', trace, re.MULTILINE):
    raise SystemExit('Quickstart did not assert PONG')
if mode == 'tarball':
    require(r'redis-cli .* SHUTDOWN NOSAVE$', 2, 'both orderly shutdowns')
    require(r': wait [0-9]+$', 2, 'both process waits')
else:
    require(r': docker stop --time 20 ', 2, 'both Docker stops')
    require(r': docker rm ', 2, 'both container removals')
    require(r'\[\[ 0 == 0 \]\]$', 2, 'both clean container exits')
set_at = re.search(r'redis-cli .* --raw SET firn:quickstart survives-restart$', trace, re.MULTILINE).start()
gets = [match.start() for match in re.finditer(r'redis-cli .* --raw GET firn:quickstart$', trace, re.MULTILINE)]
stop_pattern = r'redis-cli .* SHUTDOWN NOSAVE$' if mode == 'tarball' else r': docker stop --time 20 '
stops = [match.start() for match in re.finditer(stop_pattern, trace, re.MULTILINE)]
if not (set_at < gets[0] < stops[0] < gets[1] < stops[1]):
    raise SystemExit('Quickstart must GET once before the first stop and once after restart')
probes = [match.start() for match in re.finditer(r': assert_port_free$', trace, re.MULTILINE)]
if not (probes[0] < set_at < gets[0] < stops[0] < probes[1] < gets[1] < stops[1] < probes[2]):
    raise SystemExit('Quickstart must probe the port before SET, between the first stop and restarted GET, and after the final stop')
PY

if [[ $mode == image ]]; then
    first=$(cat "$scratch/work/first-container.id")
    second=$(cat "$scratch/work/second-container.id")
    [[ $first =~ ^[0-9a-f]{64}$ && $second =~ ^[0-9a-f]{64}$ && $first != "$second" ]]
    docker create --name "$probe_name" \
        --mount "type=volume,src=$image_name-data,dst=/data" "$FIRN_IMAGE" >/dev/null
    docker cp "$probe_name:/data/appendonlydir" "$scratch/aof"
    docker rm "$probe_name" >/dev/null
    aof=$scratch/aof
else
    aof=$scratch/work/data/appendonlydir
fi

python3 - "$aof" <<'PY'
import pathlib, sys

directory = pathlib.Path(sys.argv[1])
manifest = directory / 'appendonly.aof.manifest'
if not manifest.is_file() or manifest.stat().st_size == 0:
    raise SystemExit('AOF manifest is missing or empty')
increments = []
bases = []
for line in manifest.read_text().splitlines():
    fields = line.split()
    if len(fields) != 6 or fields[0] != 'file' or fields[2] != 'seq' or fields[4] != 'type':
        raise SystemExit(f'Malformed AOF manifest row: {line}')
    name = fields[1]
    if pathlib.Path(name).name != name or not fields[3].isdigit():
        raise SystemExit(f'Unsafe AOF manifest row: {line}')
    path = directory / name
    if not path.is_file():
        raise SystemExit(f'Manifest names a missing AOF file: {name}')
    if fields[5] == 'i':
        increments.append(path)
    elif fields[5] == 'b':
        bases.append(path)
    else:
        raise SystemExit(f'Unexpected AOF file type: {line}')
if not bases or not increments or not any(path.stat().st_size for path in increments):
    raise SystemExit('AOF must contain a base and a nonempty incremental file')
data = b''.join(path.read_bytes() for path in increments)
for value in [b'firn:quickstart', b'survives-restart']:
    record = b'$' + str(len(value)).encode() + b'\r\n' + value + b'\r\n'
    if record not in data:
        raise SystemExit(f'Persisted AOF does not contain the expected RESP value: {value!r}')
PY

cat "$scratch/output.log"
printf 'Quickstart %s passed: replies, restart, and persisted AOF checked.\n' "$mode"
