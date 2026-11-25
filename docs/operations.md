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
in this example. Its standard output holds firn's log lines, ending with
`firn is now ready to exit, bye bye...`; see [Logs](#logs). Use another
terminal in the same data directory for the following client commands.

```sh firn-check start exit=0 stdout-contains=firn is now ready to exit, bye bye...
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
executable's absolute path and the configuration file's path, and
`LimitNOFILE=` for the hard descriptor limit. These systemd settings are guidance;
the gate exercises the foreground invocation above, not systemd.

The configuration file, each `include` and `dir` take absolute paths or
paths relative to the working directory, as Redis does: `firn
/etc/redis/redis.conf` starts with the same file Redis reads. `dir` changes
the directory that later relative paths, `include`, `logfile` and the
append-only directory, start from, and an `include` reads its file where the
directive stands, with line numbers of its own. These differences from Redis
remain:

- An `include` path holding `*`, `?` or `[` is refused with a reason that
  names wildcards; Redis expands it with `glob(3)`, and skips one that
  matches nothing. List the files by name.
- `CONFIG GET dir` reports the absolute path the `dir` directives wrote, with
  `.`, `..` and repeated `/` removed, not the path the host resolves through
  symbolic links, and reports nothing while every `dir` given was relative to
  a directory firn does not know. Redis reports the resolved path.
- A configuration file given by a relative path is named as given in the
  line that reports it cannot be opened; Redis names its absolute path.
- A `logfile` whose last component is a symbolic link is refused where Redis
  follows the link.

## Configuration refusals

firn refuses unknown directives, wrong argument counts, malformed quoting
and invalid values. It prints `FATAL CONFIG FILE ERROR` on stderr, with the
source line, its line number and the reason, then exits 1 before serving
clients. A configuration file or an `include` that cannot be opened is
logged, as Redis logs it, in the log line `Fatal error, can't open config file
'path': reason` on standard output, or in the `logfile` read so far, and also
exits 1. A `dir` that cannot be entered, an `include` path with a wildcard and
a `logfile` that cannot be opened are refused in the same fatal frame, the last
without a line, as Redis reports it.

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
| 1 | Invalid invocation or configuration, an unreadable configuration file or `include`, a `dir` that cannot be entered, an unopenable `logfile`, a configuration with no enabled listener, or an open-files limit too low for even one client. Read stderr and the log's last line and correct the startup settings. |
| 3 | firn cannot register delivery of stop signals, which it does before replaying the AOF, or cannot open its TCP listener. Check the listening address, port availability and host facilities; these failures currently have no diagnostic. |
| 4 | Any AOF startup refusal, including a missing or malformed listed file, unreadable persistence state, or failure to prepare the log for appending or truncate its incomplete tail. firn has not begun listening; preserve the files and investigate or restore a known-good backup. The log's last line is `Fatal error loading the DB, check server logs. Exiting.`, the text Redis gives for a failed load; the cause is not logged. |

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

At startup firn raises the soft open-files limit to `maxclients` plus 32, as
Redis does, trying 16 fewer after each refusal, and logs what it did: `Increased
maximum number of open files to N (it was originally set to M).`, or, when the
host will not go that far, Redis's three warnings and a `maxclients` lowered to
what the limit supports. A limit that leaves no room for a client, 32 or
fewer, stops firn with status 1 and the line `Your current 'ulimit -n' of N is
not enough for the server to start. Please increase your open file limit to at
least M. Exiting.` `CONFIG SET maxclients` adjusts the limit the same way and,
when the host cannot support the number, answers Redis's error naming the
number it can and keeps the old value. firn can change only the soft limit:
where the hard limit is below the request, the limit it reaches equals
Redis's, but the host's reason in the second warning is the text the host gives
firn, and Redis lowers the hard limit to the same value where firn leaves it.
Reserve descriptors for persistence files, listeners and other process needs
by setting the hard limit through the service manager's `LimitNOFILE=`.
When the process runs out of descriptors,
firn logs `Accepting client connection: Too many open files` to stderr, at
most once a second, and keeps listening; it accepts again once descriptors
free up, up to 100 ms after they do.

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
[AOF publication design](../design/decisions/persistence/rewrite/installation.md#decision-aof-rewrite-install-keeps-the-renamed-manifest) replaces the manifest
before deleting old files; copying an active directory can race those
changes. This guide provides no tested online-backup procedure.

This orderly stop exits 0 with no output; the foreground firn process also
exits 0 in this example, after it logs `firn is now ready to exit, bye bye...`:

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
then exits 0, its standard output holding the log lines:

```sh firn-check start exit=0 stdout-contains=firn is now ready to exit, bye bye...
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

A malformed manifest makes startup fail with status 4 and a log line that
names no cause. This example, run with firn stopped, builds a data directory whose
manifest gives its base file sequence number 0, which firn does not accept,
and starts firn in it. The command exits 4, and its standard output ends with
the log line `Fatal error loading the DB, check server logs. Exiting.`:

```sh firn-check exit=4 stdout-contains=Fatal error loading the DB, check server logs. Exiting.
mkdir damaged
cp redis.conf damaged/
cd damaged
mkdir appendonlydir
: > appendonlydir/appendonly.aof.1.base.aof
printf 'file appendonly.aof.1.base.aof seq 0 type b\n' > appendonlydir/appendonly.aof.manifest
firn redis.conf
```

## Logs

firn writes Redis's log lines to standard output, or to the file `logfile`
names, which it opens for every line so that a rotated file is created anew,
as Redis does. A line has Redis's form, `pid:C dd Mon yyyy hh:mm:ss.mmm * message`,
with the process id, the role (`C` until the server is initialized, then `M`),
the host's local time and a mark for the level: `.` debug, `-` verbose,
`*` notice, `#` warning. `loglevel` hides the lines below its level, `nothing`
all of them. The lines come from these events, with Redis's message:
the start, the configuration loaded (or the warning that none was given), the
open-files limit, `Running mode=standalone, port=N.`, `Server initialized`,
`DB loaded from append only file: S seconds`, `Ready to accept connections
tcp`, the start and the end of an append-only file rewrite, a failed append
(`Error writing to the AOF file: reason` or `Short write while writing to the
AOF file: (nwritten=N, expected=M)`, at most one in thirty seconds), `AOF write
error looks solved, Redis can write again.`, the first failed sync (`Fail to
fsync the AOF file: reason`), the stop signal, `User requested shutdown...`,
`Calling fsync() on the AOF file.` and `firn is now ready to exit, bye bye...`.
These differences from Redis's log remain: the product name is `firn`, there
is no version line, the rewrite line has no `by pid` because firn rewrites in
a thread, `pidfile` and `syslog-enabled` are refused, `CONFIG SET loglevel` is
refused, and messages Redis writes for features firn does not have do not
appear. A write that fails drops the line, as Redis ignores a failed log write.
Accepting an excess client or failing to accept still writes to stderr as the
connections section says.

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

