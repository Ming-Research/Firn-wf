# Access tracking and configuration: step 1 draft

## Scope and state

At 2026-10-08 22:34 UTC, the uncommitted implementation for
[Firn-wf PR 35, maxmemory](https://github.com/Ming-Research/Firn-wf/pull/35)
adds access stamps, the six CONFIG parameters, INFO's configured limit and
policy, and OBJECT IDLETIME/FREQ/HELP. Eviction and OOM refusal remain step 2.
This is not a completed or validated implementation: script retries still
repeat LFU updates, and the settings hold described below awaits direction.
No build, compiler, Whitefoot checker, test, design lint, benchmark or CI run
was performed for this working diff. No commit was made.

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
  deliberately unsupported and have one follow-up in `docs/todo.md`.
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

## Open direction: access updates across script retries

**How should script attempts undo access updates when their interpreter
budget expires before any dataset write?**

Background: the approved script design retries such attempts from the
beginning. A GET now changes the LFU counter and the random generator, so
`redis.call('GET','k');` followed by a long calculation increments it once
per attempt. The added finite-loop test expects one increment and exposes
this unresolved behavior. Treating a refresh as a dataset write instead
would make `redis.call('GET','k'); while true do end` unkillable, including
under noeviction. That proposal was withdrawn; the draft retains retries
and therefore remains incomplete.

- **A, recommended:** journal original access stamps and random state,
  restore them before abandoning a read-only attempt, and retain normal
  SCRIPT KILL behavior. This needs an ownership/interface design for the
  journal and failure handling, followed by retry and kill tests.
- **B:** stop retrying after the first refresh, so only one attempt's
  accesses happen. It is simpler but changes the accepted meaning of a
  read-only script and can permanently hold the keyspace. Not recommended.

**Confidence 4/5.** The repeated mutation and killability conflict follow
from the existing retry loop; the rollback representation and allocation
failure behavior have not been designed or checked. Owner direction was
requested; none is recorded here yet. No journal is implemented.

## Open direction: settings and keyspace ordering

The draft reads ServerState's memory settings in the same atomic statement
as each command that tracks or inspects a stamp, and holds them through
EXEC and a script attempt. A snapshot taken before the key statement allows
CONFIG to change the policy and another client to read or modify that key
before the old-policy lookup runs, so the setting and key mutation would
not have a common command order.

The shared hold prevents that interleaving but adds ServerState to GET's
otherwise per-key statement. Whitefoot's SHARE-3 specifies exclusive access
to the targets; a read-only optimization is permitted, not promised. It may
serialize unrelated keys and delays server-state operations for a long
script. No cost is measured. The proposed fourth decision in
`design/firn/memory-limit.md` records this draft choice; it is not part of
the owner's three earlier rulings.

- **A, recommended for this step:** retain the common hold as the correct
  baseline and include it in the already planned 14900K stamp-cost
  experiment. Cost beyond the twin spread reopens the representation.
- **B:** design concurrent configuration publication before completing
  step 1. It must preserve CONFIG/key lookup ordering and SCRIPT KILL,
  without a stale-policy fallback. No such representation is selected yet.

**Confidence 3/5.** Correctness of the joint hold follows from the atomic
contract; its practical concurrency cost is unmeasured. Owner direction was
requested and remains open.

## Validation and review

Four network cases were added: CONFIG/INFO/validation and stored-only
limits; idle time, NOTOUCH, OBJECT errors and preflight; LFU initialization,
lookup multiplicity, overwrites, EXEC and script retry; and AOF load-time
initialization. Existing CONFIG GET expectations include the six new
parameters. These cases are unrun, and the script retry case is expected
to expose the known blocker until its design is resolved. Assertions were
not weakened to hide it.

Still unverified: Whitefoot form/effect/bounds acceptance, all network cases
and the Redis ratchet, LFU decay across a minute and wrap boundaries, the
probability distribution, simultaneous CONFIG/lookup behavior, SCRIPT KILL
with a refreshing script, and performance. No local execution was used in
place of CI. A long script's clock follows the existing frozen-time design;
its correspondence with Redis's cached LRU/LFU clock over a long run has
not been demonstrated.

The required separate read-only review inspected the diff and Redis source.
It found and prompted fixes to a CONFIG name boundary, floating-operation
spellings, an INFO divisor whose nonzero proof was unclear, a new atomic
binder collision, OBJECT preflight/unknown error text and license notices.
It also exposed both open direction items above. The reviewer identified itself as GPT-6 (a finer runtime identifier was
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
of the settings finding is closed, while its owner acceptance and cost
remain unverified. The script retry defect remains open. No passing gate
or exact-revision approval is claimed.

After the main review, local repairs removed the unused access-touched flag
and unnecessary settings holds from commands that neither read nor refresh
stamps (DEL, TYPE, TTL, DBSIZE, enumeration, INFO's key count and FLUSH),
and added OBJECT preflight cases in EXEC and scripts. The remaining
settings holds still have the concurrency consequence above.
