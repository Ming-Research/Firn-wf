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
   against the reference Redis and against firn.
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
A component's own tests are not production traffic: they exercise edge cases
a deployment seldom reaches and may leave other paths unexercised, so the
command profile they give shows what the tests reached, not how often a
deployment sends each command; the commands a recorded script can call are
read from its source as well.

## Method

`research/experiments/consumers/` runs each consumer's Redis tests, pinned to
a release, in CI against two servers:

- **Redis 7.2.16**, the reference of AGENTS.md, built from its release
  archive by `tests/redis-suite/reference.sh` and run with an append-only
  file (`--appendonly yes`), as firn is, with `MONITOR` started before the
  tests and recording
  every command they send, including those a script runs; `profile.py`
  reduces the record to each command and the option words it was sent with,
  and their counts, and names the commands each recorded script's source can
  call.
- **firn**, built at the same revision and run with an append-only file, whose
  test results name the behavior the consumer needs and firn lacks.

The first runs, below, used Redis 7.0.15 from Ubuntu's package without an
append-only file; the rerun after them uses the setup above.

The consumers' pins: Django 6.1.1, connect-redis v10.0.0,
rate-limiter-flexible v11.2.1.

## Results

CI runs [37456306284](https://github.com/Ming-Research/Firn-wf/actions/runs/37456306284),
[37456778784](https://github.com/Ming-Research/Firn-wf/actions/runs/37456778784)
and [37458118086](https://github.com/Ming-Research/Firn-wf/actions/runs/37458118086),
which agree in every count below, the last with `MONITOR` started before the
tests; its records and the earlier ones give the same profiles when the
current `profile.py` reduces both, on GitHub's `ubuntu-24.04` runners, Redis 7.0.15 from Ubuntu's package, firn at
`6239de8c8` with Whitefoot `wf-364f86c2fd16`. The clients the consumers
installed: redis-py 8.1.0 (Django), node-redis 6.3.0 (connect-redis), and
ioredis 5.11.1 with node-redis 4.7.1 (rate-limiter-flexible). Every
consumer's tests pass against Redis.

| Consumer | Against Redis | Against firn | What firn refused |
|---|---|---|---|
| Django | 1,338 tests: OK, 145 of them skipped and 1 an expected failure | 230 errors | `HELLO 3`: all 230 are `NOPROTO` |
| connect-redis | 4 of 4 pass | 2 of 4 fail | `HELLO 3`: `NOPROTO` |
| rate-limiter-flexible | 100 of 100 pass | 69 of 100 fail | `EVAL`, `EVALSHA`, `EXEC`; 12 time out behind them |

The commands each consumer sent Redis, by form (`profile.py`; `lua` marks a
command a script ran), with how often:

- **Django** (981 commands): `GET` 238, `SET EX` 160, `EXISTS` 144, `DEL`
  135, `FLUSHDB` 72, `SET NX EX` 50, `EXPIRE` 49, `MGET` 31, `INCRBY` 22,
  `MULTI`/`EXEC` 20, `HELLO 3` 11, `SET NX` 5, `SET` 2, `CLIENT INFO` 1,
  `PERSIST` 1. Each transaction queues one `MSET`, followed in 19 of the 20
  by one `EXPIRE` per key.
- **connect-redis** (1,046): `SET EX` 1,003, `SCAN MATCH COUNT` 31, `DEL` 4,
  `TTL` 3, `HELLO 3` 2, `EXPIRE`, `GET` and `MGET` 1 each.
- **rate-limiter-flexible** (729): from scripts `SET EX NX`, `INCRBY` and
  `PTTL` 86 each and `PEXPIRE` 8; from the client `EVAL` 73, `QUIT` 55,
  `INFO` 54, `FLUSHDB` 53, `FLUSHALL` 47, `PTTL` 42, `GET` 37,
  `MULTI`/`EXEC` 29, `SET EX` 17, `EVALSHA` 13, `SET` 6, `DEL` 4, `INCRBY` 4.
  Its transactions queue `SET` alone (5), or `SET`, `GET` or `INCRBY`
  followed by `PTTL` (24). The sources of its four recorded scripts can also
  call `EXPIRE`, in a branch for a key without an expiry that the tests did
  not reach.

### Rerun on Redis 7.2.16 with the append-only file (2026-10-12)

CI run [38240404716](https://github.com/Ming-Research/Firn-wf/actions/runs/38240404716)
at commit bb0bd6d ran the consumers against Redis 7.2.16 and against firn,
both with an append-only file, on the pins above. Both servers gave the same
results:

| Consumer | Against Redis 7.2.16 | Against firn |
|---|---|---|
| Django 6.1.1 cache tests | 1,338 run: OK, 145 skipped, 1 expected failure | 1,338 run: OK, 145 skipped, 1 expected failure |
| connect-redis 10.0.0 | 4 of 4 pass | 4 of 4 pass |
| rate-limiter-flexible 11.2.1 | 100 of 100 pass | 100 of 100 pass |

## What the milestone needs from firn

Ordered by how many selected consumers each gap stops:

1. **RESP3.** redis-py 8 and node-redis 6 open every connection with
   `HELLO 3` and do not fall back; firn answers `NOPROTO`, so two of the
   three consumers stop at their first command. Redis 7.0.15's RESP3 replies
   (null, map, set, double and the rest) follow, for every command the
   consumers send.
2. **Transactions: `MULTI` and `EXEC`.** Django's `set_many` and
   rate-limiter-flexible's non-scripted paths queue one to five commands
   (`MSET`, `EXPIRE`, `SET`, `GET`, `INCRBY`, `PTTL`) and run them with
   `EXEC`. No consumer sent `WATCH` or `DISCARD`.
3. **Scripts: `EVAL` and `EVALSHA`.** rate-limiter-flexible's scripts ran
   `SET` with `EX` and `NX`, `INCRBY`, `PTTL` and `PEXPIRE`, and their source
   can also call `EXPIRE`; scripts run only `GET`, `INCR` and `SET` today.
4. **`SCAN` with `MATCH` and `COUNT`**, which connect-redis uses to list and
   clear sessions, and **`CLIENT INFO`**, which Django sent once.

Every other form the consumers sent is one firn already answers. Transactions
and scripts both run several commands as one atomic step, so both need the
commands they queue or call written as the parts of the
[command-parts decision](../../../design/firn/command-parts.md): the commands
above first, the others as consumers need them.

## Per-case records and the comparison

The three consumers above keep an exit status and the end of the log. The
validation applications that follow them in
[design/firn/consumers.md](../../../design/firn/consumers.md) have hundreds of
cases each, and their question is whether firn passes the cases Redis 7.2.16
passes, so their runs keep one record per case and compare the two servers'
records. Four parts serve every application run this way and live in
`research/experiments/consumers/`; an application's wiring adds only a
reporter for its test runner (Sidekiq's is `minitest_record.rb`).

1. **The record**, `cases.tsv`: one row per case, two columns separated by a
   TAB, `CASE<TAB>STATUS`. CASE is `FILE::NAME`, led by the test file and
   holding no TAB or newline; STATUS is `pass`, `fail`, `error`, `skip` or
   `unfinished`. The reporter appends each row as the case ends, so a process
   that is killed leaves the cases it finished. A file whose process was killed
   at its time limit, or ended having recorded no case, gets the row
   `FILE<TAB>unfinished`. A case without a row did not pass, and a case with
   several rows passed when all of them did.
2. **The comparison**, `compare.py`, which knows no application: it reads the
   records of one or more Redis runs and of one firn run, and the unstable
   list, and writes Markdown for the job summary with the number and the names
   of the cases both servers passed, only Redis passed and only firn passed,
   and of the unstable ones it left out. Redis's stable passes are the cases
   that passed in every Redis run and are not on the list. It reports a
   difference between the lines the two cells' `what.txt` hold after the
   first (the tool and gem versions the run installed), since an application
   without a lock file installs the newest gems on each cell. `--self-test`
   checks it on records whose answer is known and on input it must refuse.
3. **The loop**, `run_each_file LIMIT COMMAND...` in `run.sh`: one process per
   test file, each on a server started anew on an empty directory (with
   `MONITOR` appending to the same `monitor.log` on Redis), under
   `timeout -k 10 LIMIT`; the process gets `CASE_RECORD`, the record's path,
   and `CASE_FILE`, its file. A server a file crashes then takes none of the
   next files with it, and a file that hangs costs its limit and no more.
   `files.tsv` records each file's exit status and seconds.
4. **The unstable list**, `<application>-unstable.tsv`: rows
   `CASE<TAB>REASON` under the header `# case<TAB>reason`, as in
   `tests/redis-suite/unstable.tsv`, with the reason `passed in K of N
   recorded runs` for a case that passed in some of the Redis runs. The
   workflow runs Redis twice on the same revision; `compare.py` leaves out a
   case the two runs disagree on and writes those not yet on the list in this
   format (`--unstable-found`), to be reviewed and committed.

## Sidekiq

Sidekiq v8.1.7 (the tag's commit is
`1bb4aa06e5aa178a114a5e855f9f3d5c24f6c61b`), the first of the validation
applications, runs in the weekly workflow beside the consumers above, against
Redis 7.2.16 and against firn, with a second Redis run for the unstable list.

- **Tests.** Every `test/**/*_test.rb`, the files of Sidekiq's own
  `bundle exec rake test` (its Rakefile's pattern is `test/**/*.rb`, which adds
  the helpers); Sidekiq's CI runs them against Redis 7.0.15, the latest Redis,
  Valkey 9 and Dragonfly. The tests start no server and run no case in
  parallel. `run.sh` runs each file in a process of its own (`bundle exec ruby
  -Ilib -Itest FILE`), with Minitest's seed fixed at 1 on both servers.
  `rake test` loads every file into one process before it runs a case, so a
  file may use what another loaded: run alone on Redis 7.2.16, four files
  failed 9 cases for a missing `Sidekiq::DeadSet`, `Rack::Session` or
  `Rails.application` (the pattern makes rake boot the dummy application in
  `test/dummy/`). `sidekiq_suite_context.rb` loads those before each file, as
  rake's process has them, and the tests stay as they are; a library it does
  not name that a file needs shows as an error on both servers.
- **Environment.** The newest cell of Sidekiq's CI matrix: Ruby 4.0 and
  `RAILS_VERSION=8.1`. The repository has no lock file, so `bundle install`
  takes the newest gems of the day, and `what.txt` records the Ruby version
  and the versions of `redis-client`, `railties`, `minitest`,
  `connection_pool` and `rack`. The `development`, `tui` and `load_test` gem
  groups (the linters, the terminal UI and the load-test tools) are not
  installed: the tests load only the default and test groups
  (`Bundler.require(:default, :test)` in `test/helper.rb`), so these gems'
  native builds would serve no case.
- **Pointing it at a server.** `REDIS_URL=redis://127.0.0.1:6379/0`, the one
  setting `test/helper.rb` reads (`ENV["REDIS_URL"] ||= ...`, line 24); its
  `reset!` runs `flushdb` on that database before each case. 127.0.0.1
  rather than `localhost`, so that the name does not resolve to `::1` first.
  Both servers listen on 6379, as for Django and rate-limiter-flexible. The
  other addresses in the tests are the unreachable ones of
  `redis_connection_test.rb`, for which a pool is built and nothing connects.
- **A correction to the usage survey.** The survey
  ([usage-survey](../usage-survey/README.md)) writes that Sidekiq's own tests
  default to `redis://localhost:6379/0`, which holds for `test/helper.rb` alone.
  `test/sharding_test.rb` builds pools for databases 6 and 5
  (`Sidekiq::RedisConnection.create(size: 1, db: 6)`, and 5) and flushes both
  before each case, and firn answers `SELECT` for database 0 only today. That
  file is neither changed nor excluded: it fails on firn until firn serves more
  databases, which is the dependency's work.
- **The heartbeat probe.** After the tests, a real `bundle exec sidekiq`
  process runs for about 30 seconds, three heartbeats (`BEAT_PAUSE` is 10
  seconds), on a server started anew, with one job that does nothing already in
  its queue. `heartbeat.txt` records whether the process registered itself
  (`SCARD processes` at 22 s), the jobs it processed (`stat:processed`), the
  jobs left in the queue and the number of ERROR lines in `heartbeat.log`.
  Sidekiq's metrics flush calls `BITFIELD` on a beat after a job was
  processed (`metrics/tracking.rb`, `metrics/shared.rb`); read from the
  source and not run: `fire_event` catches an error from that handler and the
  default error handler logs it, so the heartbeat itself continues and the
  window's metrics are lost. The probe is the run that settles it. firn has no
  `BRPOP` yet, so a `sidekiq` process on firn cannot fetch the job until the
  blocking list commands exist; the probe records what it finds.

### Results

The consumers workflow, dispatched by hand
([run 39647659581](https://github.com/Ming-Research/Firn-wf/actions/runs/39647659581))
at commit f43bea0, on `ubuntu-24.04` runners: Ruby 4.0.7, `railties` 8.1.4,
`redis-client` 0.31.0, `minitest` 5.27.0, `connection_pool` 3.0.2 and `rack`
3.2.7 on every cell, Redis 7.2.16 built from its release archive, firn built at
the same commit. Redis ran twice. All 52 files finished on every cell. An
earlier run at ddc361e, before the probe counted exception lines, gave the
same case counts.

| | Redis run 1 | Redis run 2 | firn |
|---|---|---|---|
| cases recorded | 749 | 749 | 749 |
| pass | 749 | 749 | 731 |
| fail, error | 0 | 0 | 1, 17 |
| seconds in the test files, summed | 55 | 61 | 80 |
| job time with the gems installed | 2 min 27 s | 2 min 42 s | 4 min 54 s |

The two Redis runs passed the same 749 cases, so `sidekiq-unstable.tsv` is
empty. The comparison puts 731 cases in both servers' passes, 18 in Redis's
alone and none in firn's alone. The 18, by the error the case raised (read from
`test.log`):

| Cause on firn | Cases | Files |
|---|---|---|
| `ERR unknown command 'bitfield'` or `'bitfield_ro'`, the metrics' storage | 11 | `metrics_test.rb` 8, `web_test.rb` 3 |
| `ERR DB index is out of range`, `SELECT` of database 5 or 6 | 4 | `sharding_test.rb` |
| `ERR unknown command 'brpop'`, fetching a job | 2 | `fetch_test.rb` |
| `actors_test.rb` "deals with errors": its processor never fetches the job, so the error it waits for never arises; read from the test, not yet confirmed | 1 | `actors_test.rb` |

The heartbeat probe, `heartbeat.txt` of each cell:

| | Redis | firn |
|---|---|---|
| process registered after 22 s | 1 | 1 |
| jobs processed (`stat:processed`) | 1 | none |
| jobs left in the queue | 0 | 1 |
| error lines in `heartbeat.log` | 0 | 1 |

On firn the process starts and registers itself, and its processor's first
`BRPOP` fails (`ERR unknown command 'brpop'`, logged once at INFO level with
the exception's class last on the line, which the probe counts as an error
line); the job stays queued and `BITFIELD` is never reached, so whether
Sidekiq's heartbeat tolerates an error reply on `BITFIELD` is still open: the
probe answers it once firn has the blocking list commands.
