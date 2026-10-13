# Busy scripts: control progress before timer registration

## Question and current evidence

Board item `firn-bl-01-07` requires BUSY replies after a configurable script
threshold, all-or-nothing SCRIPT KILL, and SHUTDOWN NOSAVE even after a write.
Can the pinned language run the control context beside an unbounded script
that retains its keyspace and metadata statement, including when the real
busy deadline has elapsed before its timer is submitted?

Source inspection on 2026-10-13 used Firn
`2421fe57403ccfe265c225411794f821cb7fcc1f` and the unchanged Whitefoot pin
`wf-af7665be672d`, resolving to
`af7665be672d29912d878dd5da8c3932bb6f296c` (specification v0.146).
The [witness](../../experiments/script-busy-progress/witness.wf) and
[temporary dispatch workflow](../../../.github/workflows/script-busy-progress.yml)
have not been compiled or run. The failure below is a source-inferred trace,
not an observed timeout or compiler diagnostic. Production scripting,
configuration, documentation and the suite ratchet remain unchanged.

The required next result is a supported ordinary source form covering this
boundary, or a Whitefoot fix. Requiring extra drivers, adding unrelated
periodic deadlines, or releasing the dataset between Lua commands would
conceal the missing behavior. Firn's existing expiry sleep may mask the
isolated trace; this investigation does not claim that every Firn execution
currently hits it.

## What the pin already supports

The pinned [prelude and shared-effect rules](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#14-prelude-and-host-modules-normative-counted)
provide DecisionCell with Open, Written and Killed states. Claims settle one
verdict without waiting or exposing intermediate dataset contents.
An accepted WRITE command must claim Written before it executes, even for
`DEL missing`. A separate stop cell is needed for NOSAVE because Written
permanently defeats an ordinary kill claim.

One sticky cell per invocation must survive budget retries and declared-key
fallback. A cell only for the current attempt loses the accepted-write
history of an invocation between attempts. The current single-engine design
can support all-or-nothing KILL under the pool hold: first inspect all active
cells for Written; if none is Written, claim the current invocation before
claiming any others. If that claim observes Written, stop none. Otherwise
the current invocation cannot write, and no other invocation can acquire the
engine while the pool remains held, so claiming the remaining cells is safe.
Registration, removal and current-owner changes must use the same pool hold.
This is an implementation argument, not an implemented change or a claim
about a future pool with concurrent interpreters.

The pin also supports deadline handoff while a script retains its statement.
Its [bounded-waits decision](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/design/compiler/waiting-contexts/bounded-waits.md)
provides one warm spare per driver. An overdue pending timer can transfer the
logical driver's service role to a spare while the original executor keeps
computing. The existing
[held-key handoff test](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/driver_service_test.c#L46)
retains one map entry while the replacement services another. Thus neither
“drivers do not preempt” nor “atomic bodies cannot wait” by itself proves
that busy-script control is impossible.

For a future deadline, the natural structure is an owned script worker and
a synchronous timer in its starter: spawn the worker, sleep until the real
busy deadline with completion cancellation, publish BUSY if time wins, then
join. The pending host call is submitted before the starter suspends; the
driver adopts its wait and registers the timer before running the next
queued context. Merely spawning a timer beside an immediately entered
unbounded statement does not establish this ordering.

## Source-inferred expired-deadline trace

The remaining case is a deadline already reached at submission, possible
after a host scheduling delay even when the configured threshold is positive.
The witness isolates the following sequence with one driver and no unrelated
outstanding deadline:

1. Finite atomic statements bring the parent near its completed-wait fairness
   boundary. They do not change the condition under test.
2. The parent spawns a worker that claims Written, writes a held value, and
   polls a separate stop DecisionCell without leaving its atomic statement.
3. The parent submits an already-expired sleep. This produces a completed
   record, not a pending timer.
4. On the 64th completed-wait pass, the parent yields behind the queued worker.
5. The worker enters its nonwaiting loop. The parent that would claim Killed
   is ready, but no deadline remains to trigger driver reassignment.

The pinned runtime's evidence is:

- [FIFO ready queue](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L1852).
- [Completed-wait fairness](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L1957), which can yield an already-complete host operation.
- [Immediate completion of expired sleep](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L5874).
- [Deadline-only reassignment](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L4721); ready work alone does not activate a spare.
- [Spare executor loop](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/completion/bridge.c#L4826), which waits for an assignment instead of stealing ready work independently.

SHARE-2 forbids a waiting call inside the held loop. WAIT-2's general
progress guarantee is conditional on each context eventually waiting or
finishing, so it does not settle this unbounded-loop case. No new timer API
is prescribed here: the Whitefoot session should first identify any supported
ordinary form or existing guarantee that closes the trace.

## CI comparison and acceptance

Dispatch `script-busy-progress.yml` on the work branch. It downloads the
project's pin and compiles only the witness. The witness subtracts the
executable's argv[0] from args_count (status 6 refuses a missing argv[0]).
User argument count modulo 64 selects
the number of finite warmup waits; counts below 64 use an already-reached
deadline, and counts at least 64 use a deadline one second in the future.
The witness refuses an observed driver count other than one with status 4.
Status 0 means the worker finished, its held value was 1 and its write verdict
was Written. The future-deadline control also requires the parent to observe
Written before requesting stop; otherwise it returns status 5 after stopping
and joining the child. This prevents a delayed, already-expired control timer
from passing merely because the parent stopped a worker that had not begun.
Other statuses fail the observation.

The first sample is a pending-deadline control, then the expired-deadline case
with 63 warmup waits. Only if that sample succeeds does the workflow sweep
the other 63 fairness phases. Each invocation has a five-second **external**
timeout; putting a watchdog inside the Whitefoot process would add the very
deadline whose absence matters. JSON records exits and timeouts as artifacts.
The workflow fails on a timeout; it does not turn the missing progress into
an accepted expected outcome. No timing/performance claim follows from this
hosted-runner bound.

The source-inferred explanation is rejected if the expired-deadline cases
all complete and runtime inspection establishes why the reported interleaving
cannot occur. A failing positive control or compile rejection is separate
evidence to investigate, not proof of this starvation trace. Acceptance of
a supported form or fix requires every fairness phase to return status 0
with one driver, without releasing the held value early or adding an unrelated
timer. Then the real network cases must pass before BUSY is claimed implemented.
Keep the witness while the language boundary remains open; retire the
temporary workflow after evidence is retained and the ordinary gate covers
the chosen Firn implementation.

## Redis 7.2.16 obligations retained for implementation

The verified reference source establishes these requirements:

- `config.c:3222`: busy-reply-threshold and lua-time-limit share one mutable
  integer, default 5000 ms, range 0 through 9223372036854775807 on x86-64
  Linux. Zero disables the Lua interrupt hook; negative values are refused.
- `server.c:1856,4119-4135`: BUSY is exactly
  `BUSY Redis is busy running a script. You can only call SCRIPT KILL or SHUTDOWN NOSAVE.`
- Command JSONs mark exactly AUTH, DISCARD, FUNCTION STATS, FUNCTION KILL,
  HELLO, MULTI, QUIT, RESET, REPLCONF, SCRIPT KILL, SHUTDOWN, UNWATCH and WATCH
  ALLOW_BUSY. PING, CONFIG, INFO, EXEC and SCRIPT FLUSH are not exempt.
  Firn currently lacks WATCH outside MULTI and UNWATCH; this list does not
  claim those features exist.
- `script.c:287-316,550-566`: accepted WRITE flags, including a no-effect DEL,
  make a script unkillable. KILL of an unwritten script must escape Lua pcall;
  no active script gives NOTBUSY. Multi-invocation all-or-nothing is Firn's
  owner-approved extension, not a Redis multi-script mode.
- `db.c:1182-1240`: option syntax and ABORT handling precede SHUTDOWN's busy
  check. Without NOSAVE it returns BUSY; NOSAVE can exit with status 0 and
  no success reply even after script writes.
- `server.c:4393-4403`: NOSAVE still flushes and fsyncs the AOF. Firn's
  existing orderly writer also forces append and sync after clients leave.
  Whether unfinished-script effects remain absent from Redis's AOF is an
  inference from pending propagation (`server.c:3399-3407`), still requiring
  a reference restart case. Do not append them as though the script completed
  before that oracle comparison.

The intended generic command cost remains one read of a shared busy flag;
invocation enumeration, clock reads and kill arbitration belong off that
path. No hot-path code or measurement was changed in this investigation.

These Redis test titles are absent from the current passing ratchet and are
implementation candidates, not newly observed passes:

| Unit | Candidate |
| --- | --- |
| unit/scripting | Timedout read-only scripts can be killed by SCRIPT KILL |
| unit/scripting | Timedout read-only scripts can be killed by SCRIPT KILL even when use pcall |
| unit/scripting | Timedout script does not cause a false dead client |
| unit/scripting | Timedout script link is still usable after Lua returns |
| unit/scripting | Timedout scripts and unblocked command |
| unit/scripting | Timedout scripts that modified data can't be killed by SCRIPT KILL |
| unit/scripting | SHUTDOWN NOSAVE can kill a timedout script anyway |
| unit/multi | MULTI and script timeout |
| unit/multi | EXEC and script timeout |
| unit/multi | MULTI-EXEC body and script timeout |
| unit/multi | just EXEC and script timeout |

The last three scripting cases carry external:skip in the reference suite;
the unblocked-command case also needs DEBUG. Scripting runs in both EVAL and
FUNCTION modes, so EVAL support alone does not establish every occurrence.
`hung.tsv` currently excludes “just EXEC and script timeout” for waiting on
the missing BUSY reply. Neither that exclusion nor `passing.tsv` changes
until the corresponding CI evidence exists.

## Read-only review

An independent gpt-6-astra reviewer inspected the complete three-file draft
against the unchanged Firn baseline above, the relevant design nodes, the
project checklist, pinned Whitefoot source and Redis source. The preferred
gpt-6.1-sol reviewer could not start on two attempts because it was at capacity.
The review found and rechecked two repairs: the future control now observes
Written before stopping the worker, and argument selection subtracts argv[0].
There were no remaining findings within that scope. Pin validation, witness
compilation/execution and project gates remain unverified; the proposed
cross-project request has not been filed from this worktree.
