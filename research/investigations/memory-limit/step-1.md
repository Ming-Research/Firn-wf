# Access tracking and configuration: step 1 draft

## Scope and state

The step-1 draft on
[Firn-wf PR 35, maxmemory](https://github.com/Ming-Research/Firn-wf/pull/35)
adds access stamps, the six CONFIG parameters, INFO's configured limit and
policy, and OBJECT IDLETIME/FREQ/HELP. Eviction and OOM refusal remain step 2.
The current uncommitted continuation implements the owner's A rulings on
settings snapshots and script retry rollback, recorded below. It is not
validated: no build, compilation, Whitefoot checker, test, design lint,
benchmark or CI run was performed for these working changes. No commit was
made, and `design/log.md` was left unchanged as requested.

The reference is Redis 7.0.15 source, not output from this implementation.
The Whitefoot pin and all submodules are unchanged. No Whitefoot gap has
been established: compilation and its diagnostics have not been obtained.

## Behaviors implemented for validation

- `Entry.access` holds the 24-bit seconds clock under all six non-LFU
  policies, including noeviction, or the minute stamp and logarithmic
  counter under either LFU policy. New LFU objects start at five. Decay,
  saturation, probability and the one-wrap calculations follow
  [evict.c](https://github.com/redis/redis/blob/7.0.15/src/evict.c)'s
  `LFULogIncr`, `LFUDecrAndReturn`, `LFUTimeElapsed` and
  `estimateObjectIdleTime`. The random draws use firn's existing generator;
  its sequence is not Redis's sequence. The copied algorithms retain the
  Redis notices beside their source.
- Refresh precedes type checks, so a lookup that finds a live wrong-type
  key still refreshes it. Missing and expired entries do not refresh.
  LFU replacement preserves the old stamp after the command's lookups;
  LRU replacement creates a current stamp. COPY creates a fresh stamp;
  RENAME moves its refreshed source object's stamp. These follow
  [db.c](https://github.com/redis/redis/blob/7.0.15/src/db.c)'s `lookupKey`,
  `dbOverwrite`, `setKey`, `copyCommand` and `renameGenericCommand`, and
  [object.c](https://github.com/redis/redis/blob/7.0.15/src/object.c)'s
  `createObject`. Firn has no fork child for which to suppress stamping.
- CONFIG defaults and ranges follow
  [config.c](https://github.com/redis/redis/blob/7.0.15/src/config.c).
  Memory units reuse firn's `memtoull` translation, including unsigned
  saturation and wrapping unit multiplication from
  [util.c](https://github.com/redis/redis/blob/7.0.15/src/util.c).
  CONFIG GET returns bulk strings and preserves its existing matching,
  casing and deduplication rules. CONFIG SET validates every pair before
  applying any. Policy changes reinterpret existing stamps as Redis does.
- INFO's `maxmemory_human` follows
  [server.c](https://github.com/redis/redis/blob/7.0.15/src/server.c)'s
  `bytesToHuman`, including binary units, two decimal places, ties to even
  and bytes again at 2^60 or above.
- OBJECT checks a missing key before checking policy, does not store decay
  when inspecting FREQ, and returns Redis's exact policy errors and help.
  Arity and unknown-subcommand errors are resolved before authentication
  and transaction queueing, following `commandCheckExistence` in server.c.
  IDLETIME/FREQ have arity 3 and READONLY; HELP has arity 2 and LOADING/STALE;
  the container has arity -2 and no flags. Firn does not yet expose command
  descriptors through COMMAND, so the flags are documented rather than
  added to a nonexistent descriptor table. ENCODING and REFCOUNT remain
  deliberately unsupported and have one follow-up, status-board item
  firn-bl-01-31.
- EXEC and scripts use the same command bodies. AOF replay creates stamps
  from the calendar clock passed to replay, while keeping the expiry-check
  time zero as before; access stamps are not serialized into the log.

## Complete command lookup inventory

Each site below is a logical lookup of a top-level key. Lower-level reads
of fields, members, list elements, or bookkeeping on a slot already looked
up do not add refreshes. The network wrappers and `held_run`/`script_*`
wrappers share these bodies; only the explicit script-only sites listed
below add a refresh of their own. Syntax rejected before Redis performs a
lookup does not refresh. Every refreshing site skips expired entries.

| Source and lookup site | Commands | Refresh |
| --- | --- | --- |
| strings.wf `get_body` | GET | Once |
| strings.wf `set_body` | SET, SETNX, SETEX, PSETEX, GETSET | Once; SET GET and GETSET also perform the GET lookup before the store lookup; a wrong-type GET stops before the second lookup |
| strings.wf `mget_body` | MGET | Each argument, including duplicate names |
| strings.wf `mset_body` | MSET, MSETNX stores | Each pair in argument order, including duplicate names |
| strings.wf `run_msetnx`; script.wf `script_msetnx` | MSETNX existence pass | Each reached key until a live key stops the pass; successful stores then use mset_body |
| strings.wf `incr_body` | INCR, INCRBY, DECR, DECRBY | Once |
| strings.wf `incrbyfloat_body` | INCRBYFLOAT | Once |
| strings.wf `append_body` | APPEND | Once |
| strings.wf `setrange_body` | SETRANGE | Once |
| strings.wf `getdel_body` | GETDEL | Once |
| strings.wf `strlen_body` | STRLEN | Once |
| strings.wf `getrange_body` | GETRANGE, SUBSTR | Once |
| strings.wf `getex_body` | GETEX | Once |
| hashes.wf `hset_body` | HSET, HMSET | Once |
| hashes.wf `hsetnx_body` | HSETNX | Once |
| hashes.wf `hget_body` | HGET | Once |
| hashes.wf `hmget_body` | HMGET | Once for the key, not once per field |
| hashes.wf `hdel_body` | HDEL | Once |
| hashes.wf `hfield_body` | HEXISTS, HSTRLEN | Once |
| hashes.wf `hlen_body` | HLEN | Once |
| hashes.wf `hall_body` | HGETALL, HKEYS, HVALS | Once |
| hashes.wf `hincrby_body`, `hincrbyfloat_body` | HINCRBY, HINCRBYFLOAT | Once |
| hashes.wf `hrandfield_body` | HRANDFIELD | Once |
| lists.wf `push_body` | LPUSH, RPUSH, LPUSHX, RPUSHX | Once |
| lists.wf `pop_body` | LPOP, RPOP | Once |
| lists.wf `lrange_body`, `llen_body`, `lindex_body` | LRANGE, LLEN, LINDEX | Once |
| lists.wf `lset_body`, `lrem_body`, `ltrim_body`, `linsert_body`, `lpos_body` | LSET, LREM, LTRIM, LINSERT, LPOS | Once |
| lists.wf `lmove_body` | LMOVE, RPOPLPUSH | Source, then destination only after a live source list; two lookups when the names are equal |
| sets.wf `sadd_body`, `srem_body`, `spop_body`, `scard_body`, `smembers_body` | SADD, SREM, SPOP, SCARD, SMEMBERS | Once |
| sets.wf `sismember_body` | SISMEMBER, SMISMEMBER | Once for the key, not once per member |
| sets.wf `srandmember_body` | SRANDMEMBER | Once |
| sets.wf `smove_body` | SMOVE | Source and destination, even when equal or the source is missing |
| sets.wf `survey` | SINTER, SUNION, SDIFF, SINTERCARD and their STORE variants | Each operand in argument order, including duplicates; stop at the first wrong type |
| sets.wf `set_combine_body` | SINTERSTORE, SUNIONSTORE, SDIFFSTORE | Destination lookup for a nonempty result; deletion of an empty result has no lookup |
| sorted.wf `zadd_body`, `zrem_body`, `zpop_body`, `zcard_body`, `zscore_body`, `zmscore_body`, `zrank_body` | ZADD/ZINCRBY, ZREM, ZPOPMIN/MAX, ZCARD, ZSCORE, ZMSCORE, ZRANK/ZREVRANK | Once for the key |
| ranges.wf `zrange_body` | All implemented ZRANGE, ZREVRANGE, BYSCORE and BYLEX forms | Once |
| ranges.wf `zcount_body` | ZCOUNT, ZLEXCOUNT | Once |
| ranges.wf `zremrange_body` | ZREMRANGEBYRANK, ZREMRANGEBYSCORE, ZREMRANGEBYLEX | Once |
| keys.wf `run_exists`; script.wf `script_named` | TOUCH | Each argument, including duplicates |
| Same sites | EXISTS | No: NOTOUCH |
| keys.wf `run_type`/`entry_kind`; script.wf `script_named` | TYPE | No: NOTOUCH |
| keys.wf `expire_body` | EXPIRE, PEXPIRE, EXPIREAT, PEXPIREAT | Once, including a failed NX/XX/GT/LT condition |
| keys.wf `ttl_body` | TTL, PTTL, EXPIRETIME, PEXPIRETIME | No: NOTOUCH |
| keys.wf `persist_body` | PERSIST | Once, including a live key with no expiry |
| keys.wf `rename_body` | RENAME, RENAMENX | Source, then a distinct destination if the source exists; same-name rename looks up only the source |
| keys.wf `copy_body` | COPY | Source and, when reached, destination; equal names are rejected before either lookup |
| keys.wf `remove_live`; script.wf `script_named` | DEL, UNLINK | No: direct deletion, not lookupKey |
| object.wf `object_body` | OBJECT IDLETIME, FREQ | No: NOTOUCH; HELP and unsupported subcommands do not look up a key |
| scan.wf `scan_body` | SCAN, including TYPE filter | No: NOTOUCH for the filter |
| scan.wf `keys_body`, `held_randomkey` | KEYS, RANDOMKEY | No: enumeration/direct dictionary access |
| keys.wf `dbsize_body`; info.wf `run_info`; server.wf `held_flush` | DBSIZE, INFO key count, FLUSHALL/FLUSHDB | No: count or clear, not lookupKey |
| store.wf expiry removal; persistence replay/rewrite traversal | Internal housekeeping | No additional refresh; replayed commands use the ordinary bodies |

The NOTOUCH oracle is exhaustive for firn's implemented commands:
`existsCommand`, `typeCommand` and `scanGenericCommand` in
[db.c](https://github.com/redis/redis/blob/7.0.15/src/db.c);
`ttlGenericCommand` (TTL/PTTL/EXPIRETIME/PEXPIRETIME) in
[expire.c](https://github.com/redis/redis/blob/7.0.15/src/expire.c);
and `objectCommandLookup` in
[object.c](https://github.com/redis/redis/blob/7.0.15/src/object.c).
`touchCommand`, `persistCommand` and `expireGenericCommand` in expire.c use
ordinary refreshing lookups. `delGenericCommand`, `keysCommand`,
`dbRandomKey` and `dbsizeCommand` in db.c do not perform a refreshing
lookup; they are distinct from explicit NOTOUCH calls.

New objects are made at strings.wf `put_text`; hashes.wf `fresh_hash`,
`hincrby_body`, `hincrbyfloat_body`; lists.wf `place_element`, `push_body`;
sets.wf `place_member`, `store_members`, `sadd_body`; sorted.wf `zadd_body`;
and keys.wf `held_copy`. All eleven Entry constructors put `access` after
`expires`. `put_text` and `store_members` preserve a live old LFU stamp on
replacement; COPY creates a new one; the other sites construct a missing
key and use `new_access`.

## Recorded outcome: script retry access

The owner chose **A** on board card `firn-script-retry-access` at
2026-10-09 01:17 UTC: restore abandoned attempts' access stamps and firn's
random state. Redis runs a script once; increments made by an attempt firn
abandons must not survive into another attempt. Marking a refresh as a
dataset write would instead make a read-only infinite script unkillable.

The attempt's `Env.access` owns an optional `AccessJournal`. It consists of
a `KeySet` for binary-name deduplication and a growable array of `AccessUndo`
rows, each holding an owned key and its original `u32` stamp. The first live
refresh inserts the key and saves its stamp before changing it; later
refreshes find its insertion index and keep the first row. Missing and
expired keys and NOTOUCH calls do not record a row. The journal is passed
explicitly through `ScriptCommands.call`, `script_command`, `held_run` and
the shared command bodies. Ordinary commands and EXEC pass `None` and
allocate no journal. Key-set bodies carry the matching key set with an
explicit equality contract on the entry count, so a row names the key that
was actually refreshed, including duplicate operands and wrong-type lookups.

`Env.access_random` saves `Client.random`, the generator LFU uses, before
starting the attempt. In `scripting/entry.wf`, only `Budget()` with
`env.held.wrote` false calls `store.restore_access` and restores this random
state, inside the existing keys/metadata statement, before releasing it.
The existing Lua random/configuration restoration follows there. A completed
attempt, including a script error, or an attempt that wrote retains its
refreshes. The attempt environment and its journal are dropped together;
the retry wait and SCRIPT KILL check remain between attempts, so a kill
following abandonment leaves the restored stamps intact.

Allocation follows the pinned Whitefoot specification's STOR-8: allocation
is total in source and heap exhaustion terminates from the trusted base.
There is no recoverable allocation failure to handle by skipping a row;
recording finishes before the access stamp changes.

Rejected card options:

- **B, accept LFU drift and defer it:** OBJECT FREQ could expose increments
  Redis's single execution never makes.
- **C, retain the keyspace after the first refresh:** a read-only infinite
  script could permanently block key operations and evade SCRIPT KILL,
  changing the approved retry design.

## Recorded outcome: settings snapshot

The owner chose **A** on board card `firn-policy-read` at
2026-10-09 01:19 UTC: read settings once per request read beside the clock,
then let key statements hold only the keys and metadata they otherwise need.

`server.serve` takes the snapshot immediately after `read_time` and before
its command loop, in a short statement holding only `store.server`.
`store.access_settings` copies policy, maxmemory, LFU log factor and decay
time to the connection's `Client`. `prepare_access` takes the calendar time
from that read's `Time`. Every command parsed from the read uses that client;
`run_exec` supplies it unchanged to queued commands, using the read that
contains EXEC, and `scripting.eval` supplies it to every attempt and every
script command. No command refreshes these settings while holding keys.
CONFIG GET/SET continue to access ServerState, and INFO obtains its reporting
values directly from ServerState without changing the client's snapshot.
AOF replay keeps the initialized default settings and the replay calendar
clock; access stamps remain absent from the persisted log.

A command racing CONFIG SET may write an old-policy stamp, and CONFIG SET
inside a pipeline does not change the lookup snapshot of that same read.
This is accepted: Redis itself reinterprets rather than rewrites existing
stamps when policy changes, and OBJECT's error says LRU and LFU data take
time to adjust. The prior strict CONFIG/key ordering is no longer required.

The shared hold in the draft serialized commands across cores. The owner's
roughly 7–10% two-CPU, depth-16 loss summary comes from
[Firn-wf run 37859919278](https://github.com/Ming-Research/Firn-wf/actions/runs/37859919278):
Redis benchmark GET and SET, LTO builds on the i9-14900K, two interleaved
passes of five seconds, main versus the draft and a draft twin. The artifact's
`revisions.csv` names base `ef86edfeceb20ba61d95989f5b1e465ddeb89f52` and
head `7d27b2718ef7c88a49965789cbbea3edd14888bf`, both pinned to
`wf-d9d6c92fcb64`; `host.txt` records Redis benchmark 7.0.15 and Clang/LLD
22.1.8. This measures the combined stamp-and-hold draft, not the isolated
cost of the hold. It supports removing that serialization point; a new
comparison must establish the cost of the snapshot and journal implementation.
The investigation's original stamp-cost criterion still applies.

Rejected card options:

- **B, keep the settings/key hold:** independent key commands share one
  serialization point, with the draft's two-CPU loss exceeding twin spread.
- **C, first design concurrent configuration publication:** the accepted
  snapshot already permits Redis's stale-stamp transition; a versioned or
  lock-free publication mechanism has no selected representation and is
  unnecessary for this step.

## Validation and review

Four network cases were added: CONFIG/INFO/validation and stored-only
limits; idle time, NOTOUCH, OBJECT errors and preflight; LFU initialization,
lookup multiplicity, overwrites, EXEC and script retry; and AOF load-time
initialization. Existing CONFIG GET expectations include the six new
parameters. For the current continuation they remain unrun. The existing long
read-only EVAL expectation remains `:9` unchanged. Two additional unrun cases
cover first-refresh deduplication across repeated/binary/empty keys and
command families, retaining refreshes after completion/errors/writes, and
restoring stamps when SCRIPT KILL ends a read-only attempt. The pre-fix
failure was reported by the owner; no local before/after run was made.

Still unverified: Whitefoot form/effect/bounds acceptance, all network cases
and the Redis ratchet, LFU decay across a minute and wrap boundaries, the
probability distribution and the exact random-state rollback sequence,
simultaneous CONFIG/lookup behavior, SCRIPT KILL
with a refreshing script, and performance. No local execution was used in
place of CI. A long script's clock follows the existing frozen-time design;
its correspondence with Redis's cached LRU/LFU clock over a long run has
not been demonstrated.

The initial draft's separate read-only review inspected the diff and Redis source.
It found and prompted fixes to a CONFIG name boundary, floating-operation
spellings, an INFO divisor whose nonzero proof was unclear, a new atomic
binder collision, OBJECT preflight/unknown error text and license notices.
It also exposed the two directions now ruled above. The reviewer identified itself as GPT-6 (a finer runtime identifier was
unavailable) and read origin/main `ef86edfeceb20ba61d95989f5b1e465ddeb89f52`
through HEAD `fa0a912069eff098cbe626c7980990611830d070`, the tracked working
diff, and the four then-untracked design/access/OBJECT/license files. It
read all firn design nodes, the project checklist, affected consumers and
pinned language references; no checklist group was skipped. Its review was
source inspection only, not execution.

A1, C1, T1, T2 and D1 passed within that inspection; C2 and T3 remained
unverified without compiler/gate results; measurement R1 was not applicable.
G1 and G2 passed for the three requested decisions then present. G3, DC1,
DC2 and DC4 retained the script-retry and settings-hold findings; DC3 was
not applicable. No design lint counts were obtained. The limited follow-up read the subsequent record, fourth decision, TODO,
cleanup and OBJECT preflight cases and found no new defect. G1, G2 and DC1
passed there; G3 passed for explicit disposition. The missing-record part
of the settings finding was closed at that review, while owner acceptance
and cost were then unverified and the script retry defect remained open. No passing gate
or exact-revision approval is claimed.

After the main review, local repairs removed the unused access-touched flag
and unnecessary settings holds from commands that neither read nor refresh
stamps (DEL, TYPE, TTL, DBSIZE, enumeration, INFO's key count and FLUSH),
and added OBJECT preflight cases in EXEC and scripts. The current continuation removes the remaining shared settings holds.

## Continuation review

A separate read-only GPT-6 agent (finer runtime identifier unavailable)
reviewed origin/main `dfe7cfe53a0ae56c0f07970d8162e56517655bc0` through
HEAD `92e25637be738c4d02a45c4e08ee82417d397da4` plus the current working
changes; there were no untracked files. It inspected the complete diff,
affected command bodies and callers, interfaces, cases, records, the owner's
board rulings and the earlier benchmark artifact. Its design scope included
`design/firn.md` and the memory-limit, scripts, transactions, command-parts
and reported-facts nodes. It read the pinned Whitefoot contracts and Redis
7.0.15 reference source. Every project checklist group and G1–G3/DC1–DC4
was considered; none was skipped. No checker or lint ran, so there are no
lint-derived node, depth or decision counts.

Findings and dispositions:

- **F1, effect order, fixed.** Fourteen changed signatures placed reads or
  writes outside argument order. Their rows now retain the same effects
  in pinned EFF-1 order; the reviewer inspected the repairs. This is a
  source-form correction, not evidence of compiler acceptance.
- **F2, INFO script count, prose fixed and behavior deferred.** INFO and
  the README incorrectly described firn as having no scripts. That prose
  is corrected. The preexisting `number_of_cached_scripts:0` remains false
  after EVAL or SCRIPT LOAD registers a script, contrary to reported-facts;
  the TODO of that time recorded the impact, correction, validation and
  reopening condition. Correcting that metric and its coverage is outside this
  continuation.
- **U1, random-state evidence, unverified.** The new rollback cases use
  log factor zero, so their expected counts detect stamp rollback and
  first-refresh deduplication but do not distinguish a restored random
  generator from consumed draws. The restore statement is present; its
  exact runtime sequence has no discriminating evidence yet.

| Review item | Disposition |
| --- | --- |
| A1, C1, T1, T2 | Pass within source inspection: independent Redis expectations, unchanged `:9`, no vendoring, ratchet or selection change |
| C2, T3 | Unverified: interfaces inspected and effect order repaired, but no compiler or gate result |
| R1 | Pass only for the historical comparison's stated scope; current performance unverified |
| D1, G1, G2, G3, DC1, DC3 | Pass within source inspection after the documented repairs and disposition |
| DC2 | Existing INFO count contradiction deferred as F2; no new snapshot/journal contradiction found |
| DC4 | Unverified: paths inspected, compiler and runtime evidence absent |

The reviewer found no remaining actionable defect in the snapshot or journal
implementation within that source-only scope. No build, compiler, tests,
Whitefoot checker, custom checker, design lint or measurement ran locally;
no CI validation was run for these uncommitted changes. No passing gate or
performance result is claimed. No pin or submodule moved, no Whitefoot gap
was established or filed, no commit was made, and `design/log.md` remains
unchanged as requested.
