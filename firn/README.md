# firn

firn is a server of Redis's protocol written in Whitefoot. It answers RESP2
or RESP3, the version a connection's HELLO selects, and inline requests, quoted arguments included, over TCP, pipelined or not,
and keeps strings, lists, sets, hashes and sorted sets in one keyspace, a
shared map that every connection reaches through atomic statements naming
the entries of the keys a command uses and the separate expiry, log or server
metadata objects it also needs. It is named for firn, snow that has lasted a
season: stored and compacted.

Its commands are these, which began as the ones `redis-benchmark`'s default
suite sends, with expiry, an append-only file and the connection commands
clients send on their own:

- keys: `DEL`, `UNLINK`, `EXISTS`, `TOUCH`, `TYPE`, `RENAME`, `RENAMENX`,
  `COPY` with `REPLACE` and `DB 0`, `EXPIRE`, `PEXPIRE`, `EXPIREAT` and
  `PEXPIREAT` with their options `NX`, `XX`, `GT` and `LT`, `TTL`, `PTTL`,
  `EXPIRETIME`, `PEXPIRETIME`, `PERSIST`, `DBSIZE`, `OBJECT IDLETIME`,
  `OBJECT FREQ`, `OBJECT HELP`,
  `SCAN cursor [MATCH pattern] [COUNT count] [TYPE type]`, `KEYS pattern`
  and `RANDOMKEY`;
- strings: `GET`, `SET` with its options `NX`, `XX`, `GET`, `KEEPTTL`,
  `EX`, `PX`, `EXAT` and `PXAT`, `SETNX`, `SETEX`, `PSETEX`, `GETSET`,
  `GETDEL`, `GETEX`, `MGET`, `MSET`, `MSETNX`, `INCR`, `INCRBY`, `DECR`,
  `DECRBY`, `INCRBYFLOAT`, `APPEND`, `STRLEN`, `GETRANGE`, `SUBSTR`,
  `SETRANGE`;
- lists: `LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP` and `RPOP` with or
  without a count, `LRANGE`, `LLEN`, `LINDEX`, `LSET`, `LREM`, `LTRIM`,
  `LINSERT`, `LPOS` with `RANK`, `COUNT` and `MAXLEN`, and `LMOVE` and
  `RPOPLPUSH`, which move an element in one statement holding both keys;
- sets: `SADD`, `SREM`, `SPOP` and `SRANDMEMBER` with or without a count,
  `SCARD`, `SMEMBERS`, `SISMEMBER`, `SMISMEMBER`, `SMOVE`, `SINTER`,
  `SUNION`, `SDIFF`, `SINTERCARD` with `LIMIT`, and `SINTERSTORE`,
  `SUNIONSTORE` and `SDIFFSTORE`, each in one statement holding every key
  it names;
- hashes: `HSET`, `HMSET`, `HSETNX`, `HGET`, `HMGET`, `HDEL`, `HEXISTS`,
  `HSTRLEN`, `HLEN`, `HGETALL`, `HKEYS`, `HVALS`, `HINCRBY`, `HINCRBYFLOAT`,
  in the x87 extended precision Redis computes it in on x86-64, and
  `HRANDFIELD` with a count and `WITHVALUES`;
- sorted sets: `ZADD` with `NX`, `XX`, `GT`, `LT`, `CH` and `INCR`,
  `ZINCRBY`, `ZRANGE` with `BYSCORE`, `BYLEX`, `REV`, `LIMIT` and
  `WITHSCORES`, `ZREVRANGE`, `ZRANGEBYSCORE`, `ZREVRANGEBYSCORE`,
  `ZRANGEBYLEX`, `ZREVRANGEBYLEX`, `ZCOUNT`, `ZLEXCOUNT`, `ZRANK`, `ZREVRANK`,
  `ZSCORE`, `ZMSCORE`, `ZCARD`, `ZREM`, `ZPOPMIN` and `ZPOPMAX` with a count,
  `ZREMRANGEBYRANK`, `ZREMRANGEBYSCORE` and `ZREMRANGEBYLEX`, with scores
  read and written as Redis 7.0.15 reads and writes them;
- scripting: `EVAL`, `EVALSHA`, `SCRIPT LOAD`, `SCRIPT EXISTS`,
  `SCRIPT FLUSH [SYNC|ASYNC]` and `SCRIPT KILL`, which stops a script that
  has written nothing, running Lua 5.1 scripts on the Halo engine
  of [Halo-wf](https://github.com/Ming-Research/Halo-wf), `deps/halo-wf`,
  with Redis's `KEYS`, `ARGV`, `redis` library and reply conversions, in
  either protocol. A compiled script is kept until `SCRIPT FLUSH`, as in
  Redis 7.0.15. `SCRIPT FLUSH` waits until no script is in progress before
  clearing the registry. `redis.call` and `redis.pcall` run the commands
  written as parts, those `MULTI` runs, inside the script's statement,
  through Halo's resumable host call; any other command is answered with
  Redis's error for an unknown one, noting that firn may not run it from
  scripts yet;
- connection: `PING`, `ECHO`, `QUIT`, `RESET`, `AUTH`, `HELLO` with no version,
  version 2 or version 3, which switches the connection to RESP3,
  `SELECT 0`, firn having one database, and `CLIENT ID`, `CLIENT GETNAME`,
  `CLIENT SETNAME` and `CLIENT INFO`, described below;
- transactions: `MULTI`, `EXEC` and `DISCARD`. `EXEC` runs the queued
  commands in order in one atomic statement, their time frozen at its start,
  for the commands written as parts: every keys, strings, hashes, lists, sets
  and sorted sets command this list names, including `SCAN`, `KEYS` and
  `RANDOMKEY`, with `TOUCH`, `SUBSTR`, `TIME`, `SELECT`, `PING`, `ECHO`,
  `COMMAND`, `COMMAND COUNT`, `FLUSHALL` and `FLUSHDB`; `INFO` is not among
  them. These same command parts run in scripts.
  A command without parts, or any name firn does not run, is queued and makes
  `EXEC` refuse the whole transaction without running any of it, where Redis
  refuses a name it lacks when it is sent; unknown subcommands and commands
  outside their table arity are refused when sent and make `EXEC` abort, as
  Redis refuses them; `WATCH` is refused inside one and unknown outside;
  `SHUTDOWN` is refused when sent and makes `EXEC` abort the transaction;
- server: `SHUTDOWN [NOSAVE|SAVE] [NOW] [FORCE] [ABORT]`, described below,
  `CONFIG GET`, `CONFIG SET`, `CONFIG RESETSTAT` and `INFO`,
  described below, `TIME`, and `COMMAND` and `COMMAND COUNT`, which
  describe no command. `COMMAND DOCS` is answered as an unknown subcommand,
  so that `redis-cli` uses its own help. `FLUSHALL` and `FLUSHDB`, with
  `ASYNC` or `SYNC`, empty firn's one database and its queued expiries in
  one atomic statement and are appended to the append-only file as Redis
  appends them; the old keys are released before the reply is sent, under
  either option. `FUNCTION FLUSH`, with `ASYNC` or `SYNC`,
  succeeds as Redis does with no function loaded, firn having none, and is
  appended to the file as Redis appends it; every other `FUNCTION`
  subcommand is answered as an unknown one. `DEBUG LOG` with a message
  answers OK, as Redis does with its debug command enabled, firn keeping no
  log to write it to; every other `DEBUG` subcommand is answered as an
  unknown one.

`SCAN` takes one map scan step per request; its unsigned decimal cursor is
zero when the scan ends, and a nonzero cursor can accompany an empty batch.
`MATCH` and `KEYS` patterns are binary and case-sensitive; `TYPE` names are
case-insensitive. `KEYS` walks the whole keyspace in one atomic statement
and skips expired entries without removing them. `SCAN` removes expired
entries reached after `MATCH`, returning their names only under
`TYPE none`, as Redis 7.0.15 does, and `RANDOMKEY` removes expired
candidates; both record those removals as `DEL`. `RANDOMKEY` draws as Redis's
`dbRandomKey` does, picking uniformly among the keys one scan step of count
15 gathers from a random position, wrapping at the table's end, with a
fresh position when it gathers none; its positions and draws are its own, so the key it returns
differs from Redis's. All five commands,
including both flush commands, run through their shared parts on the
network, in `EXEC` and through `redis.call` or `redis.pcall`.

`CONFIG GET` takes Redis's glob patterns over firn's parameters:
`appendfilename`, `appendonly`, `bind`, `databases`, `port`, `requirepass`,
`save` and `timeout`, and the parameters whose only effect in Redis is on its
internal encodings, leaving every value as commands read it,
`hash-max-listpack-entries`, `hash-max-listpack-value`,
`list-compress-depth`, `list-max-listpack-size`, `set-max-intset-entries`,
`stream-node-max-bytes`, `stream-node-max-entries`,
`zset-max-listpack-entries` and `zset-max-listpack-value`, with their aliases
`hash-max-ziplist-entries`, `hash-max-ziplist-value`, `list-max-ziplist-size`,
`zset-max-ziplist-entries` and `zset-max-ziplist-value`. firn has none of
those encodings: it reports their parameters with Redis's defaults and keeps
what `CONFIG SET` gives them, and they change nothing else.
`hll-sparse-max-bytes` is not among them, since a HyperLogLog's encoding is
the string `GET` reads, and firn has no HyperLogLog.

`CONFIG SET` takes these parameters with Redis's checks and errors, refusing
any other as Redis refuses one it does not know, and applies all of its pairs
or none. `requirepass` changes the password for every connection that has
not authenticated, as in Redis: a connection authenticates by giving the
password, or by being accepted while none is set, and stays authenticated;
removing the password lets the others in until one is set again. `timeout`
changes the idle limit for new connections and wakes every connection waiting
for a request to apply it, one that waited with no limit included. A receive
with no idle limit has no deadline; otherwise its deadline is the remaining
idle interval. Each client retains a watch of the current receive wake
generation; a changed limit replaces the generation and fires the old one.
A client's silence is counted from its last request or the replies to it,
as Redis counts it from its last read or write. `appendfilename` and `databases` are refused as Redis
refuses them. An `appendonly`, `port` or `bind` other than the one firn
started with, and a `save` schedule other than the empty one, are refused in
Redis's form for a refused value with firn's own reason, since firn cannot
change them while it runs and saves no snapshot; Redis would apply them.
The memory parameters are `maxmemory` (default 0, Redis memory units accepted),
`maxmemory-policy` (default `noeviction`), `maxmemory-samples` (default 5,
minimum 1), `lfu-log-factor` (default 10, minimum 0), `lfu-decay-time`
(default 1 minute, 0 disables decay), and `maxmemory-eviction-tenacity`
(default 10, from 0 through 100). The integer sampling and LFU parameters
have Redis's maximum of 2,147,483,647. All eight policy names are accepted:
`noeviction`, `allkeys-lru`, `allkeys-lfu`, `allkeys-random`, `volatile-lru`,
`volatile-lfu`, `volatile-random`, and `volatile-ttl`.
A nonzero limit compares Whitefoot's live requested heap bytes minus the
allocated capacity of both pending append-only buffers, saturating at zero.
Each command's context evicts before running, after command lookup, arity,
authentication and transaction restrictions. LRU, LFU and TTL policies keep
a shared pool of 16 candidates sampled through `map_scan`; random policies
pick uniformly from a gathered batch. Volatile policies consider only keys
with an expiry. An expired victim counts as an expiry. Removal and its AOF
`DEL` are atomic with each other.

If no eligible key remains and the counted heap is still over the limit,
Redis's DENYOOM commands, including `SET`, answer
`-OOM command not allowed when used memory > 'maxmemory'.`; `GET` and `DEL`
remain allowed. With a bounded eviction time slice, the command proceeds
and the next command continues evicting. Tenacity uses Redis's time limit,
checked every 16 deletions; 100 is unlimited. This is admission control:
a command, transaction, script or concurrent commands can overshoot it.
There is no background eviction continuation during an idle interval.

While `MULTI` is open, OOM refuses every command except `EXEC`, `DISCARD`,
`QUIT` and `RESET`, including reads and memory-releasing writes. A refused
command receives `-OOM command not allowed when used memory > 'maxmemory'.`
and marks the transaction dirty. An admitted `EXEC` then answers
`-EXECABORT Transaction discarded because of previous errors.`, even after
memory recovers. `EXEC` first checks the queued DENYOOM flags: if that
admission itself fails under OOM, it answers `-EXECABORT Transaction
discarded because of: OOM command not allowed when used memory >
'maxmemory'.` instead. After admission there are no memory checks between
queued commands. These replies follow Redis 7.0.15. `RESET` discards a
transaction and resets the name, authentication and protocol to RESP2.

A script without a shebang captures OOM at its start. A DENYOOM call is
refused until it has accepted a WRITE-flagged command: `DEL` of a missing
key counts, independently of whether it changes data or appends an effect.
The OOM state survives script retries, with write acceptance restarted on
each attempt. `EVAL` and `SCRIPT LOAD` explicitly reject **all** shebang
sources, including plain `#!lua` and the Redis flags `allow-oom`,
`no-writes`, `allow-stale`, `no-cluster` and `allow-cross-slot-keys`.
They are not executed with legacy semantics; no shebang flags are supported.

`OBJECT IDLETIME key` reports idle seconds at one-second resolution under
any non-LFU policy, including `noeviction`; `OBJECT FREQ key` reports Redis's
logarithmic, decayed frequency under an LFU policy. Each rejects the other
policy kind with Redis's error, and missing keys return null. Ordinary
reads and writes refresh access state; `EXISTS`, `TYPE`, the four TTL/time
queries, `OBJECT`, and SCAN's TYPE filter do not. `TOUCH` refreshes it.
Policy changes reinterpret the existing bits, as Redis does, so values need
time to adjust. `OBJECT HELP` returns Redis's help; `ENCODING` and `REFCOUNT`
remain unsupported. Settings used for access tracking are read once per
connection read, beside its clock; every command in that read uses them,
including queued commands run by EXEC and script calls. After this connection
executes CONFIG SET, it refreshes the snapshot for the read's remaining
commands; other connections keep their snapshot until their next read. A lookup racing
CONFIG SET may therefore write an old-policy stamp. CONFIG and INFO still
read current server settings. Scripts and EXEC use the same tracking command
bodies. An unwritten script attempt abandoned for its step budget restores
the first stamp of every key it refreshed and firn's LFU random state before
releasing the keys; completed attempts and attempts that wrote retain them.
SCRIPT KILL between attempts therefore leaves abandoned refreshes undone.

`CONFIG RESETSTAT` answers OK and zeroes connections accepted, evicted keys
and active-expiry counts. It preserves `used_memory_peak`, the observed
heap maximum since startup, as Redis 7.0.15 does.

`SHUTDOWN` sends no reply, nor the replies its connection holds unsent
from the commands before it, which Redis 7.0.15 does not send either; its
connection closes, and firn stops accepting clients. The request cancels
each client's receive through its wake generation, and the accept wait,
expiry sleep and signal wait through a separate shared shutdown state.
A command or batch already running still finishes before its client closes;
cancellation does not
interrupt scripts or the replies being sent. A send blocked on a client that does not read is abandoned
at its next one-second deadline. Once every client has left, the append-only file's writer appends
its last bytes, syncs and closes, and firn exits with status 0, as Redis
does even when that sync fails. No snapshot is written: the `save` schedule
is empty, and `SAVE` answers `ERR Errors trying to SHUTDOWN. Check logs.`
and keeps serving unless `FORCE` is also given. `NOW` changes nothing, firn
having no replicas, and `ABORT` answers `ERR No shutdown in progress.`
Options are read in either case up to a zero byte, and an unknown option,
`SAVE` with `NOSAVE`, or `ABORT` with another option is a syntax error.
Scripts cannot call `SHUTDOWN`, and a script that is running holds its
client until it ends, so one that never ends keeps firn from stopping (the
[board item `firn-bl-01-07`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip) on busy scripts).

SIGTERM and SIGINT stop firn as `SHUTDOWN` does, with the same drain, the
append-only file's last append and sync, and status 0, as Redis 7.0.15 shuts
down gracefully on either signal under its default `shutdown-on-sigterm` and
`shutdown-on-sigint`. firn hears them from just before it listens: one sent
during the replay before that still ends firn at once, where Redis stops
loading and exits with status 0, and a host that refuses to deliver them to
firn stops it with status 3, as an address it cannot listen on does. Once
firn has taken a signal, or cancellation has ended its signal wait after a
`SHUTDOWN` request or its last client leaving after the client limit, a
further SIGTERM or SIGINT ends it at once, without the rest of the drain;
Redis ends at once, with status 1, on a second SIGINT, but ignores a second
SIGTERM. `SHUTDOWN ABORT` answers `ERR No shutdown in progress.` after a
signal too, where Redis's cancels a signal's request in the moment, at most
a tenth of a second, before its next cron acts on it.

`INFO memory` reports raw `used_memory` and `used_memory_human`,
`used_memory_rss` when the host supplies it, `used_memory_peak`,
`mem_not_counted_for_evict`, the configured `maxmemory`, `maxmemory_human`
and `maxmemory_policy`, with Redis's human-size formatting. It also reports
the actual registered-script count. Heap readings use `heap_in_use`; RSS
uses `resident_bytes` on demand. Peak tracks heap readings after commands,
at admission and at INFO, rather than every transient allocation. AOF
exclusion uses capacities even when drained, not lengths. `INFO stats`
reports live `evicted_keys`. The heap meter counts the keyspace's own
storage, its shared-map tables and nodes, as well as values.

`INFO`, with no section, `default`, `all`, `everything` or named sections,
answers Redis's sections in Redis's order and form. Its fields carry real
values for the port, the calendar time, the uptime, the clients connected,
whether the append-only file is kept, the connections accepted and the keys
held, which it counts holding the table whole, as `DBSIZE` does. The other fields it reports have values
that are fixed and true of firn: Redis's
version 7.0.15, no git revision, `redis_git_sha1` being 00000000 as in
Redis's builds from a release, standalone mode, 64 bits, its active
expiry's 10 runs a second, no configuration file,
function, replica, background save, fork, module, publish
and subscribe, tracking or cluster. What firn does not measure,
processor time, per-command and per-error counts among them, is left out,
so its CPU, Commandstats, Errorstats and Latencystats sections are empty
and its keyspace line gives `keys` alone, without the `expires` and
`avg_ttl` firn does not count.

`CLIENT INFO` answers the connection's line in Redis's form, with real
values for the id, the name, the age and the protocol, and values fixed and
true of firn for the others it gives. It leaves out what firn cannot report:
the peer's and its own address, which Whitefoot's socket address does not
show, the descriptor, the events and the sizes of Redis's buffers.

`HELLO` and `INFO` report the server as `redis` version 7.0.15, the version
whose replies firn follows.

`HINCRBYFLOAT` computes in the long double of Redis on x86-64 Linux, x87's
80-bit extended format, and answers as that Redis does. Redis built where
long double has another format answers it, and `INCRBYFLOAT`, differently:
in binary128 on aarch64 Linux, and in a double where long double is one.

What is not there yet is listed on the [shared status board](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip); the
measurements and the design are in
[research/investigations/firn](../research/investigations/firn/DESIGN.md).

## Build and run

From the repository root, on Linux x86-64 or macOS arm64 with `/usr/bin/clang`
installed, and LLD on Linux:

```sh
make firn
./build/firn 6379 0 - 0
redis-cli -p 6379 PING
```

`make firn` downloads the Whitefoot compiler release that `whitefoot.pin`
names and builds `build/firn` with an incremental cache, which keeps each
module's checked and compiled parts, so a rebuild after an edit compiles only
what the edit changed. A cached build does not link the runtime under
link-time optimization; `make firn-lto` builds `build/firn-lto` with
`--full-lto` instead, to measure firn's speed, as the benchmarks do.

Up to four arguments may come first by position, in this order:

1. the port to listen on, 6379 when absent;
2. how many clients to accept before stopping, 0, the default, for no
   limit;
3. `appendfilename`, the prefix of the append-only files below `appendonlydir`
   in the working directory, or `-` for none, the default; firn replays the
   manifest's base and then its incremental files before listening, appends
   changes to the last incremental file and syncs it once a second, as
   Redis's `appendfsync everysec` does;
4. how many seconds a silent client is kept before it is closed, 0, the
   default, for no limit.

Options by name follow them, each `--` and a name, in either case, then its
value; a later value replaces an earlier one:

- `--port` and `--timeout`, as the first and fourth arguments;
- `--bind`, the address to listen on: a dotted IPv4 address, or `*` for every
  one, the loopback address 127.0.0.1 by default;
- `--appendonly yes` or `no`, whether to keep the append-only file, and
  `--appendfilename`, its name, `appendonly.aof` by default, which alone does
  not turn the file on, as in Redis;
- `--requirepass`, a password every client must give with `AUTH` or `HELLO`
  before other commands, none when empty or absent;
- `--clients`, as the second argument.

```sh
./firn --port 6380 --bind 0.0.0.0 --requirepass secret --appendonly yes
```

An unknown option, a value that does not read or an argument by position after
an option stops firn with status 1.

Persistence uses Redis 7.0.15's manifest layout: with prefix `F`, a fresh
start creates `appendonlydir/F.1.base.aof` (empty), `F.1.incr.aof` and
`F.manifest`. Base files use commands, not an RDB preamble. Manifest updates
write `temp-F.manifest`, sync it, rename it over the manifest and sync the
directory. A listed file that is missing or malformed stops firn with status
4 before it listens. Only the last file may have an incomplete tail cut,
as with `aof-load-truncated yes`; an incomplete earlier file stops startup.
Listed history files are removed after loading only after syncing appendonlydir;
a failed sync leaves them listed and on disk. Unlisted files are kept.
An old single file `F` in the working directory is upgraded by persisting a
manifest and moving `F` into `appendonlydir`. Interrupted upgrades resume;
when both copies exist and the manifest names `F`, the directory copy wins.

The rewrite below is the reconciled-scan route under comparison ([investigation](../research/investigations/aof-rewrite/scan-log.md)); its design decisions await the owner's approval.

With append-only persistence enabled, `BGREWRITEAOF` answers started after
S0 admission. The worker scans the live map in separate statements while
mutations retain complete after-images. The writer seals the old increment
and rotates once at S1, then the worker reconciles the scan with ordered
DEL-plus-reconstruction images. Installation selects base(S1) and only the
increment containing later writes, before removing old files.

R is five percent of maxmemory, or 16 MiB when maxmemory is zero. Admission
also requires max(R, 1 MiB) of service headroom. Capture overflow, oversized
images, flush, exhaustion, shutdown and I/O failure abort the attempt; client
commands and ordinary AOF appends continue. Aborts report `err`. Any
unsuccessful attempt, including a refused start, stops automatic rewrites
until a manual `BGREWRITEAOF` succeeds; Redis instead keeps retrying them with
a delay of up to an hour. A key is copied whole in one hold when the scan
reaches it or a write changes it during a rewrite, so a large key pauses
other clients for the copy; only the reserve limits its size. The fixed journal
directory permits at most 8188 chunks. If startup left the file inside an
unfinished MULTI, the rewrite cuts the sealed file before that MULTI; the new
base holds the live writes appended after it, as Redis's rewrite does.

The scan currently uses count hint 1 and charges the returned KeySet before
copying payloads. This permits one scan step's allocation before charging,
under board item `firn-gap-scan-bound`; count is a hint, and collisions can
return multiple keys. It is not the strict reserve guarantee. The exact
long-key regression remains ignored for that gap. File calls still have no
bounded cancellation contract, so prompt cleanup under stalled I/O is
unverified.

The temporary base is `temp-F.base` beside `temp-F.manifest`, so servers
sharing `appendonlydir` with distinct appendfilenames use distinct temporary
base names. Those names can still collide with another dataset's upgraded
base: if its appendfilename is literally `temp-F.base`, rewriting `F` can
truncate that dataset's base. Do not share a directory with that overlap.
Whitefoot does not yet offer exclusive file creation to refuse the collision;
Redis's `temp-F.incr` has the same class of collision. The remaining work is
exclusive creation for temporary append-only files, board item
`firn-wf-excl-create` on the
[shared status board](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip).
With appendonly off, `BGREWRITEAOF` answers
`ERR Can't execute an AOF background rewriting. Please check the server logs for more information.`
as the existing firn limitation; Redis can rewrite in that state.
The base writes hashes with `HMSET`, and collections in commands of at most
64 elements, followed by each key's absolute expiry. Scripts persist their
command effects, not their cached sources; firn has no persisted functions.

`--auto-aof-rewrite-percentage` defaults to `100`; `0` disables automatic
rewrites. `--auto-aof-rewrite-min-size` defaults to `67108864` bytes (64 MiB)
and accepts Redis's `b`, `k`, `m`, `g`, `kb`, `mb`, and `gb` suffixes. A rewrite
starts when the active files exceed that minimum and have grown by at least
the percentage over the size at the last successful rewrite. At startup the
baseline is the loaded base file's size, as Redis 7.0.15 initializes it.
These are startup options; `CONFIG SET` does not change them.

`INFO persistence` reports rewrite progress, attempts, last status, current
active-file bytes and the rewrite baseline. The firn-only field
`firn_aof_rewrite_commit_seq` is the actual dataset statement sequence, also when
AOF is disabled, following the [reported-facts rule](../design/firn/reported-facts.md).
It starts at zero after loading, advances once for an atomic statement that
changes the dataset, and is shared by every write in an `EXEC` or script
attempt. Expiry and eviction removals count; reads and access refreshes do
not. Failed or no-effect commands take no sequence unless their lookup
removes expired data. Both flush commands advance it even on
an empty database. Accepted overwrites remain writes, even when their value
is equal. The counter saturates at u64's maximum and records that later
reconciled-scan captures must be refused; it never wraps. This sequencing
support now orders capture and the S1 end cut.

`firn_aof_rewrite_reserve` and `firn_aof_rewrite_scan_peak` report the
admitted allowance and the largest PRE-2 heap increase observed across
KeySet creation and one map_scan call in the most recent attempt. The latter
supports the ignored quiescent long-key regression independently of the
reservation ledger. Other contexts can affect this process-wide sample; it
is neither total rewrite peak nor RSS. `firn_aof_rewrite_cut` is 1 once the
latest admitted attempt has crossed S1 (also retained after completion),
and 0 otherwise; it lets the backlog regression begin writes after capture
has stopped.

Failure before rotation leaves the original manifest authoritative. After
rotation it leaves the manifest naming the old base and all increments
replayable. If manifest rename succeeds but
directory sync fails, the new manifest stays in force, all files and history
entries are retained, and `aof_last_bgrewrite_status` reports `err`.
`SHUTDOWN` accepted before installation cancels the rebuild at its next step;
once installation starts, shutdown waits for it to finish. Shutdown drains
the incremental file after clients leave and joins the rebuild before exiting.
See the [reconciled-scan protocol](../research/investigations/aof-rewrite/scan-log.md).

`WF_DRIVERS` sets how many threads serve the connections, one per CPU by
default.

## Layout

`modules.wfg` registers the modules, each a directory:

- `bytes`: byte strings, their hash and order, and integers and glob
  patterns read as Redis reads them;
- `protocol`: reading requests, writing replies, and a connection's state and
  settings;
- `scores`: sorted-set scores and the ends of score ranges read as Redis's
  `strtod` reads them, to the nearest double, and written as its `%.17g`
  writes them;
- `extended`: numbers of x86-64's 80-bit long double, read as glibc's
  `strtold` reads them, added, and written as `%.17Lf` writes them, the
  arithmetic of `INCRBYFLOAT` and `HINCRBYFLOAT`;
- `store`: the keyspace, one shared state holding a keyed table of entries
  and, after it, the queued expiries, the append-only file's pending bytes
  and the server's counts
  ([firn under the shared-state design](https://github.com/Ming-Research/Whitefoot/blob/648338c31240ba64ce13c314b1afca1749d44189/research/investigations/shared-state/DESIGN.md#firn-under-the-design)),
  and beside it a second shared state, the server's, with what `CONFIG SET`
  changes, the count of accepted connections and the time the server
  started;
- `commands`: one file per kind of value, sorted sets' ranges in a second,
  the connection and server commands, and the dispatch;
- `scripting`: `EVAL`, `EVALSHA` and `SCRIPT`, the Redis Lua environment
  and the conversions between replies and Lua values; `script_pool` holds
  the one Lua engine every script takes in turn and the registry of
  scripts the keyspace shares;
- `persistence`: the append-only manifest, startup and upgrade, writer and replay;
- `server`: connections, active expiry, the invocation's options and `main`.
