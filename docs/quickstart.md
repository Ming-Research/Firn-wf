# Run a packaged firn

Use Ubuntu 24.04 on x86-64 Linux for the binary tarball. You need Bash,
`curl`, Python 3, `redis-cli`, `tar`, and GNU `sha256sum`. The container path
also needs Docker with a Linux daemon that runs linux/amd64 images. Port
6379 must be free; stop any local Redis server using it before this exercise.
The distribution's `redis-cli` is a client here, not the Redis reference
server used by firn's compatibility checks.

Open Bash in an empty, writable directory. Run the common blocks and **one**
path, tarball or image, in document order in that same shell. Both paths
write a value, stop the server, restart it, and read the value back. The
expected replies are `PONG`, `OK`, and `survives-restart`.

## Download: binary tarball path

The latest published release is selected automatically. Set `FIRN_TAG`
before this block to select a published tag explicitly. The checksum checks
the downloaded bytes against the release's `.sha256` file. The extracted
folder includes firn's license and the Redis license notice.

```sh quickstart-tarball-download
FIRN_TAG=${FIRN_TAG:-$(curl --fail --silent --show-error --location https://api.github.com/repos/Ming-Research/Firn-wf/releases/latest | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')}
archive="firn-$FIRN_TAG-linux-x86_64.tar.gz"
curl --fail --silent --show-error --location "https://github.com/Ming-Research/Firn-wf/releases/download/$FIRN_TAG/$archive" --output "$archive"
curl --fail --silent --show-error --location "https://github.com/Ming-Research/Firn-wf/releases/download/$FIRN_TAG/$archive.sha256" --output "$archive.sha256"
sha256sum --check "$archive.sha256"
tar -xzf "$archive"
FIRN_BIN="$PWD/firn-$FIRN_TAG-linux-x86_64/firn"
```

## Start: binary tarball path

AOF persistence uses the fixed everysec sync policy. The server runs in the
background, with its output in `first.log`. Wait a moment for it to listen
before sending the client commands below.

```sh quickstart-tarball-start
mkdir data
cd data
"$FIRN_BIN" --port 6379 --bind 127.0.0.1 --appendonly yes >../first.log 2>&1 &
```

## Start: Docker image path

The image's default command enables AOF and listens on all IPv4 addresses
inside the container **without a password**. Publish it only on
`127.0.0.1`, as below, or set a password before exposing it to other hosts.
Passing `--requirepass` puts the password in firn's command line, visible
in `ps`; runtime `CONFIG SET requirepass` in an already open client session
avoids command-line exposure.
The image runs as UID 10001 with `/data` as its working directory. This
example keeps data in the named volume `firn-data`; a bind-mounted host
directory must be writable by UID 10001. Choose an unused container name
and volume name for a new exercise. Set `FIRN_IMAGE` before this block to
select a published tag instead of `latest`.

Wait a moment for the container to listen before sending the client commands.

```sh quickstart-image-start
docker run --detach --name firn-quickstart -p 127.0.0.1:6379:6379 \
    -v firn-data:/data "${FIRN_IMAGE:-ghcr.io/ming-research/firn:latest}"
```

## Write and read a value: both paths

These commands should print `PONG`, `OK`, and `survives-restart`, in order.

```sh quickstart-common-write
redis-cli -h 127.0.0.1 -p 6379 PING
redis-cli -h 127.0.0.1 -p 6379 SET firn:quickstart survives-restart
redis-cli -h 127.0.0.1 -p 6379 GET firn:quickstart
```

## Stop: binary tarball path

`SHUTDOWN NOSAVE` performs an orderly AOF stop; `NOSAVE` skips RDB snapshots,
which firn does not implement. Wait for the server to exit before restarting.

```sh quickstart-tarball-stop
redis-cli -h 127.0.0.1 -p 6379 SHUTDOWN NOSAVE
```

## Stop: Docker image path

Docker sends SIGTERM, which requests firn's orderly AOF stop.

```sh quickstart-image-stop
docker stop --time 20 firn-quickstart
```

## Restart: binary tarball path

Stay in the same `data` directory and use the same options. Wait a moment
for the restarted server to listen.

```sh quickstart-tarball-restart
"$FIRN_BIN" --port 6379 --bind 127.0.0.1 --appendonly yes >../second.log 2>&1 &
```

## Restart: Docker image path

The stopped container keeps its configuration and named volume. Wait a
moment for it to listen again.

```sh quickstart-image-restart
docker start firn-quickstart
```

## Read after restart: both paths

The reply should still be `survives-restart`; there is no second `SET`.

```sh quickstart-common-read
redis-cli -h 127.0.0.1 -p 6379 GET firn:quickstart
```

## Final stop: binary tarball path

```sh quickstart-tarball-final-stop
redis-cli -h 127.0.0.1 -p 6379 SHUTDOWN NOSAVE
```

## Final stop: Docker image path

```sh quickstart-image-final-stop
docker stop --time 20 firn-quickstart
```

The tarball leaves its data below `data/appendonlydir`. Docker leaves a
stopped container and keeps the AOF below `/data/appendonlydir` in
`firn-data`, which also survives container removal. Everysec replies can
precede the next sync; an orderly stop makes a final append and sync attempt.
A storage error can still prevent durability. See
[firn's persistence and shutdown behavior](../firn/README.md).

## Translate a small redis.conf

firn accepts command-line options, not a Redis configuration file. Unknown
options fail startup. These mappings cover a small standalone setup.

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
| `requirepass secret` | `--requirepass secret`, or runtime `CONFIG SET requirepass secret`, which resets on restart; see the [password guidance](#start-docker-image-path) above |
| `save ""` | Empty snapshot schedule only; no RDB snapshots or `--save` |

See [firn's command and option reference](../firn/README.md#build-and-run)
for its remaining options and compatibility limits.

## What release CI checks

`packaging/quickstart-check.sh` executes the tagged commands literally and
checks their replies, lifecycle sequence, free port after stopping, and
persisted AOF. Readiness polling, assertions, time limits, temporary
workspaces, and cleanup belong to that harness.

Before publication, the checker redirects the documented GitHub downloads
to the local build artifacts and supplies a local Docker tag. After a
non-prerelease publication, it checks the default latest-release lookup,
GitHub downloads, and an anonymous pull of the default GHCR image; the
selected release must match the pushed tag. Prerelease checks stay local.
A manual dispatch can check the defaults against the existing latest
published release without requiring a particular tag.
