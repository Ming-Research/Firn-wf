# Streams

Board item `firn-scope-min-streams`: Redis 7.2.16's stream type with XADD
(MAXLEN/MINID, `=`/`~`, LIMIT, NOMKSTREAM), XRANGE, XREVRANGE, XLEN, XTRIM,
XDEL and XREAD (BLOCK included), without consumer groups.

Status: implemented by board item `firn-streams-core`, except XREAD's BLOCK
wait, which `firn-streams-xread-block` delivers. The facts below were read
from the Redis 7.2.16 release source (archive SHA-256 as
`tests/redis-suite/reference.sh` pins); paths are relative to its `src/`. Nothing here was measured. Where a fact is deduced
rather than read, it says so.

## Reference semantics

### Shared

- Arity (commands/*.json): XADD -5, XRANGE/XREVRANGE -4, XLEN 2, XTRIM -4,
  XDEL -3, XREAD -4, XSETID -3. Flags: XADD WRITE DENYOOM FAST; XRANGE,
  XREVRANGE, XLEN RO; XTRIM WRITE; XDEL WRITE FAST; XREAD RO BLOCKING;
  XSETID WRITE DENYOOM FAST.
- ID parse (t_stream.c:1898-1946): over 127 bytes is invalid; the ID is cut
  at the first NUL. Strict mode refuses exactly `-` and `+`; non-strict
  reads `-` as 0-0 and `+` as max-max. The text splits at the first `-`;
  each part goes through string2ull (util.c:509-522), string2ll first, a
  negative result refused, then a strtoull fallback that must consume the
  whole part without ERANGE, so a leading space, `+` and leading zeros are
  accepted (glibc behaviour). Without a dash the sequence is the caller's
  missing value. Error: `ERR Invalid stream ID specified as stream command
  argument`.
- TYPE and OBJECT ENCODING both answer `stream` (db.c:904-912,
  object.c:966).

### XADD (t_stream.c:2010-2097, options 908-1048)

- Options, case-insensitive, until exactly `*` or the first other argument,
  which is read as the ID: MAXLEN/MINID take `~` or `=` only when at least
  two arguments follow (930-936, 953-959); MAXLEN is a long long, negative
  refused; MINID is a strict ID with missing sequence 0; LIMIT; NOMKSTREAM.
- Errors, in order: `ERR syntax error, MAXLEN and MINID options at the same
  time are not compatible`, `ERR The MAXLEN argument must be >= 0.`, `ERR
  The LIMIT argument must be >= 0.`, `ERR syntax error, LIMIT cannot be used
  without specifying a trimming strategy`, `ERR syntax error, LIMIT cannot
  be used without the special ~ option`; then the field/value count; then
  an explicit `0-0` or `0`: `ERR The ID specified in XADD must be greater
  than 0-0`, before the key is touched.
- A missing key with NOMKSTREAM answers a null bulk. Otherwise the stream is
  created (dbAdd, so a blocked XREAD is signalled). A stream whose last ID
  is max-max answers `ERR The stream has exhausted the last possible ID,
  unable to add more items`.
- `*`: now-0 if now > last.ms, else last+1, with the time fixed per MULTI or
  script (server.c:1142-1149). `ms-*`: last.seq+1 when ms equals last.ms
  (refused as below if that sequence is the maximum), else ms-0, so `0-*`
  on an empty stream gives 0-1. An ID not above the last answers `ERR The
  ID specified in XADD is equal or smaller than the target stream top
  item`; an entry over 2^30 raw bytes `ERR Elements are too large to be
  stored`, leaving the just-created empty key.
- Reply: the ID as a bulk. Trimming follows the append.
- Default LIMIT with `~`: 100 × stream-node-max-entries, 10000 when that is
  0 or less, at most 1000000; `LIMIT 0` with `~` is unlimited. For the AOF
  client and a master the limit is forced to 0 (1008-1012).

### Trimming (streamTrim, t_stream.c:719-878)

- `~` removes only whole head nodes: for MAXLEN while length − node's live
  count ≥ maxlen; for MINID while the node's last entry ID, a deleted one
  included, is below minid. LIMIT counts live entries removed and stops
  before the node that would pass it.
- `=` also marks entries inside the one node where the threshold falls,
  without rewriting the node; its counters change.
- max_deleted_entry_id is untouched by trimming; first_id is recomputed.
- XTRIM answers the live entries removed; `ERR syntax error, XTRIM must be
  called with a trimming strategy` without one; missing key answers 0
  after option errors.

### XRANGE, XREVRANGE, XLEN, XDEL

- XREVRANGE swaps its two bounds. Missing sequence: 0 for the start,
  UINT64_MAX for the end. `(` (length over one) parses the rest strictly
  and steps the bound inward; stepping past the end answers `ERR invalid
  start ID for the interval` or `ERR invalid end ID for the interval`.
- COUNT may repeat; negative becomes 0; any other option is a syntax error.
  A missing key answers an empty array; COUNT 0 on an existing key a null
  array (2151). Entry shape `[id, [field, value, ...]]` in RESP2 and RESP3.
- XLEN answers 0 for a missing key.
- XDEL looks the key up first (a missing key answers 0 without validating
  IDs), validates every ID, deletes live entries, sets max_deleted_entry_id
  to the largest deleted ID and recomputes first_id.

### XREAD (t_stream.c:2186-2458)

- `+`, `-` invalid (strict); `$` is last_id, 0-0 for a missing key.
  COUNT 0 or negative means unlimited. Errors: `ERR Unbalanced 'xread' list
  of streams: for each stream key an ID or '$' must be specified.`,
  `<command> command is not allowed with BLOCK option from scripts`, and the
  timeout errors of getTimeoutFromObjectOrReply in milliseconds (an
  integer; no floating parse).
- Served when the last live ID is above the given ID, from ID+1. Reply:
  RESP2 array of `[key, entries]`, RESP3 map; nothing served answers a null
  array. Under DENY_BLOCKING (EXEC, scripts, AOF loading) it answers the
  null array at once.
- A blocked XREAD is woken by XADD and by any dbAdd of a stream key
  (creation, RENAME, COPY), not by XTRIM, XDEL or XSETID; deleting or
  replacing the key does not wake it (unblock_on_nokey 0). On wake the
  whole command runs again with `$` already rewritten and the deadline
  kept; a timeout answers a null array.

### XSETID (t_stream.c:2754-2823)

Needed to load a rewritten file. Errors in order: `ERR entries_added must
be positive`, max-deleted above the ID, `ERR no such key`, below the
current max_deleted_entry_id, then for a nonempty stream an ID below the
top item and an ENTRIESADDED below the length. It may lower last_id down to
the last live ID; MAXDELETEDID 0-0 is ignored; it wakes nothing.

### Metadata

| field | XADD | trim | XDEL | XSETID |
|---|---|---|---|---|
| last_id | new ID | unchanged | unchanged | given ID |
| entries_added | +1 | unchanged | unchanged | ENTRIESADDED if given |
| max_deleted_entry_id | unchanged | unchanged | largest deleted | MAXDELETEDID unless 0-0 |
| first_id | new ID when length becomes 1 | recomputed | recomputed | unchanged |

An emptied stream key persists.

## Node layout

The replies of `~` trimming (XLEN after XADD MAXLEN ~, XTRIM's count) depend
on where nodes start, so a reimplementation must reproduce Redis's node
boundaries without storing listpacks. Read from t_stream.c:463-659 and
listpack.c:
- Integer element size I(v): 2, 3, 4, 5, 6, 10 bytes for 0..127, ±4096,
  ±32768, ±2^23, ±2^31, otherwise.
- String element S(s): I(n) if s is a strict int64 in canonical form (1-20
  characters; `-0`, `01`, `+1` stay strings); else header 1+len (len < 64),
  2+len (< 4096) or 5+len, plus a back-length of 1 (≤ 127), 2 (< 16383), 3
  (< 2097151), 4 (< 268435455) or 5 bytes over the header-and-data size.
- Node bytes: 7 (header and end) + master entry I(live) + I(deleted) +
  I(master field count) + Σ S(master field) + I(0) + body. Counters are
  re-encoded when they change, so the size moves at 127/128 and like
  boundaries.
- Entry: I(flags) + I(ms − master ms) + I(seq − master seq, signed) +
  (same fields as master ? Σ S(value) : I(n) + Σ S(field) + Σ S(value)) +
  I(same ? n + 3 : 2n + 4).
- Before appending: a new node starts when there is no tail node, or the
  tail's bytes + Σ raw field and value lengths ≥ stream-node-max-bytes (0
  or above 2^30 means 2^30), or, when stream-node-max-entries ≠ 0, the
  tail's live + deleted count ≥ max-entries. The new node's master entry
  takes this entry's ID and fields. An entry at least max-bytes alone is
  alone in its node.
- XDEL sets the deleted flag (same size) and moves one count from live to
  deleted; a node whose last live entry is deleted is freed, and the
  previous node can become the tail again. No compaction.

## AOF

- Propagated form (server.c:3505, 3620-3654), only when the command changed
  the dataset: XADD's `*` and `ms-*` become the real ID; when `~` was given
  it becomes `=` and the threshold becomes the length after trimming
  (MAXLEN) or the first entry ID including deleted ones (MINID; max-max for
  an empty stream); LIMIT is kept and ignored on replay. XTRIM gets the same
  rewrite and is written only when it removed an entry; XDEL is written
  verbatim when it deleted one; XSETID always; XREAD never.
- Rewrite in command form (aof.c:2062-2199): one `XADD key id field value
  ...` per live entry, `XADD key MAXLEN 0 0-1 x y` for an empty stream,
  then always `XSETID key last ENTRIESADDED n MAXDELETEDID id`. Redis uses
  this form only with `aof-use-rdb-preamble no`; firn's base is always
  command form.
- After a command-form rewrite the nodes are rebuilt from live entries, so
  the layout, and later `~` results, can differ from the server that wrote
  the file (deduced; Redis behaves the same way).

## Reference suite

The gate runs `unit/type/stream` in external mode with append-only off.
The block that sets `appendonly yes` (stream.tcl line 661) aborts the unit,
because firn refuses a run-time switch, so only the first block (tests
1-50) is reachable. Of those, 49 use only this command set plus commands
firn has; test 39 (XDEL fuzz) needs XINFO STREAM. Tests 13, 20 and 45-50
depend on node layout through CONFIG SET stream-node-max-entries.
`wait_for_blocked_client` polls INFO blocked_clients.
