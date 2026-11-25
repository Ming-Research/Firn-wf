# Script engine and registry

The Lua engine scripts run on, the server-wide SHA1 registry, how EVALSHA uses it, and SCRIPT FLUSH.

### decision script-one-shared-engine-and-registry
All scripts take turns on one shared Lua engine, and one server-wide registry maps each SHA1 to its source.
- rejected: an agent proposed a pool of engines with their own library settings and random state, taken in any order; the owner rejected it, because a script's cjson settings and random state would then reach only scripts on the same engine, where Redis shares one Lua state.
- rejected: an agent proposed a registry of scripts per connection; the owner rejected it, because EVALSHA on one connection must find a script another loaded, as Redis's registry is the server's.

### decision script-evalsha-copies-source-on-engine-miss
EVALSHA answers NOSCRIPT from the registry before it takes the engine, and copies the source only for an attempt whose engine has not compiled that SHA1, reading the registry again for it.
- replaced: Copying the script's source at the registry check on every EVALSHA was on main; the owner changed it, because that copy took 2.8 to 3.8% of firn's CPU under rate-limiter-flexible's script (data: `research/investigations/deployment-performance/README.md`).

### decision script-flush-waits-for-running-scripts
SCRIPT FLUSH waits until no script is in progress before it clears the registry, so a script that has first taken the engine is never cut off by a FLUSH.
- rejected: an agent proposed answering NOSCRIPT to a running script whose later attempt needs its source after a FLUSH; the owner rejected it, because in Redis a FLUSH never takes effect while a script runs, and a running script never fails because of one.

### decision script-kept-until-flush
A script compiled by EVAL or SCRIPT LOAD is kept until SCRIPT FLUSH, with no limit on their number, and FLUSH frees the sources at once whether SYNC or ASYNC is given.
- rejected: an agent proposed evicting scripts, as later Redis versions do; the owner rejected it, because Redis 7.0.15 keeps every compiled script until its scripting is reset.
- rejected: an agent proposed freeing the sources in the background; the owner rejected it, because Redis 7.0.15's FLUSH answers the same whether SYNC or ASYNC is given.