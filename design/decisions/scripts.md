# Scripts

Lua scripts run by EVAL, EVALSHA and their read-only forms: their clock, their records, how they reach the commands, and the script environment.

### decision script-clock-frozen-at-start
A script's commands read the clock as it was when the script began, for expiry checks, relative deadlines and TTL replies alike.
- rejected: an agent proposed freezing expiry checks alone, as Redis 7.0.15 does; the owner rejected it, because Redis 7.2 freezes one time for the whole execution unit, and 7.0.15's rule would carry a live clock into every command part for a difference seen only in long scripts.

### decision script-effects-wrapped-in-multi-exec
When a script appended more than one record, the script's caller wraps them in MULTI and EXEC.
- rejected: an agent proposed that each command part wrap its own records; the owner rejected it, because the append-only file must apply a script whole or not at all.

### decision script-commands-reach-scripting-through-an-interface
The commands a script calls reach the scripting module through the ScriptCommands interface, which the commands module binds to script_command.
- rejected: an agent proposed moving the script's statement out of the scripting module into the commands module; the owner rejected it, because the handling of the Lua environment would then be split between the two.

### decision script-read-only-forms-share-the-script-path
EVAL_RO and EVALSHA_RO share EVAL's registry, compiled scripts, attempts and command parts, with read-only authority set at each call and kept through every retry.
- rejected: an agent proposed caching the read-only authority with the script; the owner rejected it, because Redis 7.2.16 takes that authority from the command that invokes the script.
- rejected: an agent proposed separate read-only command implementations; the owner rejected it, because Redis 7.2.16 refuses a script's WRITE command at one point in the shared path, after its arity check and before the persistence and OOM checks.

### decision script-shebang-sources-refused
EVAL and SCRIPT LOAD refuse every source that starts with a shebang, plain #!lua and every flag included.
- rejected: an agent proposed running a flagged script under the legacy OOM rules; the owner rejected it, because firn has no admission rule for script flags yet, and the owner allows an unsupported flag only when it is documented exactly.

### decision script-environment-protected-through-halo
When firn makes a script engine, it protects the environment once through Halo's embedding interface, as Redis 7.2.16's scriptingInit does, and leaves KEYS and ARGV writable.
- replaced: firn writing Halo's table cells itself and locating the refusal was on main; the owner changed it, because then rawset was refused where Redis does not refuse it, while through the embedding Halo raises Redis's message at Redis's location.

### decision script-set-repl-refuses-flags-firn-cannot-honor
redis.set_repl accepts REPL_AOF and REPL_ALL, which change nothing in firn, and refuses REPL_NONE, REPL_SLAVE and REPL_REPLICA with an error naming the unsupported flags.
- rejected: an agent proposed accepting the flags firn would not honor; the owner rejected it, because firn records a command's changes and its expiry deletions under one switch, so it cannot drop only the command's own records.
- rejected: an agent proposed dropping the expiry deletions together with the command's records; the owner rejected it, because Redis 7.2.16 still records the deletion of an expired key the command finds.