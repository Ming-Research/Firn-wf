# Collection scans

Investigation for board items `firn-scope-collection-scan` (2026-10-13) and
`firn-collection-scan-bounded`. The selected direction is implemented, and the
gate's network cases and Redis suite check it. Redis reference replies were
recorded in the run linked below.

## Selected scope

SSCAN, HSCAN and ZSCAN return a collection that is compact by its current
content whole, with cursor zero and MATCH applied, whatever cursor and COUNT
arrive. Any other collection is scanned in steps: one call lends the members
whose home buckets are the next COUNT of Whitefoot's `hash_map_scan` from the
cursor, and the library's next position, printed in decimal, is the reply's
cursor, zero at the end. COUNT and cursor text are validated as Redis 7.2.16
validates them. Sets and hashes scan the ordinary map; a sorted set that is not
compact scans its member-to-score map, so its steps are in hash order, not
score order.

Compact means Redis 7.2.16's default listpack limits applied to the current
content: at most 128 members of a set or sorted set, or 512 fields of a hash
(`set-max-listpack-entries`, `zset-max-listpack-entries`,
`hash-max-listpack-entries`), and no member, field or hash value longer than 64
bytes. The owner selected this reading of the content, with no encoding
history, for ZSCAN score text first, and the same classification now decides
the shape of the reply: a listpack-encoded Redis collection answers whole with
cursor zero, so a client that calls SSCAN once on a small set gets the
members it gets from Redis, and the recorded transcripts below stay exact. The
check reads at most 512 entries. An all-integer set is judged like any other
set, so one of 129 to 512 integers is scanned in steps where Redis's intset
answers whole.

The decision is in [enumeration](../../../design/firn/enumeration.md).
The [script restore decision](../../../design/firn/scripts.md) keeps logical
collection copying: the cursor is a position in the hash space, which a rebuild
of the backing leaves valid, so a scan that spans a script attempt still
returns every member present throughout it, and no layout-dependent result is
promised. The pin names the Whitefoot release that carries `hash_map_scan`.

## Reference and representation evidence

The reference is [Redis 7.2.16](https://github.com/redis/redis/tree/7.2.16).
Compiler pin `wf-d75f318d5631` names Whitefoot source
`d75f318d56318834785322bde3a3656fb93116a6`. Interfaces and the specification
were inspected at that source revision.

[`Value`](../../../firn/store/module.wfm) stores sets and hashes as ordinary
HashMap values. Sorted stores an ordered score/member map and a member/score
map. None retains Redis encoding history; sets and hashes retain no listpack
order. Encoding CONFIG values are remembered but do not affect storage, as
[the server documentation](../../../firn/README.md) states. kept_score only
normalizes negative zero. The shape of a scan reply and ZSCAN score text are
classified from the current cardinality and the member lengths.

Redis's defaults are 512 integer set members, 128 listpack set members with
64-byte members, 512 hash fields with 64-byte fields/values, and 128 sorted-set
members with 64-byte members. The requested hash figure of 128 is not Redis's
default: [config.c](https://github.com/redis/redis/blob/7.2.16/src/config.c)
sets hash-max-listpack-entries to 512. Firn's classification uses the defaults, not
the figures a CONFIG SET remembers, and leaves existing encoding CONFIG
behavior unchanged.

For Redis, current cardinality does not establish compact encoding. The
[set implementation](https://github.com/redis/redis/blob/7.2.16/src/t_set.c)
uses the first member and raw command argument-count hints to choose or promote
encodings, even before duplicate elimination. The
[hash implementation](https://github.com/redis/redis/blob/7.2.16/src/t_hash.c)
can promote based on offered pair count or an oversized offered field/value.
Deletion normally does not demote. intset iteration is numeric; listpacks
follow their physical order, with ordinary new members/fields appended and
updates retaining position. intset-to-listpack conversion retains numeric
order before appending the noninteger member. The selected Firn scope does not
reproduce these histories or compact result order, except that a compact
sorted set keeps score order.

## Resumable scan

Whitefoot's `hash_map_scan` (Whitefoot#487) takes a map, a cursor, a limit of
home buckets, the key binding and a visitor; it lends every pair whose home is
one of up to limit consecutive buckets and returns the position after the last
of them, zero when the pass is complete. Any cursor value is accepted. Between
calls the map may change in any way: every pair present from the start of a
pass to its completion is lent at least once, a pair may be lent again after
the bucket count changes, and a pass over an unchanged map lends each pair
once. The limit must be at least one, which firn guarantees with a small
function whose postcondition the call site uses. The limit is a hint for work,
not a cap on pairs. Capacity is three quarters of the bucket count, so a step
lends fewer than one pair per home bucket (deduced from the interface, not
measured). The call holds the map read-only, so SSCAN, HSCAN and ZSCAN run it
twice under one hold, as they ran the whole-map visitor twice: once to count
the matching pairs for the array header and once to write them.

Sets and hashes pass their map. A sorted set keeps its members twice, in an
ordered map of (score, member) and in a map from member to score; the cursor
of a hash map cannot address the ordered one, and a numeric offset into the
ordered map would lose members that stay when earlier ones are removed, so
ZSCAN of a sorted set that is not compact scans the member-to-score map and
writes each score as `%.17g`. The ordered map still serves a compact sorted
set, whole, in score order.

The older direct-bucket debt firn-bl-01-20 remains; this change introduces no
bucket access.

## Parsing, expiry and framing

Redis's collection handlers parse argument 2 as the cursor, then look up the
key, then check its type, then parse options from argument 3. A missing or
expired key returns bulk cursor 0 and an empty array without parsing options.
WRONGTYPE beats malformed COUNT or unknown options. An invalid cursor beats
both key absence and WRONGTYPE. Normal read access tracking applies and lazy
expiry removes and propagates DEL through the shared command metadata path.

[scanGenericCommand](https://github.com/redis/redis/blob/7.2.16/src/db.c)
accepts MATCH and COUNT in either case, with the last repeated option winning.
Option names use C-string comparison; MATCH uses the complete binary pattern.
COUNT's canonical signed-long integer parser gives
ERR value is not an integer or out of range for malformed/overflowing input,
and ERR syntax error for values below 1. Missing operands, unknown options,
TYPE and NOVALUES are syntax errors on a live correctly typed collection.
NOVALUES is not part of Redis 7.2.16.

Cursor parsing uses x86-64 strtoul: empty text, an optional sign, leading zeros
and ignored bytes after the first NUL are accepted within the unsigned-long
magnitude limit. Leading whitespace, overflow, a bare sign and trailing
non-NUL text are invalid. The implementation shares Firn's existing cursor
parser without parsing options before key lookup.

Both protocols return an outer two-element array, a bulk cursor and an array
of bulk strings. HSCAN and ZSCAN pairs remain flat. ZSCAN never emits a RESP3
double here; the selected scores-module formatter supplies text to the bulk writer. Network,
EXEC and finite declared-entry/whole-map script paths share the same entry
body. Firn's COMMAND description remains empty; it has no populated descriptor
table to extend. Shared command identities and arities cover admission instead.

## ZSCAN score formats and recorded replies

Redis's [listpack insertion](https://github.com/redis/redis/blob/7.2.16/src/t_zset.c#L1030)
uses integer storage for values accepted by double2ll; other values are stored
as d2string text. On x86-64, double2ll requires an exact integer roundtrip and
a magnitude of approximately at most 2^62, not every representable i64.
[util.c](https://github.com/redis/redis/blob/7.2.16/src/util.c#L593) defines that
check and d2string's fpconv_dtoa path. Compact ZSCAN reads this stored text.

[Dictionary ZSCAN](https://github.com/redis/redis/blob/7.2.16/src/db.c#L872)
instead calls ld2string with LD_STR_AUTO, which
[util.c](https://github.com/redis/redis/blob/7.2.16/src/util.c#L793) writes using
%.17Lg after promotion of the stored double to long double. For a widened
double these are the same digits as %.17g of the original double. Both
protocols still frame these texts as bulk strings.

All ten batches were recorded in
[the Redis 7.2.16 oracle run](https://github.com/Ming-Research/Firn-wf/actions/runs/38398911733).
Batch names, RESP selections and command arrays match
[the recorder manifest](../../../tests/collection-scan-oracle.json) exactly.
The eight comparable full transcripts are identical between RESP2 and RESP3:
direct replies total 325 bytes per protocol, MULTI/EXEC 235, scripts 211,
and compact-score probes 253. Their exact concatenated latin-1 bytes now fill
all eight transcript constants in
[the network cases](../../../tests/collection_scan.rs), without HELLO replies.
The six per-member score probes give:

| Input | Listpack text | Skiplist text |
| --- | --- | --- |
| 0.1 | 0.1 | 0.10000000000000001 |
| 1e17 | 100000000000000000 | 1e+17 |
| 1.5 | 1.5 | 1.5 |
| 3 | 3 | 3 |
| inf | inf | inf |
| Lua 2/3 | 0.6666666666666666 | 0.66666666666666663 |

These recordings confirm the earlier source predictions. Cursor and option
errors, missing-key precedence, script conversions and transaction replies
show no unexpected differences. The forced-skiplist full reply's member order
differs between the two recordings; that order is not an expected Firn result.
The former universal %.17g ZSCAN path would differ from compact Redis for
three recorded scores: 0.1, 1e17 and 2/3. That comparison is a source-level
consequence of the former formatter, not a measurement of a running Firn.

The owner selected a current-content rule: at scan time, at most 128 members
and every member at most 64 bytes use d2string; otherwise use %.17g. This
change uses those default limits, not remembered CONFIG encoding values.
Classification covers the complete sorted set before MATCH, including an
oversized member that MATCH excludes. Removing the last oversized member or
shrinking from 129 to 128 members switches Firn back to d2string. Redis normally
retains skiplist encoding and skiplist text after that shrink; this is an
explicitly selected divergence, recorded in the enumeration design node.

Compact ZSCAN calls the scores module's append_double_text d2string
interface, whose implementation and MIT-attributed helper chain use Halo-wf's
existing fpconv port. Large ZSCAN calls score_bytes_17g, which the module
publishes as:

```whitefoot
public fn score_bytes_17g(score: f64) -> text: pkg::bytes::Bytes pure doc "Redis 7.2.16's ld2string(LD_STR_AUTO) text of a score widened to long double, which ZSCAN writes for a sorted set Redis keeps as a skiplist.";
```

The other shared score_bytes interface is not a d2string interface: AOF
rewrites use it for plain fpconv text. ZSCAN therefore does not call it for
compact formatting.

The manifest remains a JSON list of name, resp, optional redis-server startup
config, and commands. Each entry was recorded on a fresh empty server; HELLO
is excluded. Forced-skiplist reference batches use CONFIG before creation,
while the new Firn cases trigger the dictionary text with 129 members or a
65-byte member and use no CONFIG. Their six exact per-member frames come from
the recorded probes; boundary policy after shrink comes from the owner's rule.

## Validation scope

The network cases compare complete member/value coverage independently of map
order, protect cursor/error precedence, options, binary matching and flat bulk
framing, and exercise direct commands, EXEC and both script admission modes.
There are nineteen tests: eight recorded transcript tests, two score-selection
boundary tests, and nine independently specified cases: cursor, options and
lookup precedence; binary MATCH and pair association; the compact bound at 128,
129, 512 and 513 entries and at 64 and 65 bytes, including a hash's value,
each checked as one call with cursor zero or as a scan that cannot finish at
COUNT 1 and then covers every member once; a 2000-member collection of each
kind followed in steps of COUNT 20, none holding a tenth of it, with and
without MATCH; arbitrary cursors, COUNT beyond the table and the repeatability
of one cursor; members present throughout returned while others are added and
removed between calls and a burst grows the table; Lua following a large scan
across calls for a declared and an undeclared key; script retry with an earlier
write; and expired direct/EXEC/script reads. The expiry case
does not isolate lazy deletion from active expiry; it checks the externally
visible missing-key behavior. The bounded cases fail on the whole-collection
code they replace: its first call at COUNT 1 returns the whole collection with
cursor zero (deduced from the code, not run).
The score-selection cases cover 128 -> 129 -> 128 members and 64 -> 65 -> 64
maximum member bytes, including a 65-byte UTF-8 member with 33 characters.
MATCH hides the triggers; direct, EXEC and declared-key Lua probes must still
select the complete collection's formatter, at COUNT 1 while the sorted set is
compact and at a COUNT that reaches every bucket once it is not. There are no
unfilled transcript placeholders. The linked run validates the Redis
observations; the gate's network cases and Redis suite run Firn against them.

Redis 7.2.16's
[unit/scan.tcl](https://github.com/redis/redis/blob/7.2.16/tests/unit/scan.tcl)
contains these tests, which now pass in the gate:

- SSCAN with encoding intset
- SSCAN with encoding listpack
- SSCAN with encoding hashtable
- HSCAN with encoding listpack
- HSCAN with encoding hashtable
- ZSCAN with encoding listpack
- ZSCAN with encoding skiplist
- SSCAN with integer encoded object (issue #1345)
- SSCAN with PATTERN
- HSCAN with PATTERN
- ZSCAN with PATTERN
- ZSCAN scores: regression test for issue #2175
- SCAN regression test for issue #4906

They are registered in `tests/redis-suite/passing.tsv`, so the gate's ratchet
requires them to pass. The PATTERN ZSCAN case has no explicit expected-result assertion. The suite runner
already uses --ignore-encoding; its encoding assertions do not validate Firn's
storage representation. The hashtable and skiplist cases loop the cursor to
zero, so they now run through the bounded steps.
