# Run a packaged firn

Use Ubuntu 24.04 on x86-64 Linux for the binary tarball, or a Linux Docker
daemon for the linux/amd64 container image. You need Bash, GNU coreutils
(including `timeout` and `sha256sum`), and a free local port 6379.
The image path also needs Docker installed and permission to use its daemon.
These examples use the distribution's `redis-cli` as a client; it is not the
Redis reference server used by firn's compatibility checks.

Run the common blocks and **one** path, tarball or image, in document order
in the same Bash shell. The exercise creates a fresh writable directory and
leaves its data there. The image path leaves its named volume too. Expected
replies are `PONG`, `OK`, and `survives-restart`; each is asserted because
`redis-cli` can exit successfully after a server error.

## Prepare the client and workspace

```sh quickstart-common
set -euo pipefail
if ! command -v curl >/dev/null || ! command -v redis-cli >/dev/null || ! command -v python3 >/dev/null; then
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends curl redis-tools python3
fi
QUICKSTART_WORKDIR=${QUICKSTART_WORKDIR:-$(mktemp -d "${TMPDIR:-/tmp}/firn-quickstart.XXXXXX")}
mkdir -p "$QUICKSTART_WORKDIR"
[[ -z $(ls -A "$QUICKSTART_WORKDIR") ]]
cd "$QUICKSTART_WORKDIR"
QUICKSTART_WORKDIR=$PWD
python3 - <<'PY'
import socket
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 6379))
PY
server_pid=
container_name=
cleanup() {
    result=$?
    trap - EXIT
    if [[ -n $server_pid ]] && kill -0 "$server_pid" 2>/dev/null; then
        kill -TERM "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    if [[ -n $container_name ]]; then
        docker rm -f "$container_name" >/dev/null 2>&1 || true
    fi
    return "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
wait_for_firn() {
    deadline=$((SECONDS + 19))
    while (( SECONDS < deadline )); do
        reply=$(timeout 1s redis-cli -h 127.0.0.1 -p 6379 --raw PING 2>/dev/null || true)
        if [[ $reply == PONG ]]; then return 0; fi
        sleep 0.2
    done
    printf 'firn did not answer PONG within 20 seconds\n' >&2
    return 1
}
```

## Start: binary tarball path

The latest published release is selected automatically. Set `FIRN_TAG` in
your shell before running this block to select a published tag explicitly.
The `.sha256` file checks the downloaded bytes against the release's
checksum. The extracted folder also contains firn's license and the Redis
license notice beside the binary.

```sh quickstart-tarball
FIRN_TAG=${FIRN_TAG:-$(curl --fail --silent --show-error --location https://api.github.com/repos/Ming-Research/Firn-wf/releases/latest | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')}
FIRN_RELEASE_URL=${FIRN_RELEASE_URL:-https://github.com/Ming-Research/Firn-wf/releases/download/$FIRN_TAG}
archive="firn-$FIRN_TAG-linux-x86_64.tar.gz"
curl --fail --silent --show-error --location "$FIRN_RELEASE_URL/$archive" --output "$archive"
curl --fail --silent --show-error --location "$FIRN_RELEASE_URL/$archive.sha256" --output "$archive.sha256"
sha256sum --check "$archive.sha256"
tar -xzf "$archive"
FIRN_BIN="$QUICKSTART_WORKDIR/firn-$FIRN_TAG-linux-x86_64/firn"
mkdir data
cd data
"$FIRN_BIN" --port 6379 --bind 127.0.0.1 --appendonly yes >../first.log 2>&1 &
server_pid=$!
```

## Start: Docker image path

The image's default command enables AOF with its fixed everysec sync policy
and listens inside the container on all IPv4 interfaces. The published port
is bound only to the host's loopback address. The image runs as a nonroot
user, with `/data` as its working directory. A named volume keeps that
directory when the container is removed. Set `FIRN_IMAGE` before this block
to select a published tag instead of `latest`.

```sh quickstart-image
FIRN_IMAGE=${FIRN_IMAGE:-ghcr.io/ming-research/firn:latest}
new_container_name=${FIRN_QUICKSTART_NAME:-firn-quickstart-$$}
if docker container inspect "$new_container_name" >/dev/null 2>&1; then
    printf 'Container name already exists: %s\n' "$new_container_name" >&2
    exit 1
fi
container_name=$new_container_name
volume_name="$container_name-data"
docker volume create "$volume_name"
docker run --detach --name "$container_name" --publish 127.0.0.1:6379:6379 \
    --mount "type=volume,src=$volume_name,dst=/data" "$FIRN_IMAGE" >first-container.id
```

## Write and read a value: both paths

```sh quickstart-common
wait_for_firn
reply=$(redis-cli -h 127.0.0.1 -p 6379 --raw PING)
[[ $reply == PONG ]]
printf '%s\n' "$reply"
reply=$(redis-cli -h 127.0.0.1 -p 6379 --raw SET firn:quickstart survives-restart)
[[ $reply == OK ]]
printf '%s\n' "$reply"
reply=$(redis-cli -h 127.0.0.1 -p 6379 --raw GET firn:quickstart)
[[ $reply == survives-restart ]]
printf '%s\n' "$reply"
```

## Stop and restart: binary tarball path

`SHUTDOWN NOSAVE` performs an orderly AOF stop; `NOSAVE` skips RDB snapshots,
which firn does not implement. Waiting for the retained process ID confirms
the server exited before the new one starts in the same data directory.

```sh quickstart-tarball
redis-cli -h 127.0.0.1 -p 6379 SHUTDOWN NOSAVE
wait "$server_pid"
server_pid=
"$FIRN_BIN" --port 6379 --bind 127.0.0.1 --appendonly yes >../second.log 2>&1 &
server_pid=$!
```

## Stop and recreate: Docker image path

Docker sends SIGTERM, which requests firn's orderly AOF stop. Check the
container's exit status before removing it, then start a new container with
the same volume. This checks persistence beyond the first container's life.

```sh quickstart-image
docker stop --time 20 "$container_name"
[[ $(docker inspect --format '{{.State.ExitCode}}' "$container_name") == 0 ]]
docker logs "$container_name" >first.log 2>&1
docker rm "$container_name"
docker run --detach --name "$container_name" --publish 127.0.0.1:6379:6379 \
    --mount "type=volume,src=$volume_name,dst=/data" "$FIRN_IMAGE" >second-container.id
```

## Read after restart: both paths

The value must still be `survives-restart` after the server has loaded its
AOF. No second `SET` is needed.

```sh quickstart-common
wait_for_firn
reply=$(redis-cli -h 127.0.0.1 -p 6379 --raw GET firn:quickstart)
[[ $reply == survives-restart ]]
printf '%s\n' "$reply"
```

## Final stop: binary tarball path

```sh quickstart-tarball
redis-cli -h 127.0.0.1 -p 6379 SHUTDOWN NOSAVE
wait "$server_pid"
server_pid=
cd "$QUICKSTART_WORKDIR"
```

## Final stop: Docker image path

```sh quickstart-image
docker stop --time 20 "$container_name"
[[ $(docker inspect --format '{{.State.ExitCode}}' "$container_name") == 0 ]]
docker logs "$container_name" >second.log 2>&1
docker rm "$container_name"
container_name=
```

```sh quickstart-common
printf 'Data retained in %s\n' "$QUICKSTART_WORKDIR"
```

The tarball stores its files below `data/appendonlydir`. The image stores
them below `/data/appendonlydir` in its named volume. Everysec replies can
precede the next sync; an orderly stop makes a final append and sync attempt.
A storage error can still prevent durability. See
[firn's persistence and shutdown behavior](../firn/README.md).

## Translate a small redis.conf

firn accepts command-line options, not a Redis configuration file. Unknown
options fail startup. These are the mappings for a small standalone setup;
they do not imply support for every Redis setting.

| Redis setting | firn equivalent or limitation |
| --- | --- |
| `port 6379` | `--port 6379` |
| `bind 127.0.0.1` | `--bind 127.0.0.1`; one IPv4 address, or `*` for all |
| `appendonly yes` / `no` | `--appendonly yes` / `no`; off by default |
| `appendfsync everysec` | Fixed everysec policy; no `--appendfsync`, `always`, or `no` policy |
| `dir /path/to/data` | Change the working directory before launching; no `--dir` |
| `appendfilename appendonly.aof` | `--appendfilename appendonly.aof`; naming it alone does not enable AOF |
| `maxmemory 64mb` | Runtime `CONFIG SET maxmemory 64mb` only; resets on restart, no `--maxmemory` |
| `maxmemory-policy allkeys-lru` | Runtime `CONFIG SET maxmemory-policy allkeys-lru` only; resets on restart, no corresponding launch option |
| `maxclients 100` | Unsupported; `--clients 100` counts all accepted clients over the server's lifetime and then stops, so it is not equivalent |
| `timeout 60` | `--timeout 60`; zero means no idle limit |
| `requirepass secret` | `--requirepass secret` |
| `save ""` | Empty snapshot schedule only; no RDB snapshots or `--save` |

See [firn's command and option reference](../firn/README.md#build-and-run)
for its remaining options and compatibility limits.

## What release CI checks

`packaging/quickstart-check.sh` extracts these tagged command blocks in
document order and executes their literal contents for each path. The
tarball check serves the newly built archive and checksum over a local HTTP
mirror; the image check supplies the newly built Docker tag. Both check the
replies, stop/restart sequence and persisted AOF independently of the prose.
Thus prerelease CI checks the download procedure and image behavior without
claiming those artifacts are already publicly available. Public GitHub
download and registry availability need a separate check after publication.
