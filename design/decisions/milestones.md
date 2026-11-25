# Milestones

What firn's milestones are, which applications prove them, and how the commands firn takes on are chosen.

### decision firn-deployment-milestone-is-standalone-application-workloads
Firn's first deployment milestone is existing applications' cache, session and scripted-update workloads, run on firn unchanged through their ordinary Redis clients.
- rejected: an agent proposed releasing a benchmark-only subset of Redis first; the owner rejected it, because benchmark commands alone show neither practical use nor whether a lead comes from the language or from work left out.

### decision firn-scripting-milestone-is-lua-written-in-whitefoot
firn runs Redis's Lua scripts on Halo, a Lua engine written in Whitefoot, which firn takes as a submodule.
- rejected: an agent proposed embedding a native Lua engine as the language's evidence; the owner rejected it, because dynamic values, closures, object reclamation and atomic command composition test Whitefoot, and a native engine would show none of it.
- rejected: an agent proposed a recognizer of known script templates; the owner rejected it, because it would not exercise the dynamic values, closures, object reclamation and atomic command composition that a real Lua engine needs from Whitefoot.

### decision milestone-consumers-are-django-connect-redis-and-rate-limiter-flexible
Django's Redis cache and sessions, connect-redis and rate-limiter-flexible run their own pinned Redis tests unchanged against Redis 7.2.16 and firn, both with AOF on.
- rejected: an agent proposed Rails' RedisCacheStore and Laravel's Redis cache and sessions now; the owner rejected it, because the three chosen components already cover both scenarios and two ecosystems; they stay in reserve for a third ecosystem.
- rejected: an agent proposed limits, the backend of Flask-Limiter; the owner rejected it, because rate-limiter-flexible covers the scripted update and is more widely used.
- rejected: an agent proposed django-redis; the owner rejected it, because Django's own Redis backend covers the same scenario.
- rejected: an agent proposed judging the milestone by a feature inventory; the owner rejected it, because the milestone is defined by existing components working unchanged.

### decision first-release-validation-applications-follow-the-usage-survey
The consumers workflow also runs Sidekiq's and BullMQ's own tests unchanged against Redis 7.2.16 and firn, as the first of the usage survey's validation applications.
- replaced: the milestone's three consumers alone, with Sidekiq and BullMQ turned down as a queue scenario the milestone had not chosen, was on main; the owner changed it, because the owner set the first release to production drop-in use, and task queues and pub/sub could hardly connect to firn.

### decision firn-command-scope-follows-consumers-and-the-usage-survey
A command, option or protocol feature enters firn because a selected consumer sends it, or because the usage survey's first-release list, which the owner approved, names it.
- rejected: an agent proposed implementing Redis's command families in order of their size or of how firn's code is arranged; the owner rejected it, because the milestone is existing components working unchanged, and a consumer's refusal by firn is what names the work.
- rejected: an agent proposed a narrower first release that leaves pub/sub, blocking lists, WATCH and collection scans for later; the owner rejected it, because each missing feature blocks a whole class of common applications, and task queues and pub/sub could not use firn.
- rejected: an agent proposed putting multiple databases, minimal streams and BITFIELD in the first release at once; the owner rejected it, because they are three more pieces of work, and multiple databases would change the keyspace's structure and the performance work.