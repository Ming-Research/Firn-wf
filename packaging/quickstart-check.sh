#!/usr/bin/env bash
# Run the user-facing commands literally; keep test machinery out of the guide.
set -euo pipefail
if [[ $# -lt 1 || $# -gt 2 || ( $1 != tarball && $1 != image ) ]]; then
    printf 'Usage: %s tarball [ARCHIVE.tar.gz] | image [DOCKER_IMAGE]\n' "$0" >&2
    exit 2
fi
mode=$1
input=${2:-}
public=no
[[ $# != 1 ]] || public=yes
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
doc=${QUICKSTART_DOC:-$repo/docs/quickstart.md}
scratch=$(mktemp -d)
http_pid=
probe_name="firn-quickstart-inspect-$$"
cleanup() {
    result=$?
    trap - EXIT
    if [[ $result != 0 ]]; then
        printf 'Quickstart %s failed (exit %s).\n' "$mode" "$result" >&2
        for log in "$scratch"/http.log "$scratch"/output.log "$scratch"/error.log "$scratch"/work/*.log; do
            if [[ -f $log ]]; then
                printf '\n%s\n' "$(basename "$log")" >&2
                tail -n 80 "$log" >&2
            fi
        done
        if [[ -f $scratch/container.owned ]]; then command docker logs firn-quickstart >&2 2>/dev/null || true; fi
    fi
    if [[ -n $http_pid ]]; then
        kill "$http_pid" 2>/dev/null || true
        wait "$http_pid" 2>/dev/null || true
    fi
    if [[ -f $scratch/probe.owned ]]; then command docker rm -f "$probe_name" >/dev/null 2>&1 || true; fi
    if [[ -f $scratch/container.owned ]]; then command docker rm -f firn-quickstart >/dev/null 2>&1 || true; fi
    if [[ -f $scratch/volume.owned ]]; then command docker volume rm firn-data >/dev/null 2>&1 || true; fi
    rm -rf "$scratch"
    return "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Named stages make deletion, duplication, reordering, or malformed fences fail
# before execution. Their bodies are copied unchanged, including newlines.
python3 - "$doc" "$mode" "$scratch" <<'PY'
import pathlib, sys
source, mode, scratch = sys.argv[1:]
expected = ['tarball-download', 'tarball-start', 'image-start', 'common-write',
            'tarball-stop', 'image-stop', 'tarball-restart', 'image-restart',
            'common-read', 'tarball-final-stop', 'image-final-stop']
blocks, active, body = [], None, []
for number, line in enumerate(pathlib.Path(source).read_text().splitlines(keepends=True), 1):
    stripped = line.rstrip('\r\n')
    if stripped.startswith('```'):
        if active is not None:
            if stripped != '```':
                raise SystemExit(f'{source}:{number}: malformed closing fence')
            if not any(part.strip() for part in body):
                raise SystemExit(f'{source}:{number}: empty quickstart block')
            blocks.append((active, ''.join(body)))
            active, body = None, []
        else:
            prefix = '```sh quickstart-'
            if not stripped.startswith(prefix) or stripped[len(prefix):] not in expected:
                raise SystemExit(f'{source}:{number}: invalid quickstart fence')
            active = stripped[len(prefix):]
    elif active is not None:
        body.append(line)
if active is not None:
    raise SystemExit(f'{source}: unclosed quickstart block')
if [name for name, _ in blocks] != expected:
    raise SystemExit(f'{source}: missing, extra, or reordered quickstart stages')
selected = []
for name, body in blocks:
    if name.startswith((mode + '-', 'common-')):
        pathlib.Path(scratch, name + '.sh').write_text(body)
        selected.append(name)
pathlib.Path(scratch, 'stages').write_text('\n'.join(selected) + '\n')
PY

if [[ $# == 1 ]]; then
    # Public checks exercise the documented defaults, regardless of caller env.
    unset FIRN_TAG FIRN_RELEASE_URL FIRN_IMAGE
elif [[ $mode == tarball ]]; then
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
import functools, http.server, pathlib, sys
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
fi

if [[ $mode == image ]]; then
    # Never adopt, stop, or delete somebody else's container or data volume.
    if command docker container inspect firn-quickstart >/dev/null 2>&1 || command docker volume inspect firn-data >/dev/null 2>&1; then
        printf 'firn-quickstart or firn-data already exists; use a fresh Docker runner.\n' >&2
        exit 1
    fi
    if [[ $# == 1 ]]; then
        timeout --kill-after=5s 120s bash -c 'command docker pull ghcr.io/ming-research/firn:latest'
    fi
    image_ref=${FIRN_IMAGE:-ghcr.io/ming-research/firn:latest}
    command docker image inspect "$image_ref" >"$scratch/image.json"
    python3 - "$scratch/image.json" <<'PY'
import json, sys
image = json.load(open(sys.argv[1]))[0]
config = image['Config']
if config.get('User') != '10001:10001' or config.get('WorkingDir') != '/data':
    raise SystemExit('Image must run as UID/GID 10001 with /data as its working directory')
argv = (config.get('Entrypoint') or []) + (config.get('Cmd') or [])
for option, value in [('--appendonly', 'yes'), ('--bind', '0.0.0.0')]:
    if (option, value) not in list(zip(argv, argv[1:])):
        raise SystemExit(f'Image default command must include {option} {value}')
if image.get('Os') != 'linux' or image.get('Architecture') != 'amd64':
    raise SystemExit('Image must be linux/amd64')
PY
fi

# This prelude is harness-only. Wrappers execute the supplied arguments and
# record observations; they never supply a missing SET, stop, or restart.
cat >"$scratch/child.sh" <<'SH'
set -euo pipefail
scratch=$1
mode=$2
stage=preflight
server_pid=
mkdir "$scratch/work"
cd "$scratch/work"
event() {
    python3 - "$scratch/events.jsonl" "$stage" "$@" <<'PY'
import json, sys
with open(sys.argv[1], 'a') as output:
    output.write(json.dumps(sys.argv[2:]) + '\n')
PY
}
port_free() {
    python3 - <<'PY'
import socket
try:
    with socket.socket() as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(('127.0.0.1', 6379))
        listener.listen(1)
except OSError:
    raise SystemExit('port 6379 is in use — is a local Redis running?')
PY
    event port-free
}
cleanup_server() {
    result=$?
    trap - EXIT
    if [[ -n $server_pid ]] && kill -0 "$server_pid" 2>/dev/null; then
        kill -TERM "$server_pid" 2>/dev/null || true
        for attempt in {1..20}; do
            kill -0 "$server_pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    return "$result"
}
trap cleanup_server EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
ready() {
    timeout --kill-after=2s 20s bash -c '
        while :; do
            reply=$(timeout 1s redis-cli --raw -h 127.0.0.1 -p 6379 PING 2>/dev/null || true)
            [[ $reply != PONG ]] || exit 0
            sleep 0.2
        done'
    event ready
}
redis-cli() {
    local reply
    reply=$(timeout 3s command-redis-cli "$@") || return "$?"
    printf '%s\n' "$reply"
    event redis-cli "$reply" "$@"
    # Validate the reply to the actual command; event validation below checks
    # arguments and stage, so changing GET/SET cannot select a different test.
    case ${5:-} in
        PING)
            if [[ $reply != PONG ]]; then printf 'quickstart PING expected PONG, received %s\n' "$reply" >&2; return 1; fi
            ;;
        SET)
            if [[ $reply != OK ]]; then printf 'quickstart SET expected OK, received %s\n' "$reply" >&2; return 1; fi
            ;;
        GET)
            if [[ $reply != survives-restart ]]; then printf 'quickstart GET expected survives-restart, received %s\n' "$reply" >&2; return 1; fi
            ;;
        SHUTDOWN)
            [[ $mode == tarball && -n $server_pid ]]
            wait "$server_pid"
            event exited "$server_pid" 0
            server_pid=
            port_free
            ;;
    esac
}
# timeout needs an executable, while command bypasses the Bash wrapper. A tiny
# launcher retains all original client arguments and only requests raw output.
cat >"$scratch/command-redis-cli" <<'CLIENT'
#!/usr/bin/env bash
exec redis-cli --raw "$@"
CLIENT
chmod +x "$scratch/command-redis-cli"
export PATH="$scratch:$PATH"
curl() {
    command curl "$@" || return "$?"
    event curl "$@"
}
sha256sum() {
    command sha256sum "$@" || return "$?"
    event sha256sum "$@"
}
tar() {
    command tar "$@" || return "$?"
    event tar "$@"
}
docker() {
    local status=0 output
    if [[ ${1:-} == run ]]; then
        # Assert resource names before execution, without rewriting any argument.
        [[ $stage == image-start && ${2:-} == --detach && ${3:-} == --name && ${4:-} == firn-quickstart && ${7:-} == -v && ${8:-} == firn-data:/data ]]
        if command docker container inspect firn-quickstart >/dev/null 2>&1 || command docker volume inspect firn-data >/dev/null 2>&1; then
            printf 'Quickstart Docker resources already exist.\n' >&2
            return 1
        fi
    elif [[ ${1:-} == stop || ${1:-} == start ]]; then
        [[ -f $scratch/container.owned && ${*: -1} == firn-quickstart ]]
    else
        printf 'Unexpected documented Docker operation: %s\n' "${1:-}" >&2
        return 1
    fi
    output=$(command docker "$@") || status=$?
    if [[ ${1:-} == run ]]; then
        # Docker may create resources even if starting the container fails.
        if command docker container inspect firn-quickstart >/dev/null 2>&1; then touch "$scratch/container.owned"; fi
        if command docker volume inspect firn-data >/dev/null 2>&1; then touch "$scratch/volume.owned"; fi
    fi
    [[ $status == 0 ]] || return "$status"
    printf '%s\n' "$output"
    event docker "$@"
    if [[ ${1:-} == stop ]]; then
        local exit_code container_id
        exit_code=$(command docker inspect --format '{{.State.ExitCode}}' firn-quickstart)
        container_id=$(command docker inspect --format '{{.Id}}' firn-quickstart)
        [[ $exit_code == 0 ]]
        event container-exited "$container_id" "$exit_code"
        command docker logs firn-quickstart >"$scratch/work/$stage.log" 2>&1
        port_free
    elif [[ ${1:-} == run || ${1:-} == start ]]; then
        event container-started "$(command docker inspect --format '{{.Id}}' firn-quickstart)"
    fi
}
port_free
while IFS= read -r stage; do
    event stage
    source "$scratch/$stage.sh"
    case $stage in
        tarball-download) event release "$FIRN_TAG" "$FIRN_RELEASE_URL" "$archive" ;;
        tarball-start|tarball-restart)
            server_pid=$!
            kill -0 "$server_pid"
            event process-started "$server_pid"
            ready
            ;;
        image-start|image-restart) ready ;;
    esac
done <"$scratch/stages"
stage=final-check
port_free
SH

timeout --kill-after=5s 180s bash "$scratch/child.sh" "$scratch" "$mode" \
    >"$scratch/output.log" 2>"$scratch/error.log"

# Verify observations, not shell spellings. A missing or substituted doc step
# must fail even if the final AOF happens to contain the expected value.
python3 - "$scratch/events.jsonl" "$mode" "$scratch/stages" "${image_ref:-}" "$public" <<'PY'
import json, pathlib, sys
rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
mode = sys.argv[2]
stages = pathlib.Path(sys.argv[3]).read_text().splitlines()
def observed(stage, kind):
    return [row[2:] for row in rows if row[:2] == [stage, kind]]
def exact(stage, kind, expected):
    actual = observed(stage, kind)
    if actual != expected:
        raise SystemExit(f'{stage}: expected {kind} {expected!r}, observed {actual!r}')
if [row[0] for row in rows if row[1] == 'stage'] != stages:
    raise SystemExit('Quickstart did not execute every stage in order')
base = ['-h', '127.0.0.1', '-p', '6379']
exact('common-write', 'redis-cli', [['PONG', *base, 'PING'],
      ['OK', *base, 'SET', 'firn:quickstart', 'survives-restart'],
      ['survives-restart', *base, 'GET', 'firn:quickstart']])
exact('common-read', 'redis-cli', [['survives-restart', *base, 'GET', 'firn:quickstart']])
exact('preflight', 'port-free', [[]])
exact('final-check', 'port-free', [[]])
for phase in ['start', 'restart']:
    exact(mode + '-' + phase, 'ready', [[]])
for phase in ['stop', 'final-stop']:
    exact(mode + '-' + phase, 'port-free', [[]])
if mode == 'tarball':
    for phase in ['stop', 'final-stop']:
        exact('tarball-' + phase, 'redis-cli', [['', *base, 'SHUTDOWN', 'NOSAVE']])
        exits = observed('tarball-' + phase, 'exited')
        if len(exits) != 1 or exits[0][1] != '0':
            raise SystemExit(f'{phase}: missing successful process wait')
    pids = [observed('tarball-' + phase, 'process-started') for phase in ['start', 'restart']]
    if any(len(found) != 1 for found in pids) or pids[0] == pids[1]:
        raise SystemExit('Tarball must start two distinct server processes')
    for start, stop in [('start', 'stop'), ('restart', 'final-stop')]:
        if observed('tarball-' + start, 'process-started')[0][0] != observed('tarball-' + stop, 'exited')[0][0]:
            raise SystemExit('SHUTDOWN must wait for the server that the document started')
    releases = observed('tarball-download', 'release')
    if len(releases) != 1 or len(releases[0]) != 3:
        raise SystemExit('Missing release download evidence')
    tag, url, archive = releases[0]
    if archive != f'firn-{tag}-linux-x86_64.tar.gz':
        raise SystemExit('Archive filename must match the selected release tag')
    flags = ['--fail', '--silent', '--show-error', '--location']
    downloads = [[*flags, f'{url}/{name}', '--output', name]
                 for name in [archive, archive + '.sha256']]
    if sys.argv[5] == 'yes':
        if url != f'https://github.com/Ming-Research/Firn-wf/releases/download/{tag}':
            raise SystemExit('Public quickstart must download from GitHub Releases')
        downloads.insert(0, [*flags, 'https://api.github.com/repos/Ming-Research/Firn-wf/releases/latest'])
    exact('tarball-download', 'curl', downloads)
    exact('tarball-download', 'sha256sum', [['--check', archive + '.sha256']])
    exact('tarball-download', 'tar', [['-xzf', archive]])
else:
    exact('image-start', 'docker', [['run', '--detach', '--name', 'firn-quickstart',
          '-p', '127.0.0.1:6379:6379', '-v', 'firn-data:/data', sys.argv[4]]])
    exact('image-restart', 'docker', [['start', 'firn-quickstart']])
    for phase in ['stop', 'final-stop']:
        exact('image-' + phase, 'docker', [['stop', '--time', '20', 'firn-quickstart']])
        exits = observed('image-' + phase, 'container-exited')
        if len(exits) != 1 or exits[0][1] != '0':
            raise SystemExit(f'{phase}: missing successful container exit')
    starts = [observed('image-' + phase, 'container-started') for phase in ['start', 'restart']]
    if any(len(found) != 1 for found in starts) or starts[0] != starts[1]:
        raise SystemExit('Docker must restart the same stopped container')
    container_id = starts[0][0][0]
    if len(container_id) != 64 or any(char not in '0123456789abcdef' for char in container_id):
        raise SystemExit('Missing actual container ID')
    for phase in ['stop', 'final-stop']:
        if observed('image-' + phase, 'container-exited')[0][0] != container_id:
            raise SystemExit('Docker must stop the container the document started')
PY

if [[ $mode == image ]]; then
    # A separate stopped container reads the named volume independently of the
    # documented container's writable layer; it runs no alternative server.
    command docker create --name "$probe_name" --mount type=volume,src=firn-data,dst=/data "$image_ref" >/dev/null
    touch "$scratch/probe.owned"
    command docker cp "$probe_name:/data/appendonlydir" "$scratch/aof"
    aof=$scratch/aof
else
    aof=$scratch/work/data/appendonlydir
fi
python3 - "$aof" <<'PY'
import pathlib, sys
directory = pathlib.Path(sys.argv[1])
manifest = directory / 'appendonly.aof.manifest'
if not manifest.is_file() or not manifest.stat().st_size:
    raise SystemExit('AOF manifest is missing or empty')
bases, increments = [], []
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
    if fields[5] == 'i': increments.append(path)
    elif fields[5] == 'b': bases.append(path)
    else: raise SystemExit(f'Unexpected AOF file type: {line}')
if not bases or not increments or not any(path.stat().st_size for path in increments):
    raise SystemExit('AOF must contain a base and a nonempty incremental file')
data = b''.join(path.read_bytes() for path in increments)
for value in [b'firn:quickstart', b'survives-restart']:
    record = b'$' + str(len(value)).encode() + b'\r\n' + value + b'\r\n'
    if record not in data:
        raise SystemExit(f'Persisted AOF lacks expected RESP value: {value!r}')
PY
cat "$scratch/output.log"
printf 'Quickstart %s passed: literal commands, replies, restart, port, and AOF checked.\n' "$mode"
