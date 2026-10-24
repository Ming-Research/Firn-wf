# firn

firn is a server of Redis's protocol written in Whitefoot. It answers RESP2
or RESP3, the version a connection's HELLO selects, and inline requests, quoted arguments included, over TCP, pipelined or not,
and keeps strings, lists, sets, hashes, sorted sets and streams in one keyspace, a
shared map that every connection reaches through atomic statements naming
the entries of the keys a command uses and the separate expiry, log or server
metadata objects it also needs. It is named for firn, snow that has lasted a
season: stored and compacted.

See [safety guarantees and their evidence](../docs/safety.md).
For service startup, connection limits and persistence recovery, see the
[operator's guide](../docs/operations.md).

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
  `SCARD`, `SMEMBERS`, `SSCAN key cursor [MATCH pattern] [COUNT count]`,
  `SISMEMBER`, `SMISMEMBER`, `SMOVE`, `SINTER`,
  `SUNION`, `SDIFF`, `SINTERCARD` with `LIMIT`, and `SINTERSTORE`,
  `SUNIONSTORE` and `SDIFFSTORE`, each in one statement holding every key
  it names;
- hashes: `HSET`, `HMSET`, `HSETNX`, `HGET`, `HMGET`, `HDEL`, `HEXISTS`,
  `HSTRLEN`, `HLEN`, `HGETALL`, `HKEYS`, `HVALS`, `HINCRBY`, `HINCRBYFLOAT`,
  in the x87 extended precision Redis computes it in on x86-64, and
  `HRANDFIELD` with a count and `WITHVALUES`, and
  `HSCAN key cursor [MATCH pattern] [COUNT count]`;
- sorted sets: `ZADD` with `NX`, `XX`, `GT`, `LT`, `CH` and `INCR`,
  `ZINCRBY`, `ZRANGE` with `BYSCORE`, `BYLEX`, `REV`, `LIMIT` and
  `WITHSCORES`, `ZREVRANGE`, `ZRANGEBYSCORE`, `ZREVRANGEBYSCORE`,
  `ZRANGEBYLEX`, `ZREVRANGEBYLEX`, `ZCOUNT`, `ZLEXCOUNT`, `ZRANK` and
  `ZREVRANK` with Redis 7.2's `WITHSCORE`, `ZSCORE`, `ZMSCORE`, `ZCARD`,
  `ZREM`, `ZPOPMIN` and `ZPOPMAX` with a count, `ZREMRANGEBYRANK`,
  `ZREMRANGEBYSCORE`, `ZREMRANGEBYLEX`, and
  `ZSCAN key cursor [MATCH pattern] [COUNT count]`, with scores read as Redis
  reads them and replies written as Redis 7.2.16's `d2string` writes them:
  signed zeros and infinities, integers through magnitude 2^62, and other
  values through Halo-wf's port of Grisu2 (`fpconv_dtoa`), without an exact
  fallback; ZSCAN's text selection is described below. Rewritten
  append-only scores use plain `fpconv_dtoa` without the reply's integer
  shortcut;
- streams: `XADD` with `MAXLEN` and `MINID` (`=` or `~`), `LIMIT` and
  `NOMKSTREAM`, `XRANGE` and `XREVRANGE` with `COUNT`, `XLEN`, `XTRIM`,
  `XDEL`, `XSETID` with `ENTRIESADDED` and `MAXDELETEDID`, and `XREAD` with
  `COUNT`. Entries are kept in nodes split where Redis 7.2.16 splits them, by
  `stream-node-max-bytes` and `stream-node-max-entries`, so that `~`
  trimming removes what Redis removes. `XREAD` with `BLOCK` answers at once
  when a stream has entries to serve and is refused on a connection when
  none has, since firn does not wait yet; under `EXEC` and in scripts it
  answers a null array, as Redis does. Consumer groups and `XINFO` are not
  supported;
- scripting: `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`, `SCRIPT LOAD`, `SCRIPT EXISTS`,
  `SCRIPT FLUSH [SYNC|ASYNC]` and `SCRIPT KILL`, which observes a kill
  between attempts only while no dataset effects or publication deliveries
  have been selected, running Lua 5.1 scripts on the Halo engine
  of [Halo-wf](https://github.com/Ming-Research/Halo-wf), `deps/halo-wf`,
  with Redis's `KEYS`, `ARGV`, `redis` library and reply conversions, in
  either protocol. A compiled script is kept until `SCRIPT FLUSH`, as in
  Redis 7.0.15. `SCRIPT FLUSH` waits until no script is in progress before
  clearing the registry. `redis.call` and `redis.pcall` run the commands
  written as parts, those `MULTI` runs, inside the script's statement,
  through Halo's resumable host call; any other command is answered with
  Redis's error for an unknown one, noting that firn may not run it from
  scripts yet. A Lua number passed to a command and a script's `{double=x}`
  reply are written as Redis 7.2.16 writes them, through Halo-wf's port of
  `fpconv_dtoa`; the globals, every table reachable from them and the string
  metatable are read-only, as in Redis 7.2.16, while `KEYS` and `ARGV` stay
  writable; the `_RO` commands refuse writes before persistence and OOM
  checks, including writes that would change nothing, and `redis.pcall`
  can catch their errors. The invocation selects read-only authority, so a
  cached script can also run through a writing EVAL command.
  `redis.replicate_commands(...)` always returns true, as in Redis 7.2.16.
  `redis.REDIS_VERSION` is 7.2.16; `redis.set_repl` accepts
  `REPL_AOF` and `REPL_ALL` and refuses `REPL_NONE`, `REPL_SLAVE` and
  `REPL_REPLICA`, whose partial propagation firn does not implement;
- connection: `PING`, `ECHO`, `QUIT`, `RESET`, `AUTH`, `HELLO` with no version,
  version 2 or version 3, which switches the connection to RESP3,
  `SELECT 0`, firn having one database, and `CLIENT ID`, `CLIENT GETNAME`,
  `CLIENT SETNAME` and `CLIENT INFO`, described below;
- pub/sub: `SUBSCRIBE`, `UNSUBSCRIBE`, `PSUBSCRIBE`, `PUNSUBSCRIBE`,
  `PUBLISH`, `SSUBSCRIBE`, `SUNSUBSCRIBE`, `SPUBLISH`, and `PUBSUB CHANNELS`,
  `NUMSUB`, `NUMPAT`, `SHARDCHANNELS`, `SHARDNUMSUB` and `HELP`;
- transactions: `MULTI`, `EXEC` and `DISCARD`. `EXEC` runs the queued
  commands in order in one atomic statement, their time frozen at its start,
  for the commands written as parts: every keys, strings, hashes, lists, sets
  and sorted sets command this list names, including `SCAN`, `SSCAN`, `HSCAN`,
  `ZSCAN`, `KEYS` and `RANDOMKEY`, with `TOUCH`, `SUBSTR`, `TIME`, `SELECT`, `PING`, `ECHO`,
  `COMMAND`, `COMMAND COUNT`, `FLUSHALL` and `FLUSHDB`, plus pub/sub commands;
  `SSUBSCRIBE` is refused during EXEC as in Redis 7.2.16. `INFO` is not among
  them. The data commands, publications and PUBSUB queries run in scripts,
  except that `EVAL_RO` and `EVALSHA_RO` refuse `PUBLISH` and `SPUBLISH`, which
  Redis 7.2.16 flags may-replicate, as it refuses write commands there;
  all six subscription commands are NOSCRIPT. All four EVAL commands
  remain unsupported inside EXEC: preserving the transaction's isolation
  while interrupting a long script waits for Whitefoot request
  `firn-req-attempt-publish`, described
  in the [script client audit](../research/investigations/script-engines/README.md#client-visible-script-details-redis-7216).
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

Channels and patterns are binary and independent of the selected database.
Patterns use Redis 7.2.16's case-sensitive `stringmatchlen`, including its
depth limit. Ordinary and shard channels have separate registries; PUBLISH
counts each direct or matching-pattern delivery, and SPUBLISH reaches shard
memberships only. Unsubscribe-all acknowledgment order follows Firn's own
registry order; each removed membership and the resulting count are returned.

In RESP2, a client with any subscription accepts only the three subscribe and
unsubscribe families, PING, QUIT and RESET, with Redis's subscriber-mode error
for other admitted commands. PING answers the two-element pong array in this
mode. RESP3 delivers subscription acknowledgments and messages as push frames
and allows ordinary commands. A client's command, EXEC or script reply group
precedes its self-publication pushes. Lua errors retain publications from
preceding accepted calls; abandoned attempts publish nothing.
On a promoted connection, a dataset command's complete reply is queued
before another client can observe its effects and publish a later message,
including data-only EXEC and declared-key scripts.

A connection transfers once to receive, command and output owners on its
first subscription. Connections that never subscribe retain the original
receive/execute/send path and ordinary GET/SET shared targets. Independent
output owners keep a slow subscriber from blocking another socket. Active
subscriptions are exempt from CONFIG timeout; the last unsubscribe restores
ordinary idle handling. Promoted query buffers close above one GiB.

The pub/sub output limits are fixed at 32 MiB hard and 8 MiB continuously for
more than 60 observed Unix seconds soft. Each connection is charged the
reserved capacities of outstanding frame bytes, including a shared message
body in full per recipient until consumed. Allocator rounding, reference
control and queue storage are not counted, so Firn does not promise Redis's
exact allocation-based disconnect point. CONFIG client-output-buffer-limit
is unsupported. Keyspace notifications remain unsupported.

Lists intentionally hold at most 4294967296 (2^32) elements, unlike Redis
7.2.16 on 64-bit. `LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LINSERT`, `LMOVE`
and `RPOPLPUSH` reject an addition beyond that limit with
`ERR list would exceed the maximum of 4294967296 elements`, before changing
any entry, including access metadata. A multi-element push is rejected as a
whole. A move checks its destination before popping its source; a rotation
within the same list needs no extra room. `LINSERT` still returns -1 for an absent
pivot, since that command would add nothing. The shared command bodies apply
the same rule on the network, in transactions, in scripts and during AOF
replay.

`SCAN` takes one map scan step per request; its unsigned decimal cursor is
zero when the scan ends, and a nonzero cursor can accompany an empty batch.
`MATCH` and `KEYS` patterns are binary and case-sensitive. The shared glob
matcher, also used by `CONFIG GET`, follows Redis 7.2.16's limit of 1000
nested evaluations of a nonterminal `*` suffix; another level does not match.
Adjacent stars collapse into one, trailing stars add no level, and retries,
escaped stars and character classes add no level. `TYPE` names are
case-insensitive. `KEYS` walks the whole keyspace in one atomic statement
and skips expired entries without removing them. `SCAN` removes expired
entries reached after `MATCH` and excludes their names even under
`TYPE none`, as Redis 7.2.16 does, and `RANDOMKEY` removes expired
candidates; both record those removals as `DEL`. `RANDOMKEY` draws as Redis's
`dbRandomKey` does, picking uniformly among the keys one scan step of count
15 gathers from a random position, wrapping at the table's end, with a
fresh position when it gathers none; its positions and draws are its own, so the key it returns
differs from Redis's. All five commands,
including both flush commands, run through their shared parts on the
network, in `EXEC` and through `redis.call` or `redis.pcall`.

`SSCAN`, `HSCAN` and `ZSCAN` return cursor `0` and the entire collection after
MATCH filtering in one call, including when the incoming cursor is nonzero.
They use firn's element order: map traversal for sets and hashes, and score
then member order for sorted sets. COUNT is validated as Redis 7.2.16 validates
it but does not limit this reply. Both protocols use an array of bulk strings;
hash field/value and sorted-set member/score pairs stay flat. A missing key
returns cursor `0` and an empty array before options are checked; a wrong type
is also checked before options, while an invalid cursor is checked first.
TYPE and NOVALUES are not collection-scan options in Redis 7.2.16.

This whole-collection path supplies SCAN's coverage guarantee but has work
and reply size proportional to the full collection. It is provisional while
board item coord-wfbl-02-01 supplies bounded resumable scanning for Whitefoot's
ordinary hash map; firn adds no direct bucket reads. Redis's compact-encoding
history, element order, cursors and dictionary batch boundaries are not
reproduced. ZSCAN uses d2string text while the current sorted set has at most
128 members and every member is at most 64 bytes; otherwise it uses `%.17g`,
which supplies Redis's dictionary ZSCAN `%.17Lg` digits for a widened double.
Both are bulk strings in both protocols. This selection examines all members
before MATCH and uses the default limits, independently of stored CONFIG
encoding values. A set that grew beyond a limit and later shrank returns to
d2string in firn, whereas Redis retains skiplist text after promotion.
The recorded reference comparison and boundary cases are in
[the collection-scan investigation](../research/investigations/firn/collection-scan.md).

`CONFIG GET` takes Redis's glob patterns over firn's parameters:
`appendfilename`, `appendonly`, `bind`, `databases`, `port`, `requirepass`,
`save`, `timeout`, `maxclients`, `auto-aof-rewrite-percentage` and
`auto-aof-rewrite-min-size`, and the parameters whose only effect in Redis is on its
internal encodings, leaving every value as commands read it,
`hash-max-listpack-entries`, `hash-max-listpack-value`,
`list-compress-depth`, `list-max-listpack-size`, `set-max-intset-entries`,
`set-max-listpack-entries`, `set-max-listpack-value`, which Redis 7.2 adds,
`stream-node-max-bytes`, `stream-node-max-entries`,
`zset-max-listpack-entries` and `zset-max-listpack-value`, with their aliases
`hash-max-ziplist-entries`, `hash-max-ziplist-value`, `list-max-ziplist-size`,
`zset-max-ziplist-entries` and `zset-max-ziplist-value`. firn has none of
those encodings: it reports their parameters with Redis's defaults and keeps
what `CONFIG SET` gives them, and they change nothing else, except
`stream-node-max-bytes` and `stream-node-max-entries`, which decide where a
stream's next node starts, as they do in Redis.
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
each attempt. Source-evaluating commands and `SCRIPT LOAD` explicitly reject **all** shebang
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

`CONFIG RESETSTAT` answers OK and zeroes connections admitted, connections
rejected, evicted keys and active-expiry counts. It preserves
`used_memory_peak`, the observed heap maximum since startup, as Redis 7.0.15 does.

`SHUTDOWN` sends no reply, nor the replies its connection holds unsent
from the commands before it, which Redis 7.0.15 does not send either; its
connection closes, and firn stops accepting clients. The request cancels
each client's receive through its wake generation, and the accept wait,
expiry sleep and signal wait through a separate shared shutdown state.
A command already running still finishes before its client closes;
cancellation does not interrupt scripts. On never-promoted connections,
sends retain their existing one-second deadline shutdown handling. Promoted
receive and output owners are cancelled directly and pending output is
discarded. Once every client has left, the append-only file's
writer makes one forced attempt to append its remaining bytes and one sync
attempt, then closes and firn exits with status 0. As in Redis 7.0.15's everysec
shutdown (`server.c:4239-4247,4301-4308`), failed appends or syncs do not retry
forever: retained bytes are abandoned and acknowledged changes may be lost.
No snapshot is written: the `save` schedule
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
`shutdown-on-sigint`. firn hears them from before it replays its append-only
file. One that comes during the replay ends it at the replay's next check,
made before the first command it reads and then before every 1024th, and firn exits with status 0
without cutting, creating, removing or rewriting anything in `appendonlydir`,
as Redis 7.2.16 serves events every 1024 commands while it loads
(`aof.c:1480-1485`) and exits there with status 0 (`server.c:1592-1599`),
opening its file for appending only afterwards (`server.c:7332`). An
old-style file is upgraded before the replay, as Redis does, so a stop during
that replay leaves the upgraded layout. One that comes after the replay's
last check stops firn in order once it listens. A host that refuses to
deliver the signals to firn stops it with status 3 before the replay, as an
address it cannot listen on does after it. Once
firn has taken a signal, or cancellation has ended its signal wait after a
`SHUTDOWN` request or its last client leaving after the `firn-clients` test shutdown limit, a
further SIGTERM or SIGINT ends it at once, without the rest of the drain;
Redis ends at once, with status 1, on a second SIGINT, but ignores a second
SIGTERM. `SHUTDOWN ABORT` answers `ERR No shutdown in progress.` after a
signal too, where Redis's cancels a signal's request in the moment, at most
a tenth of a second, before its next cron acts on it.

`INFO memory` reports raw `used_memory` and `used_memory_human`,
`used_memory_rss` and `used_memory_rss_human` when the host supplies it,
`used_memory_peak` and `used_memory_peak_human`,
`mem_not_counted_for_evict`, the configured `maxmemory`, `maxmemory_human`
and `maxmemory_policy`, with Redis's human-size formatting. It also reports
the actual registered-script count. Heap readings use `heap_in_use`; RSS
uses `resident_bytes` on demand. Peak tracks heap readings after commands,
at admission and at INFO, rather than every transient allocation. AOF
exclusion uses capacities even when drained, not lengths; `mem_aof_buffer`
reports those allocated buffers alone. `mem_not_counted_for_evict` adds the
rewrite ledger's current charges through cleanup, less the rotation-buffer
charges already covered by the AOF capacities (`firn_aof_rewrite_charged` in
`INFO persistence`), so until the attempt aborts a check can recompute it as
`mem_aof_buffer` plus `firn_aof_rewrite_used` less `firn_aof_rewrite_charged`
(after an abort `firn_aof_rewrite_used` is frozen at the first failure while
the exclusion follows the live ledger until cleanup); those charges are
conservative allowances rather than independent heap measurements. `INFO stats`
reports live `evicted_keys`. The heap meter counts the keyspace's own
storage, its shared-map tables and nodes, as well as values.

`INFO`, with no section, `default`, `all`, `everything` or named sections,
answers Redis's sections in Redis's order and form. Its fields carry real
values for the port, the calendar time, the uptime, the clients connected,
whether the append-only file is kept, the connections accepted and the keys
held, which it counts holding the table whole, as `DBSIZE` does.
`INFO clients` reports `maxclients` and `connected_clients`, excluding
connections refused by the limit. `INFO stats` reports
`total_connections_received` for admitted connections only and
`rejected_connections` for connections refused by the limit, both since
startup or the last `CONFIG RESETSTAT`.
The other fields it reports have values
that are fixed and true of firn: Redis's
version 7.2.16, standalone mode, 64 bits, its active
expiry's 10 runs a second, no configuration file,
function, replica, background save, fork, module, tracking or cluster.
The three pub/sub registry counts report distinct registered names from one
hub snapshot. What firn does not measure,
its own processor time, build revision, per-command and per-error counts
among them, is left out. CPU reports only the fixed zero child-process times;
Commandstats, Errorstats and Latencystats have empty headers,
and its keyspace line gives `keys` alone, without the `expires` and
`avg_ttl` firn does not count. Section selection is case insensitive, combines
several names without duplicates and ignores unknown names. `debug` selects
an empty Debug header explicitly; `all` and `everything` do not include it.

`CLIENT INFO` answers the connection's line in Redis's form, with real
values for the id, the name, the age, the protocol and `sub`, `psub`, `ssub`,
with the pub/sub flag reflecting active memberships, and values fixed and
true of firn for the others it gives. It leaves out what firn cannot report:
the peer's and its own address, which Whitefoot's socket address does not
show, the descriptor, the events and the sizes of Redis's buffers.

`HELLO` and `INFO` report the server as `redis` version 7.2.16, the
reference firn follows.

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
./build/firn --port 6379
redis-cli -p 6379 PING
```

`make firn` downloads the Whitefoot compiler release that `whitefoot.pin`
names and builds `build/firn` with an incremental cache, which keeps each
module's checked and compiled parts, so a rebuild after an edit compiles only
what the edit changed. A cached build does not link the runtime under
link-time optimization; `make firn-lto` builds `build/firn-lto` with
`--full-lto` instead, to measure firn's speed, as the benchmarks do.

The invocation is `firn [configfile] [--directive value ...]`. The first
argument, when it does not start with `--`, names a configuration file;
`-` reads configuration from standard input. The former four positional
arguments are removed. Command-line directives follow the file and override
its values; repeated directives take the last value. Each argument starting
with `--` begins a directive, whose values run up to the next such argument.

These argument boundaries have four differences from Redis's complete
command-line parser: firn always treats a `--` argument as a new directive,
accepts stdin `-` only in the initial configuration position, and treats a
first argument beginning with a single dash as a configuration filename.
Firn also refuses naked arguments after the configuration file and before
any `--` directive: `firn startup.conf timeout 3` is an error, while Redis
quotes those arguments into a configuration line and accepts it. Use
`firn startup.conf --timeout 3` instead.
Redis can consume a `--` argument as a preceding directive's first value,
also accepts a trailing stdin `-`, and classifies a single-dash first argument
as an option.

Configuration files use Redis 7.2.16's syntax: one directive per line,
case-insensitive names, blank lines and comments starting with `#` after
leading whitespace, and single or double quotes. Double quotes support
`\n`, `\r`, `\t`, `\b`, `\a`, `\\`, `\"` and `\xHH`; single quotes
escape only `\'`. A closing quote must be followed by whitespace or the end
of the line. Command-line values retain their spaces and quotes as argument
values rather than being parsed a second time as configuration text.

Supported directives, in files or after `--`, are:

- `port`, 6379 by default, and `timeout`, seconds before closing an idle
  client, 0 by default for no limit. A final `port 0` disables TCP and exits
  with status 1 because firn has no other listener;
- `maxclients`, the simultaneous client limit, 10000 by default, accepting
  integers from 1 through 4294967295 with Redis 7.2.16's validation errors;
- `bind`, one dotted IPv4 address or `*` for all IPv4 addresses. The default
  is 127.0.0.1; firn keeps its loopback default rather than Redis's default
  bind and protected-mode combination. More than one address and space-split
  address forms are refused. IPv6 and multiple listeners are firn limitations;
  Whitefoot's standard library supports both. `bind` with no values or one
  empty value disables the listener; with no other listener, startup exits 1
  with `Configured to not listen anywhere, exiting.`;
- `appendonly yes` or `no` (the default), and `appendfilename`, the prefix
  of the append-only files below `appendonlydir` in the working directory,
  `appendonly.aof` by default. Naming the prefix alone does not enable AOF;
- `requirepass`, the password clients give with `AUTH` or `HELLO`, empty by
  default;
- `auto-aof-rewrite-percentage` and `auto-aof-rewrite-min-size`, initially
  100 and 64 MiB;
- `maxmemory`, `maxmemory-policy`, `maxmemory-samples`,
  `maxmemory-eviction-tenacity`, `lfu-log-factor` and `lfu-decay-time`, using
  the same validation as `CONFIG SET`;
- `dir .` or `dir ./`, retaining the working directory;
- `firn-clients`, a **firn-only test directive**: how many connections to
  admit over the server's lifetime before stopping, 0 by default for no
  limit. Connections refused by `maxclients` do not count. This shutdown
  count is separate from the simultaneous client limit.

An accepted connection that would exceed `maxclients` receives
`-ERR max number of clients reached\r\n` and closes before any command is
served. The reply is best effort: partial sends share one absolute deadline
one second after the attempt starts. A send error, including expiry of that
deadline, ends the attempt and closes the connection even if sending is
incomplete. The host send contract permits zero progress, so this deadline
alone establishes no strict return-time bound.
`CONFIG SET maxclients` changes admission for subsequent connections;
lowering it below the current count leaves every existing client connected.
The pinned Whitefoot host APIs have no resource-limit facility, so firn
leaves `RLIMIT_NOFILE` unchanged. Unlike Redis 7.2.16, firn does not reduce
`maxclients` at startup or refuse `CONFIG SET` solely because the operating
system cannot supply enough descriptors; Redis can also refuse its event
loop's capacity adjustment. The configured application limit does not
promise descriptor availability. Host accept errors retain their existing
handling: any `accept` error, descriptor exhaustion included, ends the accept
loop and stops firn with exit status 0 (see the
[safety boundaries](../docs/safety.md)).

The interim path rules follow owner card `firn-q-ops-paths-interim` while
Whitefoot's path support is tracked by `firn-req-host-paths`. Configuration
paths are resolved from the working directory and must be accepted by the
standard library's relative-path interface. Absolute paths and paths it refuses
stop startup with a firn-specific diagnostic citing that gap. Every `include`
directive and every `dir` value other than `.` or `./` is refused in the
fatal configuration error frame, with a reason citing the same gap.
Startup does not create a data directory.

```sh
./build/firn startup.conf --port 6380 --requirepass secret --appendonly yes
./build/firn --port 6381 --firn-clients 1
```

Unknown directives, wrong argument counts, malformed quotes and invalid
values stop startup with status 1 and a Redis 7.2.16 fatal configuration
error on stderr. The error prints the trimmed source line and its line
number; command-line directives continue after the configuration file,
including Redis's extra separating newline. A file that cannot be opened
also stops startup with status 1 and a cannot-open diagnostic.

Common Redis configuration directives still refused include `daemonize`,
`supervised`, `pidfile`, `loglevel`, `logfile`, `databases`, `save`,
`appendfsync`, `protected-mode`, `tcp-backlog`, `tcp-keepalive`
and `aof-use-rdb-preamble`. Their default values receive no special
acceptance in this version.

Persistence uses Redis 7.0.15's manifest layout: with prefix `F`, a fresh
start creates `appendonlydir/F.1.base.aof` (empty), `F.1.incr.aof` and
`F.manifest`. Base files use commands, not an RDB preamble. Manifest updates
write `temp-F.manifest`, sync it, rename it over the manifest and sync the
directory. A listed file that is missing or malformed stops firn with status
4 before it listens. Only the last file may have an incomplete tail cut,
as with `aof-load-truncated yes`; an incomplete earlier file stops startup.
Listed history files are removed after loading only after syncing appendonlydir;
a failed sync leaves them listed and on disk. Unlisted files are kept except
this dataset's temporary journal `temp-F.journal`, whose cleanup is attempted
as described below.
An old single file `F` in the working directory is upgraded by persisting a
manifest and moving `F` into `appendonlydir`. Interrupted upgrades resume;
when both copies exist and the manifest names `F`, the directory copy wins.

The rewrite below is the reconciled-scan route under comparison ([investigation](../research/investigations/aof-rewrite/scan-log.md)); its design decisions await the owner's approval.

With append-only persistence enabled, `BGREWRITEAOF` answers started after
S0 admission. The worker scans the live map in separate statements while
mutations journal a changed key's complete image at its first write in the
rewrite and, for a command on that one key afterwards, the command's AOF
bytes. Statements stage complete records in bounded memory; the sole AOF
writer drains them outside atomic statements into `temp-F.journal`. The
writer seals the old increment and rotates once at S1 after draining the
frozen journal, then the worker reads the journal in bounded chunks to
reconcile the scan with ordered DEL-plus-reconstruction images and subsequent
effects. Installation selects
base(S1) and only the increment containing later writes, before removing old
files.

R is five percent of maxmemory, or 16 MiB when maxmemory is zero. It covers
rewrite memory, including staging, in-flight chunks and readback, while the
temporary journal can exceed R on disk. A full staging queue aborts the
rewrite without waiting for disk or blocking ordinary writes. Rewrite charges
are excluded from maxmemory, so a cache at its limit can start a rewrite;
admission requires no additional service headroom. Ordinary service pressure
still causes eviction or OOM. Sequence exhaustion, stopping or R too small
for fixed bookkeeping and one staging chunk still refuses admission. Capture
overflow, oversized images, flush, exhaustion, shutdown and I/O failure abort the attempt; client
commands and ordinary AOF appends continue. Aborts report `err`. Any
unsuccessful attempt, including a refused start, stops automatic rewrites
until a manual `BGREWRITEAOF` succeeds; Redis instead keeps retrying them with
a delay of up to an hour. A key is copied whole in one hold when the scan
reaches it or a write changes it during a rewrite, so a large key pauses
other clients for the copy. The rewrite holds about two copies of a key, so a
key above about half the reserve (8 MiB without maxmemory) aborts every
rewrite. EXEC and scripts still copy every key they change. The fixed staging
directory permits at most 8188 queued chunks, further limited by R. If startup
left the file inside an unfinished MULTI, the rewrite cuts the sealed file
before that MULTI; the new base holds the live writes appended after it, as
Redis's rewrite does.

The rewrite adds an allowance of at most R outside maxmemory, frozen at
admission even if CONFIG later changes the limit. This is not an RSS ceiling:
ordinary AOF buffers are already excluded, allocator and operating-system
overhead differ from requested heap, and commands in flight can overshoot.
After rewrite owners are released, completion removes the remaining
conservative ledger exclusion; later admission uses normal eviction or OOM
if counted memory is then above the limit.

The writer keeps the increment's syntax and unfinished-block boundary from
startup and advances it after successful appends, so a healthy switch does
not reread accumulated history. Successful append rollback preserves the
previous boundary. An uncertain append or truncation outcome invalidates it;
the next switch then uses the charged streaming reader to recover the state.
Collecting-buffer growth during rotation still consumes R and can abort a
rewrite when storage or this exceptional reread delays ordinary draining.

When a sync of the append-only file fails, firn refuses writes and `PING`
with Redis's `MISCONF` reply, still answers reads, and INFO reports
`aof_last_write_status:err`, as Redis does. firn then keeps syncing once a
second even while idle, which Redis does not. A sync that succeeds again
does not end the failure, because Linux does not write again the pages
whose writeback failed: it starts a rewrite of the dataset from memory, and firn accepts writes again only once
that rewrite's new base, incremental file and manifest are installed and the
directory synced. Meanwhile INFO shows `aof_rewrite_in_progress:1` with
`aof_last_write_status:err`. A recovery rewrite that cannot start or fails
reports `aof_last_bgrewrite_status:err`, stops automatic rewrites as any
failed attempt does, and is tried again after the next successful sync;
recovery rewrites run even while automatic rewrites are stopped. An idle
Redis 7.2.16 instead keeps refusing writes until an operator requests
a rewrite or restarts it; a successful old-file close-sync clears its saved
background sync failure before that rewrite finishes.

Each scan step calls `map_scan_within` with a limit of the reserve left
after the step's 64-KiB output buffer, and charges the bytes its KeySet then
retains as `key_set_bytes` reports them, so the scan never allocates beyond
R. Key copies, images and output-buffer growth are charged before they are
allocated. A step that does not fit is discarded unwritten and its cursor
kept. A step of several scan positions is retried at once one position at a
time. A one-position step waits outside every hold and retries while its
need so far could fit R less the fixed storage admission charged; for a
position whose keys alone do not fit the limit, that need also counts a
fresh KeySet's bytes and the step buffer. A step that could never fit, such as a
key longer than about R less 64 KiB and that fixed storage, aborts the
rewrite with `reserve`. The count hint asks for about that many keys per
step, whole scan positions kept together, which the specification leaves to
the runtime: it
starts at 64, halves after a step writes more than 64 KiB and doubles back
after one that writes at most 32 KiB. Steps run back to back until about a
millisecond has passed since the scan last waited, then it sleeps briefly,
yielding its thread to clients once per slice rather than once per key.
File calls still have no bounded cancellation contract, so prompt cleanup
under stalled I/O is unverified.

`rdb-key-save-delay`, Redis's hidden testing parameter, is answered by
`CONFIG GET` only to its whole name, never to a pattern, and `CONFIG SET`
accepts integers from `-2147483648` through `2147483647`, default `0`. Any
nonzero value makes the scan take one position per step. A positive value
then sleeps that many microseconds per key, as Redis's rewrite sleeps after
each key; a negative value `-n` sleeps one microsecond per `n` keys, where
Redis sleeps one microsecond after each key with probability `1/n`. The
driver's timer rounds each sleep up to whole milliseconds, so a delay below
1000 microseconds still costs about a millisecond. Redis's forked rewrite
keeps the value it started with; firn's scan reads it at each step, and a
sleep in progress, taken in slices of at most 100 ms, ends early when
CONFIG SET changes the value or when shutdown or an abort cancels the
rewrite, where Redis kills its rewrite child.

The temporary base `temp-F.base` and journal `temp-F.journal` sit beside
`temp-F.manifest`, so servers sharing `appendonlydir` with distinct
appendfilenames use distinct temporary names. The journal is never named by
a rewrite manifest and needs no fsync: the completed base is synced before
publication. Journal handles are closed and removal is attempted after
success or abort, including flush, shutdown and early I/O failures. Startup
attempts removal of an unlisted leftover after loading the authoritative
manifest and serves even if that cleanup fails; INFO reports the failure.
Before opening or truncating the next journal, a rewrite must remove any
unlisted old name. A failed removal refuses that rewrite with `journal_io`
and leaves the old file unchanged. A manifest-listed name is protected.
Those names can still collide with another dataset's upgraded base: an
appendfilename literally `temp-F.base` or `temp-F.journal` in another dataset
can overlap a temporary name used while rewriting `F`. Do not share a
directory with that overlap.
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
`CONFIG SET` also changes both thresholds, atomically with any other parameters
in the call, for the next automatic-rewrite decision. The percentage accepts
integers from `0` through `2147483647`; min-size accepts Redis memory units and
byte counts from `0` through `9223372036854775807`; as in Redis, a unit's
product wraps modulo 2^64 and an empty value or a bare unit reads as `0`.
`CONFIG GET` reports the
current percentage and min-size in decimal, min-size in bytes, and includes
both in glob matches. Setting the percentage to `0` disables automatic rewrites
without cancelling a rewrite already in progress.

With AOF enabled, a failed append or sync refuses Redis WRITE-flagged commands
and PING with `-MISCONF Errors writing to the AOF file: ` followed by the
host's description of the error, `strerror`'s C-locale text on Linux, such as
`File too large`; GET, INFO and other reads remain available
(`server.c:3881-3905,4324-4355`). A failed append is described when nothing
of it was written; otherwise the sync error is. After a short write Redis
reports `No space left on device` whatever the host returned
(`aof.c:1163-1184`); firn describes the short write by the host's text for the
no-space class, which leaves the suffix empty only when the host cannot
describe an error.
The command that exposed the failure can already have succeeded, since replies
do not wait for everysec durability. Partial appends attempt rollback to the
pre-flush incremental size, retaining all bytes on successful rollback and only
the unwritten suffix otherwise (`aof.c:1146-1230`). Failed appends retry every
100 ms, Redis's idle cadence at its default hz of 10; Redis also retries on
every event-loop iteration that client traffic causes (`server.c:1660-1666`),
which firn does not, so under traffic firn can recover up to 100 ms later.
Like Redis 7.2.16, firn syncs at most once a second and only after new bytes
were appended completely since its last sync, so an idle server issues no
sync and a device that fails while firn is idle shows only at the sync after
the next write (`aof.c:1071-1094,1236-1267`); a saved sync failure is the
exception, retried once a second while idle as described above.
A successful append clears an append failure, and so does a rewrite installed
directly while appends fail, described below; only a rewrite whose
installation is durable clears a sync failure; ordinary rewrite rotation
resets neither. This deliberately differs from Redis 7.2.16's successful
old-file fsync-and-close, which clears its background sync error before
rewrite completion; firn preserves the saved error until the new files are
durable ([recovery rule](../design/firn/append-only-file.md)).

A refused queued write or PING dirties MULTI. EXEC with a queued WRITE command
is discarded with EXECABORT and the MISCONF text while failure remains; read-only
EXEC runs. Legacy EVAL/EVALSHA can read, but their first write gets MISCONF through
redis.call's raised error or redis.pcall's error table. After an accepted WRITE
command, subsequent script writes skip that check (`multi.c:85-87,105-109,
136-140,170-178,229-233`; `script.c:349-386,547-557`).

`INFO persistence` reports rewrite progress, attempts, last rewrite status, current
active-file bytes and the rewrite baseline. Redis fields follow Redis 7.2.16's
order; the documented `firn_aof_rewrite_*` diagnostics follow them as a tail.
`aof_last_write_status` is `ok` only
when both append and sync errors are clear, otherwise `err`, independently of
`aof_last_bgrewrite_status`. With AOF enabled, `aof_buffer_length` reports the
collecting bytes plus bytes retained by the writer, excluding already appended
bytes awaiting sync (`server.c:5818-5832`). `aof_pending_bio_fsync` and
`aof_delayed_fsync` are zero: firn's sole writer performs sync itself, with no
background BIO queue or append postponement behind a background sync. The firn-only field
`firn_aof_rewrite_commit_seq` is the actual dataset statement sequence while
AOF is enabled. With AOF disabled a command that queues no expiry holds only
its keys and takes no sequence, so the field is left out, following the
[reported-facts rule](../design/firn/reported-facts.md).
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
KeySet creation and one map_scan_within call in the most recent attempt. The
latter supports the quiescent long-key regression independently of the
reservation ledger. Other contexts can affect this process-wide sample; it
is neither total rewrite peak nor RSS. `firn_aof_rewrite_cut` is 1 once the
latest admitted attempt has crossed S1 (also retained after completion),
and 0 otherwise; it lets the backlog regression begin writes after capture
has stopped.

`firn_aof_rewrite_journal_leftover` reports the last cleanup observation of
the unlisted temporary journal: 1 after removal failed, and 0 after absence
or successful removal was observed. It survives rewrite admissions, including
a startup observation made before any rewrite attempt. It is not a continuous
check of the current filesystem.

The following additional `INFO persistence` fields describe the current or
most recently completed attempt. They reset when admission is attempted,
including a refused admission; counters start at zero before the first attempt.

- `firn_aof_rewrite_abort_reason` is empty until failure and on success.
  The first cause is latched under Meta before cleanup, and later failures
  cannot replace it. Values are `reserve` (a rewrite buffer or boundary
  reserve charge), `capture` (statement capture allowance), `journal_slots`
  (missing staging queue),
  `journal_backlog` (insufficient staging slots or memory), `journal_io`
  (temporary-journal host I/O), `flush`,
  `sequence_exhausted`, `cancelled`, `base_io`, `writer_io`,
  `boundary` (sealed-file reading
  or validation), `install_io` (manifest publication or base installation),
  `rotation_reserve` (collecting-buffer growth), `sequence_invalid`
  (a scanned stamp beyond the commit sequence), `journal_invalid` (invalid
  framing or effect-byte range), and `admission` (insufficient reserve).
- `firn_aof_rewrite_used` and `firn_aof_rewrite_peak` are reserve ledger
  bytes, not heap or RSS measurements. While running they show current use
  and its peak; on failure both freeze at the first failure, before refunds.
  Successful completion reports zero used and retains the attempt's peak.
- `firn_aof_rewrite_journal_images` and
  `firn_aof_rewrite_journal_effects` count complete records successfully
  appended to the journal file as replacement images or statement AOF bytes,
  respectively. These asynchronous counts can lag committed writes while
  their records remain staged. A multi-key statement's image unit counts
  once, regardless of how many keys it holds; scan output does not count.
  Counts survive cleanup on failure and success.
- `firn_aof_rewrite_cleanup_failed` counts each journal close or removal
  step that returned failure during this attempt, including cleanup after
  an early error, and each failed close of the old incremental file or
  removal of an unlisted new base in a direct installation. It preserves the first abort cause and resets on admission.
  Startup cleanup leaves this count at zero and reports failed removal
  through `firn_aof_rewrite_journal_leftover` instead.
- `firn_aof_rewrite_scan_steps` counts completed map_scan_within calls,
  including the final empty step, steps refused for want of reserve, steps
  discarded to retry one position, and steps whose subsequent copying fails.
  `firn_aof_rewrite_scan_done` is 1 after the scan finishes writing its base,
  otherwise 0. It does not claim that rotation or installation succeeded.
- `firn_aof_rewrite_charged` is the part of the ledger that the rotation
  charged for collecting-buffer growth; `mem_not_counted_for_evict` excludes
  the ledger less this part, since those bytes are already in the AOF
  buffer capacities. It is the live value: unlike
  `firn_aof_rewrite_used` it does not freeze at the first failure.

A failed append does not abort a rewrite: its scan and journal continue while
writes are refused. If the bytes retained for the current incremental file
still cannot be written when the scan ends, firn seals the commands up to the
cut after them and retries only those, keeping later commands back. If they
drain, the rewrite rotates as usual; if the new base is complete first, firn
installs it directly, as Redis 7.2.16 installs a completed rewrite whatever
its append status: the new manifest names the new base and a new empty
incremental file, and the old base and every old incremental file become
history. Every retained byte belongs to a command the new base already holds,
so publication releases them and clears the append failure, and writes are
admitted again. This deliberately differs from Redis, which keeps its retained
bytes for its still-selected incremental file and refuses writes until they
are written ([direct installation](../design/firn/aof-rewrite.md)). A direct
installation that fails before publication keeps the old manifest, and the
retained bytes drain to the old incremental file once it accepts appends; the
new empty incremental file stays in place, as after a failed switch, because a
manifest whose directory sync failed may already name it.

Failure before rotation leaves the original manifest authoritative. After
rotation it leaves the manifest naming the old base and all increments
replayable. If manifest rename succeeds but
directory sync fails, the rewrite fails (`aof_last_bgrewrite_status` reports
`err`) and the error is saved as the sync error, so writes are refused with
`MISCONF` until a recovery rewrite is durably installed, as after a failed
sync of the incremental file. At the switch firn keeps appending to the old
incremental file, which either manifest names; at installation the new
manifest stays in force and all files and history entries are retained.
A directory sync that returns EINVAL, which means the filesystem cannot sync
directories, counts as synced, as in Redis 7.2.16: the rewrite completes and
writes continue.
`SHUTDOWN` accepted before installation cancels the rebuild at its next step;
once installation starts, shutdown waits for it to finish. Shutdown makes its
final append and sync attempts after clients leave and joins the rebuild before exiting.
See the [reconciled-scan protocol](../research/investigations/aof-rewrite/scan-log.md).

`WF_DRIVERS` sets how many threads serve the connections, one per CPU by
default.

## Layout

`modules.wfg` registers the modules, each a directory:

- `bytes`: byte strings, their hash and order, and integers and glob
  patterns read as Redis reads them;
- `protocol`: reading requests, writing replies, and a connection's state and
  settings;
- `connection_events`: pub/sub registries, committed output, cancellation,
  independent connection inboxes and output limits;
- `scores`: sorted-set scores and the ends of score ranges read as Redis's
  `strtod` reads them, to the nearest double; shared Redis 7.2.16 numeric
  text for score and script double replies (`d2string`), Lua command
  arguments, and rewritten append-only scores (plain `fpconv_dtoa`);
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
- `scripting`: the four EVAL commands and `SCRIPT`, the Redis Lua environment
  and the conversions between replies and Lua values; `script_pool` holds
  the one Lua engine every script takes in turn and the registry of
  scripts the keyspace shares;
- `persistence`: the append-only manifest, startup and upgrade, writer and replay;
- `server`: connections, active expiry, the invocation's options and `main`.
