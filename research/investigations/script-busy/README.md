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
The owner reported the [CI witness run](https://github.com/Ming-Research/Firn-wf/actions/runs/38399206461):
with `WF_DRIVERS=1`, the future-deadline control returned status 0 in about
one second, while the expired-deadline sample hit its five-second external
timeout. The owner filed P0 Whitefoot request `firn-req-single-driver-progress`
for progress of other contexts beside a long computing atomic statement on
one driver. This worktree could not independently retrieve the run through
GitHub's API; these outcomes are the owner's supplied CI evidence.
The [witness](../../experiments/script-busy-progress/witness.wf) remains as
the semantic reproducer. Its temporary workflow has been removed after this
result; it is not part of Firn's gate.

The owner authorized implementing the ordinary multi-driver execution path
while Whitefoot closes that request. Firn adds no driver-count branch,
unrelated polling context, spin-yield or released dataset hold. Network
coverage uses at least two drivers and retains an ignored one-driver case
named for the request, ready to enable when the fix lands. Firn's existing
expiry sleep may mask the isolated trace; this investigation does not claim
that every one-driver Firn execution currently hits it.

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
The draft implementation follows this argument; its compilation and network
acceptance remain unverified. It makes no claim about a future pool with
concurrent interpreters.

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

## Expired-deadline trace reproduced in CI

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

The retired dispatch workflow downloaded the project's pin and compiled
only the witness. The witness subtracts the
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

The recorded first sample is a pending-deadline control, then the expired-deadline case
with 63 warmup waits. Only if that sample succeeds does the workflow sweep
the other 63 fairness phases. Each invocation has a five-second **external**
timeout; putting a watchdog inside the Whitefoot process would add the very
deadline whose absence matters. JSON records exits and timeouts as artifacts.
The workflow failed on a timeout; it did not turn the missing progress into
an accepted expected outcome. No timing/performance claim follows from this
hosted-runner bound.

The source-inferred explanation is rejected if the expired-deadline cases
all complete and runtime inspection establishes why the reported interleaving
cannot occur. A failing positive control or compile rejection is separate
evidence to investigate, not proof of this starvation trace. Acceptance of
a supported form or fix requires every fairness phase to return status 0
with one driver, without releasing the held value early or adding an unrelated
timer. Then the real network cases must pass before BUSY is claimed implemented.
Keep the witness while the language boundary remains open. The temporary
workflow is retired; the real ignored network case retains the acceptance
check for Firn when Whitefoot resolves the request.

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
  Unfinished-script effects remain unpublished in the draft, following
  pending propagation (`server.c:3399-3407`). The new network restart case
  checks earlier completed writes survive but two writes from the interrupted
  script do not replay. An actual reference restart comparison is still
  unverified; this source-based expectation must not be reported as an
  observed oracle result.

The normal admission path now makes one short pool observation of busy and
running state. It reads no clock, enumerates no invocation and starts no
timer. Only a valid disallowed command that observes an active script below
its threshold enters a second, guarded pool statement; timer publication or
script completion wakes it. This is a source-level cost account, not a
14900K measurement.

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

## Multi-driver implementation and remaining validation

The implementation retains the single shared engine. Each invocation owns
its DecisionCell and a separate forced-stop cell, registered from first
engine checkout until final completion. Interpreter execution resumes in
slices of at most 1000 Halo safepoints and checks both cells at outcome
boundaries; this is interpreter interrupt handling, not a runtime yield or
an extra polling context. Read-only attempt budgets still grow across
retries, and accepted-write history survives them. One cancellable timer, started immediately before the first prepared
attempt takes its dataset hold, publishes BUSY at the invocation deadline and is cancelled before its
activation is joined at return.

BUSY-safe admission bypasses eviction and peak publication. AOF health
publication updates Meta and its refusal-text copy in ServerState together,
so busy admission preserves disk-error precedence without entering held
Meta. NOSAVE sets a sticky stopping state in the pool before terminating
invocations; commands waking as the interpreter releases its holds still
observe BUSY until shutdown. The short force-stop statement publishes server shutdown together with pool
stopping before claiming the forced-stop verdicts. The interpreter also
aborts active rewrite under its existing Meta hold before releasing modified
keys. Thus a rebuild cannot capture an unfinished deletion between the
interpreter exit and the requesting client's ordinary wake publication.
The existing orderly writer flushes and syncs
previous completed propagation, while incomplete script effects and
sequence/capture scratch are discarded.

Four enabled network cases use two drivers: configuration aliases and zero;
read-only BUSY controls and uncatchable KILL; no-effect WRITE refusal and
NOSAVE; and AOF restart after an interrupted writing script. The ignored
one-driver case runs the same control and writer helpers and names
`firn-req-single-driver-progress`. None has been compiled or run in this
worktree; the primary session must push this draft and return CI results.
The normal gate, AOF fault check and independent review are required before
completion; the suite candidates above are not added to the passing list
without their own CI evidence.

The deadline includes any wait to acquire that first attempt's dataset and
Meta holds after preparation. Redis takes no such concurrent dataset holds,
so its interpreter-start boundary has no corresponding waiting interval;
this difference needs a contention witness and disposition before claiming
identical timeout timing under concurrent commands. Moving a waiting spawn
inside the atomic statement would be refused by SHARE-2, and is not used.

An admission crossing remains a review question: a command can observe no
running script before another connection checks out the engine and takes
its keys or Meta. The already-admitted command can then wait behind the
script, even after BUSY publication. Cold memory-peak publication now
uses a guarded pool-and-Meta statement to close that local crossing, and
SCRIPT FLUSH's own pool guard wakes on BUSY as well as completion. General
admission-to-key or admission-to-disk-check crossings remain open. The guarded path covers commands that
observe the running invocation, not a retroactive cancellation of acquired
waits. No global command turn or unsupported runtime workaround is added.
The owner clarified that this ordering is not permitted: the command must
finish before script execution or receive BUSY after the threshold. The
follow-up below isolates the missing lock-or-BUSY behavior rather than
adding a global turn or polling context.

## Read-only review

An independent gpt-6-astra reviewer inspected the complete three-file draft
against the unchanged Firn baseline above, the relevant design nodes, the
project checklist, pinned Whitefoot source and Redis source. The preferred
gpt-6.1-sol reviewer could not start on two attempts because it was at capacity.
The review found and rechecked two repairs: the future control now observes
Written before stopping the worker, and argument selection subtracts argv[0].
There were no remaining findings within that scope. Pin validation, witness
compilation/execution and project gates were unverified at that review.
The later CI result and owner-filed request are recorded above. This earlier
review does not cover the production implementation added afterward.

## Review of the implementation draft

An independent gpt-6.1-sol reviewer at high effort reviewed the complete
tracked diff from `ea5108f96b2ee412a1152c94ea328023b5b35559` plus the new
interpreter control file, changed sections and direct consumers, project
checklist, design ancestors, pinned language and Redis reference source.
A focused follow-up reviewed the peak and FLUSH repairs. Review operations
were reads, searches and Git inspection only. The primary inspected the
findings and fixes in their source context. No compiler, test, gate or
performance run occurred in this worktree.

Checklist dispositions: A1, C1, T1, T2, D1, G1, G2, DC1 and DC3 pass within
source scope; R1 is not applicable; C2, T3 and G3 are unverified. DC2 and
DC4 remain findings for the unresolved admission crossing; DC4 also lacks
runtime acceptance and exact contention-timing evidence. The KILL
all-or-nothing argument and timer cancellation/join ordering were examined,
but multiple active invocations and first-write races have no dynamic
acceptance evidence yet.

Fixed findings: the forced-exit/rewrite race now publishes shutdown before
forced-stop claims and aborts rewrite under Meta; UNKILLABLE uses the exact
191-byte reference text; WRITE_DIRTY descriptions now agree with sticky
invocation history; configuration glob expectations include both new names.
The peak cold path uses a pool-and-Meta guard, so timeout skips Meta before
its first use and a new script cannot cross its nonbusy publication. FLUSH
wakes on BUSY and leaves registry/generation untouched when refusing it.
Focused source review found no further issue in these repairs. Their
compilation, lock behavior and regression results remain unverified.

Two findings remain: F1 (P1), a command already admitted before script
registration may block at eviction, disk admission or its key operation
without a BUSY wake; and F3 (P2), the deadline includes initial hold
acquisition, unlike Redis's interpreter-start timing. The primary session
must choose and record their disposition and obtain CI evidence; this
worktree does not claim the requested BUSY milestone complete. No global
admission turn, interruptible-wait workaround or driver-count path was
introduced. Board MCP tools are unavailable in this session, so these
additional dispositions have not been filed from this worktree. The
owner-filed single-driver request remains the only confirmed filed gap.

The owner committed that implementation as
`9c25e9395e2520ad13aa46274e290a09f5adf826` and reported its gate running.
Whitefoot pin and submodule revisions were unchanged. The earlier temporary
progress workflow is removed; its witness remains under experiments, cited
above. The following boundary investigation is a new, uncommitted delta.

## F1/F3 follow-up: required forms and pinned limits

### Question and scope

Can the pinned language express both required behaviors without polling
contexts: a command waiting on a script-held key finishes on BUSY instead
of waiting forever, and the deadline starts after the first dataset hold is
acquired, immediately before interpreter execution? The follow-up uses
Firn `9c25e9395e2520ad13aa46274e290a09f5adf826` and unchanged Whitefoot
`af7665be672d29912d878dd5da8c3932bb6f296c`. No production code is changed
in this delta. No new witness has been compiled or run; the conclusions
below are source-derived, separate from the earlier observed one-driver
failure.

### F1: BUSY cannot bypass an indexed target acquisition

The complete [key-or-BUSY witness](../../experiments/script-busy-lock/witness.wf)
uses at least two drivers. A writer takes key x, writes its value, publishes
Written through a DecisionCell and continues computing while retaining the
key. The starter submits a genuine future sleep and then checks Written,
so it refuses a run in which the writer never began. It publishes an
ordinary shared BUSY flag before entering the reader's guarded keyed
statement. The reader's BUSY branch does not read or write the held key;
the writer is stopped only after the reader returns. Thus even an already
true BUSY condition must be able to finish without acquiring that key.
This stronger, minimal condition avoids ambiguity about exactly when a
protocol GET reaches its lock wait.

The pin acquires the entry before inspecting that independent guard:

- [Indexed and key-set targets are eager groups](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L123).
- [Eager acquisition precedes guard evaluation](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L744).
- [Guard notifications are registered only after acquisition and a false guard](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/atomic.rs#L765).
- [The emitted entry lock has no cancellation or other-object notification input](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/emitter/shared.rs#L526).
- [A map acquisition does not park its context](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/keyed_table.c#L314).

Accordingly, adding BUSY to a keyed guard alone cannot implement F1 on this
pin. Replacing entries with a whole-map target merely to change acquisition
would conceal poor compilation of the natural keyed form and is not used.
The current specification permits coarse target holding, and WAIT-2's
conditional progress premise does not cover an infinitely computing holder;
this finding is a missing supported guarantee, not yet proof of a compiler
violation of the existing specification. Inspection establishes that the
requested guarded keyed form cannot select BUSY on this pin; it does not
prove that no other ordinary composition could ever express the requirement.
The Whitefoot request must first ask for such a supported form, or a change
that supplies it.

Whitefoot must provide a supported form whose guard/control alternative can
complete while an unused keyed target remains held, including after an
acquisition has already begun. No particular new API is prescribed. It must
preserve ordinary key isolation and the single-point meaning of dataset
state, without releasing the script's holds or requiring a polling helper.

### F3: the held interpreter start cannot arm its owned timer

The complete finite [interpreter-start witness](../../experiments/script-busy-lock/start-deadline.wf)
first uses its held entry, samples the monotonic clock there, and attempts
an expression-statement spawn of the existing owned cancellable timer.
After the statement it cancels the timer before activation exit and its
implicit join. This is the natural placement of the real start; it passes
only owned timer inputs and no reference to the parent body.

`now` is nonwaiting and is permitted in the body. However, the pin treats
the timer spawn as a waiting call because its callee waits, and SHARE-2
refuses it before spawn-specific checking:
[checker](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/semantic/check/expressions/calls/user.rs#L254).
The maintained conformance case
[waiting spawn inside atomic](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/tests/conformance/cases/wait3-neg-inside-atomic.wf)
expects SHARE-2. The new witness is predicted to fail for that rule; no
actual diagnostic has been collected for it.

Moving the timestamp to another ordinary shared target holds that target
until the interpreter finishes; it does not publish the start to the timer.
DecisionCell carries only a verdict, not the timestamp, and its reads are
forbidden in guards by the maintained
[decision-cell guard case](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/tests/conformance/cases/decision-cell-neg-guard-read.wf).
The time interface has no nonwaiting alarm registration. Encoding time in
verdict cells or adding a context that polls for it is not an accepted form.

The required result is a supported way to arm an owned deadline from this
held start without yielding its creator or releasing the dataset. Supporting
this activation-scoped expression spawn is one candidate; a supported
nonwaiting timer registration is another. Selection belongs to Whitefoot.

### Before-run criteria and regression coverage

The [temporary boundary workflow](../../../.github/workflows/script-busy-lock.yml)
runs on the work branch or explicit dispatch. It compiles with lock-ledger
output and preserves compile logs. The key witness requests four drivers
and refuses an actual count below two. First its no-argument control must
exit 0 after confirming the writer began, stopping it, joining it and
checking the retained value. Only then does the reader sample run. The
reader must exit 0 within the external five-second bound while the writer
still owns x; a timeout fails acceptance. It has no internal watchdog that
could accidentally release the key. A failure to compile, missing start
(status 5) or control failure is not evidence of a lock-or-BUSY failure.

The separate start job requires the natural timer form to compile and its
finite program to exit 0 within five seconds. An unrelated diagnostic is
not evidence for F3. A SHARE-2 diagnostic at the timer spawn confirms the
reported rejection, but still fails desired acceptance; the workflow does
not mark rejection or a hang as a pass. These bounds distinguish completion
from failure and are not performance measurements. Retire the workflow
once this evidence is retained and normal Firn coverage validates the
supported replacement.

The new network case caches an infinite script that writes its declared
key, submits GET before the configured 1000 ms deadline (measured from
before EVAL submission, a lower bound on the real deadline), and
requires the exact Redis BUSY reply within a three-second socket bound.
Initial old-value replies may precede script admission and are retried only
inside that original interval. It then uses NOSAVE to stop the writer and
checks the captured GET result after cleanup. This is real protocol
coverage, but cannot prove when the server actually admitted GET or entered
its indexed wait. It may pass the existing cold admission guard without
exercising the crossing; it is not a deterministic proof that F1 is fixed.
The semantic witness supplies the held-key/start observation instead.

The source explanation is rejected if the exact key witness compiles and
returns 0 with the holder still retained, and inspection explains how the
busy alternative bypasses the lock. F3's conclusion is rejected if the
natural held-start form compiles and satisfies its cancellation/join
criteria, or Whitefoot identifies another supported ordinary form with the
same timing and isolation. Only after those boundaries are settled may
production F1/F3 be changed and accepted with the normal gate and relevant
network and contention cases.

Work stops here under the owner's no-workaround instruction. The two
additional Whitefoot requests have not been filed from this worktree because
board MCP tools are unavailable. The primary session must retain the CI
artifacts and file the witness, acceptance and exact supported-form question
into Whitefoot. No local compiler, suite, benchmark, commit or approval log
was run or written; production, pin, submodules and ratchet lists remain
unchanged in this follow-up.

### Follow-up source review

An independent gpt-6.1-sol reviewer at high effort inspected the full
five-file delta from `9c25e9395e2520ad13aa46274e290a09f5adf826`, including
the two untracked witnesses and temporary workflow, against pinned language
and runtime sources, Redis source, the project checklist and relevant design
ancestors. Operations were read-only. It found one reversed deadline-bound
phrase, now corrected: pre-EVAL time plus the configured threshold is a
lower bound on the real deadline. Final source inspection found no remaining
finding within that scope.

A1, C1, C2 (unchanged interfaces/contracts), static T2, D1, G2, DC1 and DC2
pass; T1, R1, G1 and DC3 are not applicable. T3, G3 and DC4 remain
unverified. Neither source witness, the GET case nor the new harness has
been compiled or run, and the harness's rejection of wrong results/missing
inputs has no executed fault-injection evidence. The reviewer confirmed
that F1's probe tests the already-true BUSY alternative, not the later
notification race, and that the network case cannot establish actual waiter
admission. These limits remain part of the acceptance evidence.

## Existing second-signal case after BUSY admission

The owner reported [gate run 38406825693](https://github.com/Ming-Research/Firn-wf/actions/runs/38406825693)
on the implementation revision: the second-stop-signal network case failed
at its registry observation, expecting `*1\r\n` and reading `-BUS`.
That assertion reads exactly four bytes, so the diagnostic shows the prefix
of the BUSY reply, not a separate truncated protocol result. The case sends
`SCRIPT EXISTS` for the SHA1 of its infinite `while true do end` EVAL. It
sets no script threshold, so the default is 5000 ms. The BUSY path implies
that at least 5000 ms had elapsed since the prepared-attempt timer was
started;
the supplied failure does not establish precise interpreter duration or CPU
time, and the outstanding F3 lock-wait qualification still applies.

Redis 7.2.16's `src/commands/script-exists.json` gives EXISTS only NOSCRIPT,
not ALLOW_BUSY. Its `server.c:4125` rejects that command while the script is
timed out, and `script.c:88` compares elapsed time with the threshold.
Firn now queues nonexempt commands below the threshold; with this infinite
script a larger threshold delays the BUSY result rather than producing the
array on which the old test depended. No production exemption is added.

The fixture now explicitly sets the threshold to 100 ms before EVAL, then
uses a complete exact BUSY response to the same SCRIPT EXISTS query as its
live-script barrier. An initial cache array may precede script execution and
is fully consumed, but even presence 1 is not accepted as readiness. The
observer then remains idle. Both backend routes, the first signal's EOF,
the assertion that the process is still draining, and the second TERM/INT
signal's exact exit status remain unchanged. This changes the preparation
observation, not the signal behavior being checked. The network case uses
its existing watchdog and socket bounds; it assumes no finite Lua-loop
execution time.

The original failed assertion supplies before-change evidence. The modified
case has not been run in this worktree; its CI result remains required.
Only this test and the investigation record change, with no compiler,
production, pin, submodule, ratchet or approval-log edits and no commit.

An independent gpt-6.1-sol reviewer at high effort inspected this two-file
delta from `cdc5c1011f1a848321c8034574d1a9f8ac4d2a45`, the existing signal
assertions and relevant design and Redis source. It confirmed full reply
consumption and unchanged signal coverage. One ambiguous elapsed-time
phrase was corrected and rechecked; no findings remained within source
scope. Static checklist items passed or were not applicable; final gate and
behavioral acceptance remain unverified.
