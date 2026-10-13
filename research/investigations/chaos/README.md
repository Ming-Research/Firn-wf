# Concurrent workload under faults

Board: `firn-safety-showcase`, `chaos-tool` and `chaos-run`.

## Question and rejection criterion (before measurement)

Does firn preserve a serial execution and every acknowledged effect while
many clients contend, connections disappear, storage fails, and the process
is killed and restarted? Run the same seeded workload separately against
firn, Redis 7.2.16, Valkey, Dragonfly, and Garnet. This is a correctness
experiment, not a throughput comparison or a power-loss simulation.

The owner's proposed acceptance is 86,400 wall-clock seconds on the i9-14900K, zero
unrequested firn exits, zero inconsistencies, and zero lost acknowledged
writes, with checks consuming at most 10% of reported wall time. Report actual
workload time separately; 24 hours does not mean 24 hours of active traffic.
An injected SIGKILL is not an unrequested crash. A missing server,
skipped storage fault, incomplete history/check, failed restart, lack of
successful writes, or shorter run cannot establish that acceptance. First
run the 120-second hosted smoke mode, inspect its time and coverage, then
choose the long-run scale. The original zero-exit, zero-inconsistency,
zero-loss criterion and workload were written before the first measurement.
The check-time budget and revised scheduling are follow-up requirements
from review of the short run, not predictions made before that run. The
first hosted observation is recorded under Results.

**Known expected failure:** before [append before reply, Firn-wf#97](https://github.com/Ming-Research/Firn-wf/pull/97)
lands, firn can reply before its AOF record reaches the file. SIGKILL can
therefore lose approximately the last 10 ms of acknowledged writes. A run
explicitly identified as lacking that fix labels restart losses as the
known acknowledgment-before-append failure, not a newly discovered defect.
It still fails acceptance. Unknown fix status never silently assumes the
fix is present. Classification uses the lost request's timing and the
actual fault intervals, not whether an EIO occurred anywhere earlier in
the run. A request overlapping an active EIO interval is not attributed
solely to the acknowledgment-before-append window.

## Workload and fault schedule

The default is 32 clients, four shared key groups, and a 250 ms pause per
client operation. The pause bounds history and AOF growth for a day; clients
remain concurrent and contend on the same four groups. Each operation has
a globally unique 16-byte ASCII token. Its seeded kind is one of:

* APPEND the token to the group's journal;
* MULTI; APPEND journal token; SET register token; EXEC;
* MULTI; APPEND journal token; INCR counter; EXEC;
* MULTI; APPEND journal token; SET register token; INCR counter; EXEC.

SET and INCR deliberately share a transaction with the journal operation:
this makes an independent, tractable witness possible even when an EXEC
reply is lost. They are not claimed to cover arbitrary Redis histories or
standalone ambiguous INCR operations. Shared registers may overwrite older
SETs; the journal retains their acknowledged effects. There is no expiry,
eviction, deletion, or acknowledged-data retirement in this workload.

Each epoch uses fresh key names, so histories in memory are bounded. Closed
epochs remain in the server and their independently checked expectations
remain outside the faulty filesystem. After each persistent-server restart,
check epochs created since the last full recheck, including the newest closed
epoch. Only the current epoch's keys are mutated by traffic. The first
checkpoint is full; later checkpoints become full when time since the last
full check finished reaches the greater of 60 seconds and 19 times its
measured duration. A full check replaces that restart's incremental check.
Always finish with a full check, unless the last in-loop full check already
covered every closed epoch and no further traffic occurred. Loss of older
data remains a failure, detected at the next full check rather than necessarily
the immediately following restart. Never reset the database between epochs.
Record wall, workload, and check time and operation counts; this is no fixed
throughput promise. Provision space for both AOF and history before a day
run; the workflow's larger sparse fault device is not the existing 64 MiB
device's default. Unexpected exhaustion of artifact storage is a harness
failure, never a clean result.

The deadline is harness-start time plus `CHAOS_SECONDS`, including startup.
No new epoch begins at or after that deadline. An already-started fault cycle
may complete its bounded three-second fresh-acknowledgment wait, kill, restart,
and mandatory final verification afterward. Report actual wall time including
this tail, workload time while writers may issue requests, and complete
checkpoint time; joins, restart, and cleanup are separate wall-time overhead.
Do not pad an early failure to the requested duration. A day run exceeding
the 10% check-time budget fails that acceptance even if its functional checks
pass. The following estimate is a forecast, not execution evidence.

Seeded epochs last 15–25 seconds of active traffic. One in sixteen requests
is sent and its connection closed before reading a reply; reconnect before
the next operation. Other clients continue. At the planned end of an epoch,
the controller waits at most three seconds for a fresh complete write
acknowledgment and then sends SIGKILL with writers still running, followed
by joining writers and restarting from the same files. Record the actual
age of the last acknowledgment when the signal is sent; it is not assumed
to be within the known approximately 10 ms window. A disk-refusing server
may produce no fresh acknowledgment: record the timeout and kill it without
flushing. A healthy epoch with no fresh acknowledgment reports no progress.
Do not flush or wait for the AOF before the kill. Every
third epoch injects EIO for three seconds while clients continue, through
`tests/aof_sync_fault_device.sh fail/heal`; the other epochs isolate process
crashes. With no `CHAOS_FAULT_MOUNT`, report disk faults skipped. Full-disk
ENOSPC is not additionally injected: this experiment selects the requested
failing-disk alternative. Heal before restart; a panic also attempts heal.
The mount is never reset between a server's epochs.

Dragonfly has no AOF in this comparison. It receives connection drops and
concurrent traffic, is checked before each deliberate restart, then begins
an explicitly new volatile lifetime. Its old lifetime's data is excluded
from restart durability, labelled not applicable rather than passing.

## Independent checker

Record each request's complete commands, monotonic invocation/completion
times, complete RESP replies (including transaction queue replies), client,
epoch, token, and outcome. Also record fault intentions and actual times,
startup command and executable identity, configuration, restart results,
snapshots, checker findings, and skipped coverage. Keep these on healthy
storage. Each checked epoch logs one compact SNAPSHOT summary: key count and
a 64-bit standard-library hash of the observed journal/register/counter values.
The digest is diagnostic only: exact values and the independent checker decide
correctness, so a hash collision cannot conceal a mismatch. Dump complete
observed values and RESP bytes only on mismatch or malformed input. Retain
one exact independently validated baseline per epoch as the continuity
check's expected input, rather than repeatedly dumping clean observations.
Each request still records its own timestamps and complete reply.
A complete successful APPEND or EXEC reply is an acknowledgment;
partial frames and deliberately unread replies remain unknown, never
successes and never definite non-execution. Do not retry an ambiguous token.

Reuse `tests/durability.rs`'s APPEND witness by moving its pure checker into
a shared module. Split the observed journal into fixed-width tokens. Each
must name exactly one submitted operation on that key; reject duplicate,
unknown, rejected-operation, or malformed tokens. An unknown request may
have no effect or one complete effect. Infer its APPEND rank only from its
unique token, and keep its completion interval open until the quiescent
checkpoint. A successful request retains its actual reply rank and response
time. The existing witness checks contiguous ranks, exact token order, and
whether rank order fits invocation/completion intervals. Different groups
compose because no transaction touches two groups.

Walk that witness with a small model: SET changes the register to its token,
INCR adds exactly one, APPEND-only changes neither. Require every successful
transaction's SET reply to be OK and INCR reply to equal the model's count
at that rank. Final GETs must equal the modeled register and count. Thus
partial transactions, duplicate effects, contradictory replies, and
impossible real-time orders fail. Missing acknowledged tokens are reported
separately as lost writes, without treating an unknown request's absence
as loss. Expectations come from client requests/replies and Redis command
semantics, never from parsing the server's AOF. A quiescent snapshot is
evidence to validate, not the oracle.

After a closed epoch first passes that independent check, retain its exact
state as a continuity check as well. An unknown request observed once cannot
later disappear or appear in a different position without an inconsistency.
This observation does not retroactively become a client acknowledgment.

Disk-error MISCONF refusals and EXECABORT during a disk-failure/recovery
window are recorded as refused operations. Other server errors, malformed
or partially successful transaction replies, unrequested exits, transport
failures outside injected fault windows, or inability to resume successful
writes fail the run. Normal cleanup kills are recorded separately.

A server that fails readiness after an injected kill is a server finding
of kind `refused restart`, with its name, startup failure, bounded stdout
and stderr tails, and preceding fault records. Stop that server's run with
`status=failed` and a complete report; this expected failure path does not
become a harness panic. Report initial startup failures separately.

The final report always carries partial coverage, including on early
failure: epochs started/completed, SIGKILLs sent and reaped, restart attempts
and completed recoveries, write-EIO windows, flush-EIO windows, ENOSPC
injections, connection drops, and skipped faults with reasons. The device's
`fail` action arms write and flush EIO in the **same** window; two such
windows mean two write-EIO and two flush-EIO injections, not four independent
fault scenarios or a count of failed syscalls. Setup probes and the separate
writeback self-check are excluded from workload injection counts. ENOSPC
and independently isolated flush-only failure remain explicitly unselected.
With a device, a 120-second-or-longer smoke cannot pass without at least one
completed SIGKILL restart, one combined EIO window, and one connection drop.

Pure negative self-tests include a lost acknowledged token, duplicate token
effect, duplicate reply rank, impossible non-overlapping order, wrong INCR
reply/count, wrong SET value, and a partial transaction. Positive cases
include overlapping operations, an unknown operation present once, and an
unknown operation absent. Existing APPEND negative tests remain wired to
the ordinary gate; new pure tests use its existing Cargo invocation. The
long-running test is ignored and never added to `make check`.

Use one atomic MGET for an epoch's journal, registers, and counters, so
Dragonfly's pre-kill observation cannot tear across a late unknown request.
After startup, require PING/PONG before checking restored data: Redis can
listen while still loading its AOF. `CHAOS_RESTART_SECONDS` bounds both
listener and command readiness (default 300 seconds); record startup time
and fail when recovery does not complete within that bound.

## Competitor guarantees and interpretation

Redis is exactly 7.2.16 built by `tests/redis-suite/reference.sh`, checked at
startup. Redis and Valkey run AOF with `appendfsync everysec`, automatic
rewrites disabled, and snapshots disabled. Report any lost acknowledgment,
but label it an observation under their weaker everysec policy rather than
a breach of an always-sync guarantee. SIGKILL retains the OS page cache;
storage EIO adds a different failure from simply losing unsynced bytes.
[Valkey's persistence documentation](https://valkey.io/topics/persistence/)
describes its everysec policy; it is not a promise of zero loss here.

Dragonfly uses the benchmark's snapshot-disabled invocation, with restart
durability not applicable. Garnet runs AOF enabled and recovery requested;
its exact command and executable identity are artifacts. Its installed
version's AOF commit semantics are **unverified**. The
[Garnet configuration reference](https://microsoft.github.io/garnet/docs/getting-started/configuration)
documents commit-frequency and wait-for-commit settings, but their behavior
must be checked on the installed binary before making a durability claim.
Unsupported flags/startup failure are reported failures, never a fallback
to nonpersistent operation. No competitor's weaker guarantee changes firn's
zero-loss acceptance. Missing competitor executables are explicit skips
and leave the all-server comparison incomplete.

## CI and ownership

Only CI builds or runs this experiment. The dispatch-only chaos workflow
selects servers, duration, seed, and hosted or 14900K runner. Servers run
sequentially; requested servers still run after another server fails. A
hosted run is a short functional probe, not the day-long acceptance.
Histories and reports upload even on failure, and device teardown runs
even after partial setup. A device setup/probe failure with sudo available
is infrastructure failure, not an unsupported-host skip.

The device helper owns one global mapper and changes machine-wide writeback
settings. Refuse an existing device; never tear down another run's device.
Its saved state includes the owning run's unique mount path, checked before
fault injection or cleanup. Both setup and reset initialize inode tables
eagerly: ext4 can otherwise write those tables in the background, separately
from the periodic flusher disabled by the helper. This follows the
[kernel's lazy-initialization description](https://www.kernel.org/doc/html/v6.6/filesystems/ext4/overview.html#lazy-block-group-initialization);
the existing CI device probe and writeback check still have to verify the
actual device behavior.
The 14900K must be reserved through the coordinator for a long run. Reuse
the benchmark's recorded-session cleanup for cancellation, plus the test
support's port leases, monitored child processes and bounded I/O. The
workflow reports disk-fault coverage unavailable when passwordless sudo is
absent. That limitation blocks the full acceptance, not the short probe.

Dispatch `.github/workflows/chaos.yml` with `servers=firn,redis`,
`seconds=120`, `seed=1`, and `runner=github` for the first probe. Set
`firn_append_before_reply=absent` for a revision without the append-before-
reply fix, `present` only after verifying that revision contains it, and
`unknown` when its status has not been established. The workflow builds
the separate `chaos` Cargo test target and runs its ignored `chaos_run`
test; pure checker tests also run through the existing network gate.

The harness accepts `CHAOS_SERVER`, `CHAOS_SECONDS`, `CHAOS_SEED`,
`CHAOS_CLIENTS`, `CHAOS_OUT` (required healthy artifact directory), optional
`CHAOS_FAULT_MOUNT`, `CHAOS_FAULT_SKIP_REASON` (the workflow supplies the
passwordless-sudo skip reason), `CHAOS_RESTART_SECONDS`, and
`CHAOS_FIRN_APPEND_BEFORE_REPLY`. `FIRN` and
`REDIS_SERVER` are absolute executable paths; optional `VALKEY`,
`DRAGONFLY`, and `GARNET` override the binaries on PATH. The workflow adds
`~/.local/bin` to PATH. A direct invocation belongs in CI too, after builds:

```sh
CHAOS_SERVER=firn CHAOS_SECONDS=120 CHAOS_SEED=1 \
  CHAOS_OUT="$RUNNER_TEMP/chaos-probe" FIRN="$GITHUB_WORKSPACE/build/firn-test" \
  cargo test --manifest-path tests/Cargo.toml --locked --test chaos -- \
  --ignored --exact chaos_run --nocapture --test-threads=1
```

The all-server day comparison requires five sequential jobs, each with its
own 86,400-second wall-clock budget, and a reserved 14900K. Inspect the
smoke artifacts before scheduling it. The optional device size is 32 GiB
in this workflow; the existing fault suite retains its 64 MiB default.

## Results

### First hosted observation, 2026-10-13

[Actions run 38438582178](https://github.com/Ming-Research/Firn-wf/actions/runs/38438582178)
ran revision `e62e5ec9aa15ed20acf3701c70e380455b8f32a1` with 120 seconds,
seed 1, 32 clients, and the fault device, separately for firn and Redis
7.2.16. The hosted machine reported an AMD EPYC 9V45 and four logical CPUs;
this is not the proposed i9-14900K day acceptance. Both jobs built the
harness and passed its 24 tests (14 chaos fixtures and 10 shared support
tests). These artifacts provide execution evidence for that revision;
later observations below establish subsequent corrections only on the
revisions they name.

| Observed workload coverage | firn | Redis 7.2.16 |
| --- | ---: | ---: |
| Elapsed seconds | 120.511 | 62.521 |
| Epochs started / completed | 6 / 6 | 3 / 2 |
| SIGKILLs sent / signal-9 exits reaped | 6 / 6 | 3 / 3 |
| Successful post-kill restarts | 6 | 2 |
| Combined write-and-flush EIO windows | 2 | 1 |
| Connection drops | 951 | 484 |
| Acknowledged writes | 13,466 | 6,938 |
| ENOSPC injections | 0 (unselected) | 0 (unselected) |

The firn job reported `passed-functional-probe` with zero findings. Its
history confirms EIO windows at 57.524–60.581 seconds and
115.142–118.198 seconds, followed by successful recovery checks, and six
SIGKILL/restart cycles. The schedule was frequent enough to exercise the
selected fault kinds in two minutes; no shorter epoch cadence is needed.
Valkey, Dragonfly, and Garnet were not selected for this run.

The zero-loss observation has a narrower limit than the log's old phrase
"while writers active" suggested. No recorded request interval spanned a
kill. In the first five firn epochs, the last acknowledged reply preceded
SIGKILL by 216–227 ms; in the last epoch it preceded SIGKILL by 46.500 ms.
Thus this run did not exercise the known approximately 10 ms
acknowledgment-before-append window, and does not show that the gap is
closed. The follow-up controller waits for a fresh acknowledgment before
the signal and records its actual age, preserving the original zero-loss
criterion.

**First Redis observation:** Redis 7.2.16 with `appendfsync everysec`
reported `Fail to fsync the AOF file: Input/output error` in the third
epoch. After the device healed and the server was SIGKILLed, its next
startup exited with status 1 and reported `Bad file format reading the
append only file appendonly.aof.1.incr.aof`. The raw evidence is in the
Redis artifact's `server-2.stdout`, `server-3.stdout`, `history.log`, and
third epoch archive. The old report recorded a panic naming firn and zero
findings; that was a harness classification/reporting defect. The third
epoch's recovery check never ran, so zero findings cannot be read as no
lost or inconsistent writes in that epoch. The corrected report treats this
outcome as Redis's `refused restart` finding and retains partial coverage.

It is **unverified** whether the malformed AOF arose from the fault
device's partial or failed data writeback, filesystem metadata damage, or
Redis's own record/error handling. The setup probe confirms buffered
`write(2)` succeeds while new-data and repeated `fsync` return EIO; the
writeback self-check passed before the workload. Neither that check nor
the startup message establishes the origin of the bad record. The current
artifacts do not retain the damaged AOF bytes or a post-workload filesystem
check.

To distinguish those explanations in a targeted CI reproduction, preserve
the raw AOF, manifest, and backing image before any repair or teardown;
capture Redis's write payloads and write/fsync return values; and inspect
copies before kill and before reload with a non-repairing parser. Valid
serialized payloads accepted by successful writes but missing, partial, or
changed in the backing image identify the storage/writeback path; malformed
payloads emitted by Redis or an incorrect continuation after an observed
short/error return identify its record/error-handling path. A filesystem
check on the preserved image separates metadata damage from a damaged
record. An otherwise identical no-EIO control shows whether EIO is needed
to reproduce the observation. No causal attribution or Redis defect claim
is made before those observations exist.

### Second hosted observation: loss attribution

Observation date: 2026-10-13.

[Actions run 38442238079](https://github.com/Ming-Research/Firn-wf/actions/runs/38442238079)
ran revision `842392bd55455a42a86526526e52572248123ace` with the same
120-second duration, seed 1, 32 clients, and the fault device. Its firn
artifact reports 13,307 acknowledged writes, 940 deliberate connection
drops, six completed epochs and SIGKILL restarts, two combined write/flush
EIO windows, six LOST findings, no INCONSISTENT finding, and `panic=None`.
Both jobs passed 31 self-tests on that revision. No local build or server
was used for this analysis, and the downloaded evidence was not modified.

Each lost operation received a complete successful reply. The following
intervals use the recorded invocation/completion times and the signal's
`at_ns`, not the later log-write timestamp. The millisecond delta is signal
time minus successful reply completion. Operation names include the journal
APPEND performed inside each transaction.

| Epoch / lost token | Operation | Reply completed (s) | SIGKILL sent (s) | Ack before kill (ms) | Relation to EIO | Firn artifact evidence |
| --- | --- | ---: | ---: | ---: | --- | --- |
| 1 / `0000010300000060` | MULTI/EXEC: APPEND + SET | 24.097894731 | 24.098156531 | 0.261800 | No earlier EIO | `epoch-1.tsv:389`; `history.log:3078,3079`; `findings.log:1` |
| 2 / `000002180000005c` | MULTI/EXEC: APPEND + SET | 47.523677133 | 47.523740598 | 0.063465 | No earlier EIO | `epoch-2.tsv:2311`; `history.log:6067,6068`; `findings.log:2` |
| 3 / `0000030b0000003c` | APPEND | 62.962842700 | 62.963346433 | 0.503733 | Invoked 1895.677448 ms after heal | `epoch-3.tsv:726`; `history.log:8027,8028`; `findings.log:3` |
| 4 / `0000040f00000058` | APPEND | 85.350676784 | 85.350836188 | 0.159404 | New lifetime, no EIO in that lifetime; invoked 24,283.597030 ms after prior heal | `epoch-4.tsv:1416`; `history.log:10874,10875`; `findings.log:4` |
| 5 / `0000050500000058` | MULTI/EXEC: APPEND + INCR | 107.776386814 | 107.776643487 | 0.256673 | New lifetime, no EIO in that lifetime; invoked 46,709.150520 ms after prior heal | `epoch-5.tsv:534`; `history.log:13720,13721`; `findings.log:5` |
| 6 / `0000060900000030` | MULTI/EXEC: APPEND + INCR | 120.220815595 | 120.220982534 | 0.166939 | Invoked 2016.116280 ms after heal | `epoch-6.tsv:487`; `history.log:15301,15302`; `findings.log:6` |

The loaded journal for each affected key is exactly one 16-byte token
shorter than the acknowledged APPEND length: 12,240 instead of 12,256;
12,128 instead of 12,144; 5,984 instead of 6,000; 10,848 instead of 10,864;
11,296 instead of 11,312; and 4,288 instead of 4,304 bytes, respectively
(`snapshot-1.txt` through `snapshot-6.txt`, each line 1). The last two
transactions acknowledged INCR values 351 and 139, but restored counters
are 350 and 138. The first two SETs restore the previous register token.
The surviving state is consistent with losing the last complete operation,
not a newly observed partial transaction.

Firn did refuse writes after both recorded heals. These are observed
protocol boundaries, not an exact timestamp of the internal recovery
rewrite completing:

| Epoch | EIO active / healed (s) | Last observed refusal completion (s) | First subsequent successful reply completion (s) | Success after heal (ms) | Evidence |
| --- | --- | ---: | ---: | ---: | --- |
| 3 | 58.016086126 / 61.067025844 | 61.975001402 | 62.211804620 | 1144.778776 | `report.txt:22`; `history.log:7930,7931` |
| 6 | 115.150674337 / 118.204481189 | 118.230770053 | 118.469077228 | 264.596039 | `report.txt:22`; `history.log:15076,15077` |

Requests invoked after heal include 115 refusals in epoch 3 and 30 in
epoch 6. The actual errors are `MISCONF Errors writing to the AOF file:
Input/output error`, plus `EXECABORT Transaction discarded because of
previous errors` for a denied transaction; there is no `-ERR` reply.
Both lost operations after an EIO window were invoked after successful
writes had resumed. The 14 firn stdout/stderr files are empty, so the
artifacts provide no additional AOF-writer or rewrite-installation trace.

**Conclusion:** the six acknowledged losses are real and still fail
acceptance. Every loss is within 0.504 ms of SIGKILL, outside active EIO,
and consistent with the already known pre-fix acknowledgment-before-append
mechanism. These artifacts do not reveal an older acknowledged write lost
outside that nominal window or a separate disk-recovery defect. They also
do not prove the precise AOF-writer cause, because its append/install
events and the persistence bytes were not captured.

The labels for epochs 3–6 are a classifier error. The old condition was
`fix == "absent" && !ever_disk`, with callers passing cumulative
`disks > 0`. It used **no millisecond threshold** and did not inspect the
lost request or EIO interval. Once epoch 3 injected EIO, it disabled the
known label for every later epoch, including the fresh, non-faulted
lifetimes of epochs 4 and 5. A generic label therefore did not constitute
evidence of another firn defect.

The correction evaluates each lost acknowledged event against the current
recorded SIGKILL and actual EIO intervals. An explicitly unfixed firn image
may retain the known-mechanism label when its reply completed no more than
10 ms before that signal and no active EIO overlapped the request-to-kill
interval. A healed EIO before invocation is not a veto. Overlapping EIO,
missing attribution evidence, a fixed/unknown image, or an older reply
keeps a separate investigation or unverified label. Ten milliseconds is a
conservative diagnostic criterion for consistency with the known mechanism,
not a guaranteed upper bound on AOF-writer delay or proof that an older
loss has a different cause. It never changes loss detection or turns a
failed run into a pass. Repeated checks use the current signal, so a loss
first observed only at a later restart cannot borrow its earlier signal's
short interval.

Redis again reported fsync EIO (`server-2.stdout:12`), then refused its
third startup with a bad AOF (`server-3.stdout:8`). This time its report
correctly records one `refused restart` finding, `panic=None`, and an
unexecuted recovery check (`report.txt:24–31`). The damaged-record origin
remains unverified as in the first observation; this repeat does not
resolve the storage-versus-Redis attribution.

The classifier correction carries structured lost-write keys/tokens rather
than parsing diagnostic text. It logs invocation, acknowledgment, signal,
age, nominal threshold, and EIO overlap evidence alongside each loss. The
checker predicate and error count are unchanged. These three regression
cases compiled and passed with all 34 chaos self-tests in
[gate run 38446017652](https://github.com/Ming-Research/Firn-wf/actions/runs/38446017652)
at revision `6cd2b51`:

| Test | Wrong result it detects |
| --- | --- |
| `loss_attribution_uses_ack_age_and_actual_eio_overlap_instead_of_ever_failed_disk` | A healed earlier EIO vetoes a new near-kill write, an overlapping or intervening EIO is called the sole known cause, the 10 ms boundary is mishandled, or a later-observed loss borrows the source epoch's old kill. |
| `loss_attribution_missing_or_reversed_metadata_cannot_establish_a_known_window` | Missing, duplicated, non-acknowledged, reversed, or malformed event/fault metadata establishes a known cause; competitor policy labels also remain covered. |
| `six_known_window_losses_remain_six_failures_after_diagnostic_reclassification` | Reclassifying the six measured acknowledgment-age patterns erases checker losses or makes their six harness failures pass. |

An independent read-only review covered all four changed files against the
second hosted revision, including the structured-finding consumers, design
commitments, and original artifacts. It found one stale test-status sentence,
which was corrected to distinguish the seven already validated cases from
the three new classifier cases. Gate run 38446017652 subsequently compiled
and passed all 34 chaos self-tests at `6cd2b51`, including those three cases.
A real SIGKILL exercising the corrected classifier remains unverified:
hosted run 38442238079 predates that correction. The original artifacts are
retained unchanged.

### Follow-up corrections to the first run (validated by the second run)

Expected startup failures now use a named readiness result and a structured
server finding. A refused restart stops that server's workload, retains
its last 4096 bytes from each output file and the preceding fault history,
and returns a failed test result after writing the report. The report marks
its recovery check `not_executed_server_unavailable`; it does not describe
unreadable restored state as loss-free. Genuine harness errors retain a
separate panic field.

Coverage is retained outside the normal-completion path, so an exit after
the third kill still reports three sent/reaped kills, three restart
attempts, two completed restarts, and the one combined EIO window. Missing
root permissions propagate the workflow's concrete reason into the report.
Zero-count EIO coverage and work abandoned by an early failure carry skip
or abort reasons; counters represent confirmed injection actions, not
intentions.

The epoch cadence remains 15–25 seconds because the first hosted run did
exercise all selected fault kinds. The kill trigger changes to a fresh
acknowledgment: it requires a completion at or after the wait begins, not
merely a delayed publication of an old reply. Concurrent publications keep
the newest completion time even if an older completion is published later.
Successful trigger logging occurs after the signal, so artifact I/O does
not delay it. The measured count of signals within 10 ms of an acknowledgment
is coverage evidence, not a scheduling guarantee or a change to acceptance.

The following seven regression cases passed in each job of the second
hosted run. The later per-loss classifier correction compiled and passed its
three cases in gate run 38446017652 at `6cd2b51`; its real-kill runtime report
remains unverified:

| Test | Wrong result it detects |
| --- | --- |
| `readiness_result_preserves_named_server_exit_without_panicking` | An exited competitor becomes a panic or carries firn's name instead of its own identity and exit status. |
| `readiness_result_does_not_retry_nonrefusal_or_panic` | A post-admission timeout reconnects or panics instead of returning failure to its owner. |
| `refused_restart_report_retains_partial_coverage_fault_context_and_bounded_tails` | Refused recovery has zero findings, loses partial counts/fault context or log tails, or omits the unavailable recovery check. |
| `smoke_coverage_rejects_missing_fault_kinds_and_preserves_unwind_progress` | A kill without a completed restart or missing EIO coverage can count as a covered smoke; unwinding erases completed actions. |
| `seeded_schedule_reaches_first_combined_eio_epoch_within_75_active_seconds` | A seeded epoch duration leaves the first combined EIO window outside the first three epochs' 75-second active-time bound. |
| `kill_trigger_requires_fresh_ack_and_timeout_retains_prior_faults` | An old completion with a new publication sequence triggers the kill; a timeout fabricates a signal or discards earlier fault evidence. |
| `ack_publication_keeps_latest_completion_when_an_older_reply_is_published_last` | Out-of-order publication overwrites the actual latest acknowledgment time. |

These cases join the existing test targets; no gate target, compiler pin,
submodule, production server, or approval log changes. The first hosted
run validates the original revision only; the second hosted run validates
these corrected readiness, reporting, and signal paths, including the
observed sub-millisecond signal adjacency and real refused-restart report.

A fresh gpt-6.1-sol review covered the five-file follow-up against the
hosted revision. It found and reinspected two local repairs: out-of-order
acknowledgment publication must retain the maximum completion time, and
zero-count EIO coverage needs an explicit skip reason on short or aborted
runs. No static findings remain. YAML parsing, all 14 current workflow
shell blocks' `bash -n`, and `git diff --check` passed; no build, test, or
server ran on the Mac. Rust compilation, regression execution,
real refused-restart reporting, and actual signal adjacency remained for CI
at that review; the second hosted observation supplies that evidence for
its revision only.

### Initial implementation review, 2026-10-13 (before the hosted run)

The harness, shared checker, dispatch workflow, and proposed design node
are prepared in the worktree. No build, server, self-test, gate, or chaos
run was executed on the Mac. At that review, compilation, fault behavior,
competitor startup, smoke results, and 86,400-second acceptance were
unverified; the hosted observation above supersedes that execution status
for its revision only. The owner commits, pushes, and reads CI. No pin, submodule,
compiler, production-server code, gate target, or approval log changed.

Static validation: the workflow YAML parsed; all 13 shell blocks and the
device helper passed `bash -n`; `git diff --check` passed. These checks do
not establish Rust compilation or GitHub Actions expression validity.

A separate gpt-6.1-sol review examined the complete draft against the base,
the project's checklist, and relevant design commitments. Its findings
were fixed: readiness helpers shared with both Cargo targets and all their
consumers; bounded PING/PONG readiness after TCP admission; nonpanicking
device cleanup with ownership checks; atomic snapshots and retained
unknown-effect continuity; snapshot reply references instead of repeated
payload copies; CPU identity recorded for later acceptance review. The
review found no remaining code finding within that static scope; execution
evidence and design approval remain unverified.

### Self-tests and the wrong results they reject

The 14 original chaos tests below passed in each job of the first hosted run.
The existing durability negative test remains in the network gate, which
these chaos artifacts do not establish. The seven first-run correction
cases above passed in the second run; the three classifier cases compiled
and passed in gate run 38446017652 at `6cd2b51` (34 chaos self-tests total).
The corrected classifier still lacks a real-kill run. The pure tests
run without starting a database; shared process-support tests remain wired.

| Test | Discriminating case |
| --- | --- |
| Existing `append_checker_rejects_lost_updates_duplicate_replies_and_impossible_orders` | Lost token, duplicate/missing reply rank, reversed bytes, and a rank order incompatible with real time. |
| `chaos_checker_accepts_serial_overlapping_and_independent_key_histories` | Positive controls prevent rejecting permitted overlap, out-of-order replies, or independent key groups. |
| `chaos_checker_rejects_lost_acknowledged_writes` | An otherwise consistent surviving prefix is still missing an acknowledged token. |
| `chaos_checker_rejects_duplicate_effects_and_duplicate_reply_ranks` | One token appears twice, or two successful requests claim the same rank. |
| `chaos_checker_rejects_impossible_real_time_order` | A lower-ranked request begins after a higher-ranked request completed. |
| `chaos_checker_rejects_partial_transactions_and_wrong_final_fields` | Journal/SET survives without INCR, journal/INCR survives without SET, or side effects survive without the journal. |
| `chaos_checker_rejects_wrong_counter_reply_even_when_snapshot_is_correct` | Correct final state cannot excuse an incorrect acknowledged INCR reply. |
| `chaos_checker_accepts_unknown_once_or_absent_and_keeps_its_interval_open` | Positive controls prevent treating an unread request as definitely absent or completed at socket close. |
| `chaos_checker_rejects_unknown_partial_transaction_and_refused_effect` | An ambiguous transaction has partial effects, or a refused request has an effect. |
| `chaos_checker_rejects_unknown_tokens_and_malformed_histories_or_snapshots` | Invented/wrong-key tokens, truncated/non-ASCII journal, invalid key/interval/rank, duplicate request, or missing state field. |
| `resp_parser_rejects_truncated_malformed_and_oversized_frames` | Partial bulk reply, malformed CRLF/integer/length, or oversized allocation. |
| `protocol_rejects_partial_transactions_bad_set_and_unexpected_errors` | Partial EXEC, failed SET inside an apparently successful transaction, or MISCONF outside the injected disk window. |
| `archive_parser_rejects_missing_truncated_and_extra_history` | Missing header, omitted event, or extra event cannot produce a clean check. |
| `retained_snapshot_detects_a_previously_witnessed_unknown_effect_disappearing` | Two individually permitted unknown histories cannot conceal a previously observed effect disappearing. |
| Former `snapshot_reply_references_preserve_changes_and_reject_missing_or_stale_files` (retired by the compact-summary change) | Covered the former immutable-reply reference format; replaced by mismatch-only evidence coverage below. |

The workflow also requires an invalid device size to fail before device
operations, verifies the saved ownership path, and reuses the device's
existing failure/writeback self-checks.

### Day-run check and artifact budget

The 120-second hosted run completed six epochs, approximately one every
20 seconds, and checks cost approximately 8 ms per checked epoch (review
of run 38442238079). Rechecking every old epoch after every restart costs
`0.008 * N * (N + 1) / 2` seconds. Solving approximately
`86400 = 20 * N + 0.008 * N * (N + 1) / 2` gives about 2,780 epochs,
about 36% wall time in checks and about 15.4 hours of workload before other
overhead. The review estimated 2.4 GB of repeated SNAPSHOT lines plus
5.6 GB of OP lines, or roughly 8–9 GB of history per server. That scheme
does not meet the revised time budget.

With the revised schedule, a late full check of roughly 4,100 epochs costs
about 33 seconds and is followed by at least roughly 620 seconds before
another full check is due. The 19-times interval targets approximately 5%
full-check cost, reserving headroom for incremental checks. At the observed
cadence, a cycle then accumulates about 31 new epochs; incremental checks
read those new epochs, not the entire old dataset. Including the final full
check, a forecast of roughly 570,000–600,000 checked epochs costs about
4,600–4,800 seconds, or 5–6% of 24 hours. Startup/restart overhead further
reduces workload time; report its actual value rather than promising active
hours. A slower or nonlinear checker can overturn this forecast, which is
why the report rejects a day run above 10% check time.

At at most roughly 160 bytes per compact summary, 600,000 summaries occupy
about 0.1 GB rather than 2.4 GB. Keeping the review's conservative 5.6 GB OP
estimate gives about 5.7 GB of clean history per server, plus exact baseline
files, epoch archives, server logs, and fault records. Request logs remain
linear in duration and paced request count (32 clients / 250 ms gives at
most 128 requests per second, approximately 11.1 million per day plus the
bounded final fault-cycle tail). Nothing truncates request evidence. Failed
checks additionally dump full mismatching observations, so provision extra
space for failures. These estimates reuse short-run numbers; they are not
a measured day result or a fixed byte cap under arbitrary configuration.

The new fixtures below are pending CI compilation and execution; the gate's
34-test evidence applies to `6cd2b51`, before this scheduler change.

| Test | Wrong result it detects |
| --- | --- |
| `periodic_full_check_detects_corruption_in_an_old_epoch` | Old acknowledged data disappears after a full check, incremental checks skip it, and disabling scheduled full checks conceals the loss. |
| `final_full_check_is_not_repeated_and_interval_tracks_cost` | A terminal full check is omitted after an incremental check, duplicated after a complete full check, or the adaptive interval ignores measured cost. |
| `snapshot_dumps_only_failures_and_preserves_raw_evidence` | A valid 4096-byte journal expands the production day-time summary beyond 160 bytes or leaks full values; clean snapshots retain raw dumps; or a mismatch loses complete observed RESP/state evidence. |
| `phase_timings_preserve_partial_work_without_join_time` | Unwinding erases elapsed work/check time, or completed workload timing includes later join overhead. |
| `day_check_budget_rejects_more_than_ten_percent_but_not_smoke_or_boundary` | A requested day run above 10% check time passes, the exact 10% boundary fails, or the day-only budget rejects a short smoke. |

The old snapshot-reference fixture is retired because clean repeated RESP
payloads are no longer retained. Its replacement must show that clean checks
produce only a summary while mismatches retain the complete observed bytes.
The exact-baseline continuity fixture remains: a hash alone cannot justify
unknown-effect continuity. A scheduled-full regression must corrupt an old
epoch after its first full check, show that the next incremental range skips
it, and then observe LOST through the independent checker at the scheduled
full check. It must fail if periodic full checks are disabled.

### Remaining experimental limits

The completion-review follow-up changed the harness, workflow wording,
this record, and the existing chaos design node. A fresh read-only
gpt-6.1-sol review covered all four files against `6cd2b51`, their consumers,
the project checklist, and relevant design ancestors and persistence/transaction
nodes. It found one misleading prior-criterion sentence; that sentence now
distinguishes the original functional criterion from the later resource budget.
No unresolved static findings remain. Four workflow shell-block copies passed
`bash -n`, both inline Python payload copies passed `python3 -m py_compile`,
and `git diff --check` passed. Those commands executed no workload scripts.
The design lint, Rust compilation, five revised/new fixtures, smoke run, and
day acceptance remain pending CI. No gate target, Whitefoot pin, submodule,
production server, or approval log changed.

The revised scheduler, compact summaries, and timing report still need CI
compilation, self-test execution, and a new smoke run. Actual day-run resource
use remains unmeasured; the forecast below does not establish acceptance.
Distinct failure deduplication uses additional memory in an already-failed
run. The experiment measures process crashes and storage EIO, not power
loss, ENOSPC, arbitrary mixed-command linearizability, or a throughput target.
Garnet commit semantics remain unverified; absent competitors and missing
sudo leave explicit coverage gaps. No Whitefoot gap was identified by this
Rust-only implementation; only a real CI run can supply language/runtime
evidence.
