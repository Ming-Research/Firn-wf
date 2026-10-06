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

Pending the first runs.
