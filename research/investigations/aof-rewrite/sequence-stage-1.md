# Stage 1: dataset mutation sequencing

Implementation record, 2026-10-10. The work is uncommitted on
`claude/rewrite-scanlog`, above `13bc4939012e0d3065b7cf1d0ef7634395a0e92c`, for
[draft PR #42, reconciled-scan AOF rewrite](https://github.com/Ming-Research/Firn-wf/pull/42).
The owner's implementation request approves option A on `firn-q-rw-stamp`
(and the separate cut and limits cards); the [sequence node](../../../design/firn/aof-rewrite/sequence.md)
records the approved stage-1 decision. This record supersedes the earlier
investigation-only stopping point, not the [scan-log protocol](scan-log.md).

The worktree adds entry stamps, statement sequencing, exhaustion state and
`INFO persistence` observation. It does not implement scanning, capture,
images, rotation, emission, reserve enforcement or scan-abort handling.
The existing replay rewrite is unchanged. The new flag is for stage 2 to
consult; it does not disable the existing replay worker.

## Shared mutation contract

[store/sequence.wf](../../../firn/store/sequence.wf) implements one contract:

- `sequence_new` copies held Meta's counter/exhaustion into a local
  `WriteSequence`, initially untaken. All command parts of this atomic
  statement receive that same token.
- `sequence_change` lazily increments that token once. It saturates at
  `u64::MAX` and sets exhaustion as soon as the maximum is reached. Later
  service changes keep the saturated value and flag; no addition wraps.
- `sequence_stamp` is called only after an actual entry change; it takes
  the sequence and stamps the surviving entry through `sequence_entry`.
  The latter also supports a payload already borrowed by a match.
- `sequence_delete` checks for an entry, then takes the same sequence and
  removes it. Missing-key deletion takes none. Flush uses `sequence_change`
  directly, including an empty flush, as required by the owner's request.
- `sequence_commit` publishes a taken token to Meta before the same atomic
  statement releases its keys and Meta. The increment is local until this
  publication; other statements cannot observe an intermediate value.
  An unwritten token changes nothing. Loading takes no sequence and stamps
  surviving entries zero, including copies of loaded entries.

Startup initializes the counter and entry constructors to zero and the
exhaustion flag to false. No per-statement dirty-key collection is allocated.
AOF being disabled does not suppress this contract. Command bodies use their
existing actual-change results, including insertion/removal counts and
transfer results, rather than treating an AOF record count as a mutation.
Accepted value overwrites remain writes; refused conditional commands,
missing removals and unchanged operations such as duplicate SADD/ZADD do not.
LRU/LFU refresh and restoration retain their existing access-only path.

`run_exec` owns one token for all held command parts. `eval` owns one for
each outer script attempt, passing it through `answer_call`,
`ScriptCommands.call`, `script_command` and `held_run`. A real dataset
mutation sets the script's existing `held.wrote` state even if no effect
record was appended, so the attempt cannot be discarded and retried after
changing data. Done/error publication occurs before releasing the same
hold; a discarded read-only attempt has no taken token. The forwarding
signatures in command parts carry `writes(sequence)` with reads first.
Existing `commands -> store` and `scripting -> store` module edges cover
these references; no new edge or alias leakage is needed.

Lazy read cleanup uses `remove_expired`'s key-and-Meta hold. Active expiry
uses `expire_key`'s hold after confirming the queued deadline still matches.
Eviction takes a token only in the actual victim-removal hold. Held reads
remove expired entries through their shared token. No path unable to reach
Meta under the same dataset hold was found in implementation or independent
source review; no Whitefoot gap was filed.

## Tests and distinguishing failures

[tests/rewrite_sequence.rs](../../../tests/rewrite_sequence.rs) is included by
the existing network test target through `mod rewrite_sequence`. Every
positive delta is checked against the actual INFO counter, independently
of the command reply. Expected deltas follow the approved sequencing
contract; ordinary replies follow Redis 7.0.15's existing command behavior.
No test, compiler, reference server or gate was run for this worktree.

| Test (`rewrite_sequence_` prefix) | Evidence intended and missing-publication failure |
| --- | --- |
| `all_value_types_and_no_effects` | New and in-place string/list/set/hash/sorted-set writes each require exactly +1; reads, malformed/conditional refusals and no-ops require +0; MSET/MSETNX and multi-key deletion still require +1; both clears count. Omitting a command's sole mutation hook yields +0 instead of +1; eager counting fails the negative cases. The test runs with AOF both off and on, so an AOF-only hook fails. |
| `transfers_stores_and_expiry_metadata` | RENAME, COPY, LMOVE/RPOPLPUSH, SMOVE, set-algebra stores, expiry changes, PERSIST and GETDEL require +1; refused/same-key transfers, missing sources and already-empty stores require +0. Omitting participation entirely from a transfer/store/expiry path fails its delta; allocating separately for both changed transfer keys fails +1. |
| `exec_and_script_are_single_units` | One EXEC and one script write every value type, touch multiple keys and repeat writes. Each requires +1, including an error between writes and a script error after a write. A read/no-effect/error EXEC requires +0. Per-command counters fail +1 and losing the outer publication fails +0 versus +1, with AOF both off and on. |
| `reads_access_refreshes_and_read_only_retries` | GET, TOUCH and MGET under LRU/LFU and a long read-only script require +0. LFU frequency must increase only for the completed script's two reads, following the existing retry witness. A no-effect script and error before any write require +0. Eager token acquisition or treating access refreshes as dataset writes fails. The long script is intended to cross the existing interpreter budget; retry execution has not been observed in this worktree. |
| `active_expiry_and_stale_due_records` | SET and PEXPIRE require +1 separately; polling physical DBSIZE without looking up the key witnesses the active worker's one removal and requires another +1. Duplicate/stale due records must not produce a second increment. Missing active-removal publication gives +0, while counting stale queue work gives excess increments. |
| `held_lazy_expiry` | Loading a past deadline starts at zero. DBSIZE followed by GET in the same EXEC or script witnesses physical presence before lazy removal, then requires sequence 1. If active expiry already won, the test explicitly recognizes that reply and retries the witness, following the existing held-record helper. A missing lazy-removal hook gives zero and fails, rather than qualifying for retry. |
| `eviction_counts_actual_removals` | A three-key MSET requires +1; maxmemory eviction removes all three in separate atomic victim statements, requiring +3 and actual `evicted_keys:3`. Missing removal publication, counting candidate search or grouping distinct eviction statements produces the wrong delta. |
| `replay_starts_at_zero` | Legacy AOF replay includes every value type, repeated writes, expiry metadata and COPY in a transaction. Loaded sequence must be zero, the data must be present, and the first live mutation must produce one. Counting replay or failing to publish after replay violates these observations. |

**Observability limit:** these tests detect missing sequence participation,
not every independent per-key stamp store. In particular, removing only
`set value^.last_write = stamp` from `sequence_entry` would leave the INFO
counter correct and all these tests could pass. A transfer/EXEC/script with
another participating write can also mask omission of one key's stamp.
Source review covers those assignments in stage 1; independently observing
every entry stamp requires later capture behavior or a separately approved
internal test path. No DEBUG subcommand or invented INFO field was added.
The INFO field is Meta's real value, consistent with
[reported facts](../../../design/firn/reported-facts.md).

Counter exhaustion has no executable boundary evidence: the public protocol
cannot feasibly reach `u64::MAX` in a network case. Its guarded increment,
saturation, flag and continued service writes were inspected only.

## Memory and validation

The new `last_write: u64` is **eight logical bytes per live key**, or
8,000,000 logical bytes per million live keys. This is not a compiled stride
or heap delta. Entry padding/alignment, hash-map cell capacity and allocated
memory must be measured in CI with the pinned compiler before any footprint
claim. The local token and Meta fields are not a second per-key allocation.
No runtime, throughput or latency result is claimed.

No build, execution, test, lint, measurement, commit, push or CI dispatch was
performed, as requested. Compiler acceptance, effect-row exactness, the new
network cases, Redis ratchet, design lint/readiness and performance remain
unverified. The gate is still `make check` in CI on a subsequent push; there
is no CI result for these uncommitted edits. The first CI sample should be
the focused sequence cases before choosing any larger new experiment.
No existing suite was removed or weakened.

## Independent review

A separate GPT-6 Codex agent reviewed the complete implementation diff from
`13bc4939012e0d3065b7cf1d0ef7634395a0e92c` through the worktree and its new
files, plus the investigation on PR #42 against
`c1144b666f57b2bd5cf567f1686e7abd6e387003`. It read the requested outcome,
scan-log design, changed regions and consumers, pinned language rules,
project checklist, and the firn, AOF rewrite/sequence, reported-facts,
command-parts, scripts, transactions, append-only-file and memory-limit
nodes. Checks were read-only git/source inspection; no suite ran.

Finding F1 was fixed: the active-expiry test originally waited for an
unreported `INFO stats expired_keys` field. It now polls physical DBSIZE,
which does not expire the key itself, then checks the sequence delta. The
reviewer inspected that focused repair. No other concrete defect was found
within scope. Checklist A1, T1, T2, D1, G1, G2, G3, DC1 and DC2 passed source
inspection; C1, C2, T3 and DC4 remain unverified because of the execution and
observability limits above. R1 and DC3 were not applicable. No lint-derived
node/depth counts were obtained under the prohibition on local checks.

The reviewer also inspected this report, its scan-log link and the final
documentation clarifications without finding another defect. A subsequent
local test repair recreates the retry test's key after switching to LFU and
asserts its initial frequency is five: a retained LRU bit pattern could
otherwise already saturate the LFU counter and make the expected two-refresh
increment depend on wall time. This repair was checked against the existing
access implementation by inspection; no execution result is claimed.

## Changed files and delivery

- `firn/store/module.wfm`, `firn/store/sequence.wf`, `firn/store/store.wf`:
  representation, shared contract, zero initialization and expiry holds.
- `firn/commands/{strings,keys,lists,hashes,sets,sorted,ranges}.wf`:
  constructors, in-place changes, deletion, transfer and store publication.
- `firn/commands/{scan,object,eviction,server,dispatch}.wf`: expired-key
  removal in iteration/introspection, victim removal, clear and loading time.
- `firn/commands/{script,transaction}.wf`,
  `firn/scripting/{module.wfm,entry.wf}`: one outer token through held
  execution and the scripting interface, including retry/error behavior.
- `firn/commands/info.wf`, `firn/README.md`: actual sequence observation
  and its documented meaning.
- `tests/network.rs`, `tests/rewrite_sequence.rs`: existing target wiring
  and the eight network cases above.
- `design/firn/aof-rewrite/sequence.md`, `design/log.md`: approved stamp
  decision and the owner's approval record.
- `research/investigations/aof-rewrite/{scan-log,sequence-stage-1}.md`:
  dated implementation link and this evidence/call-site record.

`whitefoot.pin`, all submodule pins, `docs/todo.md` and `firn/modules.wfg`
are unchanged. No unrelated defect was deferred or declined. Work stops
at the requested uncommitted worktree delivery, pending CI validation.
The status board could be read, but this session exposes no ArtifactData
row-writing capability, so no board item or log was updated. This record
keeps the delivery and verification limits available without claiming a
published board report or a pushed implementation.

## Direct shared-helper call sites

This inventory is from the delivered source. `new`, `change`, `stamp`,
`entry`, `delete` and `commit` below abbreviate the corresponding
`sequence_*` helper names. Each listed number is a source line, not a
measurement or an independent execution result. Forwarding command parts
reuse the caller's token; they do not create another sequence.

| File | Function | Helper calls (line) |
| --- | --- | --- |
| `firn/commands/eviction.wf` | `evict_before` | `new` (352), `delete` (368), `commit` (380) |
| `firn/commands/hashes.wf` | `fresh_hash` | `stamp` (185) |
| `firn/commands/hashes.wf` | `run_hset` | `new` (206), `commit` (210) |
| `firn/commands/hashes.wf` | `run_hsetnx` | `new` (280), `commit` (286) |
| `firn/commands/hashes.wf` | `run_hdel` | `new` (483), `commit` (487) |
| `firn/commands/hashes.wf` | `run_hincrby` | `new` (769), `commit` (773) |
| `firn/commands/hashes.wf` | `run_hincrbyfloat` | `new` (884), `commit` (887) |
| `firn/commands/hashes.wf` | `hset_body` | `delete` (1378), `stamp` (1385) |
| `firn/commands/hashes.wf` | `hsetnx_body` | `delete` (1415), `stamp` (1422) |
| `firn/commands/hashes.wf` | `hdel_body` | `delete` (1510), `stamp` (1513) |
| `firn/commands/hashes.wf` | `hincrby_body` | `delete` (1632), `stamp` (1646), `stamp` (1651) |
| `firn/commands/hashes.wf` | `hincrbyfloat_body` | `delete` (1688), `stamp` (1705), `stamp` (1713) |
| `firn/commands/keys.wf` | `entry_kind` | `delete` (261) |
| `firn/commands/keys.wf` | `remove_live` | `delete` (282) |
| `firn/commands/keys.wf` | `run_del` | `new` (373), `commit` (394) |
| `firn/commands/keys.wf` | `run_exists` | `new` (421), `commit` (445) |
| `firn/commands/keys.wf` | `run_type` | `new` (459), `commit` (463) |
| `firn/commands/keys.wf` | `expire_body` | `delete` (726) |
| `firn/commands/keys.wf` | `run_expire` | `new` (788), `commit` (792) |
| `firn/commands/keys.wf` | `ttl_body` | `delete` (808) |
| `firn/commands/keys.wf` | `run_ttl` | `new` (855), `commit` (858) |
| `firn/commands/keys.wf` | `run_persist` | `new` (872), `commit` (875) |
| `firn/commands/keys.wf` | `rename_body` | `stamp` (1100) |
| `firn/commands/keys.wf` | `copy_body` | `stamp` (1216) |
| `firn/commands/keys.wf` | `run_rename` | `new` (1254), `commit` (1258) |
| `firn/commands/keys.wf` | `run_copy` | `new` (1287), `commit` (1291) |
| `firn/commands/keys.wf` | `persist_body` | `delete` (1337), `stamp` (1340) |
| `firn/commands/lists.wf` | `run_push` | `new` (222), `commit` (225) |
| `firn/commands/lists.wf` | `run_pop` | `new` (306), `commit` (309) |
| `firn/commands/lists.wf` | `run_lset` | `new` (609), `commit` (612) |
| `firn/commands/lists.wf` | `run_lrem` | `new` (720), `commit` (723) |
| `firn/commands/lists.wf` | `run_ltrim` | `new` (819), `commit` (822) |
| `firn/commands/lists.wf` | `run_linsert` | `new` (915), `commit` (918) |
| `firn/commands/lists.wf` | `place_element` | `entry` (1104), `stamp` (1134) |
| `firn/commands/lists.wf` | `run_lmove` | `new` (1171), `commit` (1174) |
| `firn/commands/lists.wf` | `push_body` | `delete` (1210), `stamp` (1229), `stamp` (1237) |
| `firn/commands/lists.wf` | `pop_body` | `delete` (1309), `stamp` (1322) |
| `firn/commands/lists.wf` | `lset_body` | `delete` (1493), `stamp` (1500) |
| `firn/commands/lists.wf` | `lrem_body` | `delete` (1559), `stamp` (1573) |
| `firn/commands/lists.wf` | `ltrim_body` | `delete` (1626), `stamp` (1636) |
| `firn/commands/lists.wf` | `linsert_body` | `delete` (1690), `stamp` (1697) |
| `firn/commands/lists.wf` | `lmove_body` | `delete` (1874), `delete` (1889), `delete` (1908), `stamp` (1921) |
| `firn/commands/object.wf` | `run_object` | `new` (133), `commit` (136) |
| `firn/commands/ranges.wf` | `zremrange_body` | `delete` (1013), `stamp` (1016) |
| `firn/commands/ranges.wf` | `run_zremrange` | `new` (1053), `commit` (1056) |
| `firn/commands/scan.wf` | `run_scan` | `new` (385), `commit` (387) |
| `firn/commands/scan.wf` | `run_randomkey` | `new` (402), `commit` (404) |
| `firn/commands/script.wf` | `script_get` | `delete` (264) |
| `firn/commands/script.wf` | `script_hget` | `delete` (2195) |
| `firn/commands/script.wf` | `script_hmget` | `delete` (2214) |
| `firn/commands/script.wf` | `script_hfield` | `delete` (2249) |
| `firn/commands/script.wf` | `script_hlen` | `delete` (2267) |
| `firn/commands/script.wf` | `script_hall` | `delete` (2285) |
| `firn/commands/script.wf` | `script_hrandfield` | `delete` (2353) |
| `firn/commands/script.wf` | `script_lrange` | `delete` (2399) |
| `firn/commands/script.wf` | `script_llen` | `delete` (2415) |
| `firn/commands/script.wf` | `script_lindex` | `delete` (2431) |
| `firn/commands/script.wf` | `script_lpos` | `delete` (2510) |
| `firn/commands/script.wf` | `script_scard` | `delete` (2586) |
| `firn/commands/script.wf` | `script_smembers` | `delete` (2601) |
| `firn/commands/script.wf` | `script_sismember` | `delete` (2617) |
| `firn/commands/script.wf` | `script_srandmember` | `delete` (2635) |
| `firn/commands/script.wf` | `script_zcard` | `delete` (2741) |
| `firn/commands/script.wf` | `script_zscore` | `delete` (2764) |
| `firn/commands/script.wf` | `script_zmscore` | `delete` (2785) |
| `firn/commands/script.wf` | `script_zrank` | `delete` (2805) |
| `firn/commands/script.wf` | `script_zrange` | `delete` (2827) |
| `firn/commands/script.wf` | `script_zcount` | `delete` (2849) |
| `firn/commands/server.wf` | `flush_body` | `change` (168) |
| `firn/commands/server.wf` | `run_flush` | `new` (188), `commit` (190) |
| `firn/commands/sets.wf` | `run_sadd` | `new` (156), `commit` (159) |
| `firn/commands/sets.wf` | `run_srem` | `new` (224), `commit` (227) |
| `firn/commands/sets.wf` | `run_spop` | `new` (338), `commit` (341) |
| `firn/commands/sets.wf` | `place_member` | `entry` (838), `stamp` (869) |
| `firn/commands/sets.wf` | `run_smove` | `new` (879), `commit` (882) |
| `firn/commands/sets.wf` | `survey` | `delete` (977) |
| `firn/commands/sets.wf` | `store_members` | `stamp` (1127) |
| `firn/commands/sets.wf` | `run_combine` | `new` (1156), `commit` (1159) |
| `firn/commands/sets.wf` | `run_sintercard` | `new` (1174), `commit` (1178) |
| `firn/commands/sets.wf` | `sadd_body` | `delete` (1206), `stamp` (1219), `stamp` (1224) |
| `firn/commands/sets.wf` | `srem_body` | `delete` (1264), `stamp` (1267) |
| `firn/commands/sets.wf` | `spop_body` | `delete` (1329), `stamp` (1336) |
| `firn/commands/sets.wf` | `smove_body` | `delete` (1622), `delete` (1632), `delete` (1665), `stamp` (1683) |
| `firn/commands/sets.wf` | `set_combine_body` | `delete` (1781), `delete` (1790) |
| `firn/commands/sorted.wf` | `zadd_body` | `delete` (428), `stamp` (445), `stamp` (452) |
| `firn/commands/sorted.wf` | `run_zadd` | `new` (505), `commit` (508) |
| `firn/commands/sorted.wf` | `zrem_body` | `delete` (592), `stamp` (595) |
| `firn/commands/sorted.wf` | `run_zrem` | `new` (630), `commit` (633) |
| `firn/commands/sorted.wf` | `zpop_body` | `delete` (824), `stamp` (831) |
| `firn/commands/sorted.wf` | `run_zpop` | `new` (874), `commit` (877) |
| `firn/commands/strings.wf` | `put_text` | `stamp` (252) |
| `firn/commands/strings.wf` | `hold_expiry` | `stamp` (378) |
| `firn/commands/strings.wf` | `set_key` | `new` (466), `commit` (476) |
| `firn/commands/strings.wf` | `run_getdel` | `new` (700), `commit` (704) |
| `firn/commands/strings.wf` | `run_getex` | `new` (726), `commit` (729) |
| `firn/commands/strings.wf` | `run_mset` | `new` (784), `commit` (802) |
| `firn/commands/strings.wf` | `run_incr` | `new` (896), `commit` (901) |
| `firn/commands/strings.wf` | `incr_body` | `stamp` (931) |
| `firn/commands/strings.wf` | `run_append` | `new` (1268), `commit` (1272) |
| `firn/commands/strings.wf` | `run_setrange` | `new` (1295), `commit` (1299) |
| `firn/commands/strings.wf` | `run_strlen` | `new` (1315), `commit` (1318) |
| `firn/commands/strings.wf` | `run_getrange` | `new` (1338), `commit` (1342) |
| `firn/commands/strings.wf` | `run_mget` | `new` (1368), `commit` (1389) |
| `firn/commands/strings.wf` | `run_msetnx` | `new` (1420), `commit` (1462) |
| `firn/commands/strings.wf` | `run_incrbyfloat` | `new` (1563), `commit` (1566) |
| `firn/commands/strings.wf` | `append_body` | `stamp` (1603) |
| `firn/commands/strings.wf` | `setrange_body` | `stamp` (1651) |
| `firn/commands/strings.wf` | `getdel_body` | `delete` (1662) |
| `firn/commands/strings.wf` | `incrbyfloat_body` | `delete` (1746), `stamp` (1768) |
| `firn/commands/strings.wf` | `getex_body` | `delete` (1858) |
| `firn/commands/transaction.wf` | `run_exec` | `new` (256), `commit` (289) |
| `firn/scripting/entry.wf` | `eval` | `new` (435), `commit` (495) |
| `firn/store/sequence.wf` | `sequence_stamp` | `change` (28), `entry` (31) |
| `firn/store/sequence.wf` | `sequence_delete` | `change` (50) |
| `firn/store/sequence.wf` | `sequence_entry` | `change` (67) |
| `firn/store/store.wf` | `expire_key` | `new` (144), `delete` (160), `commit` (165) |
| `firn/store/store.wf` | `remove_expired` | `new` (175), `delete` (186), `commit` (191) |
