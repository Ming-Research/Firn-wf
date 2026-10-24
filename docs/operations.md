# Operating firn

This guide covers firn's current startup, connection and append-only
persistence behavior. firn remains a research server; check the
[command reference](../firn/README.md) against the application you intend to
run. The gate executes every marked configuration and shell block below
literally, checking the exit status and output stated in its fence; a fence
tagged `untested-example` instead is not run, and the gate rejects any other
fence.

## Run as a service

Install the firn executable and Redis 7.2.16's `redis-cli` on `PATH`. Create
a writable data directory before starting firn, then run all examples below
from that directory. firn creates its `appendonlydir` subdirectory when AOF
is enabled; it does not create the data directory itself.

Save this configuration as `redis.conf`. It binds only to IPv4 loopback,
requires a password, enables AOF, sets `maxmemory` to 128 MiB with
`noeviction`, admits at most 10000 simultaneous clients and closes idle
clients after 300 seconds. Writes needing eviction are refused; `maxmemory`
is not a hard process-memory ceiling. Replace the example password with your
deployment's secret in both the configuration and client invocations.

```conf firn-check file=redis.conf
port 6379
bind 127.0.0.1
requirepass ops-example-secret
appendonly yes
appendfilename appendonly.aof
maxmemory 134217728
maxmemory-policy noeviction
maxclients 10000
timeout 300
```

Start firn in the foreground. The shell sets its open-file limit to 16384
before starting the server; this requires a hard limit of at least 16384.
The start command stays running until shutdown and then exits with status 0
in this example, with no output. Use another terminal in the same data
directory for the following client commands.

```sh firn-check start exit=0
ulimit -n 16384
firn redis.conf
```

This authenticated health check exits 0 and prints `PONG`:

```sh firn-check exit=0 stdout=PONG
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 PING
```

A service manager must set the process's working directory to the data
directory and run firn in the foreground. With systemd, use
`WorkingDirectory=` for that directory, `ExecStart=` with the installed
executable's absolute path and the relative configuration filename, and
`LimitNOFILE=` for the descriptor limit. These systemd settings are guidance;
the gate exercises the foreground invocation above, not systemd.

Configuration filenames must be relative to firn's working directory.
Absolute configuration paths are refused, as are all `include` directives
and every `dir` value except `.` or `./`. An absolute executable path is
unaffected by this restriction. Keep the configuration in the data directory
and select that directory before launching firn. These interim rules expose
the Whitefoot path-support gap, [board item `firn-req-host-paths`][board].

## Configuration refusals

firn refuses unknown directives, wrong argument counts, malformed quoting
and invalid values. It prints `FATAL CONFIG FILE ERROR` on stderr, with the
source line, its line number and the reason, then exits 1 before serving
clients. A missing configuration file or a refused configuration path also
exits 1, with its own startup diagnostic.

The [invocation reference](../firn/README.md#build-and-run) lists the
accepted directives, their validation and the common Redis directives that
are still refused.

For example, save this deliberately unsupported directive in `refused.conf`:

```conf firn-check file=refused.conf
appendfsync everysec
```

This invocation exits 1, produces no stdout and prints the fatal
configuration error on stderr. `appendfsync everysec` is refused even though
every-second syncing is firn's fixed persistence policy.

```sh firn-check exit=1 stderr=FATAL CONFIG FILE ERROR
firn refused.conf
```

## Exit statuses

| Status | Current meaning and operator action |
| --- | --- |
| 0 | After startup, an orderly `SHUTDOWN`, SIGTERM or SIGINT drains clients and attempts a final AOF append and sync before exiting. A SIGTERM or SIGINT during AOF replay also exits with status 0, at once and with `appendonlydir` as the replay found it. A failed final append or sync can still lose acknowledged writes; status 0 does not certify durability. |
| 1 | Invalid invocation or configuration, an unreadable configuration file, a refused configuration path, or a configuration with no enabled listener. Read stderr and correct the startup settings. |
| 3 | firn cannot register delivery of stop signals, which it does before replaying the AOF, or cannot open its TCP listener. Check the listening address, port availability and host facilities; these failures currently have no diagnostic. |
| 4 | Any AOF startup refusal, including a missing or malformed listed file, unreadable persistence state, or failure to prepare the log for appending or truncate its incomplete tail. firn has not begun listening; preserve the files and investigate or restore a known-good backup. This path currently emits no diagnostic. |

These are the current implementation's statuses: AOF startup refusals use
4, not 1. A SIGTERM or SIGINT during AOF replay ends firn at the replay's
next check, made at most 1024 commands later, as Redis 7.2.16 exits while it
loads. Host termination, including a second SIGTERM or SIGINT after the
first stop request, need not produce one of these statuses. See the
[abnormal-stop audit](../research/investigations/firn/abnormal-stops.md) and
[safety boundaries](safety.md) for resource failures.

## Connections and descriptor limits

`maxclients` defaults to 10000. A new connection beyond the configured limit
receives `-ERR max number of clients reached` followed by CRLF, as a
best-effort send, and closes without serving a command. `INFO clients`
reports `maxclients` and `connected_clients`. `INFO stats` reports
`rejected_connections`, counting limit refusals since startup or the last
`CONFIG RESETSTAT`; rejected clients do not increase admitted-connection
counts. Lowering the live limit leaves existing clients connected.

firn neither raises the operating system's file-descriptor limit nor reduces
`maxclients` to fit it. Set the limit yourself in the shell that launches
firn, as in the start block, or through the service manager. Reserve
descriptors for persistence files, listeners and other process needs in
addition to client sockets. If the hard limit cannot support the example,
raise it through the service configuration or reduce the client limit with
headroom. The configured client limit alone does not promise that the host
can supply enough descriptors. When the process runs out of descriptors,
firn logs `Accepting client connection: Too many open files` to stderr, at
most once a second, and keeps listening; it accepts again once descriptors
free up, up to 100 ms after they do. Whitefoot's missing resource-limit
facility is tracked by [board item `firn-req-fd-limit`][board].

## Persistence and recovery

With the example configuration, AOF state lives in `appendonlydir` below
the working directory. A fresh dataset starts with
`appendonly.aof.1.base.aof`, `appendonly.aof.1.incr.aof` and
`appendonly.aof.manifest`. The manifest selects the base and incremental
files to replay; filenames and sequence numbers change on rewrite. Base
files contain commands, not RDB snapshots.

firn uses only the `appendfsync everysec` policy: writes are appended
asynchronously and the file is synced once a second. A successful write
reply does not guarantee that a power loss will preserve the write. Known
AOF append or sync failures refuse writes and `PING` with `MISCONF` while
reads remain available. `INFO persistence` reports `aof_last_write_status`;
after a sync failure, writes resume only after a recovery rewrite is
installed and its directory is synced.

After a crash, startup replays the selected files before listening. Only
the last file may have an incomplete final record cut off; an incomplete
earlier file, malformed record or missing listed file refuses startup with
status 4. An unfinished `MULTI` block is not applied. If a partial record
leaves that block open, subsequent writes can also disappear on the next
restart until a successful rewrite removes the retained block. Preserve a
copy of damaged state before attempting recovery.

### Rewrite

`BGREWRITEAOF` compacts the current dataset into a new command-format base
while clients continue using the server. The started reply acknowledges
admission, not completion. Memory pressure and I/O failures can abort a
rewrite. `INFO persistence` reports `aof_rewrite_in_progress` and
`aof_last_bgrewrite_status`; an unsuccessful attempt stops automatic rewrites
until a manual rewrite succeeds.

Create a value for the backup demonstration; this exits 0 and prints `OK`:

```sh firn-check exit=0 stdout=OK
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 SET operator:probe saved
```

Start a rewrite; this exits 0 and prints the started reply:

```sh firn-check exit=0 stdout=Background append only file rewriting started
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 BGREWRITEAOF
```

For this small example, wait up to 30 checks, one second apart, for a
completed successful rewrite. This exits 0 with no output on success and
exits nonzero if the check fails or the deadline is reached. A large dataset
may require a longer observation period.

```sh firn-check exit=0
attempt=0
while :; do
    info=$(redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 INFO persistence)
    info=$(printf '%s\n' "$info" | tr -d '\r')
    if printf '%s\n' "$info" | grep -q '^aof_rewrite_in_progress:0$' &&
       printf '%s\n' "$info" | grep -q '^aof_last_bgrewrite_status:ok$'; then
        break
    fi
    attempt=$((attempt + 1))
    test "$attempt" -lt 30
    sleep 1
done
```

### Back up and restore

Stop firn before copying persistence state and wait for the process to
exit. This avoids both writes to the active increment and rewrite changes
to the manifest or its selected files. The
[AOF publication design](../design/firn/aof-rewrite.md) replaces the manifest
before deleting old files; copying an active directory can race those
changes. This guide provides no tested online-backup procedure.

This orderly stop exits 0 with no output; the foreground firn process also
exits 0 in this example:

```sh firn-check stop exit=0
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 SHUTDOWN NOSAVE
```

With firn stopped, copy the whole directory, including the manifest and
every file it lists. The following uses a new `backup` directory and exits
0 with no output:

```sh firn-check exit=0
mkdir backup
cp -R appendonlydir backup/
```

To restore, keep firn stopped and put the backed-up directory back under
the same working directory with the same configured `appendfilename`.
This example preserves the existing directory under a different name and
exits 0 with no output; both destination names must be unused:

```sh firn-check exit=0
mv appendonlydir discarded-appendonlydir
cp -R backup/appendonlydir .
```

Start firn again from the data directory, then query the restored value in
another terminal. The start remains running until the final shutdown and
then exits 0 with no output:

```sh firn-check start exit=0
ulimit -n 16384
firn redis.conf
```

The read exits 0 and prints `saved`:

```sh firn-check exit=0 stdout=saved
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 GET operator:probe
```

End the example with another orderly stop, exiting 0 with no output:

```sh firn-check stop exit=0
redis-cli --raw --no-auth-warning -a ops-example-secret -p 6379 SHUTDOWN NOSAVE
```

The gate checks this stop-copy-restore sequence after a successful rewrite
and verifies the restored value. It does not simulate a power loss during
the backup, validate storage hardware, or exercise service-manager startup.

### Damaged state

A malformed manifest makes startup fail with status 4 and no diagnostic.
This example, run with firn stopped, builds a data directory whose
manifest gives its base file sequence number 0, which firn does not accept,
and starts firn in it. The command exits 4 and prints nothing:

```sh firn-check exit=4
mkdir damaged
cp redis.conf damaged/
cd damaged
mkdir appendonlydir
: > appendonlydir/appendonly.aof.1.base.aof
printf 'file appendonly.aof.1.base.aof seq 0 type b\n' > appendonlydir/appendonly.aof.manifest
firn redis.conf
```

## Logs

firn writes no routine log lines yet. Startup configuration refusals are
reported on stderr; current AOF startup and listener/signal-registration
failures are silent, as the exit-status table notes. `logfile`, `loglevel`
and `pidfile` are refused rather than applied. Capture stderr and process
status through the service manager; the log format awaits Whitefoot's
process-ID and local-time support,
[board item `firn-req-pid-localtime`][board].

## Migrate from Redis

The existing [AOF migration description](../research/investigations/aof-rewrite/README.md#design)
and [persistence reference](../firn/README.md#build-and-run) describe firn's
command-format multi-part layout and upgrade of an old single-file AOF.
firn cannot read an RDB snapshot or an AOF with an RDB base preamble.
Configure the source Redis with `aof-use-rdb-preamble no`, complete a
successful rewrite there, and stop it before copying its AOF directory.
Use the same `appendfilename` when starting firn. Do not copy that Redis
directive into firn's configuration: firn refuses it.

Check that the source workload and AOF use commands firn implements before
migrating. The [known replay gap](safety.md#evidence-and-known-deviations)
includes unknown AOF commands whose error replies are discarded; a
successful startup alone is not evidence that unsupported commands restored
the correct dataset. The gate's case `redis_aof_directory_replays_in_firn`
follows the steps above with Redis 7.2.16: it writes strings, lists, hashes,
sets and sorted sets, deletes a key and rewrites once, so that the base and
the increment both hold records, then copies the directory and requires firn
to answer reads for those keys exactly as Redis did. It does not validate an
arbitrary Redis deployment's migration: other commands, expiry times and other
Redis versions are not checked.

[board]: https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip
