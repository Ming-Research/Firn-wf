# Script engines per driver and declared-key holds

## Question and ruling

Can firn remove the single-engine queue and whole-keyspace script hold while preserving Redis 7.0.15 replies,
effects and script atomicity, and make the scripted rate limiter scale from one to two physical cores?

The owner selected A on [board card
`firn-q-script-engine`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip): each driver gets its own script
engine; an attempt holds only the entries named by its declared KEYS; accessing an undeclared key abandons
that attempt and retries under a whole-map hold. FLUSH and KILL cover every engine, and each engine compiles a
registered script on its first use. Standalone Redis permits undeclared keys: fallback is internal control
flow, never a Lua error.

This is an investigation and preregistered validation plan, not an implemented capability. Source inspection
uses Firn-wf `e5c9565bf0544dc23ef2b482109221c4ba7945c7` (the worktree's base and observed origin/main),
Whitefoot release `wf-78223721f77d`, and Halo-wf `e8f3f9b68294411574e74a6713ea1c1feeafbb63`. No build, test or
measurement was run for this investigation.

Evidence is in deployment-performance: [native-host
scripts](../deployment-performance/README.md#scripts-on-two-cores-on-the-native-host), [the reference
comparison](../deployment-performance/README.md#against-the-reference-redis-7015) and [engine
waits](../deployment-performance/README.md#where-scripts-wait). The Redis 7.0.15 run found two-CPU script
rates below their one-CPU rates; the separate checkout probe attributed 89% of 50 connections' time to engine
waiting. Earlier packaged-Redis 8.0.5 columns are not reference results. These observations motivate A; they
do not establish that A alone scales.

## Current execution and its owners

Anchors below mean `file:function` or `file:type`; links open the owning file. The governing nodes are
[scripts](../../../design/firn/scripts.md), [command parts](../../../design/firn/command-parts.md),
[transactions](../../../design/firn/transactions.md), [memory limit](../../../design/firn/memory-limit.md) and
[rewrite](../../../design/firn/aof-rewrite.md), under [firn](../../../design/firn.md). The implementation
branch must revise the single-engine and whole-map decisions together with code; this research does not edit
approved nodes.

### Engine, admission, callbacks and retries

- [script_pool/module.wfm](../../../firn/script_pool/module.wfm):EnginePool owns one idle PooledEngine, an
  `out` checkout flag, the server-wide SHA1/source registry, source payload bytes, flush generation, running
  count and kill counter. PooledEngine owns Halo's Engine and a SHA1/ScriptId cache.
  [script_pool/pool.wf](../../../firn/script_pool/pool.wf):new_pool starts empty;
  [store/module.wfm](../../../firn/store/module.wfm):Keyspace carries its shared handle.
  [store/store.wf](../../../firn/store/store.wf):keyspace_share retains the same pool for every connection.

- [commands/dispatch.wf](../../../firn/commands/dispatch.wf):execute sends EVAL and EVALSHA to
  [scripting/entry.wf](../../../firn/scripting/entry.wf):eval through FirnCommands. There is no implemented
  FCALL/FUNCTION execution path here; do not describe one as another existing checkout caller. EVAL validates
  numkeys and hashes the source; EVALSHA checks the global registry before checkout, folds SHA case, and
  answers NOSCRIPT if absent.

- `entry.wf:take_engine` executes `atomic p = &store^.scripts when p^.out == 0_u64`. The first take increments
  running and captures kills with the generation. `new_pooled` constructs Halo, installs ScriptHost, and
  protects globals; a stale generation replaces the entire environment. `compiled_at` compiles on a
  local-cache miss; EVALSHA copies source from a second registry read only on that miss. `register` retains
  successfully compiled EVAL/LOAD sources, once per SHA, without count limits or cache eviction.

- `entry.wf:eval` calls [environment.wf](../../../firn/scripting/environment.wf):prepare to recreate
  KEYS/ARGV, then runs Halo's start/resume under `atomic t = &store^.keys, m = &store^.meta`. Both objects
  remain held for the entire attempt. The initial interpreter budget is 1,000 safepoints.

- [host.wf](../../../firn/scripting/host.wf):host_call handles redis.call/pcall as builtin IDs 4096/4097.
  [scripting/commands.wf](../../../firn/scripting/commands.wf):command validates Lua argument types and leaves
  a Pending host call. `entry.wf:answer_call` converts arguments to protocol spans, formats numbers with
  Redis's command rules, and invokes ScriptCommands::call.
  [commands/script.wf](../../../firn/commands/script.wf):script_command checks identity, arity, noscript and
  OOM rules, then held_run invokes shared command parts. Reply bytes become a Lua value; call raises errors,
  pcall returns them.

- On Budget before any propagated effect, `eval` restores access stamps, client LFU random state, VM random
  state and cjson configurations, resets Halo, releases the statement and engine, sleeps 1 ms and doubles the
  budget. After an effect, it resumes with an unlimited budget: written state cannot currently be undone. An
  expired key's deletion is an effect too.

- [store/store.wf](../../../firn/store/store.wf):record_access saves the first live access stamp per binary
  key; restore_access restores it under the original whole-map hold. AccessJournal contains no value/expiry
  undo. ScriptMemory.write_dirty records accepted WRITE commands, including a missing-key DEL, separately from
  HeldState.wrote (actual effects). Retries reset write_dirty using the original pre-command OOM snapshot.

### Effects, cache controls and compatibility boundaries

`entry.wf:answer_call` counts records in an attempt-local effects buffer. `record_effects` appends them to
Meta.log at Done **and Error**, wrapping more than one record in MULTI/EXEC. Redis errors preserve earlier
writes; ordinary Lua errors must never become an internal rollback. Command parts log effective operations,
including lazy-expiry DEL and absolute expiry deadlines, rather than replaying EVAL.
[persistence/persistence.wf](../../../firn/persistence/persistence.wf):replay and write_log consume these
bytes; [rewrite.wf](../../../firn/persistence/rewrite.wf):aof_rw_worker currently replays closed files into a
private dataset before emitting a new base.

`entry.wf:script` implements LOAD, EXISTS, FLUSH and KILL. LOAD takes the engine without incrementing running,
validates/compiles and registers source. EXISTS consults only the global registry (case-sensitive SHA), not
local compiled objects. FLUSH, with either SYNC or ASYNC, waits for running == 0, empties registry/source
accounting and advances generation. Later checkout replaces stale Lua state; a registered, already-begun
EVALSHA remains protected across retries. KILL increments kills while running > 0, returning OK; otherwise
NOTBUSY. eval checks that counter between attempts, after retaking the engine. `return_engine` resets
suspended execution and puts the engine back.

Known differences must not be mistaken for Redis requirements: current KILL answers OK for written scripts
where Redis's [script.c:scriptKill](https://github.com/redis/redis/blob/7.0.15/src/script.c#L294) answers
UNKILLABLE; EXEC rejects queued EVAL/EVALSHA as commands without held parts; shebang scripts are explicitly
refused. The script-time node also chooses frozen TTL time following Redis 7.2 rather than all of 7.0.15's
clocks. This work must expose these limits and register reference cases, not quietly retain wrong KILL/EXEC
results or weaken expectations to get a green gate.

### What a driver is, and the pinned-language gap

Following [downstream.md:Reading the language](../../../whitefoot-kit/downstream.md#reading-the-language),
inspection used the local Whitefoot clone through `git show` at `78223721f77db615c950b6df1bcc690629f9fe4b`:
normative [spec
v0.121](https://github.com/Ming-Research/Whitefoot/blob/78223721f77db615c950b6df1bcc690629f9fe4b/spec/kernel-spec.md),
`lib/std/**/module.wfm`, `docs/patterns.md` and maintained shared-map programs. No build manifest exists in
this checkout; the [release metadata](https://github.com/Ming-Research/Whitefoot/releases/tag/wf-78223721f77d)
and local tag resolve to that same full commit and specification version.

At that revision,
[completion/bridge.c](https://github.com/Ming-Research/Whitefoot/blob/78223721f77db615c950b6df1bcc690629f9fe4b/compiler/src/backend/completion/bridge.c):
wf_driver is an OS thread running ready asynchronous contexts, with its own run queue, parked contexts, timers
and completion ring. wf_drivers_begin uses WF_DRIVERS or the allowed CPU count, subject to runtime/platform
availability. Driver 0 runs the entry. wf_driver_steal moves ready contexts to another driver and changes
context->driver; a connection context is not permanently assigned to its accepting driver. firn's
[server.wf](../../../firn/server/server.wf):serve is one such context per connection, started by main,
alongside background work.

Neither the specification/prelude nor the pinned stdlib interfaces expose current driver identity, actual
driver count or driver-local owned storage. WF_DRIVERS is configuration, not an observable driver capability.
C's internal wf_driver_self and index are not writer APIs. One engine per connection, a fixed numbered pool,
hashing connection IDs or adding C glue would conceal the missing language/runtime contract and is not an
implementation of A.

Minimal required semantic witness (pseudocode, **not valid pinned Whitefoot**):

```text
lease = current_driver_lease()           // identity + lifetime/affinity authority
engine = checkout_engine_for(lease)      // exactly one slot for that driver
await acquisition of declared-key hold  // may suspend; affinity stays valid
run one nonwaiting script attempt(engine)
return_engine_to(lease, engine)
release(lease)                          // retry sleep may migrate the context
```

An identity read alone is insufficient: engine checkout and key acquisition are waiting atomic statements, and
the context may migrate between them. Whitefoot must define a driver lease or a driver-local acquisition
construct, including safe suspension, release, enumeration and teardown. Acceptance: two contexts on one
driver share its slot; two drivers have distinct slots; wait/migration cannot use a slot as another driver's;
FLUSH/KILL can enumerate all registered slots safely. This is a Whitefoot gap to file with its runtime
session, linked to firn-q-script-engine; no Whitefoot repository edits here.

A different design needs no driver identity: a pool of engines, one per configured driver, from which
a script checks out any idle engine. At most as many scripts run at once as there are drivers, so such a
pool removes the single-engine wait without affinity. It is not "one engine per driver" in the ruling's
wording, but it meets the ruling's aim, and it hides no gap: it never claims driver locality. The driver
count is configuration (WF_DRIVERS), which firn would read as an option rather than observe. The owner
decides between this pool and waiting for a Whitefoot driver lease (status board card
`firn-q-script-pool`).

## Proposed option-A structures and attempt protocol

Separate the global ScriptRegistry/control from DriverEngineSlot records. The registry retains SHA/source,
source_bytes and generation. Each slot owns idle Option<PooledEngine>, checked-out state, engine generation,
active invocation identity and published memory counters. Global control retains running across retries and a
kill epoch; keep its holds outside key holds. Register lazily created driver slots through the future driver
capability. Take/return touches the selected slot rather than holding one global pool through execution.
Source registration and engine-local compilation stay distinct: an EVALSHA local miss is not NOSCRIPT if the
registry knows its SHA.

Build an immutable declared KeySet from numkeys arguments before execution, preserving the original ordered
KEYS table, duplicate values and binary names. Deduplicate only the hold set; Lua may mutate KEYS, but cannot
thereby expand the invocation's declaration. The narrow form is exactly:

```whitefoot
atomic slots = &store^.keys[keys], m = &store^.meta {
  // slots: &Entries<Entry>; only the deduplicated declared entries are reachable
  // execute the attempt and publish its accepted effects before releasing
}
```

For one key this is the existing `atomic slot = &store^.keys[key], m = &store^.meta` pattern. A whole attempt
instead uses today's map-and-Meta form. SHARE-2 provides no map reference under an entry-set hold; guarding
today's full-map ScriptCommands::call is insufficient. Add command planning that returns no keys, a finite
ordered key list, or WholeMap, and entry-based execution adapters using the same existing command parts. Keep
references as parameters, not stored in an owned Env. Plan all source/destination keys (RENAME, COPY, stores,
list moves and set algebra); global commands such as KEYS/SCAN/DBSIZE/RANDOMKEY require WholeMap even when
they return no key. No command-specific Lua parser or benchmark shortcut: use the runtime redis.call arguments
and command table.

`host.wf:host_call` remains the Lua entry boundary. Its Pending call reaches `entry.wf:answer_call`, where
numeric arguments have their final key bytes. Through ScriptCommands planning, apply existing admission/error
precedence before inspecting any entry. If an admitted command needs WholeMap or any key outside the immutable
declaration, return internal NeedWholeMap **before the command executes**. Do not call fail_call: redis.pcall
and Lua pcall must not catch fallback. Preserve both redis.call and redis.pcall error behavior on the eventual
accepted attempt; unknown/forbidden commands keep their Redis errors.

NeedWholeMap restores the entire narrow attempt under its still-held slots, resets engine/client state,
discards unpublished effects, releases slots and Meta, then retries the invocation from its beginning in Whole
mode. Whole is sticky for that invocation, including later budget retries; no lock upgrade, nested atomic or
incremental addition of the discovered keys. The undeclared key is first read in the whole attempt. No-key
invocations hold an empty set plus Meta initially: pure Lua and keyless commands need no entries, but any key
access or global enumeration triggers Whole. Do not treat numkeys == 0 as read-only or forbid writes.

### Rollback must include earlier writes

`EVAL "redis.call('INCR',KEYS[1]);return redis.call('GET','other')" 1 counter` must increment counter once,
even though fallback follows a write. Today's access journal and wrote guard cannot do that. Proposed
AttemptUndo saves each declared entry's Option<Entry> before its first dataset mutation, including value,
expiry and future last_write. AccessJournal still saves the stamp before its first refresh; abort restores
payloads first, then those original stamps. Use owned deep copies and ordinary slot-based command parts,
preserving absence and all five Value variants; repeated dataset mutations save only one original. Snapshot
client reply position/settings/random and engine-visible state; stage expiry queue insertions, counters and
random draws in attempt-local metadata deltas. Publish those deltas only on an accepted Done or Lua Error.
Abort restores slots before release and drops every delta, AOF byte and capture.

This is an explicit new rollback capability, not a feature atomic statements already provide. An undo copy of
a large list/hash is expensive; validate its cost and coverage before adopting it. Access-only budget retries
can retain the lighter AccessJournal; budget exhaustion after actual writes still runs to completion, but
later undeclared access must be undoable. Accepted WRITE without an effect still sets write_dirty. Never turn
a runtime error after a write into rollback. Fallback is the only new written-attempt abandonment.

### Metadata, sequence numbers, capture, expiry and eviction

**Remaining serialization:** every narrow script above still holds the same Shared<Meta> for its entire
interpreter run. SHARE-3 gives exclusive access; even disjoint keys cannot execute these attempts
concurrently. SET/INCRBY outside scripts hold Meta only for a short command, unlike a whole Lua run. Engine
sharding removes the engine queue but shifts waiting to Meta. Splitting Meta fields alone still leaves the
shared expiry queue/log/sequence writer held by every limiter script. A scalable design needs short metadata
commit holds plus entry isolation, and a language-expressible way to combine them; SHARE-2 currently forbids
nested atomic statements or waits inside an attempt. This investigation cannot claim that the literal
keys-plus-Meta form scales.

The requested sequence_new/sequence_new_key are **absent from this base**. They belong to [PR #42,
scan-and-log rewrite](https://github.com/Ming-Research/Firn-wf/pull/42), inspected at
`1f0efa5737bbb62be0944832914ec748d7976071` (open, not merged). Its
[store/sequence.wf](https://github.com/Ming-Research/Firn-wf/blob/1f0efa5737bbb62be0944832914ec748d7976071/firn/store/sequence.wf):sequence_new
creates one lazy WriteSequence under Meta; sequence_change allocates its number on the first dataset mutation;
sequence_commit publishes commit_seq, capture and exhaustion. sequence_new_key opts into anchored one-key
AOF-effect capture. Its scripting entry uses sequence_new once per attempt and commits at exit.

Integration must keep one token for an accepted script (or enclosing EXEC), stamp all changed entries with it,
and capture final replacements/tombstones before releasing the hold. Discard an abandoned token and restore
last_write; publish neither capture failures nor reserve charges from abandoned scratch. Account temporary
allocations while owned and return their charge on drop. Do not choose sequence_new_key merely because numkeys
== 1: an accepted whole fallback can modify several keys, and script effects may read other keys. Initially
use sequence_new for all scripts/EXEC, including Lua errors after effects. Capturing read-only access stamps
takes no dataset sequence. After PR #42 lands, re-read its actual contract and base the integration on that
revision; do not import or duplicate an unmerged rewrite here.

Keep Time and the per-read memory settings across retries. Lazy expiry is an entry mutation and must enter
undo before deletion; its DEL belongs only to accepted effects. Apply queued expiry deltas/eligibility_changed
on commit; active expiry and eviction continue using their ordinary entry-and-Meta holds, so they cannot
remove a held declared key. Whole retries exclude them as today. `commands/eviction.wf:evict_before` stays
outside the script; carry its pre-command OOM into every attempt, with no inner eviction pass. Added
engine/undo memory remains real heap, not an AOF exclusion or a per-engine maxmemory allowance.

### Controls, memory and MULTI/EXEC

FLUSH needs one ordering between LOAD/EVAL registration, first checkout and generation publication. Validate
LOAD outside the control hold, but register only if its checkout generation is still current; otherwise retry
validation. FLUSH waits for all begun invocations across all drivers, including retry gaps, then atomically
clears the global registry and advances generation. All slots invalidate caches and Lua state; recommend lazy
reconstruction on next use. An inactive old engine may retain memory until then, which accounting must report.
Eager destruction requires moving idle engines out in short holds and dropping them outside; it must not
expose half-flushed registry contents.

KILL reaches every active engine through control independent of keys/Meta. Track invocation IDs, epoch and
accepted WRITE_DIRTY, not engine count alone. An all-read-only set can be marked killed; with no active
invocation return NOTBUSY. If any active invocation has accepted a WRITE, recommend global UNKILLABLE with no partial
kill. Publishing that flag only after the attempt exits is too late, including DEL of an absent key. The
needed in-attempt publication must have a safe Whitefoot contract; nesting a scripts atomic under Meta is
forbidden. Do not substitute effects != 0 for Redis's WRITE_DIRTY.

Per-slot memory records should distinguish logical Halo heap (`embed:heap_bytes`), compiled-cache payload and
measured process heap; the logical API excludes slab/intern reserve. Aggregate owned engine memory once,
including checked-out engines, plus the single source registry; number_of_cached_scripts remains registry
cardinality, not the sum of compiled copies. `commands/info.wf:run_info` and eviction's heap_read already
observe process-wide allocation. Initially retain that authoritative accounting and publish idle/checkpoint
per-slot diagnostics without claiming a live exact breakdown or inventing Redis fields.

`commands/transaction.wf:run_exec` currently holds whole keys and Meta and runs only held command parts, so
queued EVAL marks the queue foreign. Redis allows EVAL in EXEC. Prepare script sources/engine checkout outside
the EXEC hold, preserving queue-time versus execution-time errors and in-queue SCRIPT LOAD/FLUSH/EXISTS order;
compilation is not permission to run early. Add a nonwaiting held-script entry using the enclosing whole-map
hold, shared effects and sequence token. It needs no undeclared fallback. Never call waiting eval from inside
EXEC or release EXEC between commands. Internal budget retries of EXEC would have to undo/replay the **whole**
transaction, including preceding writes; this and KILL/busy behavior need a dedicated contract before claiming
full EXEC scripting support.

## Real choices within A (proposals needing owner decisions)

1. **Driver capability:** recommend a runtime-owned driver lease/local slot API with migration semantics over
   a numeric current-driver query alone. The former adds runtime/language surface but expresses actual
   ownership; the latter is smaller but stale across waits. Confidence 4/5 on the gap, 2/5 on API shape; the
   Whitefoot runtime session must settle its contract.

2. **Written fallback:** recommend full first-mutation entry undo plus staged metadata over a private cloned
   working dataset for every declared key. Undo copies only dataset changes but requires auditing every
   mutation; cloning simplifies abandonment but copies unused large keys. Both need canonical command parts
   and cost measurements. Confidence 3/5; no option may simply reject undeclared access after a write or retry
   it twice visibly.

3. **Meta lifetime:** recommend a separate design with the runtime session for entry isolation and short
   ordered metadata publication, before the performance implementation. Literal keys-plus-Meta is a valid
   correctness stage but retains serialization; merely splitting fields does not solve the common log/sequence
   hold. Confidence 5/5 on shared exclusion, 2/5 on the replacement. Owner decision required on expanding this
   mechanism.

4. **Shared Lua state:** recommend canonical portable library settings and random state synchronized between
   engines over driver-local settings. `tests/network.rs:firn_scripts_share_one_lua_state_as_redis_does`
   requires one connection's cjson precision to reach another. Canonical synchronization preserves replies but
   needs ordering (and may serialize affected scripts); driver-local state changes replies and is incompatible
   with this task. Confidence 4/5 on the obligation, 2/5 on covering every library state.

5. **Control policy:** recommend lazy generation reset over eager idle-engine destruction (shorter FLUSH holds
   versus delayed memory reclamation), and all-or-none KILL over killing only unwritten engines (one atomic
   response versus partial results). Confidence 3/5; the latter extends Redis's single running-script rule to
   concurrent scripts and needs an owner ruling.

## Validation registered before measurements

All execution below is CI-only; this document registers cases, not results. Use Redis 7.0.15 replies/files or
its source as the oracle, independently of firn. First run the smallest selected correctness case in hosted CI
to learn duration, then the relevant network group and the complete make check gate. Existing tests remain
wired; demonstrate representative new faults fail.

- Declared keys: binary/empty/duplicate names, multiple sources/destinations, mutated Lua KEYS and overlapping
  concurrent scripts; disjoint scripts must have an equivalent serial history, and readers see no partial
  result.

- Undeclared reads/writes: zero-key EVAL, keys taken from ARGV or computed strings, redis.call and
  redis.pcall, globals/enumeration, undeclared expiry, and undeclared access **after**
  INCR/SET/list/hash/set/sorted-set mutations. Assert one visible increment and one AOF effect, original
  expiry/access state on abandonment, and identical final replies, including errors.

- Budget/fallback composition: read-refresh then fallback, fallback then budget exhaustion/KILL, randomness
  and cjson changes before abandonment, lazy deletion before fallback, no-effect WRITE under OOM, and Lua
  errors after actual writes. Extend existing access rollback and script AOF cases.

- Cache/control: LOAD on one driver followed by EVALSHA/EXISTS on others; syntax errors not registered;
  hot/cold engine caches; both FLUSH modes, races with LOAD/first checkout and retry gaps; repeated FLUSH
  resets Lua settings; KILL with no scripts, multiple read-only scripts and written scripts. Use a bounded
  finite writer for UNKILLABLE; no unkillable CI hang.

- Persistence/EXEC: accepted script error effects, one/multiple effect wrappers, restart from AOF, concurrent
  rewrite with scripts/fallback and INCR/APPEND hot keys, deletion/recreation and absolute expiries. After the
  rewrite pin moves, assert aborted attempts publish no sequence/capture. Exercise queued EVAL/EVALSHA,
  NOSCRIPT, SCRIPT LOAD/FLUSH ordering, error continuation, preceding transaction writes and atomic visibility
  against Redis; today unsupported paths are gaps to close, not expected refusals.

- Run make check's Redis suite ratchet without --tolerant. Retain passing.tsv; add new reported passes in the
  implementation PR. Exclusions require the record workflow's repeated evidence, not this design's compiler
  limitations.

Performance comparison: use `redis-bench.yml`, runner `14900k`, mode `workloads`, workload_images true, tests
`limiter-script`, cpus `1 2`, connections `8 50`, and named revisions `main=<resolved current main>`,
`main-twin=<same revision>`, `engines=<candidate>`. Its name-ending-in-twin facility reuses the identical main
image. Build all firn images with make firn-lto and the same pinned compiler/Halo; if a new Whitefoot API
requires an upgrade, include a same-source main control on that compiler to separate compiler changes from
firn changes. Verify Redis server and client are 7.0.15.

Use native-host placement: one/two distinct performance cores for servers, separate client cores, depth 1,
existing random 100,000-key limiter script. Check runner idle and coordinate its slot first. Probe 2
interleaved passes of 5 seconds before 3 passes of 10 seconds, retaining rate and p50/p99 spread and reversing
line order as the harness does. Image workload mode currently runs AOF off; do not claim AOF-on evidence from
it. Add an explicitly wired image/AOF cell if that comparison is needed before adopting the change.

Proposed acceptance, fixed before the probe: at each connection count the candidate's two-CPU rate must exceed
both its one-CPU rate and current main's two-CPU rate beyond main/twin and pass spread, with two-CPU p99
improved over main beyond that spread. One-CPU rate/p99 must not regress beyond noise; report firn/Redis
ratios in all four cells, even if below one. A measurable gain is required, not a promise of exactly 2x. Any
semantic discrepancy rejects the design. Reproducible lack of two-core gain, regression, or merely moving the
queue to Meta rejects its performance premise; noise overlap is inconclusive, not success. Lengthen only
inconclusive cells after examining the probe. Keep CSV/settings/revisions and accepted/rejected criteria here.

## Implementation files and small reviewable order

1. Resolve driver/Meta/state/control contracts with the owner and Whitefoot sessions; keep minimal driver and
   atomic-composition witnesses under research/experiments, exercised by CI. Whitefoot API/pin work belongs to
   its owning session; no downstream native shim. Record approved decisions in design/firn/scripts.md and
   affected command/transaction/rewrite nodes.

2. Refactor firn/script_pool/{module.wfm,pool.wf}, firn/store/{module.wfm,store.wf} and
   firn/scripting/entry.wf into global registry/control and per-driver slots, preserving whole holds first.
   Update scripting/module.wfm contracts, server/server.wf capability threading and commands/info.wf
   accounting.

3. Add complete command key planning and entry adapters in commands/script.wf and commands/module.wfm,
   touching owning parts in strings/keys/lists/hashes/sets/sorted/ranges/scan/object/access as required.
   Reuse semantics across network, scripts and EXEC. Cover every admitted command's footprint.

4. Add undo/staged metadata helpers in store and scripting/host.wf; update
   scripting/{entry,environment,commands,module}.wf or .wfm as appropriate. Introduce narrow attempts and
   sticky fallback only after all rollback cases pass; resolve the Meta publication contract before scaling
   claims.

5. Integrate all-engine controls, canonical Lua state, EXEC's nonwaiting scripting path in
   commands/transaction.wf, and PR #42's actual sequence/capture helpers after adoption. Persistence/rewrite
   files change only where their publication interfaces require it; memory eviction keeps its contract unless
   the owner approves a change. Do not edit Halo locally; any required portable-state/embed API belongs to the
   Halo session.

6. Extend tests/network.rs and tests/memory_limit.rs, update Redis passing.tsv only from CI, and wire any new
   witness/benchmark caller in the existing Makefile/workflows. Run the registered 14900K comparison; retain
   evidence here and update the deployment-performance conclusion at its source. Review the complete
   implementation and tree correspondence before readiness.

This investigation changes no pin/submodule and files no external gap itself. Open owner questions are the
five proposals above; the blocking discoveries are missing driver authority, written fallback undo, shared
Meta serialization and cross-engine Lua-state/control ordering. Research can finish now; implementation cannot
honestly claim scalability or full compatibility until those contracts and their independent validation are
settled.
