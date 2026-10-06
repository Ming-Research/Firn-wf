Decision: The deployment milestone's consumers are Django's Redis cache with its cache sessions (redis-py), connect-redis, the Redis store of express-session (node-redis), and rate-limiter-flexible's Redis limiter (ioredis and node-redis), each exercised through its own Redis tests pinned to a release and run unchanged against Redis 7.0.15 and against firn, because together they cover cache storage, session storage and a conditional update made atomic by a script or a transaction, across two ecosystems and three clients, with workloads that need only a host and port ([criteria and candidates](../../research/investigations/consumers/README.md#candidates)), instead of leaving the consumers open or judging the milestone by a feature inventory.

Decision: A command, option or protocol feature enters the milestone because a selected consumer sends it, ordered by how many consumers its absence stops ([gaps](../../research/investigations/consumers/README.md#what-the-milestone-needs-from-firn)), because the milestone is defined by existing components working unchanged and a consumer's refusal by firn is the observation that names the work, instead of implementing Redis's command families in order of their size or of how firn's code is arranged.

Rejected:
- Rails' RedisCacheStore and Laravel's Redis cache and sessions now: rejected because the three selected components already cover both scenarios and two ecosystems; they stay in reserve for a third ecosystem.
- BullMQ or Sidekiq: rejected because a queue is a candidate scenario, not one the milestone selected.
- limits, the backend of Flask-Limiter: rejected because rate-limiter-flexible covers the scripted update with wider use.
- django-redis: rejected because Django's own Redis backend covers the scenario.
