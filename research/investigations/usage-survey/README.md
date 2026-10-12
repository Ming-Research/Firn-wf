# What production use needs from firn: commands, features, and the applications that validate them

## The question

firn's goal is that, when Whitefoot is released, users can run firn in production as a drop-in Redis
replacement for common workloads, with performance well ahead of Redis, Valkey, Dragonfly and Garnet. Its
reference is moving from Redis 7.0.15 to 7.2.16. What firn supports today was chosen from the traffic of three
applications ([consumers investigation](../consumers/README.md)): Django's cache and sessions, connect-redis
sessions, and rate-limiter-flexible's Lua scripts. Those three cannot say what the rest of production needs.

With data: which Redis commands and features does production use need, which of them does firn lack, and which
applications and clients should validate firn? This record also reviews the compatibility gaps the owner
cancelled on 2026-10-12 when "full parity with Redis" stopped being a goal, because that decision said to
look at them again once this survey gave a production scope.

## Method

Evidence comes in four kinds. Each claim below says which kind it rests on.

- **Source** (read from the application's code on GitHub). Five research subagents read the Redis use of about
  80 applications and client libraries at named release tags or default-branch commits, and reported the
  commands, shipped Lua scripts and feature flags with file locations. I re-checked a sample myself (marked
  "re-checked"); the rest are subagent reads that I did not repeat line by line.
- **Measured** (a number read from a public API on 2026-10-12): GitHub stars (`gh api`), npm weekly downloads
  (api.npmjs.org, 2026-10-04 to 2026-10-10), PyPI last-month downloads (pypistats.org), RubyGems and NuGet
  all-time downloads, Packagist downloads. These counts are not comparable across registries: PyPI and npm are
  recent windows that CI inflates, RubyGems and NuGet are lifetime totals.
- **Published vendor data** (compatibility pages, release notes, GitHub issues of peers).
- **Judgment** (my coding and inference). In particular the application-by-command matrix in the
  [appendix](#appendix-coded-command-sets) is my coding of the subagents' command lists. It counts command
  names an application sends on its normal path or in a Lua script it ships, once per application. It is not
  traffic: it says which applications a command stops, not how often production sends it.

Limits that matter for reading the ranking:

- Production traffic by command is not published anywhere I could find (see [Published usage data](#published-usage-data)).
  The ranking is therefore breadth of use across applications, not frequency of calls.
- The matrix works at command-name level. It does not check options (for example `SET ... IFEQ`), Lua library
  functions, reply shapes or error text. Those are listed separately where the sources showed them.
- Applications are weighted equally in the counts; popularity is shown beside them, not folded in.

firn's own state is read from `firn/README.md` and `firn/commands/info.wf` at `97b054b` (origin/main when
this was written). No build or run of firn or of any surveyed application was done.

## What firn supports today

From `firn/README.md` (I count about 135 of Redis 7.2.16's 242 top-level command names, counting a family
such as `CLIENT` once even where only some subcommands exist; 7.2.16 has 376 callable commands counting
subcommands, 7.0.15 has 372, per the [Redis command tables](#redis-7015-to-7216)).

- Keys, expiry, `SCAN` and `KEYS`, strings, lists, sets, hashes and sorted sets: nearly all of the single-key
  commands and the store variants for sets. Sorted sets lack `ZUNIONSTORE`, `ZINTERSTORE`, `ZUNION`, `ZINTER`,
  `ZDIFF`, `ZRANGESTORE`, `ZRANDMEMBER`, `ZMPOP` and every blocking `BZ*`. Lists lack `LMPOP` and every
  blocking `B*`.
- Scripting: `EVAL`, `EVALSHA`, `SCRIPT LOAD|EXISTS|FLUSH|KILL` on Halo, which has `cjson`, `cmsgpack`,
  `struct` and `bit` libraries (file names in Halo-wf `lib/halo/vm/library-*.wf`, read 2026-10-12). Scripts
  with a `#!` line are refused. `EVALSHA_RO`, `EVAL_RO`, `FUNCTION` (except `FLUSH`) and `FCALL` are unknown.
- `MULTI`, `EXEC`, `DISCARD`. `WATCH` is unknown.
- Connection: `HELLO` 2 and 3 (RESP3), `AUTH`, `PING`, `ECHO`, `QUIT`, `RESET`, `SELECT 0` only, and
  `CLIENT ID|GETNAME|SETNAME|INFO`. `CLIENT SETINFO`, `CLIENT LIST`, `CLIENT KILL`, `CLIENT TRACKING` and the
  rest are unknown subcommands.
- Server: `INFO`, `CONFIG GET|SET|RESETSTAT` over a fixed list of parameters, `TIME`, `COMMAND` (empty),
  `FLUSHALL`, `FLUSHDB`, `BGREWRITEAOF`, `SHUTDOWN`, `DEBUG LOG`.
- Not present: pub/sub, blocking operations, Streams, `SSCAN`/`HSCAN`/`ZSCAN`, bitmaps, HyperLogLog, GEO,
  `SORT`, `DUMP`/`RESTORE`, `OBJECT ENCODING`, `MONITOR`, `SLOWLOG`, `LATENCY`, `MEMORY`, ACL, replication,
  cluster, modules, more than one database.
- `INFO` has `redis_version`, `role`, `loading`, `uptime_in_days`, `connected_clients`, `maxmemory_policy`,
  `evicted_keys`, `total_connections_received`, `aof_enabled` among its fields. It has no
  `total_commands_processed`, `keyspace_hits`, `keyspace_misses`, `expired_keys`, `total_net_input_bytes` or
  command statistics (grep of `firn/commands/info.wf`).

## Data

### Popularity

Measured 2026-10-12. Downloads use each registry's own window, so compare within a registry only.

| Application or library | GitHub stars | Downloads (registry, window) |
|---|---|---|
| Django | 91,387 | |
| Rails | 58,816 | |
| Laravel (framework) | 34,959 | Packagist 16.9M/month |
| Symfony | 31,189 | Packagist symfony/cache 11.7M/month |
| Celery / kombu | 28,947 / 3,147 | PyPI celery 43.2M, kombu 44.3M /month |
| Redisson | 24,413 | |
| Bull | 16,254 | npm 1.66M/week |
| Sidekiq | 13,557 | RubyGems 340.5M total; sidekiq-cron 98.1M total |
| Asynq | 13,750 | |
| RQ | 10,692 | PyPI 9.2M/month |
| BullMQ | 9,497 | npm 9.72M/week (@nestjs/bullmq 1.77M/week) |
| Resque | 9,467 | RubyGems 55.1M total |
| Huey | 6,045 | PyPI 12.9M/month |
| Dramatiq | 5,330 | PyPI 1.64M/month |
| arq | 3,016 | PyPI 3.85M/month |
| express-session / connect-redis | 6,352 / 2,819 | npm 4.32M / 1.49M /week |
| rate-limiter-flexible | 3,589 | npm 3.19M/week |
| limits | 649 | PyPI 36.8M/month (dependency of Flask-Limiter and slowapi) |
| Envoy ratelimit | 2,707 | |
| Socket.IO redis adapter | 2,768 | npm @socket.io/redis-adapter 1.92M/week |
| redsync / node-redlock | 4,049 / 1,981 | npm redlock 1.08M/week |
| Spring Data Redis / Spring Session | 1,876 / 1,938 | |
| Flipper / Unleash Edge / LaunchDarkly relay | 3,971 / 67 / 134 | |
| redis_exporter (monitoring) | 3,696 | |

Client libraries: redis-py 13,647 stars (PyPI `redis` 200.4M/month); node-redis 17,582 (`@redis/client`
15.7M/week, `redis` 14.7M/week); ioredis 15,339 (31.1M/week); go-redis 22,256; Jedis 12,370; Lettuce 5,780;
StackExchange.Redis 6,204 (NuGet 1.23 billion total); redis-rb 4,002 (RubyGems `redis` 588.8M total,
`redis-client` 165.6M); phpredis 10,224; Predis 7,778 (Packagist 9.3M/month); rueidis 2,986; redis-rs 4,265;
hiredis 6,684.

Peers: Dragonfly 31,781 stars, Valkey 27,402, Garnet 12,048, Redis 76,741.

### Published usage data

I looked for command-frequency or feature-adoption data and found none. No vendor publishes call shares by
command; vendor pages send users to `INFO commandstats` for their own workload. Searched and not found: a Redis
Inc. "State of Redis" report, Datadog per-command statistics, ElastiCache or re:Invent frequency data. Not
usable: the Twitter OSDI 2020 cache study (it analyses Twemcache and Pelikan, and its trace operations are
memcached operations: [repo](https://github.com/twitter/cache-trace)).

What exists is adoption evidence only: the Stack Overflow 2025 survey puts Redis at 28% of all respondents and
30.7% of professionals, fifth among databases
([survey](https://survey.stackoverflow.co/2025/technology)); Valkey is at 2.4%. Nothing there is about commands.

### The applications read

Source reads, by category. "Needs beyond firn today" lists command-level gaps in the application's normal
path or shipped scripts (the matrix, so not options). All 63 coded applications are in the
[appendix](#appendix-coded-command-sets); this table is the 29 of them that need something firn lacks.

| Application | Stars | Needs beyond firn today | Source |
|---|---|---|---|
| Celery broker (kombu 5.6.2) | 28,947 | `BRPOP` (4 priority keys, 1 s), `PSUBSCRIBE`/`PUBLISH` fanout (on by default: Mingle, Gossip, Heartbeat, Control worker steps), `WATCH` (`restore_by_tag`), `MULTI` for ack | `kombu/transport/redis.py:963-977,903,435,365`; `celery/worker/consumer/consumer.py:169-180` |
| Celery result backend (5.6.3) | | `SUBSCRIBE` for `AsyncResult.get`, `PUBLISH` inside `MULTI` | `celery/backends/redis.py:107-172,432-438` |
| Sidekiq 8.1.7 | 13,557 | `BRPOP` (every processor, 2 s); `SSCAN`, `ZSCAN` in the Web UI and API; `BITFIELD`/`BITFIELD_RO` for metrics; one shipped script (`ZRANGE ... BYSCORE LIMIT` + `ZREM`) via `SCRIPT LOAD` + `EVALSHA`; refuses to start below Redis 7.0 (checks `INFO` `redis_version`) | `fetch.rb:48`, `scheduled.rb:12-62`, `metrics/tracking.rb`, `cli.rb:76-78` |
| BullMQ 6.3.13 | 9,497 | `BZPOPMIN` (idle worker), `XADD`/`XTRIM`/`XREAD BLOCK` (events), `CLIENT LIST` (`getWorkers`), `SSCAN`/`HSCAN` inside Lua, 50 Lua scripts using `cjson`, `cmsgpack` and keys built from `ARGV`; version 5.0 hard floor, 6.2 recommended, parsed from `INFO` | `src/classes/redis-queue-backend.ts:2791,2863,1995`, `src/commands/*.lua`, `redis-connection.ts:154` |
| RQ 2.12 | 10,692 | `BLMOVE` (>= 6.2) or `BLPOP`, `SUBSCRIBE`/`PUBLISH` (worker commands), `WATCH`, Streams for results (`XADD`, `XREAD BLOCK`, `XREVRANGE`, `XLEN`), `CLIENT LIST` (tolerates failure) | `queue.py:1533,1558`, `worker/base.py:1056,252`, `results.py:263,290` |
| Bull 4.16.5 | 16,254 | `BRPOPLPUSH`, `PSUBSCRIBE`/`SUBSCRIBE`, `PUBLISH` inside 29 Lua scripts, `CLIENT LIST`, `SCAN` inside a script | `lib/queue.js:1246,543`, `lib/commands/*.lua` |
| Asynq 0.26.0 | 13,750 | `SUBSCRIBE`/`PUBLISH` (task cancel channel); 49 Lua scripts otherwise | `internal/rdb/rdb.go:1488,1501` |
| Dramatiq 2.2.1 | 5,330 | `BRPOPLPUSH` (results, rate-limit barriers), `WATCH` (rate limiter) | `results/backends/redis.py:93`, `rate_limits/backends/redis.py:49-107` |
| Laravel queue (13.35.0) | 34,959 | `BLPOP` only when `block_for` is set (default off: polls and sleeps) | `Queue/RedisQueue.php` |
| Laravel Horizon 5.50.0 | 4,176 | `ZSCAN` inside the purge script | `src/LuaScripts.php` |
| Huey 3.4.0 | 6,045 | `BRPOP` (default), `BZPOPMIN` (priority), `BLPOP` (result wait) | `huey/storage.py:557,793,646` |
| node-resque 9.7.1 | 1,414 | `WATCH`/`UNWATCH` in the scheduler cleanup (optional) | `scheduler.ts:324` |
| arq 0.28.0 | 3,016 | `WATCH` on enqueue and job start (grep-level read only) | `connections.py:156`, `worker.py:451` |
| Spring Session indexed repository 4.1.1 | 1,938 | `PUBLISH`, `SUBSCRIBE`, `PSUBSCRIBE`, and keyspace notifications (`CONFIG SET notify-keyspace-events`, `__keyevent@db__:expired`). The simple repository, `@EnableRedisHttpSession` in 4.x, needs none of it | `RedisIndexedSessionRepository.java:925,476`, `ConfigureNotifyKeyspaceEventsAction.java:55-80`, `RedisHttpSessionConfiguration.java:64` |
| Laravel cache tags (13.35.0) | 34,959 | `ZSCAN` (tag listing). Untagged cache and sessions need nothing new | `Cache/RedisTagSet.php` |
| Symfony tag-aware cache 8.1.8 | 31,189 | `SSCAN` and `SUNIONSTORE` inside Lua; reads `maxmemory_policy` from `INFO` and refuses anything but `noeviction` or `volatile-*` | `RedisTagAwareAdapter.php` |
| @neshca/cache-handler (Next.js) | 541 | `HSCAN` (strings variant), or RedisJSON and RediSearch (stack variant) | `handlers/redis-strings.ts`, `redis-stack.ts` |
| python-redis-lock 4.0.1 | 572 | `BLPOP` on a signal key when the lock is busy | `src/redis_lock/__init__.py:230` |
| Redisson 4.8.0 (default `RLock`) | 24,413 | `SUBSCRIBE` while waiting, `PUBLISH` inside the unlock script (`redis.call(ARGV[4], ...)`), startup `EVAL` touching two keys; `PUBSUB NUMPAT` probe | `RedissonLock.java:214,347,313`, `PublishSubscribeService.java:1189`, `MasterSlaveConnectionManager.java:285` |
| Socket.IO redis adapter 8.3.0 | 2,768 | `PSUBSCRIBE`/`SUBSCRIBE`/`PUBLISH`, `PUBSUB NUMSUB`; sharded mode `SSUBSCRIBE`/`SPUBLISH` | `lib/index.ts:153,894` |
| Socket.IO redis-streams adapter 0.3.1 | 42 | `XADD`, `XREAD BLOCK`, `XRANGE`, `PUBSUB NUMSUB` | `lib/util.ts:123-196` |
| Rails ActionCable (8.1.x) | 58,816 | `SUBSCRIBE`/`PUBLISH`, over a RESP3 connection by default | `subscription_adapter/redis.rb:39,113` |
| Django channels_redis 4.3.0 | 650 | core layer: `BZPOPMIN`; pub/sub layer: `PUBLISH`/`SUBSCRIBE`; Lua with `KEYS` | `core.py:217-237`, `pubsub.py:274` |
| ASP.NET SignalR backplane (10.0.12) | 38,469 | `SUBSCRIBE`/`PUBLISH` | `RedisHubLifetimeManager.cs:299` |
| Laravel broadcaster / Reverb 1.12.0 | 1,627 | `PUBLISH` (also from a zero-key `EVAL`), `SUBSCRIBE`/`PSUBSCRIBE` | `RedisBroadcaster.php:173`, `RedisSubscribeClient.php:17` |
| Phoenix.PubSub Redis | | `PUBLISH`/`SUBSCRIBE` | `redis_server.ex:31` |
| GrowthBook proxy 1.4.0 | | `PUBLISH`/`SUBSCRIBE` | `RedisCache.ts:216,248` |
| LaunchDarkly relay store | 134 | `WATCH` (default upsert mode; Lua upsert mode is an option) | `redis_impl.go:415-460` |
| agoragames/leaderboard | 481 | `ZUNIONSTORE`, `ZINTERSTORE` | `lib/leaderboard.rb` |

The 34 coded applications that need only commands firn has include all the cache and session stores that were
read apart from the ones above (Rails cache store and session stores, Spring Data cache, Spring Session simple
repository, Symfony plain cache and lock and session handler, Flask-Caching, Flask-Session, django-redis,
Django's cache, connect-redis, keyv, unstorage, the ASP.NET Core cache), all the rate limiters (rate-limiter-flexible,
limits, redis_rate, rate-limit-redis, Envoy ratelimit, Laravel's limiters), the lock libraries that use
only `SET NX PX` and scripts (node-redlock, redsync, redis-py `Lock`, Laravel `RedisLock`, Spring Integration's
default spin lock), and Resque, redbeat, APScheduler's job store, sidekiq-cron, Flipper, Unleash Edge and the
Split synchronizer. That is by command name; option and client-handshake gaps follow below.

Further findings from the reads that decide scope details:

- **Lua carries the hot path of limiters, locks and schedulers** [source]. Commands called inside shipped
  scripts include `PUBLISH` (Bull, Redisson unlock, Laravel broadcaster), `KEYS` (limits `clear_keys.lua`,
  python-redis-lock, channels_redis, Dramatiq, Split), `SCAN` (Bull), `TIME` (redis_rate), `MEMORY USAGE`
  (Asynq inspector), and a command name taken from `ARGV` (Redisson, BullMQ for `LPUSH`/`RPUSH`).
  Library features used: `cjson`, `cmsgpack` (BullMQ, 18 uses), `redis.pcall` (node-redlock),
  `redis.replicate_commands()` (redis_rate), `redis.error_reply` (BullMQ), `unpack` with large argument lists
  (Dramatiq probes the stack limit), and keys not declared in `KEYS` (BullMQ, Asynq, Dramatiq). Whether Halo
  supports `redis.replicate_commands` and the `unpack` limit is not verified by this survey.
- **Scripts that begin with `#!`: none** [re-checked]. A grep for `#!lua` over the cloned sources of the
  queue, lock, limiter and cache applications found it only in the `FUNCTION LOAD` tests of go-redis and
  ioredis, not in any script an application ships. So the refusal of script flags costs no surveyed
  application.
- **EVAL versus EVALSHA** [source]. EVALSHA with a fallback: ioredis, redis-py `Script`, go-redis,
  node-redlock, redsync, Redisson, rate-limit-redis, Sidekiq (`SCRIPT LOAD` first). `EVAL` only: Laravel,
  node-redis in rate-limiter-flexible, django-redis `incr`, channels_redis, Envoy's decrement script.
  Error text is matched: NOSCRIPT by prefix or substring (redsync `"NOSCRIPT "`, rate-limit-redis
  `/^NOSCRIPT(\s|$)/`, ioredis, Redisson, redigo, the LaunchDarkly SDK); Spring Integration falls back from
  `SET ... IFEQ` and `DELEX` only on `ERR syntax error` or `ERR unknown command`.
- **`SELECT n`** [source]. Symfony's `RedisTrait` sends `SELECT dbindex` on every connect; Rails, Celery
  (`_prepare_virtual_host`), Sidekiq, Resque and Spring Session accept a database in the URL. Sidekiq's own tests
  default to `redis://localhost:6379/0` (re-checked, `test/helper.rb:24`). Whether production deployments
  commonly use a non-zero database is not measured here.
- **`INFO` is parsed** [source]: Sidekiq (`redis_version` at start-up and in the Web UI, plus
  `uptime_in_days`, `connected_clients`, `used_memory_human`, `used_memory_peak_human`), BullMQ
  (`redis_version`, `maxmemory_policy`), Rails, Symfony, keyv, RQ, Huey, Spring Boot's health check
  (`redis_version`), and ioredis's ready check (`loading` absent or 0).

### What the clients send on their own

The applications reach Redis through a client, and the client's own handshake is the first thing firn sees
[source; subagent read of each client at the tag named in the sources list, summarized].

| Client | Sent on connect by default | If the server errors |
|---|---|---|
| redis-py 8.1.0 | `HELLO 3`, `CLIENT MAINT_NOTIFICATIONS`, `CLIENT SETINFO` x2, `SELECT` if db != 0 | `HELLO` error is fatal, no RESP2 fallback; the other two are tolerated |
| node-redis 6.2.1 | `HELLO 3`, `SELECT`, `CLIENT SETINFO` x2, `CLIENT MAINT_NOTIFICATIONS` | `HELLO` error is fatal; the other two ignored |
| ioredis 6.0.0 | `HELLO 3`, `SELECT`, `CLIENT SETNAME`, `CLIENT SETINFO` x2, then `INFO` ready check | `HELLO` unknown falls back to RESP2; `INFO` error (other than NOPERM) is fatal; `loading` must be absent or 0 |
| Jedis 8.0.2 (`RedisClient`) | `HELLO 3`, `CLIENT SETNAME`, `CLIENT SETINFO` x2, `SELECT` | falls back to RESP2 if `HELLO` unknown; the legacy `Jedis` class sends no `HELLO` |
| Lettuce 7.8.0 | `HELLO 3` (+`SETNAME`), `CLIENT SETINFO` x2, `CLIENT MAINT_NOTIFICATIONS` | falls back to RESP2 on unknown command or NOPROTO |
| Redisson 4.8.0 | `AUTH`, `SELECT`, `CLIENT SETNAME`, `PING` every 30 s; `HELLO 3` only if configured; no `CLIENT SETINFO` | every handshake command must succeed; startup `EVAL`; `PUBSUB NUMPAT` probe |
| go-redis 9.23.0 | `HELLO 3`, `SELECT`, `CLIENT SETNAME`, `CLIENT SETINFO` x2, `CLIENT MAINT_NOTIFICATIONS` | falls back to RESP2 on a Redis error from `HELLO` |
| rueidis / valkey-go 1.0.78 | `HELLO 3`, `CLIENT TRACKING ON OPTIN`, `CLIENT SETINFO` x2; cluster-first discovery with `CLUSTER SLOTS` | an error on `CLIENT TRACKING` fails the connection (`ErrNoCache`) unless `DisableCache` is set |
| StackExchange.Redis 3.4.0 | `HELLO 3`, `CLIENT SETNAME`, `CLIENT SETINFO` x2, `CLIENT ID`, `CLUSTER SLOTS`, `CLUSTER NODES`, `SENTINEL MASTERS`, `INFO replication`, `INFO server`, `CONFIG GET` x3, `GET __Booksleeve_TieBreak`, `ECHO <guid>` | only the `ECHO` tracer must succeed; the others are fire-and-forget |
| redis-client 0.31.0 / redis-rb 6.0.0 (Sidekiq, Rails, Resque) | `HELLO 3`, `SELECT`, `CLIENT SETINFO` x2 | `HELLO` "unknown command" raises `UnsupportedServer` (redis-rb falls back to RESP2); `CLIENT SETINFO` errors are skipped |
| Predis 3.7.0 | `HELLO 2 SETNAME`, `CLIENT SETINFO` x2, `SELECT` | falls back to `AUTH` |
| redis-rs 1.7.1 | `CLIENT SETINFO` x2 (RESP2 default) | replies ignored |
| phpredis 6.3.0, redigo 1.9.3, hiredis 1.4.1 | `AUTH`/`SELECT` only if configured; nothing else | |

Consequences, all [source] unless noted:

- **RESP3 is the default of most current clients** (redis-py 8, node-redis 6, ioredis 6, go-redis 9, Lettuce,
  rueidis, redis-client, StackExchange.Redis). firn answers `HELLO 3` already; for redis-py 8 and node-redis a
  `HELLO` failure is fatal, which is the failure the consumers investigation observed.
- **`CLIENT SETINFO` is sent by every mainstream client except Redisson and the PHP and C clients**, since 2023
  (redis-py 5.0.0, node-redis 4.6.11, go-redis 9.2.0 and 9.5.0, Jedis 4.4.0, Lettuce 6.3.0). Every client
  tolerates an error reply today, so firn works as is; but it is a Redis 7.2 command and the reference moves to
  7.2.16.
- **Subscriber connections and client-side caching** matter for two clients: rueidis and valkey-go fail to
  connect unless `DisableCache: true` is set; Redisson cannot take its default lock.

### Peers

What other servers chose, from their own pages (counts by script over the raw tables, subagent-run):

- **Garnet 2.2.1** ([compatibility table](https://microsoft.github.io/garnet/docs/commands/api-compatibility),
  commit `7fc2af6`): about 279 of about 375 core command rows supported. Supported: every list command
  including blocking ones, all 44 sorted-set commands including `BZPOPMIN` and `ZUNION`, `WATCH`, pub/sub,
  bitmaps, GEO, HyperLogLog. Not supported: all 23 Streams commands, all 11 `FUNCTION` commands, modules,
  `RANDOMKEY`, `SORT`, `TOUCH`, `WAIT`, `COPY`, `MOVE`, `SHUTDOWN`, `BGREWRITEAOF`, ten `CLIENT`
  subcommands (`TRACKING` among them). Lua is opt-in with `--lua`. Its own page says it is "not meant to be a
  fully identical Redis replacement".
- **Dragonfly 2.0.2** ([compatibility](https://www.dragonflydb.io/docs/command-reference/compatibility),
  docs commit `fe3bea5`): List 22/22, Set 17/17, Sorted Set 35/35, Pub/Sub 13/13, Transactions 5/5 supported;
  Streams 10 supported and 4 partial; Scripting 6 supported, 1 partial, 5 unsupported (`FCALL` and `FUNCTION`
  all, `SCRIPT KILL`, `SCRIPT DEBUG`); cluster 8 of 30 `CLUSTER` subcommands; `OBJECT ENCODING|FREQ|IDLETIME|
  REFCOUNT`, `MIGRATE`, `LCS`, most `LATENCY` and several `CLIENT` subcommands unsupported. Its README says it
  started at "~185 Redis commands (roughly equivalent to Redis 5.0 API)".
- **Valkey** 9.1.2 has every top-level command of Redis 7.2.16 plus 16 of its own (hash field expiry,
  `MSETEX`, `DELIFEQ`, `COMMANDLOG`, `CLUSTERSCAN`) (counted from `src/commands/*.json`).
- **Managed services** restrict administration, not data commands. ElastiCache and MemoryDB block `CONFIG`,
  `DEBUG`, `SAVE`, `BGSAVE`, `BGREWRITEAOF`, `REPLICAOF`, `PSYNC`, `MIGRATE`, `SHUTDOWN` and cluster slot
  management; ElastiCache Serverless in addition blocks `KEYS`, `MONITOR`, `SLOWLOG`, `LATENCY`, `MEMORY`,
  `OBJECT`, `WAIT`, `PSUBSCRIBE`, `WATCH`, `FUNCTION`, and most `CLIENT` subcommands. Memorystore (7.2) blocks
  `ACL`, `CLIENT`, `CLUSTER`, `COMMAND`, `CONFIG`, `DEBUG`, `OBJECT`, `MIGRATE`, `REPLICAOF`, `SAVE`,
  `SHUTDOWN`. Azure disables `ACL`, `CONFIG`, `DEBUG`, `MIGRATE`, `REPLICAOF`, `SAVE`, `SHUTDOWN`
  ([ElastiCache](https://docs.aws.amazon.com/AmazonElastiCache/latest/dg/SupportedCommands.html),
  [Memorystore](https://docs.cloud.google.com/memorystore/docs/redis/supported-commands),
  [Azure](https://learn.microsoft.com/en-us/azure/azure-cache-for-redis/cache-configure)). Apache Kvrocks
  supports 343 top-level commands, including Streams, pub/sub, blocking operations and `WATCH`.
- **What users asked of Garnet**, from its issues (GitHub, read 2026-10-12): blocking list operations were
  called "a non-starter for job systems" by Sidekiq's author
  ([#216](https://github.com/microsoft/garnet/issues/216)); `EVAL` was a blocker for a team using Redisson
  ([#509](https://github.com/microsoft/garnet/issues/509)); `SELECT` and multiple databases is the most-reacted
  feature issue (22 reactions, [#61](https://github.com/microsoft/garnet/issues/61)); Streams
  ([#1379](https://github.com/microsoft/garnet/issues/1379)) and keyspace notifications
  ([#915](https://github.com/microsoft/garnet/issues/915)) are open requests. This is the only user-demand
  evidence I found, and it is small (a handful of issues).
- Neither Dragonfly nor Garnet documents why it chose its order. The cross-peer pattern is inference: all of
  them implement the five core types, `MULTI`/`EXEC`, `EVAL`/`EVALSHA`, pub/sub and blocking list
  operations (Garnet only after the Sidekiq issue); the commonly dropped items are `FUNCTION`, modules,
  replication, latency and memory introspection, cluster topology and several `CLIENT` subcommands.

### Operational tools

A drop-in replacement is also read by monitoring. `redis_exporter` (3,696 stars, v1.93.0, commit `6de1e8b`,
re-checked in `exporter/*.go`) sends `INFO` and `INFO ALL`, `CONFIG GET`, `CLIENT LIST`, `SLOWLOG LEN|GET`,
`LATENCY`, `MEMORY USAGE`, `COMMAND INFO`, `SCAN`, `TYPE`, `PING`, and exports
`total_commands_processed`, `keyspace_hits`, `keyspace_misses`, `expired_keys`, `evicted_keys`,
`total_net_input_bytes`, `connected_clients` and `used_memory`. firn's `INFO` lacks five of those (commands processed, keyspace hits, keyspace misses, expired keys, net input bytes; grep of `firn/commands/info.wf`).
This is one tool, not a survey of tools.

### Redis 7.0.15 to 7.2.16

Counted from `src/commands/*.json` at the tags (subagent-run): 7.0.15 has 372 callable commands, 7.2.16 has
376. The additions are `WAITAOF`, `CLIENT NO-TOUCH`, `CLIENT SETINFO` and `CLUSTER MYSHARDID`; none removed.
Behavior changes between the two that firn's replies must follow ([7.2 release notes](https://github.com/redis/redis/releases/tag/7.2-rc1)
and later 7.2.x notes):

- `ZRANK` and `ZREVRANK` take `WITHSCORE`; the reply is a two-element array.
- A failed `HELLO` no longer changes the client's state (#11659).
- Time is frozen for the duration of a command and of a script (`TIME`, expiry checks).
- Blocking commands lose the `NOSCRIPT` flag: inside a script they run like inside `MULTI` (non-blocking).
- `HINCRBYFLOAT` no longer creates a key when the increment is invalid; `LPOS` with `RANK` equal to the
  minimum 64-bit value no longer returns a wrong result.
- Lua number to argument conversion regressed in 7.2.0 and was fixed in 7.2.5 (#13115); from 7.2.11 a script
  that modifies the shared string metatable errors with "Attempt to modify a readonly table" (board item
  `halo-firn-redis72-needs`, which already tracks the Halo side).
- `QUIT` is deprecated but works.

## Ranking

### Commands by number of coded applications that send them

Of 63 coded applications. The right column says whether firn has it (Y), has it partly (P) or lacks it (-).
All 63 are listed in the appendix; the top of the order, then every command firn lacks:

| Count | Commands | firn |
|---|---|---|
| 49, 48, 46 | `GET`, `DEL`, `SET` | Y |
| 27 | `EXISTS`, `EXPIRE` | Y |
| 23 | `MULTI`, `EXEC` | Y |
| 22 | `HSET` | Y |
| 21 | `EVALSHA` | Y |
| 20 | `ZADD` | Y |
| 18 | `HGET`, `MGET` | Y |
| 17 | `ZREM`, `SADD` | Y |
| 16 | `SMEMBERS`, `SREM`, `EVAL` | Y |
| **14** | **`PUBLISH`** | **-** |
| 14 | `HGETALL`, `SCAN` | Y |
| **13** | **`SUBSCRIBE`** | **-** |
| 13 | `KEYS`, `HDEL`, `RPUSH` | Y |
| 11 | `INFO`, `UNLINK`, `LPUSH`, `LLEN` | Y (INFO partial in fields) |
| 7 | `SCRIPT` | P (no `EVALSHA_RO`) |
| **6** | **`PSUBSCRIBE`, `WATCH`** | **-** |
| 4 | `CLIENT` | P (no `LIST`, `SETINFO`) |
| **4** | **`UNSUBSCRIBE`, `BLPOP`** | **-** |
| **3** | **`BRPOP`, `ZSCAN`, `SSCAN`, `XREAD`, `BZPOPMIN`, `XADD`, `PUBSUB`** | **-** |
| 2 | `SELECT` | P (0 only) |
| **2** | **`HSCAN`, `BRPOPLPUSH`, `UNWATCH`** | **-** |
| **1** | **`BITFIELD`, `BITFIELD_RO`, `XTRIM`, `XLEN`, `BLMOVE`, `XREVRANGE`, `XRANGE`, `PUNSUBSCRIBE`, `ZINTERSTORE`, `ZUNIONSTORE`** | **-** |

Every command firn already has is in the top half of the table. The commands firn lacks are few in number
but each stops a whole category.

### Features by number of applications they stop

Applications needing the family, of 63 (judgment: family unions of the commands above):

| Feature family | Applications | Of which only this family blocks them (adds if built alone) |
|---|---|---|
| Pub/sub (`PUBLISH`, `SUBSCRIBE`, `PSUBSCRIBE`, ..., `PUBSUB`) | 15 | 10 (Celery results, Asynq, ActionCable, SignalR, Phoenix, GrowthBook, Laravel broadcaster, Redisson default lock, Socket.IO adapter, Spring Session indexed) |
| Blocking list operations (`BLPOP`, `BRPOP`, `BRPOPLPUSH`, `BLMOVE`) | 8 | 2 (Laravel queue, python-redis-lock) |
| `WATCH`/`UNWATCH` | 6 | 3 (LaunchDarkly relay, arq, node-resque) |
| `SSCAN`/`HSCAN`/`ZSCAN` | 6 | 4 (Laravel cache tags, Horizon, Symfony tags, Next.js handler) |
| `BZPOPMIN` | 3 | 0 (BullMQ, Huey, channels_redis each also need something else) |
| Streams (`XADD`, `XREAD`, `XRANGE`, `XREVRANGE`, `XLEN`, `XTRIM`) | 3 | 0 (BullMQ, RQ, the Socket.IO streams adapter) |
| `BITFIELD` | 1 | 0 (Sidekiq metrics) |
| `ZUNIONSTORE`, `ZINTERSTORE` | 1 | 1 (agoragames/leaderboard; Redisson uses more) |

Applications fully served at command-name level, adding the families in this order:

| Adding | Served of 63 |
|---|---|
| Today | 34 (queues 4 of 17, caches and sessions 15 of 19, limiters and locks 12 of 14, realtime 0 of 7, flags and other 3 of 6) |
| + pub/sub | 44 |
| + blocking list operations | 47 |
| + `WATCH` | 52 |
| + `SSCAN`/`HSCAN`/`ZSCAN` | 56 |
| + `BZPOPMIN` | 58 |
| + `BITFIELD` | 59 |
| + minimal Streams | 62 |
| + `ZUNIONSTORE`/`ZINTERSTORE` | 63 |

This is a count of applications, not users. The 29 unserved applications include the six queue systems with
the most stars among those read (Celery, Bull, Asynq, Sidekiq, RQ, BullMQ; Resque, which is served, comes
next with 9,467) and Redisson, so the families above matter more than the application count suggests:
firn serves 4 of 17 queue and scheduler applications today.

## Proposal

### Production scope

What firn must support at release, each with the applications that need it. "Have" means firn has it today.
The scope follows the data above; where it rests on judgment the text says so.

| # | Feature | Needed by | Status |
|---|---|---|---|
| 1 | The command families firn has, with RESP2 and RESP3 | all 63 | have |
| 2 | Client handshake: `HELLO 2|3`, `AUTH [user] pass` for the default user, `CLIENT SETNAME|GETNAME|ID|INFO`, **`CLIENT SETINFO LIB-NAME|LIB-VER` answering OK** (a 7.2 command; every client tolerates an error, but the reference now has it), **`CLIENT LIST`** with the fields clients read (id, name, addr), **`CLIENT NO-TOUCH`** and the other 7.2 additions the reference adds | redis-py, node-redis, ioredis, Jedis, Lettuce, go-redis, redis-client send `SETINFO`; `CLIENT LIST`: BullMQ `getWorkers`, RQ, Bull, redis_exporter | partly |
| 3 | **`INFO`** fields clients and monitors read: `redis_version`, `redis_mode`, `role`, `loading:0`, `maxmemory_policy`, `uptime_in_days`, `connected_clients`, `used_memory*`, plus **`total_commands_processed`, `keyspace_hits`, `keyspace_misses`, `expired_keys`, `total_net_input_bytes`** | ioredis (ready check), Sidekiq (version gate and Web UI), BullMQ, Symfony tags, Rails, keyv, RQ, Huey, redis_exporter | most; the five counters missing |
| 4 | **Pub/sub**: `SUBSCRIBE`, `UNSUBSCRIBE`, `PSUBSCRIBE`, `PUNSUBSCRIBE`, `PUBLISH`, `PUBSUB CHANNELS|NUMSUB|NUMPAT`, subscriber mode in RESP2 and push frames in RESP3, `PUBLISH` from `redis.call`; `SPUBLISH`/`SSUBSCRIBE`/`PUBSUB SHARDNUMSUB` as the same mechanism on a standalone server | 15 applications: Celery (broker fanout is on by default and result backend), Bull, RQ, Asynq, Redisson, Spring Session indexed, Socket.IO, ActionCable, channels_redis, SignalR, Laravel broadcasting, Phoenix, GrowthBook | missing |
| 5 | **Blocking list operations**: `BLPOP`, `BRPOP`, `BRPOPLPUSH`, `BLMOVE` (fractional timeouts), woken by pushes from other connections, `MULTI`, and scripts. In 7.2 these run non-blocking inside a script or `MULTI` | Celery, Sidekiq, RQ, Bull, Dramatiq, Huey, python-redis-lock, Laravel queue (optional) | missing |
| 6 | **`BZPOPMIN`/`BZPOPMAX`** with fractional timeouts, woken by a `ZADD` from a script | BullMQ (idle loop), Huey, channels_redis | missing |
| 7 | **`WATCH`/`UNWATCH`** with abort on a changed, expired or evicted key | Celery broker (ack restore), RQ, Dramatiq, arq, LaunchDarkly, node-resque; redis-py `transaction()`, go-redis `Watch`, StackExchange.Redis conditions | missing |
| 8 | **`SSCAN`, `HSCAN`, `ZSCAN`** with `MATCH` and `COUNT`, also inside scripts | Sidekiq Web UI and API, BullMQ (inside Lua), Horizon, Laravel and Symfony tagged caches, Next.js handler | missing |
| 9 | **Scripts as clients use them**: `EVALSHA` after `SCRIPT LOAD` with the exact `NOSCRIPT` error text, `EVAL_RO`/`EVALSHA_RO` (Redisson, go-redis `RunRO`, rueidis), `cjson`, `cmsgpack`, `redis.pcall`, `redis.error_reply`, `redis.replicate_commands`, commands named from `ARGV`, undeclared keys, `unpack` of long argument lists | BullMQ, redis_rate, node-redlock, Redisson, Dramatiq, Laravel | mostly; `*_RO` and the library items to check |
| 10 | **`SELECT n`** for databases 0 through 15 (`CONFIG GET databases` is 16 in Redis) | Symfony sends it on every connect; URLs of Rails, Celery, Sidekiq, Resque, Spring Session take a database; Garnet's most-wanted feature | `SELECT 0` only; **decision needed** (see below) |
| 11 | **Minimal Streams**: `XADD` (with `MAXLEN ~`), `XREAD` with `BLOCK`, `XRANGE`, `XREVRANGE`, `XLEN`, `XTRIM`, `XDEL`, ids and trimming exactly as Redis | BullMQ events, RQ job results, Socket.IO streams adapter. No surveyed queue uses consumer groups | missing; **decision needed on release or after** |
| 12 | **`ZUNIONSTORE`, `ZINTERSTORE`** (and `ZUNION`, `ZINTER`, `ZDIFF`, `ZINTERCARD`, `ZRANGESTORE`, `ZMPOP`, `ZRANDMEMBER` as one family) | agoragames/leaderboard and Redisson's sorted set; the Redis suite tests that left the ratchet when these went missing come back | missing |
| 13 | **Redis 7.2.16 behavior** for the commands firn has: `ZRANK WITHSCORE`, `HELLO` failing without side effects, time frozen per command, `WAITAOF` (answering the error a server without replicas gives), `CLIENT NO-TOUCH` | the reference move | in progress |
| 14 | **`BITFIELD`/`BITFIELD_RO`** | Sidekiq 8 metrics at each heartbeat. Whether Sidekiq tolerates an error reply is not known; settle by running it | conditional on the Sidekiq run |
| 15 | **Operations basics**: `SLOWLOG GET|LEN|RESET`, `LATENCY` as an empty answer, `MEMORY USAGE` | redis_exporter reads them; managed services block them, so no application depends on them | later |

Items 4 to 8 take the coded applications from 34 to 58 of 63 and stop being the reason any one of the
widely used queues, pub/sub libraries or tag caches cannot use firn.

### Not doing

Each with its reason and what would reopen it.

| Not doing | Reason (evidence) | Reopen when |
|---|---|---|
| **Cluster mode** (`CLUSTER *`, slots, `MOVED`/`ASK`, `READONLY`) | None of the 63 applications requires a server in cluster mode; each works on one node, and cluster support in clients is optional (Sidekiq refuses it: `redis_connection.rb:21`). firn's scale-out plan is cores on one node. Dragonfly emulates 8 of 30 `CLUSTER` subcommands, Garnet partly; no peer is judged on the rest. Cost is high. | An application on the validation list requires it |
| **Replication, `WAIT`, `WAITAOF`, `FAILOVER`, Sentinel** | No application needs a replica; Redisson sends `WAIT` and `WAITAOF` probes only in non-single modes (an `ERR unknown command` clears them, `CommandAsyncService.java:1095-1200`). Every managed service blocks `REPLICAOF`/`PSYNC`. Clients configured for Sentinel need Sentinel servers, which firn is not. | A durability story that needs a second copy |
| **Modules** (RedisJSON, RediSearch, ...) | Only the Next.js stack handler and Upstash's own extensions need them; the strings handler needs `HSCAN`. | A flagship workload needs one |
| **`FUNCTION`/`FCALL`** | Not needed by any of the 63 applications' normal paths (Redisson's API and Valkey Glide's limiter class use it optionally). Dragonfly lists `FUNCTION` unsupported and Garnet lists all 11 commands unsupported. | An application on the list uses it |
| **`#!` script flags** | None of the shipped scripts surveyed starts with `#!` (re-checked). Keep refusing. | A selected application ships flagged scripts |
| **Keyspace notifications** | Only Spring Session's opt-in indexed repository needs them (the 4.x default does not). Garnet puts them on a long-term roadmap; Dragonfly has open issues. | Spring Session indexed becomes a validation target |
| **Streams consumer groups** (`XGROUP`, `XREADGROUP`, `XACK`, `XCLAIM`, `XAUTOCLAIM`, `XPENDING`, `XINFO`) | None of the surveyed queues uses them; BullMQ and RQ read streams as logs. | A selected application uses a group |
| **Client-side caching** (`CLIENT TRACKING`, `CLIENT CACHING`) | Only rueidis and valkey-go use it by default (set `DisableCache`), plus opt-in features of go-redis and redis-py. Garnet and ElastiCache Serverless lack it. | A target client needs it |
| **RDB snapshots** (`SAVE`, `BGSAVE`, `LASTSAVE`, RDB read and write) | No application calls them; firn persists with an append-only file. Managed services block `SAVE`/`BGSAVE`. This ignores migration from an existing Redis, which the survey did not study (judgment). | A migration path from Redis data is required |
| **GEO, HyperLogLog, bitmaps beyond `BITFIELD`, `SORT`, `LCS`, `DUMP`/`RESTORE`/`MIGRATE`, `OBJECT ENCODING`/`REFCOUNT`, `DEBUG`, `MONITOR`** | None of the 63 applications' normal paths (Redisson defines them in its command table, but its locks do not use them); Garnet stubs `OBJECT`; ElastiCache Serverless blocks `MONITOR`, `OBJECT`, `KEYS`. Low value per line of code. | A selected application sends one |
| **ACL users beyond `default`** | No application surveyed uses ACL commands; managed services restrict them. firn answers `AUTH [user] pass` for the default user. This is a judgment about drop-in use, not measured. | A deployment requires a second user |

### Validation applications and clients

Selection criteria, in order: coverage of the scope in the table above, popularity in its ecosystem, and
whether its own tests run unchanged against a server named by host and port (the criterion of the
[consumers investigation](../consumers/README.md#criteria-stated-before-choosing)).

Keep the existing three (Django cache and sessions, connect-redis, rate-limiter-flexible). Add these, in
this order:

1. **Sidekiq** (Ruby, redis-client 0.31). The most used Ruby job system on Redis among those read
   (13,557 stars against 9,467 for Resque; RubyGems 340.5M downloads in total against 55.1M; sidekiq-cron and
   sidekiq-scheduler add 98.1M and 47.4M). Command set is narrow: `BRPOP`, `EVALSHA` after `SCRIPT LOAD`, `MULTI`, sorted sets, `SSCAN` and
   `ZSCAN`, `BITFIELD`, plus the 7.0 version gate from `INFO`. Its tests take `REDIS_URL` and default to
   `redis://localhost:6379/0` and `FLUSHDB` between tests (`test/helper.rb:24-36`, re-checked), so they run
   unchanged against another server. Covers items 2, 3, 5, 8, 9 and 14, and adds Ruby and the
   RESP3 `redis-client` path, which none of the first three uses.
2. **Celery** with the Redis broker and result backend (Python, kombu 5.6 and redis-py). The most starred Python
   task system read (28,947 stars; PyPI celery 43.2M and kombu 44.3M a month). Covers `BRPOP`, pub/sub fanout
   (on by default), `WATCH` and `MULTI` in one application, and its integration tests take
   `TEST_BACKEND` (default `redis://`, `t/integration/conftest.py:22`, re-checked). Covers items 2, 4, 5, 7.
3. **BullMQ** (Node, ioredis). The most downloaded JavaScript queue of those measured (npm 9.72M a week against 1.66M for Bull; 9,497 stars). It is
   the stress case for scripts: 50 Lua scripts using `cjson` and `cmsgpack`, keys from `ARGV`,
   `BZPOPMIN`, Streams, `CLIENT LIST`, `INFO`. Covers items 6, 8, 9, 11. Its tests take `REDIS_HOST`
   (`tests/ioredis-client.test.ts:144`, re-checked); whether every test file does was not checked.
4. **A pub/sub application**: Socket.IO's redis adapter (npm 1.92M a week) and Rails ActionCable
   (RESP3 `SUBSCRIBE`), which between them use classic and RESP3 subscriber modes. Covers item 4.
5. **A lock library**: Redisson's default `RLock` (24,413 stars), which needs `PUBLISH` inside a script and
   `SUBSCRIBE` in one operation, and redis-py `Lock` / redsync as the plain-script locks. Covers items 4, 9.
   Their tests were not checked for host and port configuration.
6. **RQ** as a second queue only if Celery and BullMQ leave a gap: it adds `BLMOVE`, Streams for results,
   `CLIENT LIST` and `WATCH` (10,692 stars). Not needed for coverage beyond items 5, 7, 11.

Resque, Asynq, Dramatiq, Huey, Laravel queues and Horizon would add little coverage that the first three
do not. Rails' cache store, Spring Data cache and Laravel cache remain the reserve named in
[design/firn/consumers.md](../../../design/firn/consumers.md); this survey finds nothing that changes that
(they use only commands firn has), except tagged caches, which need `SSCAN`/`ZSCAN`.

**Clients**, validated by connecting and running a fixed smoke set (handshake, `GET`/`SET`, pipeline,
`MULTI`/`EXEC`, a script through the client's own script helper, pub/sub, one blocking pop) rather than by
their full test suites: redis-py, node-redis, ioredis, go-redis, Jedis, Lettuce, redis-client/redis-rb,
StackExchange.Redis, Predis, phpredis, redis-rs, rueidis. The handshake table identifies the specific risks:
redis-py 8 and node-redis 6 fail on a `HELLO` error; ioredis fails on an `INFO` error or `loading:1`;
StackExchange.Redis sends `CLUSTER SLOTS`, `CLUSTER NODES`, `SENTINEL MASTERS` and `CONFIG GET` and needs the
errors to be ignorable and `ECHO` exact; rueidis needs `DisableCache`; Redisson needs pub/sub and its startup
`EVAL`. Some of these clients are covered indirectly by the application runs (redis-py by Django and Celery,
node-redis by connect-redis, ioredis by BullMQ, redis-client by Sidekiq).

### Cancelled compatibility gaps

The status board shows the owner's cancellation on 2026-10-12 of the items under the parity root and of that
root, with the reason "usage investigation to decide; reopen if the survey finds a need". No record of them
exists in `research/`; the board is the record (read 2026-10-12). Disposition, by what the data says:

| Cancelled item | What it was | Evidence in this survey | Recommendation |
|---|---|---|---|
| Table of current differences (`firn-bl-01-12`) | Unknown commands, partial commands, config, numeric corner cases, 13 Redis-suite tests lost | Breaks apart; see below | Split: reopen the parts named below, leave the rest cancelled |
| - blocking list commands (in `firn-bl-01-12`) | `BLPOP` and the rest answered as unknown | 8 applications; Sidekiq, Celery, Huey need them in the idle loop | **Reopen** |
| - `WATCH`/`UNWATCH` (same) | unknown | 6 applications | **Reopen** |
| - pub/sub (same) | unknown | 15 applications | **Reopen** |
| - `SSCAN`, `HSCAN`, `ZSCAN` (same) | unknown | 6 applications | **Reopen** |
| - `BZPOPMIN`, `BZPOPMAX` (same) | unknown | BullMQ, Huey, channels_redis | **Reopen** |
| - `ZUNIONSTORE`, `ZINTERSTORE` and the other sorted-set algebra, `ZMPOP`, `ZRANDMEMBER` (same) | unknown; also the reason 13 Redis-suite tests left the ratchet | 1 coded application plus Redisson | **Reopen** as a family (low frequency, cheap, and returns the 13 suite tests) |
| - `LMPOP`, `SORT`, `LCS`, `DUMP`/`RESTORE`, `MOVE`, `MIGRATE`, `WAIT` (same) | unknown | 0 applications (Redisson probes `WAIT` only in non-single modes) | Keep cancelled; make `WAIT 0 0` answer 0 if it comes up |
| - `CLIENT` subcommands, `COMMAND` subcommands (same) | partial | `CLIENT SETINFO` sent by every mainstream client; `CLIENT LIST` by BullMQ, RQ, Bull, redis_exporter; `CLIENT INFO` `addr` missing for want of Whitefoot socket addresses (a separate request) | **Reopen** `SETINFO` and `LIST`; keep the rest cancelled |
| - `SELECT` only 0 (same) | partial | indirect: Symfony sends it on connect; common in URLs; Garnet's top issue | **Decision card** (below) |
| - `FUNCTION`, `DEBUG`, only `default` user, `--bind`, `CONFIG SET appendonly|port|bind` | partial | 0 applications | Keep cancelled |
| - negative-zero score in a skip list, expiry overflow | numeric corner cases | 0 applications | Keep cancelled |
| Scripts with `#!` flags (`firn-bl-01-32`) | refused | no shipped script uses them (re-checked) | Keep cancelled |
| `OBJECT ENCODING` and `REFCOUNT` (`firn-bl-01-31`) | unknown subcommands | 0 applications | Keep cancelled |
| `INFO` omits unmeasured metrics (`firn-bl-01-22`) | no stats counters or command statistics | redis_exporter exports five counters firn lacks; Sidekiq's Web UI reads memory fields | **Reopen the five counters and `commandstats`** (monitoring); leave CPU and error statistics cancelled |
| `INFO` inside `EXEC` or a script (`firn-bl-01-27`) | refused | no application | Keep cancelled |
| `BGREWRITEAOF` with `appendonly no` (`firn-bl-01-29`) | cannot rebuild | no application | Keep cancelled |
| `SPOP` with a count writes one `SREM` (`firn-bl-01-08`) | AOF differs from Redis | 0 applications use `SPOP` | Keep cancelled |
| noscript refusal checks only arity (`firn-bl-01-11`) | different error text in one case | no application | Keep cancelled |
| Clock read once per read (`firn-bl-01-14`) | commands in one pipelined read share a time | Redis 7.2 itself freezes time per command; no application is sensitive to a pipeline's sub-millisecond clock (judgment) | Keep cancelled |
| No case for expiry at the last millisecond (`firn-bl-01-15`) | missing test | not a usage question | Keep cancelled |
| Idle timeout does not close a client blocked on sending (`firn-bl-01-21`) | stuck connection | no application sets `timeout`; a production server would also want output-buffer limits (judgment, not measured) | Keep cancelled; revisit with the operations scope |
| Table of Redis command table versus suite (`firn-redis-parity-list`) and the parity root | check of completeness | superseded by this survey | Keep cancelled |

### Decisions the data does not settle

1. **Multiple databases.** The evidence is indirect (clients send `SELECT`, URLs carry a database,
   Garnet's most-reacted request) and no deployment measurement. Recommendation: support databases 0 through
   15 if the keyspace design allows it without costing the single-keyspace performance path; otherwise
   reject `SELECT n` with Redis's error and say so in the README. Confidence 2 of 5; a survey of real
   `redis://host/N` URLs would settle it.
2. **Streams at release or after.** BullMQ (the most downloaded JavaScript queue measured) cannot start its event
   stream without them. Recommendation: the minimal subset in the release scope, consumer groups after. The
   cost is a new data type, so the owner may prefer to ship without BullMQ first; the data says only that
   BullMQ, RQ's results and one Socket.IO adapter need it.
3. **`BITFIELD`.** Resolve by running Sidekiq: if its heartbeat tolerates an error reply on `BITFIELD`, drop it.
4. **RDB and migration.** Unstudied here. A drop-in claim for users who have data in Redis raises a question
   this survey did not answer: how existing data gets into firn.

## Unverified and limits

- The command lists of the applications are subagent reads of named tags; I re-checked a sample (Sidekiq's
  and Celery's test setup, the shebang grep, redis_exporter), not every line.
- Counts are by application, command name and normal path or shipped Lua; options, reply shape and error
  text are not in them. Optional and admin paths are excluded from the coded sets unless stated (for example
  Laravel's `BLPOP` and `KEYS`).
- Not read: Laravel's and Predis's wire behavior beyond their connectors, Spring Data's `DefaultScriptExecutor`
  (assumed `EVALSHA`, then `EVAL`), Celery's remote-control extras, Bull and Resque cluster support, Sidekiq
  Pro and Enterprise scripts, APScheduler 4.x, kombu's cluster mode, Python and other leaderboard libraries
  besides the Ruby gem, and which StackExchange.Redis handshake steps are fatal on error.
- Not verified: whether Sidekiq tolerates an error from `BITFIELD`; whether Halo supports
  `redis.replicate_commands` and large `unpack`; Predis's release that re-enabled `CLIENT SETINFO` by
  default; whether every BullMQ and Redisson test file takes a host and port.
- No published data on Redis command frequency exists in what I could reach; the Twitter cache study and the
  academic microservice-database study (arXiv 2510.20582) give adoption or memcached facts only.
- Popularity: stars and downloads are proxies, not market share, and downloads are inflated by CI and
  dependency chains (`limits` is a dependency of Flask-Limiter and slowapi).
- The rankings weight each application equally.

## Appendix: coded command sets

My coding of each application's normal-path commands and shipped Lua, from the sources above. Counts in
the ranking are computed from these sets. Q is a queue or scheduler, C a cache or session store, L a limiter or
lock, R a realtime or pub/sub library, F a flag store or other. `*` marks an admin or UI path included.

```
Q Celery broker (kombu)       LPUSH BRPOP MULTI EXEC ZADD HSET ZREM HDEL SET ZREVRANGEBYSCORE WATCH HGET PUBLISH PSUBSCRIBE UNSUBSCRIBE SADD SREM EXISTS DEL LLEN SMEMBERS RPOP RPUSH PING EVALSHA GET
Q Celery result backend       MULTI EXEC SETEX SET PUBLISH GET MGET DEL INCR EXPIRE ZADD ZCOUNT ZRANGE RPUSH LLEN LRANGE SUBSCRIBE UNSUBSCRIBE
Q Sidekiq                     BRPOP SADD LPUSH ZADD ZREMRANGEBYSCORE ZREMRANGEBYRANK MULTI EXEC SCARD EVALSHA SCRIPT ZRANGE ZREM SET SSCAN* SREM HGET INCRBY EXPIRE UNLINK HSET PING EXISTS RPOP RPUSH HSETNX HGETALL HINCRBY BITFIELD BITFIELD_RO HMGET INFO ZSCAN* TYPE LRANGE LINDEX LLEN LREM SISMEMBER MSET MGET SMEMBERS ZCARD ZPOPMIN ZINCRBY GET
Q BullMQ                      EVALSHA BZPOPMIN XREAD XADD XTRIM SCAN CLIENT INFO HGETALL HGET HMGET HSET HDEL HEXISTS GET SET DEL LTRIM LPOS ZSCORE ZRANGE ZREVRANGE ZCARD LLEN LRANGE SMEMBERS MULTI EXEC EXISTS HINCRBY HLEN HMSET INCR LINDEX LPOP LPUSH LREM LSET PEXPIRE PERSIST PTTL RENAME RPOP RPOPLPUSH RPUSH SADD SCARD SISMEMBER SREM TYPE ZADD ZCOUNT ZPOPMIN ZRANGEBYSCORE ZREM ZREMRANGEBYRANK ZREMRANGEBYSCORE ZREVRANGEBYSCORE SSCAN HSCAN
Q RQ                          BLPOP BLMOVE SUBSCRIBE PUBLISH WATCH MULTI EXEC EVALSHA SCRIPT CLIENT INFO XADD XREAD XREVRANGE XLEN HSET HGETALL HGET HINCRBY EXPIRE PERSIST DEL EXISTS ZADD ZREM ZRANGE ZRANGEBYSCORE ZREMRANGEBYSCORE ZSCORE ZCARD ZREMRANGEBYRANK RPUSH LPUSH LRANGE LLEN LREM RENAME LPOP SADD SREM SMEMBERS SET GET LMOVE
Q Resque                      RPUSH SADD LPOP LLEN LRANGE LINDEX LREM SMEMBERS SREM UNLINK LSET SET GET MGET SISMEMBER HSET HGET HGETALL HDEL INCRBY DECRBY TIME KEYS*
Q Bull                        BRPOPLPUSH PSUBSCRIBE SUBSCRIBE PUBLISH CLIENT EVALSHA MULTI EXEC KEYS INFO DEL EXISTS GET HDEL HGET HGETALL HINCRBY HMGET HMSET HSET INCR LINDEX LINSERT LLEN LPUSH LRANGE LREM LSET LTRIM PSETEX PTTL RENAME RPOPLPUSH RPUSH SADD SCAN SCARD SET SISMEMBER SMEMBERS SREM TYPE ZADD ZCARD ZCOUNT ZRANGE ZRANGEBYSCORE ZREM ZREMRANGEBYRANK ZREMRANGEBYSCORE ZREVRANGE ZREVRANGEBYSCORE ZSCORE HEXISTS TTL
Q Asynq                       EVALSHA SADD SMEMBERS ZSCORE ZADD ZREM DEL HSET PING PUBLISH SUBSCRIBE EXISTS LPUSH HDEL HGET RPOPLPUSH EXPIREAT GET INCR LREM SET ZRANGE RPUSH ZRANGEBYSCORE ZCARD ZREMRANGEBYRANK ZREVRANGE EXPIRE SETEX ZREMRANGEBYSCORE
Q Dramatiq                    EVALSHA ZADD ZRANGEBYSCORE SMEMBERS HEXISTS RPUSH DEL KEYS ZREM HDEL HSET LPOP SADD HMGET SREM HGET HLEN SCARD BRPOPLPUSH RPOPLPUSH LINDEX MULTI EXEC LPUSH PEXPIRE SET WATCH GET
Q Laravel queue               EVAL RPUSH LLEN ZCARD ZADD LPOP ZREM ZRANGEBYSCORE ZREMRANGEBYRANK DEL LRANGE ZRANGE LINDEX KEYS BLPOP MULTI EXEC
Q Laravel Horizon             EVAL HSETNX SADD HMGET HMSET ZSCAN ZREM DEL GET ZREMRANGEBYSCORE ZADD ZRANGE HGET EXPIREAT EXPIRE ZCARD SMEMBERS SET ZREVRANGEBYSCORE ZREMRANGEBYRANK MULTI EXEC LLEN HDEL ZREVRANGE ZCOUNT SREM SETNX RPUSH LTRIM LRANGE INCR HSET HKEYS HGETALL EXISTS SCAN
Q Huey                        LPUSH BRPOP RPOP ZADD BZPOPMIN ZPOPMIN EVALSHA ZRANGEBYSCORE ZREMRANGEBYSCORE ZCARD ZRANGE DEL HSET HGET HMGET HDEL HEXISTS HLEN HGETALL HINCRBY HSETNX EXPIRE BLPOP SET GET MGET EXISTS SCAN INFO
Q redbeat                     HGET HSET HSETNX ZADD ZRANGEBYSCORE ZREM DEL ZRANK SMEMBERS SADD SREM SET GET EVALSHA PTTL PEXPIRE MULTI EXEC
Q APScheduler job store       HGET ZRANGEBYSCORE HMGET ZRANGE HGETALL HEXISTS MULTI EXEC HSET ZADD ZREM HDEL DEL
Q node-resque                 LPOP EVALSHA SET SADD SMEMBERS DEL LLEN LRANGE LREM GET MGET EXPIRE MULTI EXEC ZRANGEBYSCORE SISMEMBER SREM SELECT QUIT SCAN WATCH UNWATCH
Q arq                         WATCH EXISTS MULTI EXEC PSETEX ZADD ZRANGEBYSCORE ZCARD ZSCORE GET INCR EXPIRE SET ZREM DEL KEYS INFO DBSIZE
Q sidekiq-cron                SADD SREM SMEMBERS HSET HGETALL HGET ZADD ZREMRANGEBYSCORE LPUSH LTRIM LRANGE DEL EXISTS SCARD KEYS
C Rails RedisCacheStore       GET MGET SET UNLINK INCRBY EXPIRE TTL FLUSHDB SCAN INFO
C redis-session-store         GET EXISTS SETEX SET DEL
C redis-store/redis-actionpack SETNX EXPIRE MULTI EXEC GET SET SETEX DEL
C Spring Data Redis cache     GET GETEX SET SETNX DEL KEYS UNLINK SCAN EXISTS
C Spring Session simple       HGETALL HSET EXPIREAT EXISTS DEL RENAME
C Spring Session indexed      HSET HGETALL EXPIRE APPEND PERSIST DEL SADD SREM SMEMBERS PUBLISH SUBSCRIBE PSUBSCRIBE CONFIG EXISTS
C Laravel cache + sessions    GET MGET SETEX SET INCRBY DECRBY EXPIRE DEL FLUSHDB MULTI EXEC EVAL ZADD ZREMRANGEBYSCORE ZSCAN SCAN ZREM EXISTS PERSIST
C Symfony Cache plain         GET MGET EXISTS SET SETEX UNLINK DEL FLUSHDB INFO SCAN EVAL SELECT
C Symfony Cache tag-aware     SETEX SADD SREM EVAL UNLINK GET DEL SSCAN SUNIONSTORE INFO
C Symfony session handler     GET SETEX UNLINK DEL EXPIRE
C Flask-Caching/cachelib      GET MGET SET SETNX EXPIRE DEL EXISTS INCR KEYS FLUSHDB UNLINK PING
C Flask-Session               GET DEL SET
C django-redis                SET GET MGET DEL FLUSHDB EXISTS TTL PTTL PERSIST EXPIRE PEXPIRE PEXPIREAT EXPIREAT EVAL INCRBY KEYS SCAN MULTI EXEC
C Django RedisCache + sessions SET GET DEL EXPIRE PERSIST MGET EXISTS INCRBY MSET MULTI EXEC FLUSHDB
C connect-redis               GET SET EXPIRE DEL SCAN MGET
C keyv (@keyv/redis)          SET GET MGET EXISTS UNLINK DEL MULTI EXEC SCAN INFO FLUSHDB
C unstorage                   EXISTS GET MGET SET UNLINK SCAN
C @neshca/cache-handler       GET HMGET SET HSET HSCAN UNLINK HDEL EXPIREAT
C ASP.NET Core cache          HSET HMGET HGET EXPIRE DEL
L Symfony Lock                EVALSHA SCRIPT TYPE ZREMRANGEBYSCORE ZSCORE ZCOUNT ZADD ZREVRANGE PEXPIREAT ZREM
L rate-limiter-flexible       EVALSHA EVAL SET INCRBY PTTL EXPIRE MULTI EXEC GET DEL
L limits                      EVALSHA SCRIPT INCRBY EXPIRE GET TTL DEL LINDEX LPUSH LTRIM PTTL RENAME SET EXISTS KEYS PING
L redis_rate                  EVALSHA TIME GET SET DEL
L rate-limit-redis            SCRIPT EVALSHA PTTL SET INCR GET DECR DEL
L Envoy ratelimit             INCRBY EXPIRE GET PING EVAL SET AUTH
L Laravel limiters            EVAL EXISTS HMSET EXPIRE HGET HINCRBY MGET SET GET DEL SETEX INCRBY
L node-redlock                EVALSHA EVAL EXISTS SET GET DEL
L redsync                     SET GET EVALSHA EVAL DEL PEXPIRE
L redis-py Lock               SET GET EVALSHA SCRIPT DEL PTTL PEXPIRE
L python-redis-lock           SET BLPOP EVALSHA GET DEL LPUSH PEXPIRE TTL EXPIRE EXISTS KEYS
L Redisson default RLock      EVALSHA SCRIPT PUBLISH SUBSCRIBE PSUBSCRIBE PUBSUB HEXISTS HINCRBY HGET HSET HDEL PEXPIRE PTTL GET SET DEL EXISTS ZADD ZREM ZSCORE LRANGE LPOP RPUSH LREM INFO PING CLIENT
L Laravel RedisLock           SET SETNX EVAL GET DEL EXPIRE PERSIST
L Spring Integration lock     EVAL GET PEXPIRE SET
R Socket.IO redis adapter     PUBLISH SUBSCRIBE PSUBSCRIBE PUBSUB UNSUBSCRIBE PUNSUBSCRIBE
R Socket.IO streams adapter   XADD XREAD XRANGE SET MULTI EXEC GET DEL PUBSUB
R Rails ActionCable           PUBLISH SUBSCRIBE UNSUBSCRIBE
R Django channels_redis       BZPOPMIN ZADD ZPOPMIN ZREMRANGEBYSCORE ZCOUNT ZRANGE EXPIRE ZREM EVAL KEYS DEL PUBLISH SUBSCRIBE MULTI EXEC
R ASP.NET SignalR backplane   PUBLISH SUBSCRIBE
R Laravel broadcaster/Reverb  PUBLISH EVAL SUBSCRIBE PSUBSCRIBE
R Phoenix.PubSub              PUBLISH SUBSCRIBE
F Flipper                     SADD SREM SMEMBERS HSET HDEL HGETALL DEL GET MGET SETEX
F Unleash Edge                GET SET
F LaunchDarkly relay store    WATCH UNWATCH MULTI EXEC HSET HGET HGETALL DEL SET EXISTS SMEMBERS GET PING EVALSHA SADD SREM
F GrowthBook proxy            GET SET MGET PUBLISH SUBSCRIBE
F Split synchronizer          GET SET DEL MGET SMEMBERS SADD SREM SISMEMBER SCARD LRANGE LTRIM LLEN RPUSH EXPIRE INCR DECR HINCRBY HSET HGETALL SCAN KEYS PING EVAL
F agoragames/leaderboard      ZADD ZINCRBY ZSCORE ZRANK ZREVRANK ZRANGE ZREVRANGE ZRANGEBYSCORE ZREVRANGEBYSCORE ZCOUNT ZCARD ZREM ZREMRANGEBYRANK ZREMRANGEBYSCORE ZUNIONSTORE ZINTERSTORE HSET HGET HMGET HDEL DEL EXPIRE MULTI EXEC
```

Command-name support in firn used for the counts: the names in `firn/README.md` at `97b054b`, with `CLIENT`,
`CONFIG`, `SELECT` and `SCRIPT` counted as partial and `CLIENT`/`CONFIG` not counted as missing.
