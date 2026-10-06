# The deployment milestone's consumers

## The question

firn's deployment milestone ([direction](../firn/DESIGN.md#deployment-direction))
is cache and session storage and scripted conditional updates, used by
existing components through their ordinary clients with their logic
unchanged. Which components are those consumers, and what Redis behavior do
they use? The answer selects what firn builds next: a command, an option or a
protocol feature enters the milestone because a selected consumer sends it,
not because Redis has it.

## Criteria, stated before choosing

A candidate is selected when it meets all of these:

1. **Scenario.** Its main use of Redis is one of the milestone's scenarios:
   cache storage, session storage, or a conditional update a script or a
   transaction makes atomic.
2. **Ordinary client.** It talks to Redis through a mainstream client
   library, unmodified.
3. **Reproducible workload.** Its own tests exercise its Redis use against a
   server named by host and port alone, so the same workload runs unchanged
   against Redis 7.0.15 and against firn.
4. **Adoption.** It is widely used, so its traffic stands for real
   deployments; GitHub stars are the recorded proxy.

and the selected set as a whole:

5. **Diversity.** It covers both scenarios and at least two client
   ecosystems, so one client's habits do not decide the milestone.

A candidate is rejected when its workload needs a Redis module, Cluster or
Sentinel, or cannot be pointed at a server by configuration.

## Candidates

Stars as GitHub reported them on 2026-10-06.

| Candidate | Scenario | Client | Workload | Stars | Verdict |
|---|---|---|---|---|---|
| Django's `RedisCache` and cache sessions | cache, session | redis-py | its `cache` and `sessions_tests` suites, the server named in `CACHES` | 91,345 | selected |
| connect-redis, the Redis store of express-session | session | node-redis | its tests, which start `redis-server` from `PATH` on a fixed port | 2,822 (express-session 6,352) | selected |
| rate-limiter-flexible's `RateLimiterRedis` | conditional update by `EVAL` and `MULTI` | ioredis, node-redis | its Redis tests, against 127.0.0.1:6379 | 3,588 | selected |
| Rails' `RedisCacheStore` | cache | redis-rb | Active Support's cache-store tests, `REDIS_URL` | 58,807 (Rails) | reserve: a third ecosystem when the first three are served |
| Laravel's Redis cache and sessions | cache, session | phpredis, Predis | the framework's integration tests | 34,948 | reserve |
| BullMQ, Sidekiq | queue | ioredis, redis-rb | their tests | 9,475; 13,562 | not now: a queue is a candidate scenario, not a selected one |
| limits (Flask-Limiter, slowapi) | conditional update by `EVALSHA` | redis-py | its tests, several backends at once | 648 (Flask-Limiter 1,207) | not selected: rate-limiter-flexible covers the scenario with wider use |
| django-redis | cache | redis-py | its tests | 3,086 | not selected: Django's own backend covers the scenario |

The three selected components span two ecosystems (Python and Node.js) and
three clients, and cover the cache, the session and the scripted update.
A component's own tests are not production traffic: they also exercise edge
cases a deployment seldom reaches, so the command profile they give bounds
what the component can send rather than measuring how often it sends it.

## Method

`research/experiments/consumers/` runs each consumer's Redis tests, pinned to
a release, in CI against two servers:

- **Redis 7.0.15**, with `MONITOR` recording every command the tests send,
  including those a script runs; `profile.py` reduces the record to each
  command and the option words it was sent with, and their counts.
- **firn**, built at the same revision, whose test results name the
  behavior the consumer needs and firn lacks.

The consumers' pins: Django 6.1.1, connect-redis v10.0.0,
rate-limiter-flexible v11.2.1.

## Results

CI runs [37456306284](https://github.com/Ming-Research/Firn-wf/actions/runs/37456306284)
and [37456778784](https://github.com/Ming-Research/Firn-wf/actions/runs/37456778784),
which agree in every count below, on GitHub's `ubuntu-24.04` runners, Redis 7.0.15 from Ubuntu's package, firn at
`6239de8c8` with Whitefoot `wf-364f86c2fd16`. The clients the consumers
installed: redis-py 8.1.0 (Django), node-redis 6.3.0 (connect-redis), and
ioredis 5.11.1 with node-redis 4.7.1 (rate-limiter-flexible). Every
consumer's tests pass against Redis.

| Consumer | Against Redis | Against firn | What firn refused |
|---|---|---|---|
| Django | 1,338 tests, all pass (145 skipped) | 230 errors | `HELLO 3`: all 230 are `NOPROTO` |
| connect-redis | 4 of 4 pass | 2 of 4 fail | `HELLO 3`: `NOPROTO` |
| rate-limiter-flexible | 100 of 100 pass | 69 of 100 fail | `EVAL`, `EVALSHA`, `EXEC`; 12 time out behind them |

The commands each consumer sent Redis, by form (`profile.py`; `lua` marks a
command a script ran), with how often:

- **Django** (982 commands): `GET` 238, `SET EX` 160, `EXISTS` 144, `DEL`
  135, `FLUSHDB` 72, `SET NX EX` 50, `EXPIRE` 49, `MGET` 31, `INCRBY` 22,
  `MULTI`/`EXEC` 20 (each around `MSET` and one `EXPIRE` per key), `HELLO 3`
  11, `SET NX` 5, `SET` 2, `CLIENT INFO` 1, `PERSIST` 1.
- **connect-redis** (1,048): `SET EX` 1,003, `SCAN MATCH COUNT` 31, `DEL` 4,
  `TTL` 3, `HELLO 3` 2, `EXPIRE`, `GET` and `MGET` 1 each.
- **rate-limiter-flexible** (730): from scripts `SET EX NX`, `INCRBY` and
  `PTTL` 86 each and `PEXPIRE` 8; from the client `EVAL` 73, `QUIT` 55,
  `INFO` 54, `FLUSHDB` 53, `FLUSHALL` 47, `PTTL` 42, `GET` 37,
  `MULTI`/`EXEC` 29 (around `SET` or `GET` or `INCRBY`, then `PTTL`), `SET EX`
  17, `EVALSHA` 13, `SET` 6, `DEL` 4, `INCRBY` 4.

## What the milestone needs from firn

Ordered by how many selected consumers each gap stops:

1. **RESP3.** redis-py 8 and node-redis 6 open every connection with
   `HELLO 3` and do not fall back; firn answers `NOPROTO`, so two of the
   three consumers stop at their first command. Redis 7.0.15's RESP3 replies
   (null, map, set, double and the rest) follow, for every command the
   consumers send.
2. **Transactions: `MULTI` and `EXEC`.** Django's `set_many` and
   rate-limiter-flexible's non-scripted paths queue two to five commands
   (`MSET`, `EXPIRE`, `SET`, `GET`, `INCRBY`, `PTTL`) and run them with
   `EXEC`. No consumer sent `WATCH` or `DISCARD`.
3. **Scripts: `EVAL` and `EVALSHA`.** rate-limiter-flexible's scripts call
   `SET` with `EX` and `NX`, `INCRBY`, `PTTL` and `PEXPIRE`; scripts run
   only `GET`, `INCR` and `SET` today.
4. **`SCAN` with `MATCH` and `COUNT`**, which connect-redis uses to list and
   clear sessions, and **`CLIENT INFO`**, which Django sent once.

Every other form the consumers sent is one firn already answers. Transactions
and scripts both run several commands as one atomic step, so both need the
commands they queue or call written as the parts of the
[command-parts decision](../../../design/firn/command-parts.md): the commands
above first, the others as consumers need them.
