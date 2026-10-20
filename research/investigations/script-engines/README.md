# Script engines per driver and declared-key holds

## Client-visible script details: Redis 7.2.16

This source audit serves board item `firn-scope-script-client`. It compares Firn's
base `c6f6058fb17277d4d44d364a45efad1eae01a22b`, Halo
`56878ac4d607f7496c8e23a4c2ffe2d45e9cb552` and Whitefoot
`d75f318d56318834785322bde3a3656fb93116a6` against Redis's **7.2.16 tag**.
The question is whether ordinary clients receive the exact script replies they
depend on, including inside EXEC. Acceptance requires the RESP2/RESP3 network
cases to pass against that reference, with transaction isolation preserved.
The direct network cases run in the gate on every push; the five cases that
run scripts inside EXEC are ignored until `firn-scope-script-client` supports
that path (see the held transaction interruption below). Neither pins nor
submodules changed.

| Behaviour | Redis 7.2.16 | Firn before | Firn after | Halo gap |
| --- | --- | --- | --- | --- |
| Missing EVALSHA, both protocols | `-NOSCRIPT No matching script. Please use EVAL.\r\n`; wrong SHA length wins over invalid numkeys, but a 40-byte SHA's numkeys is checked before lookup | Exact direct error and ordering | Unchanged; new reference cases also cover EVALSHA_RO | None identified |
| Missing SHA inside EXEC | Queued; NOSCRIPT occupies its array element and later commands execute | Whole transaction refused as unsupported | Waits for `firn-req-attempt-publish`; reference cases keep Redis's expected reply | Not a Halo issue |
| EVAL_RO / EVALSHA_RO | Arity -3; READONLY and NOSCRIPT; no NO_MULTI, WRITE or DENYOOM; same registry as writing invocations | Unknown network commands | Direct legacy-source invocations supported through existing retries and command parts; authority is invocation-local, not cached; the existing explicit refusal of all shebang sources remains | None identified |
| Write attempted by a read-only script | After command arity checks and before disk/OOM checks: `ERR Write commands are not allowed from read-only scripts.`; call raises with SHA/line, pcall returns an error table; even a no-effect DEL is refused. The test is CMD_WRITE or CMD_MAY_REPLICATE (script.c:349-352); apart from the EVAL family, already refused as noscript, only PFCOUNT, PUBLISH and SPUBLISH carry MAY_REPLICATE (commands.def of 7.2.16), and Firn runs none of them from scripts | No RO entry | Shared admission refuses before mutation or WRITE_DIRTY; reads may still perform lazy expiry | None identified |
| EVAL-family calls from a script | NOSCRIPT refusal after arity check | Incorrect unsupported-command error | Exact arity/NOSCRIPT errors for all four entries | None identified |
| EVAL-family commands inside MULTI | Permitted; execution, registration and errors occur in queue order | Queued then whole EXEC refused | No execution workaround installed; `firn-req-attempt-publish` blocks support | Not a Halo issue |
| cjson | encode/decode, null, new/configuration APIs, default precision 14 and depth 1000 | Exposed through Halo | No codec changes; new network cases check deterministic JSON bytes, decode and error paths | None identified in inspected exposed paths; exhaustive codec equivalence is unverified |
| cmsgpack | pack/unpack/unpack_one/unpack_limit; binary strings and multiple values | Exposed through Halo | No codec changes; new network cases check exact packed bytes, decoding and error paths | None identified in inspected exposed paths; exhaustive codec equivalence is unverified |
| redis.replicate_commands(...) | Unconditional Lua true, ignoring arguments and earlier writes | Missing redis table member | Installed host function, unconditional true | Not a Halo issue |
| redis.pcall | Returns command/admission errors as an error table without raising; Lua pcall extracts the err field from raised Redis errors | Implemented | Existing conversions retained; new API and RO cases | None identified |
| redis.error_reply / redis.status_reply | One string argument; otherwise an error table; returned tables use Redis's error/status wire conversion | Implemented | Existing implementation retained; new byte and validation cases | None identified |
| Long unpack | LUAI_MAXCSTACK=8000; results plus actual unpack arguments must fit: implicit bounds 7999/8000, explicit three arguments 7997/7998; oversized ranges report `too many results to unpack` | Halo implements the limit | No engine changes; new boundary and long redis.call argument cases | None identified |

Reference owners:

- [eval.c](https://github.com/redis/redis/blob/7.2.16/src/eval.c):
  `luaRedisReplicateCommandsCommand`, `evalGenericCommand`, `evalShaCommand`
  and the two RO entry wrappers.
- [script.c](https://github.com/redis/redis/blob/7.2.16/src/script.c):
  `scriptPrepareForRun`, `scriptVerifyWriteCommandAllow`, `scriptCall`,
  `scriptInterrupt` and `scriptKill`.
- [script_lua.c](https://github.com/redis/redis/blob/7.2.16/src/script_lua.c):
  `luaRedisGenericCommand`, `luaRedisPcall`, `luaRedisErrorReplyCommand`,
  `luaRedisStatusReplyCommand` and the reply converters.
- [server.c](https://github.com/redis/redis/blob/7.2.16/src/server.c):
  `shared.noscripterr` supplies the complete protocol-independent error bytes.
- [RO source entry metadata](https://github.com/redis/redis/blob/7.2.16/src/commands/eval_ro.json)
  and [RO SHA entry metadata](https://github.com/redis/redis/blob/7.2.16/src/commands/evalsha_ro.json).
- [lua_cjson.c](https://github.com/redis/redis/blob/7.2.16/deps/lua/src/lua_cjson.c),
  [lua_cmsgpack.c](https://github.com/redis/redis/blob/7.2.16/deps/lua/src/lua_cmsgpack.c),
  [lbaselib.c](https://github.com/redis/redis/blob/7.2.16/deps/lua/src/lbaselib.c),
  [lapi.c](https://github.com/redis/redis/blob/7.2.16/deps/lua/src/lapi.c)
  and [luaconf.h](https://github.com/redis/redis/blob/7.2.16/deps/lua/src/luaconf.h)
  define library output and the unpack stack check.
- [Redis scripting tests](https://github.com/redis/redis/blob/7.2.16/tests/unit/scripting.tcl):
  unpack invalid ranges, error handling, codecs and RO commands. The local
  `tests/redis-suite/` runner downloads this pinned suite; it does not vendor
  `unit/scripting.tcl` directly.

### Held transaction interruption: Whitefoot request firn-req-attempt-publish

Redis's `scriptInterrupt` still processes control requests while a script runs
inside EXEC. Adding a held script interpreter which runs indefinitely would
make a read-only infinite script unkillable. Releasing EXEC between commands
would allow other clients to see the transaction halfway through. Replaying
only the current script would repeat or lose the transaction's preceding writes.
None of these satisfies the reference.

At the pinned Whitefoot revision, SHARE-2 prohibits waiting calls and nested
atomic statements inside an atomic block, and the block exclusively holds every
target. Including the KILL flag in EXEC's targets therefore prevents another
context from updating it. `std::time::cancel_state` returns a SharedRead view
which also needs atomic access; it offers no independent observation inside
held computation. WAIT-2's progress guarantee requires every context to reach
completion or a wait in finite steps. Runtime overdue-deadline handoff does not
provide general preemption of an interpreter loop.

The following complete semantic witness intentionally uses the unavailable
nested observation. It is **not an accepted Whitefoot program**. No compiler
diagnostic was obtained locally; the rejection follows from pinned SHARE-2.

```whitefoot
alias ExitStatus = std::process::ExitStatus;

fn request_stop(stop: Shared<Bool>) -> result: unit pure waits {
  atomic flag = &stop {
    set flag^ = True();
  }
  return unit;
}

fn transaction(data: Shared<u64>, stop: Shared<Bool>) -> result: unit pure waits {
  atomic value = &data {
    set value^ = 1_u64;
    loop {
      let killed = False();
      atomic flag = &stop {
        set killed = flag^;
      }
      if killed {
        break;
      }
    }
    set value^ = 2_u64;
  }
  return unit;
}

fn main() -> status: ExitStatus pure waits {
  let data = shared_new::<u64>(value: 0_u64);
  let initial = False();
  let stop = shared_new::<Bool>(value: initial);
  let sender = shared_share::<Bool>(shared: &stop);
  spawn request_stop(stop: move sender);
  transaction(data: move data, stop: move stop);
  return std::process::exit_status(code: 0_u8);
}
```

The existing Whitefoot request `firn-req-attempt-publish` owns this gap. A long
atomic statement must settle "written first" versus "killed first" with another
context while preserving the data transaction's isolation, so SCRIPT KILL can
answer as Redis does. Observation of a kill request must also let the unwritten
script terminate; a local flag alone does not establish this arbitration.
Its language owners are pinned [SHARE-2 and SHARE-3](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/spec/kernel-spec.md#L2320),
[WAIT-2](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/spec/kernel-spec.md#L2268)
and the [time interface](https://github.com/Ming-Research/Whitefoot/blob/d75f318d56318834785322bde3a3656fb93116a6/lib/std/time/module.wfm).
The attempted uninterruptible EXEC implementation was removed, and
`firn-scope-script-client` remains incomplete while it waits for that existing
request.

The acceptance witness uses two connections and an observing third connection,
on one and multiple Whitefoot drivers:

```text
Connection A:
MULTI
SET exec-before 1
EVAL_RO "while true do end" 0
SET exec-after 1
EXEC

Connection B, after the script becomes busy:
SCRIPT KILL
```

KILL must answer OK. EXEC must return an array containing OK, a killed-script
error and OK, then both keys must hold `1`. An ordinary third-client read must
not return an intermediate dataset. Repeat for each EVAL entry and check that
an already accepted script WRITE makes KILL answer UNKILLABLE. Exact
killed-script SHA/line wire bytes need reference recording in CI; this witness
is not an asserted byte expectation in the network cases.

The new network cases assert reference results for direct and EXEC paths,
without Firn-specific expected replies. The five MULTI/EXEC tests are explicitly
ignored with the reason "waits for firn-scope-script-client: scripts inside
EXEC", as directed by the owner, until that item supports this execution path. Their Redis expectations remain unchanged. Direct cases
require CI validation; source inspection alone does not establish byte
equivalence.

### Files and regression evidence for the client audit

- `firn/commands/dispatch.wf`: identify both RO names in full, case-insensitively,
  and pass invocation authority to the existing evaluator.
- `firn/commands/admission.wf`: give both RO entries arity -3 without WRITE or
  DENYOOM admission flags.
- `firn/commands/script.wf`: add the RO command identities, refuse script writes
  before AOF/OOM checks, and give recursive calls to all four EVAL entries the
  reference arity and NOSCRIPT errors.
- `firn/scripting/module.wfm`: add invocation authority to ScriptMemory and the
  public evaluator's signature, retaining its effects and contracts.
- `firn/scripting/entry.wf`: initialize that authority on every attempt,
  preserving it across budget and whole-map retries.
- `firn/scripting/environment.wf`: install redis.replicate_commands before
  protecting the globals.
- `firn/scripting/host.wf`: return true for that host call, ignoring arguments
  and previous writes.
- `tests/network.rs`: add the five groups below, each split into direct and
  MULTI tests and each exercised with RESP2 and RESP3. The five MULTI tests are
  ignored pending `firn-scope-script-client`; direct tests remain in the gate.
- `design/firn/scripts.md`: propose shared invocation-local read-only execution
  instead of caching authority or duplicating command implementations.
- `design/firn/command-parts.md`: include the EVAL family in the NOSCRIPT rule.
- `firn/README.md`: describe the direct RO entries and replicate_commands,
  and state the remaining EXEC and inherited shebang boundaries.
- This research record: retain the source audit, stopped EXEC work and minimal
  language witness. No unused implementation file remains.

| Test suffix (under both `firn_script_clients_direct_` and `firn_script_clients_multi_`) | Fault it detects |
| --- | --- |
| lookup_and_read_only | Wrong SHA/numkeys precedence, wrong error bytes or locations, cache authority leakage, no-effect writes accepted, arity versus RO ordering, lost authority across both retry paths, mutation after refusal, or wrong in-EXEC registration order |
| arity_and_execution_errors | Queue-time arity errors fail to dirty a transaction, or an accepted script error aborts later commands rather than occupying one EXEC element |
| error_status_and_replication | Error-table or Lua-pcall conversion is wrong, status/error control bytes split replies, argument validation is wrong, recursive EVAL bypasses NOSCRIPT, or replicate_commands differs by arguments, alias or preceding write |
| json_and_messagepack | JSON values/escapes/object bytes or independent decoding are wrong; MessagePack integers/floats/arrays/objects, binary strings, multiple results or offsets are wrong; malformed input is accepted |
| unpack_boundaries | The 7999/8000 or 7997/7998 limits are wrong, signed ranges overflow or reversed ranges return values, or a 128-argument host command truncates the argument list |

Expected command replies match recordings from redis-server 7.2.16 in
[CI run 38398315312](https://github.com/Ming-Research/Firn-wf/actions/runs/38398315312).
Each of the 20 batches ran against a fresh reference server: the ten new test
names, each in RESP2 and RESP3. All 2,336 recorded command/reply pairs were
compared byte for byte with the test expectations, decoding recorded reply
strings as Latin-1 so binary bytes remain unchanged. The comparison includes
helper PING replies, MULTI/QUEUED/EXEC wrappers, dynamic arguments and all three
handwritten transaction batches. Every recorded command also matches the
exported sequence. Two independent static comparisons found zero differences,
so no expected reply or Firn implementation changed for this recording.

Every new test's documentation cites that run. The recorder negotiated HELLO 3
separately and omitted its reply; the ten greeting expectations remain
source-derived and are outside this recording comparison. There are no ORACLE
placeholders. Blocked EXEC expectations remain recorded Redis expectations,
not Firn's current unsupported reply. This reference evidence confirms the
cases' expected bytes; Firn's own results are the gate's.

Remaining uncertainty is broader codec equivalence outside the inspected paths.
The transaction cases stay ignored for the held transaction interruption
contract above. No Whitefoot or Halo source was modified; no dependency pin
moved.

## Question and ruling

Can firn remove the single-engine queue and whole-keyspace script hold while preserving Redis 7.0.15 replies,
effects and script atomicity, and make the scripted rate limiter scale from one to two physical cores?

The owner selected A on [board card
`firn-q-script-engine`](https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip): an attempt holds only the entries
named by its original declared KEYS; accessing an undeclared key abandons that attempt and retries under a
whole-map hold. Standalone Redis permits undeclared keys: fallback is internal control flow, never a Lua error.
The later ruling on `firn-q-script-pool` selects any-idle-engine checkout from N slots, where N is the configured
WF_DRIVERS count with the runtime's allowed-CPU default, rather than driver-local ownership. That pool waits
for Whitefoot request `firn-req-driver-count`; the declared-key implementation keeps the single engine.

The owner selected first-write Option<Entry> undo (`firn-q-script-undo`) and holding Meta throughout each
attempt for this correctness step (`firn-q-script-meta`). The accepted-WRITE publication needed for correct
KILL is filed as Whitefoot request `firn-req-attempt-publish`, under firn's busy-script backlog; this work keeps
KILL's existing behavior. The separate string type-metatable protection backlog is outside this work.

The declared-key draft adds ordered command planning, narrow entry adapters, logical entry copies, staged
expiry insertions and sticky whole-map fallback against the merged sequence/capture implementation. The owner
ruled that [logical collection restore](#logical-collection-restore) meets the current compatibility contract;
collection backing reconstruction is not a gap. These source changes await CI and are not validated capability
or performance results. The pool and concurrency remain pending; the shared Meta hold still serializes attempts.

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

## Inspected baseline execution and its owners

Anchors below mean `file:function` or `file:type`; links open the owning file. The governing nodes are
[scripts](../../../design/firn/scripts.md), [command parts](../../../design/firn/command-parts.md),
[transactions](../../../design/firn/transactions.md), [memory limit](../../../design/firn/memory-limit.md) and
[rewrite](../../../design/firn/aof-rewrite.md), under [firn](../../../design/firn.md). The baseline descriptions below record the original investigation revision. The declared-key draft revises
the script and command-parts nodes; single-engine operation remains until the configured-count gap is closed.

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

The millisecond wait between attempts exists because Whitefoot does not yet let other contexts on a
driver run, or let an idle driver take over its connections, while one context computes without waiting:
Whitefoot requests `firn-req-ring-assist` and `firn-req-single-driver-progress`. It bounds the starvation
only for scripts that have not written; a script that has written runs to its end without waiting, and
other connections on its driver wait for it. When those requests land, the wait is to be re-examined.

Known differences must not be mistaken for Redis requirements: current KILL answers OK for written scripts
where Redis's [script.c:scriptKill](https://github.com/redis/redis/blob/7.0.15/src/script.c#L294) answers
UNKILLABLE; EXEC rejects queued EVAL/EVALSHA as commands without held parts; shebang scripts are explicitly
refused. Correct KILL arbitration and scripts inside EXEC wait for Whitefoot request
`firn-req-attempt-publish`. The script-time node also chooses frozen TTL time following Redis 7.2 rather than all of 7.0.15's
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

The selected pool design needs no driver identity: a pool of engines, one per configured driver, from which
a script checks out any idle engine. At most as many scripts run at once as there are drivers, so such a
pool removes the single-engine wait without affinity. It is not "one engine per driver" in the ruling's
wording, but it meets the ruling's aim, and it hides no gap: it never claims driver locality. The driver
count is configuration (WF_DRIVERS), which firn would read as an option rather than observe. The owner selected this pool on `firn-q-script-pool`. It waits for public access to the configured count and
allowed-CPU default, filed as `firn-req-driver-count`; firn's current argv configuration interface cannot read
the runtime's environment setting or its default.

## Proposed option-A structures and attempt protocol

Separate the global ScriptRegistry/control from DriverEngineSlot records. The registry retains SHA/source,
source_bytes and generation. Each slot owns idle Option<PooledEngine>, checked-out state, engine generation,
active invocation identity and published memory counters. Global control retains running across retries and a
kill epoch; keep its holds outside key holds. Create configured-count slots without claiming driver affinity; creation waits for
`firn-req-driver-count`. Take/return touches the selected slot rather than holding one global pool through execution.
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
expiry, access and last_write stamps. AccessJournal still saves the stamp before its first refresh; abort restores
payloads first, then those original stamps. Use owned deep copies and ordinary slot-based command parts,
preserving absence and the logical contents of all five Value variants; collection backing may be rebuilt
under the [logical restore](#logical-collection-restore) proposed below, and repeated mutations save only one copy. Snapshot
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

The sequence/capture rewrite is now merged into this worktree. Its
[store/sequence.wf](../../../firn/store/sequence.wf):sequence_new creates an attempt-local lazy token;
sequence_change changes that token and entry stamps, while sequence_commit alone publishes Meta.commit_seq,
capture, exhaustion and rotation reserve charges. The declared-key path therefore restores entry stamps and
drops the token, effects and LogBudget on abandonment without calling sequence_commit. Accepted Done and Lua
Error publish once before releasing their hold. All script attempts currently use image capture, including
an accepted narrow attempt with one declared key. This is a conservative deferral under firn-script-keys,
not a claim that single-key byte capture is inherently unsafe for that accepted narrow attempt. Selecting
byte capture from numkeys alone would be insufficient because whole fallback may access or change other keys.
Integration must keep one token for an accepted script (or enclosing EXEC), stamp all changed entries with it,
and capture final replacements/tombstones before releasing the hold. Discard an abandoned token and restore
last_write; publish neither capture failures nor reserve charges from abandoned scratch. Account temporary
allocations while owned and return their charge on drop. Do not choose sequence_new_key merely because numkeys
== 1: an accepted whole fallback can modify several keys, and script effects may read other keys. Initially
use sequence_new for all scripts/EXEC, including Lua errors after effects. Reopen the one-key script
optimization when capture evidence distinguishes accepted narrow attempts from abandoned attempts and
whole retries; until then, a growing scripted key still incurs full image copies. Capturing read-only access stamps
takes no dataset sequence. The draft uses the merged contract directly and imports no historical rewrite implementation.

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
needed in-attempt arbitration of "written first" versus "killed first" lacks a pinned Whitefoot contract;
`firn-req-attempt-publish` records the gap,
since nesting a scripts atomic under Meta is forbidden. This work keeps the existing KILL behavior until that interface exists. Do not substitute effects != 0 for Redis's WRITE_DIRTY.

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
transaction, including preceding writes. Correct KILL/busy behavior waits for the existing Whitefoot request
`firn-req-attempt-publish` before claiming full EXEC scripting support.

## Rulings and remaining contracts

1. **Engine ownership:** `firn-q-script-pool` selects any-idle-slot pooling with the configured driver count,
   not driver affinity. The pool is blocked on `firn-req-driver-count` and is not part of the declared-key round.

2. **Written fallback:** `firn-q-script-undo` selects an owned logical Option<Entry> copy saved before first
   mutation, with exact contents and stamps but rebuilt collection backing under the
   [logical restore](#logical-collection-restore) proposed below, rather than a cloned working dataset. The draft conservatively saves an operand before a WRITE
   command that may mutate it, or before a read that can delete its expired entry; no-effect commands may
   allocate an undo copy. Dataset and access restoration precede hold release.

3. **Meta lifetime:** `firn-q-script-meta` selects full-attempt Meta holds for this step. Narrow key holds
   improve the authority boundary but establish no parallel execution or performance result.

4. **Shared Lua state:** Redis-compatible library settings and random state remain required. The single
   engine keeps today's behavior; cross-engine synchronization belongs to the pending pool round.

5. **Controls:** correct accepted-WRITE KILL arbitration waits for `firn-req-attempt-publish`. The recommendation
   for several active engines remains all-or-none: any accepted WRITE makes KILL UNKILLABLE and stops none;
   otherwise kill all, or return NOTBUSY when none run. This round changes no KILL or FLUSH behavior.

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
   firn/scripting/entry.wf into global registry/control and any-idle-engine slots sized by the configured driver count, preserving whole holds first.
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
   commands/transaction.wf, and the merged firn/store/sequence.wf sequence/capture helpers. Persistence/rewrite
   files change only where their publication interfaces require it; memory eviction keeps its contract unless
   the owner approves a change. Do not edit Halo locally; any required portable-state/embed API belongs to the
   Halo session.

6. Extend tests/network.rs and tests/memory_limit.rs, update Redis passing.tsv only from CI, and wire any new
   witness/benchmark caller in the existing Makefile/workflows. Run the registered 14900K comparison; retain
   evidence here and update the deployment-performance conclusion at its source. Review the complete
   implementation and tree correspondence before readiness.

This draft changes no pin/submodule. Firn filed `firn-req-driver-count` and
`firn-req-attempt-publish`; this round files no additional external gap. Declared-key implementation and its
network regressions await CI. Engine pooling, corrected busy-script KILL and metadata concurrency remain
outside this round, and no scaling claim follows from holding Meta throughout attempts.

## Logical collection restore

This work proposes that undo retain COPY's logical deep copy, pending the owner's approval of the design tree. Restore preserves absence, set members, hash
fields and values, sorted-set members and scores, list order, absolute expiry, access and last-write stamps
exactly; collection backing may be rebuilt. This satisfies the current compatibility contract and is not a
Whitefoot gap. The declared-entry hold remains `atomic t = &store^.keys[declared], m = &store^.meta`.

Source inspection found no SSCAN, HSCAN or ZSCAN command in `firn/commands` (case-insensitive whole-name
search). The supported [SCAN](../../../firn/commands/scan.wf#L206) calls `map_scan` on the keyspace map,
which entry restoration does not rebuild. Thus no client collection cursor depends on restored collection
bucket positions. Unordered collection replies do not promise backing order.

Random choices can depend on backing, as [set member selection](../../../firn/commands/sets.wf#L250) and
[hash field selection](../../../firn/commands/hashes.wf#L898) show. Their particular choice cannot be compared
with Redis 7.0.15's different dictionary layout, and firn promises no layout-dependent random result. The
pinned [hash-map interface](https://github.com/Ming-Research/Whitefoot/blob/a3205af0bbea6e8356279f160f3fcd523f085ed1/lib/std/collections/hash_map/module.wfm#L64)
already permits ordinary `hash_map_put` to rebuild at the same capacity when vacated buckets accumulate;
`hash_map_rehash` likewise rebuilds at current capacity. Backing changes therefore occur without scripts.
An abandoned attempt's random picks are never returned to the client and its effects are unpublished;
restoring random state does not promise reproducing picks from that abandoned execution.

Rebuilt capacity can change heap charge, just as rehash can. Maxmemory continues to use process live heap
through [heap_read](../../../firn/commands/eviction.wf#L9), with the existing AOF exclusion and admission
rules; undo introduces no alternative accounting model or fixed-capacity charge.

Reopen if firn adds SSCAN, HSCAN or ZSCAN, or promises a layout-dependent result. Restore must then preserve
what the new command guarantees. An exact layout snapshot is rejected for the present command surface,
not claimed to be expressible by the pinned library. No Halo/Whitefoot edit or pin change is required.
