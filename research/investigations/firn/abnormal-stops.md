# Abnormal stops at Firn's pinned Whitefoot version

Read-only source audit, 2026-10-13, for board item `firn-safety-showcase`,
row `final`. Firn revision: `b83b67fa0d7b463dbafe5429b013efc04e39076a`;
all **48** tracked `firn/**/*.wf` files are in scope. The pin is
`wf-af7665be672d`, resolved in the local Whitefoot Git object database to
[`af7665be672d29912d878dd5da8c3932bb6f296c`][wf], specification **v0.146**.
This follows [the kit's pinned-language procedure](../../../whitefoot-kit/downstream.md#reading-the-language).
Firn locations below are relative to the repository root at that revision;
Redis citations use the fetched **7.2.16** source (`src/version.h:1`),
cross-checked with upstream. They use physical source lines, not the 7.0.15
references in older Firn comments. No compiler, build, test or fault-injection run was performed.

## 1. What the specification guarantees

**Checker acceptance does not guarantee continued service, termination, or
survival of resource exhaustion.** SCOPE-3 conditions absence of undefined
behavior on the trusted base and explicitly excludes resource availability.
In particular, calling an allocation failure a *Whitefoot trap* is incorrect:
STOR-8 explicitly says allocation never traps and never returns an allocation
failure value. Exhaustion terminates from outside the source model.
([SCOPE-3][scope], [STOR-8][storage], [ERR-4][errors].)

| Event | Pinned language classification | Exit status / signal guaranteed by the spec |
| --- | --- | --- |
| Heap exhausted, or requested allocation size cannot be represented/served | External-resource termination; no `Result`, source trap, unwinding or cleanup guarantee. Runtime allocation-size arithmetic is checked; oversized requests become exhaustion, not wrapped allocations. | None. [SCOPE-3][scope], [STOR-8, STOR-3, STOR-6][storage], [OP-9][operations] |
| Stack exhausted by deep recursion, large frames, or recursive release | Not excluded by acceptance. Frame representability is checked, available stack capacity is not. Unmarked recursion has no tail-transfer guarantee. | None. [STOR-6][storage], [FN-10][functions], [SCOPE-3][scope] |
| Plain integer `+ - *` would overflow | Statically excluded at every reachable exact operation; unproved domain rejects compilation. No synthesized runtime check/trap. `+checked` etc. instead return `Err(Overflow)`; explicit wrap/saturating modes have their stated total semantics. | No runtime failure for an accepted exact operation, conditional on the TCB. [OP-2][operations] |
| Out-of-bounds index | Bounds must be proved; otherwise compilation rejects. Accepted indexing has no runtime check. Map indexing selects an `Option` entry for every key, not an array-bound failure. | No source trap. [OP-4][operations], [SHARE-1][sharing] |
| Integer division/remainder by zero, or signed minimum divided/remaindered by −1 | Plain `/ %` require a static domain proof; checked forms return `DivideByZero` or `DivOverflow`. Floating division has separate IEEE semantics. | No source trap. [OP-2, OP-3][operations] |
| Explicit abort/panic/unchecked trap | No writer-reachable construct or effect. `.trap` arithmetic spellings do not derive. `exit_status(code:)` constructs an ordinary return value; it does not immediately kill other contexts. | Returning that value from the entry reports its `u8` code; not an abort. [EFF-4][effects], [ERR-1][errors], [PRE-2][host], [PROG-3][programs], [WAIT-3][waiting] |
| Host I/O failure | Contracted failures are values (`IoError`, or `ReadStop::ReadFailed`; EOF is `ReadEnd`). Nonzero host error codes are errno on POSIX; code zero denotes a synthesized condition. Resource/startup failures outside that contract remain outside the source guarantee. | No automatic process exit for a returned I/O error. [ERR-4][errors], [PRE-2][host] |
| Context waits forever | Possible: false guard, join dependency, or host outcome that never arrives. Progress is conditional. When every remaining context waits on guards/contexts and no host operation is outstanding, the implementation **may** stop with a report; that is not a program outcome. | No required exit or signal, no required deadlock detection. [WAIT-2, WAIT-3][waiting], [SHARE-3][sharing] |

The pinned implementation is narrower than the language allowance: its POSIX
stack guard handler attempts to write `{"resource":"stack"}` then calls
`abort()`; other faults restore their host signal disposition
([wf_floor.c:152–191][floor]). Emitted heap-failure code similarly attempts
`{"resource":"heap"}` and calls `abort()`
([emitter.rs:504–510][heap-emitter], [resource_records.rs:187–210][heap-abort]).
On POSIX that is SIGABRT, **not a specified Whitefoot exit code**; an OS kill
can precede the handler. Even diagnostic output can fail: the stack writer
returns on an unsuccessful write ([wf_floor.c:127–147][floor-write]). The
[pinned README's stack record claim][wf-readme] must be read as implementation
behavior, not a SCOPE-3 guarantee. The [resource-exhaustion design][floor-design]
explicitly leaves record bytes and status outside the language.

## 2. Deliberate Firn exits and loss of service

There are **zero explicit process abort/panic/trap calls**, **three nonzero
entry-return sites**, **one signal-default restoration site**, and **one
additional error-driven service-stop path that returns zero**. This is a
source-site count, not a count of possible machine failure instructions.
`exit_status` resolves to the standard process function
(`firn/server/server.wf:36`); all four entry returns are at
`:1000`, `:1050`, `:1147`, `:1149`.

Classification: **(a)** intended fatal decision matching Redis, **(b)**
intended fatal decision differing from Redis, **(c)** avoidable stop.
“Matching” compares whether service stops for that condition, not the exit
number or diagnostic; those differences are stated explicitly.

| Firn site | Trigger and result | Redis 7.2.16; classification |
| --- | --- | --- |
| `firn/server/server.wf:1000` | Invalid invocation; entry returns **1** after closing directory handles. | Invalid configuration exits **1** ([config.c:638–645][redis-config]); **(a)** for comparable supported options, not a claim that both CLIs accept the same syntax. |
| `firn/server/server.wf:1050` | `aof_start` returns `Err`; entry returns **4** before opening the network listener. Detailed causes below. | AOF loading failure exits **1** ([server.c:6774–6776][redis-load]); mostly **(a)**, with Firn-specific limits **(b)** below. |
| `firn/server/server.wf:1149` | `stop_listen` error (`:1067–1068`) or TCP listen error (`:1123–1124`); after shutdown/join paths, entry returns **3**. | TCP listener initialization failure exits **1** ([server.c:2812–2813][redis-listen]): **(a)**. Refusing service without Whitefoot's stop-listener capability: **(b)**; no equivalent Redis capability failure was established. |
| `firn/server/server.wf:514` | Closing the stop listener restores host defaults. After a first observed stop signal, a later SIGINT/SIGTERM can terminate immediately. The close also runs after a `stop_next` error (`:511`). | Second SIGINT during shutdown exits **1**; SIGTERM remains handled ([server.c:6493–6522][redis-signals]). **(b)** for the deliberate second-signal policy; unexpected loss of interception after a non-cancellation wait error is **(c)**. See [the existing decision](../../../design/firn/orderly-stop.md). |
| `firn/server/server.wf:1116` → `:1147` | **Any** `tcp_accept` error breaks acceptance, including a recoverable resource/I/O error; listener closes, clients drain, and entry can return **0**. Pending clients may instead keep the drain waiting (`:1128`). | The accept handler retries selected errors or returns to the event loop ([socket.c:311–330][redis-accept]); **(c)**. Cancellation after an intentional shutdown is the normal exception. |

The status-4 path has these trigger sites; propagation/cleanup sites are not
counted as additional exits:

| Firn trigger site | Fatal condition | Redis comparison / class |
| --- | --- | --- |
| `firn/persistence/startup.wf:305,307,332` | Cannot create/open the AOF directory for writing or open its read half. | Directory startup refusal, [aof.c:723–727][redis-dir]; **(a)**. |
| `firn/persistence/startup.wf:225` (`manifest.wf:238–310`) | Manifest cannot be read or parsed. | Manifest open/read/format failure, [aof.c:263–267][redis-manifest-open], [aof.c:378–383][redis-manifest-bad]; **(a)** for shared malformed/input-error cases. |
| `firn/persistence/startup.wf:62,76,96,104` → `:230` | Old-file upgrade cannot probe, persist its manifest, or move its base file. | Upgrade failure, [aof.c:619–657][redis-upgrade]; **(a)** for corresponding I/O failures. |
| `firn/persistence/startup.wf:119,141` → `:235`; `persistence.wf:301,220,286` | Listed file missing/inaccessible; open/read failure; malformed request during replay. | Missing/read/format failure, [aof.c:1596–1639][redis-replay], [aof.c:1689–1692][redis-missing]; **(a)** for common invalid inputs. |
| `firn/persistence/startup.wf:146,152` → `:235`; `:177–194` | An earlier active file needs truncation, or opening/truncating the last file fails. | Earlier-file truncation refused; final-file repair can fail, [aof.c:1713–1721][redis-truncated], [aof.c:1608–1633][redis-replay]; **(a)**. |
| `firn/persistence/startup.wf:249,294,297` | Initial base replacement fails; manifest is not durable; active incremental file cannot open for append. | Startup base/open/manifest failure, [aof.c:730–760][redis-start]; **(a)** for corresponding failures. |
| `firn/persistence/persistence.wf:188` | Replay retains an unfinished command/transaction until its input reaches **2 GiB**, and cannot grow (`protocol/protocol.wf:632–641`). | Loader reads arguments separately ([aof.c:1503–1537][redis-args]); **(b)**, no equivalent whole-transaction window ceiling there. |
| `firn/protocol/protocol.wf:333` → `persistence/persistence.wf:286` | Shared network parser rejects a bulk argument longer than **512 MiB**, including in AOF replay. | AOF loader has no corresponding network bulk-length check ([aof.c:1509–1537][redis-args]); **(b)**. |

Running AOF write/fsync failures are **not** process-abort sites in Firn:
`firn/persistence/rewrite.wf:285–360` retains append failures and latches sync
errors; `persistence.wf:85–86` continues until shutdown. Redis's everysec
failure path also retains work; its **always** mode has fatal paths
([aof.c:1198–1205][redis-write], [aof.c:1247–1253][redis-sync]). Do not infer
Firn has an always-mode abort from Redis's code. `rewrite_abort` only marks
capture state (`firn/store/rewrite-capture.wf:10–26`); `EXECABORT` is a client
reply (`firn/commands/transaction.wf:259`), not process termination.

## 3. Resource-failure sites

Allocation and context/frame resources require a separate inventory: a source
call may allocate inside a helper or linked implementation; a function entry
or compiler-derived release may exhaust the stack. SCOPE-3 permits these
external stops even though the source contains no trap. Therefore no finite
list of *trap calls* can honestly claim to enumerate all machine crash sites.
The inventory below counts direct resource operations and names indirect
categories; it does not count every caller of the same allocating helper.

Each table entry below cites a physical Firn source line. For every positive
allocation/growth site, the trigger is inability to serve that operation's
allocation, including unrepresentable requested byte size: [STOR-8][storage]
and [OP-9][operations]. Key-set/map/shared storage uses [SHARE-1][sharing]
and [PRE-1][prelude]; spawn resources use [WAIT-3][waiting].

The allocation entries inherit **(a), at the allocator-policy level only**:
Redis's ordinary `zmalloc` calls its OOM handler, which panics
([zmalloc.c:144–147][redis-alloc], [server.c:6842–6847][redis-oom]). This does
**not** establish a matching Redis allocation at every Firn line, nor identical
resource budgets; exact per-operation equivalence remains unverified.
Key-set, concurrent-map, cancellation and spawn bookkeeping have no direct
one-to-one Redis counterpart; resource stops specific to that machinery are
**(b)**. No resource-exhaustion site is established as **(c)** merely because
it allocates. The 37 literal-zero boxed constructors below are **not heap
failure sites**; they are retained in the inventory to make its count auditable.

One site is one written operation token in a body, excluding declarations,
aliases and documentation; a loop counts once. Total: **181 written sites** in **31 files**. Per-operation counts: `box_array_filled` 5, `box_ring_new` 1, `box_slots_new` 92, `grow` 25, `key_set_insert` 18, `key_set_new` 25, `map_scan` 5, `map_scan_within` 1, `shared_map_new` 1, `shared_new` 3, `spawn` 5.

| File | Operation: exact source lines |
|---|---|
| `firn/bytes/bytes.wf` | `box_slots_new`: 258, 268† |
| `firn/commands/connection.wf` | `box_slots_new`: 390 |
| `firn/commands/eviction.wf` | `box_slots_new`: 207, 217, 244†; `grow`: 273; `key_set_new`: 230; `map_scan`: 231 |
| `firn/commands/info.wf` | `box_slots_new`: 245, 483†; `grow`: 131 |
| `firn/commands/keys.wf` | `grow`: 315; `key_set_insert`: 343, 427, 998, 999; `key_set_new`: 338, 422, 1179, 1216 |
| `firn/commands/lists.wf` | `box_slots_new`: 1098†; `grow`: 1007; `key_set_new`: 1241 |
| `firn/commands/scan.wf` | `box_array_filled`: 188; `key_set_insert`: 232, 237, 321; `key_set_new`: 208, 293, 301, 332, 348; `map_scan`: 209, 304, 349, 353 |
| `firn/commands/script.wf` | `box_slots_new`: 367, 3352†; `key_set_insert`: 355; `key_set_new`: 347, 1547, 2664, 2692, 3390, 3614 |
| `firn/commands/sets.wf` | `box_slots_new`: 1840†, 1975†; `key_set_insert`: 945, 957, 970, 1665, 1666; `key_set_new`: 913, 942 |
| `firn/commands/strings.wf` | `box_slots_new`: 1109; `grow`: 1140; `key_set_insert`: 813, 1473, 1538; `key_set_new`: 804, 1467, 1529 |
| `firn/commands/transaction.wf` | `box_slots_new`: 60, 61, 62, 242, 243; `grow`: 103, 122 |
| `firn/extended/extended.wf` | `box_slots_new`: 88†, 90, 1004, 1062 |
| `firn/persistence/manifest.wf` | `box_slots_new`: 45†, 75, 158, 172†, 245, 285, 424; `grow`: 63 |
| `firn/persistence/persistence.wf` | `box_slots_new`: 63†, 65, 174 |
| `firn/persistence/rewrite-boundary.wf` | `box_slots_new`: 43†, 121; `grow`: 152 |
| `firn/persistence/rewrite-dataset.wf` | `box_array_filled`: 111; `box_slots_new`: 96; `key_set_new`: 52; `map_scan_within`: 60 |
| `firn/persistence/rewrite.wf` | `box_ring_new`: 594; `box_slots_new`: 42, 295, 936; `spawn`: 737 |
| `firn/persistence/startup.wf` | `box_slots_new`: 245†, 269†, 309 |
| `firn/protocol/protocol.wf` | `box_slots_new`: 622, 684, 691, 692, 693, 700†; `grow`: 217, 641, 753, 774, 796 |
| `firn/script_pool/pool.wf` | `box_slots_new`: 2†, 3† |
| `firn/scripting/entry.wf` | `box_slots_new`: 45†, 89†, 184, 236, 276, 277, 315, 316, 409†, 463†, 489†, 496, 782†; `grow`: 102, 124, 171; `key_set_insert`: 436; `key_set_new`: 430 |
| `firn/scripting/environment.wf` | `box_slots_new`: 87, 228†, 294†, 370; `grow`: 76 |
| `firn/scripting/errors.wf` | `box_slots_new`: 112† |
| `firn/scripting/host.wf` | `box_slots_new`: 244†, 310†; `grow`: 162 |
| `firn/scripting/numbers.wf` | `box_slots_new`: 92, 100 |
| `firn/scripting/replies.wf` | `box_slots_new`: 15†, 89†, 96†, 100†, 146, 147; `grow`: 135 |
| `firn/server/server.wf` | `box_slots_new`: 299, 470, 604†, 613, 813, 931†; `spawn`: 1040, 1057, 1065, 1113 |
| `firn/store/attempt.wf` | `box_array_filled`: 217; `box_slots_new`: 164†, 261†; `grow`: 179 |
| `firn/store/rewrite-capture.wf` | `box_array_filled`: 182; `box_slots_new`: 105, 263; `grow`: 151 |
| `firn/store/rewrite-image.wf` | `box_slots_new`: 184 |
| `firn/store/store.wf` | `box_array_filled`: 496; `box_slots_new`: 57, 60, 476†; `grow`: 246, 283, 493; `key_set_insert`: 484; `key_set_new`: 475; `shared_map_new`: 52; `shared_new`: 64, 72, 74 |

**37** boxed constructor sites (marked †) pass literal zero capacity/count and therefore allocate no heap at that site under STOR-1. The remaining boxed constructors may allocate when their dynamic capacity/count is positive. Growth may reuse storage or allocate. Shared and KeySet constructors do not share the Box empty-header exclusion. Calls to `place_back`, `insert_at`, `append`, `split_off`, array construction, bare struct/enum construction, or `key_set_read_key` do not create implicit automatic growing heap storage; their capacity/domain obligations are checked statically.

### Allocation-bearing wrapper calls (separate inventory)

These calls are not added to the builtin-site count. Standard-library allocating methods are ordinary Whitefoot wrappers, and project copy/new helpers eventually reach the direct sites above. The table lists explicit calls of known immediate storage constructors/growth and materializing/copy helpers, rather than every transitively allocating command/reply wrapper.

Pinned [hash-map contracts][hash-contract] say `hash_map_new` materializes buckets and `hash_map_put` may rebuild at three-quarter occupied/vacated load below ceiling; HashMapFull reports logical capacity, not allocator failure. `deque_new` and `deque_rebase` [construct backing][deque-impl]; deque push methods explicitly never allocate and are omitted. `priority_queue_push` [may grow below its ceiling][priority-impl]; `priority_queue_new` starts at zero capacity and is omitted as nonallocating. `ordered_map_put` [may allocate nodes][ordered-impl]; `ordered_map_new` explicitly allocates no node and is omitted.

`bytes_new` keeps at most 24 bytes inline (`firn/bytes/bytes.wf:63–77`); `bytes_copy` and `copy_key` follow that representation. `bytes_boxed` allocates for a nonempty Short value via `copy_bytes` but moves an existing Long backing without allocation. `copy_bytes` allocates positive lengths up to 2^62 and otherwise returns the empty header. `make_key` delegates to `bytes_new`. `copy_value` recursively copies collection contents, materializes hashes/rings/ordered nodes and copies strings (`firn/store/attempt.wf:100–160`). These are conditional allocation sites, not guaranteed allocation on every call.

| File | Wrapper: exact source lines |
|---|---|
| `firn/bytes/bytes.wf` | `bytes_boxed`: 59; `bytes_copy`: 46; `bytes_new`: 40, 86, 92, 314; `copy_bytes`: 76, 228 |
| `firn/commands/connection.wf` | `bytes_boxed`: 395; `bytes_copy`: 361, 394; `bytes_new`: 178 |
| `firn/commands/hashes.wf` | `bytes_copy`: 857, 884, 1760; `hash_map_new`: 179, 1697, 1757; `hash_map_put`: 127, 764, 885, 1701, 1761 |
| `firn/commands/keys.wf` | `copy_value`: 964; `make_key`: 186 |
| `firn/commands/lists.wf` | `bytes_boxed`: 518; `bytes_new`: 517; `copy_bytes`: 160; `deque_new`: 122, 1202, 1309; `deque_rebase`: 126 |
| `firn/commands/ranges.wf` | `bytes_new`: 116, 142, 152, 153; `copy_key`: 311 |
| `firn/commands/script.wf` | `bytes_new`: 554, 2481; `hash_map_new`: 3637 |
| `firn/commands/server.wf` | `bytes_copy`: 384, 395, 404; `bytes_new`: 1097 |
| `firn/commands/sets.wf` | `copy_key`: 286, 1101; `hash_map_new`: 658, 899, 1174, 1207, 1291, 1976; `hash_map_put`: 102, 669, 871, 900, 1102 |
| `firn/commands/sorted.wf` | `copy_key`: 160, 686, 716; `hash_map_new`: 446; `hash_map_put`: 163; `ordered_map_put`: 162, 253 |
| `firn/commands/strings.wf` | `bytes_copy`: 1646, 1914; `bytes_new`: 458, 898, 1746 |
| `firn/extended/extended.wf` | `bytes_new`: 1066, 1170 |
| `firn/persistence/manifest.wf` | `aof_copy`: 51; `bytes_boxed`: 623; `input_new`: 244 |
| `firn/persistence/persistence.wf` | `client_new`: 170; `input_new`: 175 |
| `firn/persistence/rewrite-boundary.wf` | `input_new`: 120 |
| `firn/persistence/rewrite-journal.wf` | `input_new`: 288 |
| `firn/persistence/rewrite.wf` | `aof_copy`: 28, 48, 123, 453, 942 |
| `firn/persistence/startup.wf` | `aof_copy`: 87, 275, 315 |
| `firn/protocol/protocol.wf` | `bytes_copy`: 720, 721 |
| `firn/scores/write.wf` | `bytes_new`: 225 |
| `firn/scripting/entry.wf` | `access_journal_new`: 492; `attempt_due_new`: 570; `attempt_undo_new`: 568; `copy_source`: 116, 421, 434, 796 |
| `firn/scripting/environment.wf` | `copy_source`: 174 |
| `firn/server/server.wf` | `bytes_new`: 1010, 1011, 1014; `client_new`: 292; `input_new`: 300, 585; `keyspace_new`: 1021; `settings_copy`: 1100; `transaction_new`: 301 |
| `firn/store/attempt.wf` | `bytes_boxed`: 53; `bytes_copy`: 77, 104; `bytes_new`: 52; `copy_key`: 68, 76, 85, 94; `copy_value`: 185; `deque_new`: 109, 111; `hash_map_new`: 120, 122, 132, 134, 149, 151; `hash_map_put`: 70, 79, 96; `ordered_map_put`: 88 |
| `firn/store/rewrite-capture.wf` | `copy_bytes`: 122 |
| `firn/store/rewrite-image.wf` | `bytes_new`: 183 |
| `firn/store/store.wf` | `bytes_new`: 237; `priority_queue_push`: 258 |

Higher-order collection calls can allocate through callbacks: deep-copy
traversals at `firn/store/attempt.wf:114,126,138,146,155`, and AOF-emitting
traversals at `firn/store/rewrite-image.wf:86,90,94`, reach allocation/growth
sites already listed. They are not extra allocator tokens.

### Halo wrapper calls (unexpanded external dependency)

These calls can allocate through Halo's engine, heap, compiler, or interpreter execution. They are external ordinary definitions, not Whitefoot builtin rows, and not added to the direct-site count. The table is a conservative set of allocation-bearing embedding/heap entry points; it does not count each internal Halo allocation. Halo's own logical Lua-memory-limit outcome does not make physical Whitefoot heap exhaustion catchable.

| File | Halo entry point: exact source lines |
|---|---|
| `firn/scripting/entry.wf` | `embed::compile`: 161; `embed::complete_call`: 375; `embed::fail_call`: 370; `embed::new_engine`: 86; `embed::resume`: 511, 580; `embed::set_pcall_error_field`: 87; `embed::start`: 506, 573 |
| `firn/scripting/environment.wf` | `embed::intern`: 141, 176; `embed::new_table`: 102, 140, 146, 166, 167; `embed::set_global`: 64, 168, 169; `embed::table_set`: 99, 119, 143, 186; `heap::new_table`: 11 |
| `firn/scripting/host.wf` | `heap::intern`: 38; `heap::new_table`: 144; `heap::table_set`: 29 |
| `firn/scripting/replies.wf` | `heap::new_table`: 486, 506 |

### Implicit/runtime resources and boundaries of exhaustiveness

1. Concurrent-map entry materialization: assigning a surviving value to a previously absent key may allocate entry/key/bucket storage without a named source allocator. The central project replacement path is `firn/store/sequence.wf:74` (`set slot^ = move replacement`); rollback restoration additionally writes entries at `firn/store/attempt.wf:208`. Whether either allocates depends on the actual map-entry backing and membership. Entry-bound atomic statements, whole-map references and Entries references also require runtime hold/entry-selection machinery whose temporary allocation is not specified by a finite source allocation count. Consult the pinned lowering/runtime when judging physical failure sites; no language claim says that merely acquiring a missing-key target allocates.
2. Every activation, inline array/aggregate and recursion chain consumes frame/stack resources; copying a copy type copies frame-resident value representation, without hidden heap promotion. Therefore arbitrary call sites can face stack exhaustion. A finite heap-site table cannot enumerate all allowed resource termination locations.
3. Spawn and automatic parallel actualization can need context/runtime-start resources. No source `spawn` count covers compiler-created parallel work or invocation startup. Firn startup, accepted sockets and file/host handle calls may need OS/runtime resources even when their ordinary IO refusal path returns a Result. This inventory does not claim every host call physically allocates or promise catchable resource failure.
4. Cancellation constructors create/retain shared backing under [PRE-2][host]. The **seven** new-state calls below are separate from the 181 direct sites; ordinary shares/watches and closes are not fresh-state constructors. Physical allocation policy belongs to the linked implementation.

| File | New-state constructor: exact source lines |
| --- | --- |
| `firn/commands/server.wf` | `cancel_source`: 1105 |
| `firn/persistence/persistence.wf` | `cancel_never`: 71 |
| `firn/persistence/rewrite.wf` | `cancel_never`: 527 |
| `firn/scripting/entry.wf` | `cancel_never`: 444 |
| `firn/server/server.wf` | `cancel_never`: 74; `cancel_source`: 1028 |
| `firn/store/store.wf` | `cancel_source`: 70 |



## 4. Trusted base and counting C

| Trusted component | What the checker does not establish |
| --- | --- |
| Whitefoot checker and compiler, including lowering and release generation | That their implementation is sound and preserves the accepted semantics; [SCOPE-3][scope], [STOR-3, STOR-6][storage]. |
| Linked prelude/host definitions, C runtime and allocator | That implementations honor declared values, effects, ownership, contracts and call lifetimes. Includes shared maps, scheduler/completion machinery, signal handling and allocation; [PRE-1, PRE-2][host], [runtime.rs:1–85][runtime]. |
| LLVM, clang, linker and libc | Correct optimization, code generation, ABI, allocation, thread and signal implementations; [pinned README:52–56][wf-readme]. Source proofs are not proofs of these tools. |
| OS and execution environment | System-call semantics, scheduling and resource availability; the OS can refuse resources or terminate the process; [SCOPE-3][scope], [PRE-2][host]. |

The pinned repository **does contain a historical count**: the I/O-model
investigation records **23,039 physical lines of shipped runtime C across
29 units**, excluding its test harness/default probe, with a seven-file
subtotal of **8,083** ([PARK-ON-MISS.md:1174–1194][c-census]). This is not a
count of the current pin's linked Firn runtime: that record includes the old
parallel and writer schedulers which the current inventory replaces with
`sched/core.c` and related units ([runtime.rs:77][runtime]). An older compute
comparison separately records **947** lines versus **659**, for two explicitly
named historical revisions ([PRIOR-BUNDLE.md:52–58][compute-census]); those
are not whole-TCB counts either.

The measured census at the pin (file list, method, line counts and
exclusions) is owned by [docs/safety.md](../../../docs/safety.md#trusted-base-and-source-sizes);
this record does not repeat the numbers. Do not substitute all C files in the
repository for it: that includes probes, tests and other platforms.
`concurrent_map.c` is included by `keyed_table.c`, so a translation-unit-only
count misses it ([runtime.rs:78–85][runtime]).

## 5. Gaps, uncertainty and disposition

* **Resource survival is deliberately outside the contract, not an unproved
  bounds/overflow escape.** A minimal semantic witness is an ordinary
  `box_array_filled::<u8>(count: n, value: 0_u8)` allocation whose size the
  host cannot serve: it offers no failure arm. Deep unmarked recursion can
  likewise exhaust stack. These are schematic expressions, not compiled
  witness programs. A requirement to survive either needs an owner-directed
  Whitefoot resource contract, not a downstream spelling workaround
  ([STOR-8, STOR-6][storage], [OP-9][operations], [FN-10][functions]).
* **Accept error can stop the server with success status.** Static finding,
  not fault-injection evidence. Minimal scenario: an otherwise live listener's
  next `tcp_accept` returns `Err(ResourceExhausted(...))` with no clients left;
  Firn takes `:1116` then `:1147`. A regression case should inject one
  non-cancellation accept failure and require the next client to be accepted.
  This is a Firn recovery-policy issue, not a Whitefoot checker gap.
* **An unknown AOF command is not fatal in Firn.** Replay executes then clears
  the reply at `firn/persistence/persistence.wf:263–265,275–277`, including
  the unknown-command reply from `firn/commands/dispatch.wf:1286`. Minimal
  AOF bytes: `*1\r\n$7\r\nBOGUSXX\r\n`. Redis refuses unknown commands
  ([aof.c:1540–1547][redis-unknown]); Firn appears to continue. This is a
  source-derived persisted-state compatibility gap, not an extra crash site;
  confirm against both servers before fixing it. No execution was performed.
* **Limits of this audit:** no fault injection, stack bound, generated-binary
  call graph, dependency-internal site enumeration, or CI rerun. All 48 Firn
  source files were scanned; transitive Halo/library/runtime allocation and
  stack use remain relevant but cannot be assigned a complete Firn-only
  machine-site count. Redis allocation equivalence is policy-level, not a
  claim that identical inputs exhaust both implementations simultaneously.

Disposition: the accept-error defect is board item
`firn-accept-error-keeps-serving`, with fix PR Firn-wf#104. The
unknown-command-in-AOF finding is a step of board item `firn-bl-01-10` (AOF
loading rules), not yet confirmed by a run. No Whitefoot request was filed.

[wf]: https://github.com/Ming-Research/Whitefoot/tree/af7665be672d29912d878dd5da8c3932bb6f296c
[scope]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L7-L21
[storage]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L825-L935
[operations]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1013-L1241
[functions]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1543-L1550
[effects]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1616-L1618
[errors]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1638-L1658
[programs]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1677-L1681
[waiting]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2268-L2287
[sharing]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2296-L2357
[host]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2618-L3216
[wf-readme]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/README.md#L26-L56
[floor-design]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/design/compiler/resource-exhaustion-floor.md
[floor]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/wf_floor.c#L152-L191
[floor-write]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/wf_floor.c#L127-L147
[heap-emitter]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/emitter.rs#L504-L510
[heap-abort]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/emitter/resource_records.rs#L187-L210
[runtime]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/runtime.rs
[driver]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/bin/whitefootc.rs#L86-L190
[redis-config]: https://github.com/redis/redis/blob/7.2.16/src/config.c#L638-L645
[redis-load]: https://github.com/redis/redis/blob/7.2.16/src/server.c#L6774-L6776
[redis-listen]: https://github.com/redis/redis/blob/7.2.16/src/server.c#L2812-L2813
[redis-signals]: https://github.com/redis/redis/blob/7.2.16/src/server.c#L6493-L6522
[redis-accept]: https://github.com/redis/redis/blob/7.2.16/src/socket.c#L311-L330
[redis-dir]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L723-L727
[redis-manifest-open]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L263-L267
[redis-manifest-bad]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L378-L383
[redis-upgrade]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L619-L657
[redis-replay]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1596-L1639
[redis-missing]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1689-L1692
[redis-truncated]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1713-L1721
[redis-start]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L730-L760
[redis-args]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1503-L1537
[redis-write]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1198-L1205
[redis-sync]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1247-L1253
[redis-unknown]: https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1540-L1547

[redis-alloc]: https://github.com/redis/redis/blob/7.2.16/src/zmalloc.c#L144-L147
[redis-oom]: https://github.com/redis/redis/blob/7.2.16/src/server.c#L6842-L6847

[c-census]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/research/investigations/io-model/PARK-ON-MISS.md#L1174-L1194
[compute-census]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/research/investigations/compute-runtime/PRIOR-BUNDLE.md#L52-L58

[prelude]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2363-L2620
[hash-contract]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/collections/hash_map/module.wfm#L44-L68
[deque-impl]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/collections/deque/deque.wf#L8-L79
[priority-impl]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/collections/priority_queue/priority-queue.wf#L17-L29
[ordered-impl]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/collections/ordered_map/ordered-map.wf#L80-L137
