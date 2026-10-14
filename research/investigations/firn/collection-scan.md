# Whole-collection scans

Investigation for board item `firn-scope-collection-scan`, 2026-10-13. The
selected direction is implemented on this worktree as proposed design changes;
compiler acceptance and Firn network results remain for CI. Redis reference replies
were recorded in the run linked below.

## Selected scope

SSCAN, HSCAN and ZSCAN return Firn's existing element order, with the entire
MATCH-filtered collection and cursor zero in one held statement. COUNT and
cursor text are validated as Redis 7.2.16 validates them, but do not bound or
partition this provisional traversal. Sets and hashes use the ordinary map's
whole-map visitor; sorted sets use their existing score/member order.

The owner selected this behavior instead of retaining Redis encoding history
and listpack order. It also applies above Redis's compact thresholds: one
complete held traversal returns every continuously present matching element
and terminates the iteration. The cost is work and reply storage proportional
to the whole collection, tracked by Whitefoot board item coord-wfbl-02-01.
There is no new direct bucket reader or partial result disguised as a completed
scan. Bounded continuation replaces this path when the standard library
supports it. This ruling supersedes the earlier proposal to introduce encoding
history and insertion-order representations for this task.

The proposed decision is in [enumeration](../../../design/firn/enumeration.md).
The [script restore decision](../../../design/firn/scripts.md) keeps logical
collection copying: cursor zero carries no continuation tied to backing, and
map order is not guaranteed across a rebuild. Adding bounded continuation
must revisit restore semantics. No pin, submodule or approval log changes.

## Reference and representation evidence

The reference is [Redis 7.2.16](https://github.com/redis/redis/tree/7.2.16).
The worktree base is `c6f6058fb17277d4d44d364a45efad1eae01a22b`; compiler pin
`wf-d75f318d5631` names Whitefoot source
`d75f318d56318834785322bde3a3656fb93116a6`. Interfaces and the specification
were inspected at that source revision. No compiler, build, test suite,
reference server or benchmark ran on this machine.

[`Value`](../../../firn/store/module.wfm) stores sets and hashes as ordinary
HashMap values. Sorted stores an ordered score/member map and a member/score
map. None retains Redis encoding history; sets and hashes retain no listpack
order. Encoding CONFIG values are remembered but do not affect storage, as
[the server documentation](../../../firn/README.md) states. kept_score only
normalizes negative zero. Every size still takes the same complete traversal;
only ZSCAN score text is classified from current cardinality and member lengths.

Redis's defaults are 512 integer set members, 128 listpack set members with
64-byte members, 512 hash fields with 64-byte fields/values, and 128 sorted-set
members with 64-byte members. The requested hash figure of 128 is not Redis's
default: [config.c](https://github.com/redis/redis/blob/7.2.16/src/config.c)
sets hash-max-listpack-entries to 512. Firn's universal traversal does not depend
on either figure and leaves existing encoding CONFIG behavior unchanged.

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
reproduce these histories or compact result order.

## Whitefoot resumable-scan gap

The pinned
[hash-map interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/collections/hash_map/module.wfm)
has hash_map_each for the whole map, in no promised order, and allows rebuilds
to change bucket positions. It has no resumable scan. The prelude map_scan and
map_scan_within accept ConcurrentHashMap, not the ordinary HashMap held by
Firn collections.

Minimal existing-API witness, a fragment intentionally not presented as a
working program and not compiled:

```whitefoot
alias HashMap = std::collections::hash_map::HashMap;

fn resume(map: &HashMap<u64, unit, 64>, cursor: u64, keys: &KeySet) -> next: u64 reads(map), writes(keys) {
  return map_scan::<unit>(map: map, cursor: cursor, count: 1_u64, keys: keys);
}
```

The map argument requires &ConcurrentHashMap<unit> but receives
&HashMap<u64, unit, 64>. No compiler diagnostic is claimed. The owner is raising
coord-wfbl-02-01 for ordinary-map resumable scanning; this session has no
wf-board tools and has not filed a duplicate request. The older direct-bucket
debt firn-bl-01-20 also remains; this change introduces no new bucket access.

The needed facility returns an owned continuation and lends keys/values to a
callback without retaining references across calls. Acceptance must check
coverage of continuously present entries across growth, reserve, rehash,
tombstone cleanup, unrelated deletions and colliding keys; duplicates are
allowed, and an unchanged finite map must terminate. A numeric offset into
repeated whole-map visitors does not provide this guarantee. Script snapshot
restoration must preserve the future cursor's semantic validity even when
logical copies rebuild backing.

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

Compact ZSCAN calls the shared append_double_text d2string interface; its
implementation and MIT-attributed helper chain are adopted from the companion
score-writer work in this same project and use Halo-wf's existing fpconv port.
The scores module gains its direct fpconv dependency. The old formatter is
retained independently as score_text_17g and exposed through score_bytes_17g
for large ZSCAN. Its public signature is:

```whitefoot
public fn score_bytes_17g(score: f64) -> text: pkg::bytes::Bytes pure doc "A double in C's %.17g text, without RESP framing.";
```

The other shared score_bytes interface is not a d2string interface: the
companion uses it for AOF's plain fpconv text. ZSCAN therefore must not call it
for compact formatting. This patch leaves the broader reply_score/AOF writer
migration to the companion; ZSCAN itself now has both required formatters.

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
There are fifteen tests: eight recorded transcript tests, two score-selection
boundary tests, and five independently
specified cases for cursor/options/lookup precedence, binary MATCH and pair
association, whole returns at count and byte-length boundaries, script retry
with an earlier write, and expired direct/EXEC/script reads. The expiry case
does not isolate lazy deletion from active expiry; it checks the externally
visible missing-key behavior. New cases could not be run against the old
revision under the machine rule; missing command dispatch would reject the
scan commands rather than returning their source-defined replies.
The score-selection cases cover 128 -> 129 -> 128 members and 64 -> 65 -> 64
maximum member bytes, including a 65-byte UTF-8 member with 33 characters.
MATCH hides the triggers; direct, EXEC and declared-key Lua probes must still
select the complete collection's formatter. There are no unfilled transcript
placeholders. The linked run validates the Redis observations, not the changed
Firn implementation. CI is driven by the owner; no passing Firn suite or
compiler result is claimed in this record.

Redis 7.2.16's
[unit/scan.tcl](https://github.com/redis/redis/blob/7.2.16/tests/unit/scan.tcl)
contains these predicted newly reachable passes:

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

These are candidates, not measured passes, and passing.tsv is unchanged. The
PATTERN ZSCAN case has no explicit expected-result assertion. The suite runner
already uses --ignore-encoding; its encoding assertions do not validate Firn's
storage representation. Full held traversal can reach the dictionary and
shrink-under-scan cases too, but supplies no bounded-work evidence.
