# Active AOF write and sync errors

## Question and reference

Can firn preserve Redis 7.0.15's everysec failure admission and retained-byte
semantics while recovering without new client writes and terminating shutdown?
References below name files and lines in Redis 7.0.15 `src/`.

`aof.c:1024-1040,1146-1230` retries short writes, retains failed appends and
attempts to truncate a partial append to the previous incremental size. Successful
rollback retains the whole buffer; failed rollback accounts the written prefix
and retains its suffix. Redis reports ENOSPC for partial failure, while preserving
the actual errno for zero progress. A complete append clears the append error.
`bio.c:231-247` publishes sync errors (except EBADF/EINVAL) and clears them on
success. Ordinary rewrite rotation preserves both error states
(`aof.c:790-852`); `aof.c:999-1013` resets them only when CONFIG re-enables AOF,
not when a rewrite opens a new increment. This corrects the earlier investigation's
overbroad reopening summary.
Failed appends are retried from `beforeSleep` on every event-loop iteration
(`server.c:1660-1666`), at least hz (default 10) times a second, besides the
once-per-second `serverCron` retry (`server.c:1384-1393`). An earlier version of
this record cited only the cron retry and concluded once per second.

`server.c:3881-3905,4324-4353` refuses CMD_WRITE and PING after command validation,
authentication and eviction; reads remain available. `multi.c:85-87,105-109,
136-140,170-178,229-233` and `server.c:3568-3576,3735-3740` dirty refused queues
and discard writing EXEC at admission. Legacy scripts can read; their first
accepted write checks disk health (`script.c:349-386,547-557`), with call raising
and pcall returning an error (`script_lua.c:988-994,1711-1722`).
`server.c:5596-5650` reports combined write/sync health independently of rewrite
health. Shutdown forces a flush and sync and exits even when these fail
(`server.c:4239-4247,4301-4308`).

## Before this change

`append_all` discarded the terminal IoError and returned only progress;
`aof_rw_drain` retained only the suffix without rollback. `aof_rw_flush` discarded
sync errors. Meta had no active-file health, so commands, transactions and scripts
continued accepting writes and INFO could not expose failure. `write_log` retried
until success even during shutdown, contradicting the documented bounded attempt.

## Proposed design and alternatives

The proposed decisions belong to [append-only-file](../../../design/firn/append-only-file.md),
[transactions](../../../design/firn/transactions.md),
[scripts](../../../design/firn/scripts.md) and
[orderly-stop](../../../design/firn/orderly-stop.md).

Keep firn's everysec-only scope; reopen selectable always/no when a selected
consumer needs them. Preserve separate append and sync IoErrors in existing Meta,
publishing alongside writer accounting, and observe admission jointly with mutation.
A separate Shared adds no needed capability. One reply writer owns the exact
MISCONF prefix; its native description suffix was pending Whitefoot item
`firn-req-io-error-text`, with no errno-to-text substitute, and is now filled
as [below](#misconf-description-whitefoot-v0132).

Rollback partial appends before retrying, retaining either the whole buffer or
only its unwritten suffix according to truncation success. Track the incremental
size separately from total AOF size. Retry failed appends every 100 ms, Redis's
idle cadence at hz 10, and failed syncs once per second while idle; only an
attempted append or sync that fails counts as a writer failure for an active
rewrite. Idle sync retry deliberately differs from Redis's new-byte-dependent
scheduling (`aof.c:1067-1089,1255-1261`), avoiding a permanently latched idle error.
Preserve health when rotation installs a new incremental file; successful drain
and sync before rotation can clear their respective errors normally. Preserve
the actual terminal append IoError and a partial-result flag so the eventual
native formatter can select Redis's ENOSPC for partial failure without losing
the original host kind and code. Shutdown takes the
remaining buffers for one final flush and sync attempt and abandons failures,
instead of retrying forever or adding a new nonzero exit policy.

CMD_WRITE metadata governs ordinary, queued and EXEC admission; PING is additionally
refused at normal admission. Read-only EXEC and legacy scripts remain available.
A script holds Meta throughout an attempt, so the writer cannot publish a status
change during it: checking held Meta at its first write is equivalent to the live
published-status check, without another status channel. Accepted writes suppress
subsequent checks. INFO follows [reported-facts](../../../design/firn/reported-facts.md):
combined health and actual collecting plus retained buffer bytes are reportable.
The existing shared WRITE table covers held commands; supported network-only
FUNCTION FLUSH also has WRITE (`commands.c:3506`), whereas legacy EVAL/EVALSHA
have MAY_REPLICATE (`commands.c:7303-7304`). The writer has no BIO sync queue or
background-sync append postponement, so their INFO counts are truthfully zero.

## Method and rejection criteria (before implementation)

Linux network cases lower the owned child's RLIMIT_FSIZE soft limit with prlimit,
inheriting ignored SIGXFSZ before exec. A limit at current incremental size faults
appends; a limit slightly above it forces a real prefix followed by EFBIG. Raise
the limit and poll INFO without a new write to establish autonomous recovery.
Compare explicit expected RESP bytes and replay non-idempotent effects, exercise
both runtime routes, and check write/PING refusal, readable GET/INFO, transactions,
call/pcall and bounded successful shutdown under a latched error.

Reject the design if any mutation can pass its admission boundary after a failure
is published, retained bytes duplicate or disappear on recovery, INFO reports ok
with either error set, recovery requires a new client write, or shutdown retries
permanent errors indefinitely. A fault not reached is not evidence. Native error
description equality was an explicitly pending check (`firn-req-io-error-text`);
[below](#misconf-description-whitefoot-v0132) replaces it.
No local build or runtime test is authorized; only the named compiler check,
diff whitespace check and design lint may run here. Runtime results await CI.

## Sync-failure fixture options (built 2026-10-12)

Update, 2026-10-12: the dm-flakey fixture now exists
(`tests/aof_sync_fault_device.sh`, `tests/aof_sync_fault.rs`, the
`aof-sync-fault` job of `.github/workflows/check.yml`). It is an ext4
filesystem without a journal, mounted `errors=continue`, on a flakey device
with up 0 and down 1 using `error_writes`. Its probe confirmed that write(2)
succeeds while fsync returns EIO, and that fsync succeeds after the device
heals. It was verified on CI in
[run 38235060604](https://github.com/Ming-Research/Firn-wf/actions/runs/38235060604).
A mutation that never clears the sync error fails the case in
[run 38237361217](https://github.com/Ming-Research/Firn-wf/actions/runs/38237361217).
The comparison of options below is the history from before the fixture was
built.

RLIMIT_FSIZE cannot fault fdatasync. A pass-through FUSE filesystem can return EIO
from fsync for one incremental inode while recording write/truncate/fsync requests;
it needs /dev/fuse, mount permission, a maintained fixture, controlled caching and
proof both runtime routes reach the callback. A loop device with dm-flakey under
sudo can fail block I/O, but needs device-mapper support, privileged setup/cleanup
and careful cache/writeback control to isolate sync failure from append failure.
Neither availability nor deterministic behavior has been verified on CI here.
FUSE is the more direct operation-level experiment; dm-flakey tests the full block
stack at greater setup and attribution cost. No fixture is built in this change;
sync failure and failed truncate receive source review only.

## Implementation and validation (2026-10-11)

Implemented on the uncommitted worktree based on
`8967d1f4af6b3e5042e56bf3b0235cd8524565bb` (`origin/main`). These are source and
check-only results, not executed fault experiments. The design-tree additions
remain proposed; no approval record was written. At this step `whitefoot.pin`
remained `wf-a3205af0bbea` and no submodule moved; the branch now pins
`wf-94d353e33034` (specification v0.132) for `error_copy_description`.

| File, relative to the repository | Change |
| --- | --- |
| `firn/modules.wfg` | Declare store's direct `std::io` dependency for saved errors, and later protocol's for describing them in the MISCONF reply. |
| `firn/store/module.wfm` | Meta's append/sync errors, partial-result flag, incremental size and retained length; shared health query. |
| `firn/store/store.wf` | Initialize those fields and implement the health query. |
| `firn/persistence/persistence.wf` | Return append progress with its terminal error; track append retry time and exit after the final forced attempt. |
| `firn/persistence/module.wfm` | State the final flush/sync failure contract. |
| `firn/persistence/rewrite.wf` | Rollback or retain partial suffixes, publish size/status/length together, retry idle failures, preserve rotation health, and leave one final flush to the outer writer. |
| `firn/persistence/startup.wf` | Initialize last incremental length from the loaded manifest. |
| `firn/persistence/manifest.wf` | Consume the new append result when writing a manifest. |
| `firn/persistence/rewrite-dataset.wf` | Consume the new append result for the temporary base without confusing rewrite failure with active-file health. |
| `firn/protocol/module.wfm`, `firn/protocol/protocol.wf` | Declare and implement the single ordinary/EXECABORT MISCONF emitter. |
| `firn/commands/admission.wf` | Reuse shared WRITE flags, include FUNCTION FLUSH, and provide held-Meta refusal. |
| `firn/commands/module.wfm`, `firn/commands/transaction.wf` | Aggregate queued WRITE flags, dirty refused queues and preserve error ordering for writing EXEC. |
| `firn/commands/script.wf`, `firn/scripting/module.wfm` | Check active AOF health before the first accepted script write; document WRITE_DIRTY's shared role. |
| `firn/commands/strings.wf` | Guard nine string mutation boundaries. |
| `firn/commands/keys.wf` | Guard five key mutation boundaries. |
| `firn/commands/hashes.wf` | Guard five hash mutation boundaries. |
| `firn/commands/lists.wf` | Guard seven list mutation boundaries. |
| `firn/commands/sets.wf` | Guard five set mutation boundaries, including only STORE variants of combined reads/writes. |
| `firn/commands/sorted.wf` | Guard three sorted-set mutation boundaries. |
| `firn/commands/ranges.wf` | Guard sorted-set range removal. |
| `firn/commands/server.wf` | Guard FLUSHALL/FLUSHDB and FUNCTION FLUSH. |
| `firn/commands/info.wf` | Report combined write status, collecting plus retained length, and truthful zero BIO/postponement counters. |
| `tests/support.rs` | Child-only inherited SIGXFSZ ignore, leaving the parent unchanged. |
| `tests/network.rs` | Wire the new fault cases into the ordinary network suite. |
| `tests/aof_write_errors.rs` | Runtime soft-limit faults, disposition verification and the cases below. |
| `design/firn/append-only-file.md` | Propose everysec scope, Meta admission, rollback, idle retry, rotation preservation and pending native error text. |
| `design/firn/transactions.md` | Propose queue/EXEC persistence admission. |
| `design/firn/scripts.md` | Propose first-WRITE admission and the held-Meta equivalence argument. |
| `design/firn/orderly-stop.md` | Propose final attempted persistence followed by exit zero despite failure. |
| `firn/README.md` | Document refusal, recovery, INFO and possible final data loss. |
| `research/investigations/aof-write-errors/README.md` | Record the pre-implementation question, oracle, alternatives and falsifier, and these limited results. |

Each new test loops over both runtime routes:

- `aof_failure_refuses_write_flags_and_ping_and_recovers_without_new_writes`
  covers the implemented WRITE flags, no-effect writes, FUNCTION FLUSH, PING,
  readable GET/INFO, validation/authentication precedence, retained bytes and
  INFO-only recovery. The old code lacks the status fields and accepts the
  commands that must be refused.
- `aof_failure_transaction_admission_and_legacy_script_first_write` covers
  a writing queue admitted before the fault, dirty queue-time WRITE/PING
  refusals, read-only EXEC including PING queued before failure, legacy
  EVAL/EVALSHA reads and first-write call/pcall behavior, and healthy writes
  after recovery. The old code executes the writes instead of refusing them.
- `aof_short_append_rolls_back_and_replays_exactly_once_on_both_routes`
  faults seven bytes into RPUSH, checks unchanged size/full retained buffer,
  exact independently constructed RESP bytes after rollback/recovery and
  non-idempotent replay. The old code leaves the prefix and retains only the
  suffix. This case does not cover failed truncation.
- `aof_latched_error_shutdown_and_signals_exit_zero_on_both_routes` covers
  permanent append failure followed by SHUTDOWN, SIGTERM or SIGINT, requiring
  exit zero within the existing five-second deadline while peers stay open.
  The old writer never exits. This case does not cover a failing sync.

Inspection caught and fixed a test publication race: seeing file bytes does
not prove the writer has published its empty buffer yet, so seed readiness
polls for both healthy status and zero buffer length. No case has been run
before or after this change; the failure descriptions above explain what
each assertion detects, not measured red/green results.

The permitted pinned compiler check completed with exit 0 and `firn: accepted`;
`git diff --check` and the permitted design lint also passed. Earlier compiler
refusals were corrected against the pinned syntax and module rules:

- GRAM-9: nested constructors/calls in argument positions became preceding
  bindings, as the language's flat argument grammar requires.
- FORM-2: inline match arms became the required multiline blocks.
- MOD-5: store's use of IoError received its missing direct `std::io` graph edge.
- FORM-4: `//` comments in Whitefoot became explanations in `doc` fields.
- FORM-3: the reserved identifier `writes` became `write_commands` for the
  transaction field and `writing` for local values.

These were source mistakes, not semantic language gaps. The known native error
text gap `firn-req-io-error-text` was then pending, with prefix-only assertions;
it is superseded by the [MISCONF description](#misconf-description-whitefoot-v0132)
section;
no new Whitefoot gap was found or filed. No build, Rust compilation, runtime
test, CI run, GitHub operation, board update or commit was performed. Native
and fallback RLIMIT behavior, call/pcall wire output, recovery and shutdown
still require CI execution. Sync-only failure, failed truncate and rotation
under fault have source review only. Removing the infinite retry loop does
not impose a deadline on a host filesystem call that itself blocks.

An independent read-only GPT-6.1-Sol review examined the complete base-to-worktree
change and untracked files, changed sections and direct consumers, the project's
checklist, and G1-G3/DC1-DC4 against the relevant design ancestors and siblings.
It found no additional defect within scope. A1, C1, C2, T2, D1, G1-G3 and
DC1-DC3 passed source review; T1, T3 and DC4 remain unverified where they require
runtime or full-gate evidence, and R1 is not applicable. It reran no check or
suite and relied on the primary agent's observed check-only results. The test
readiness race was fixed before that final review. Owner approval remains pending.

## MISCONF description (Whitefoot v0.132)

Question: with PRE-2's `std::io::error_copy_description` (Whitefoot
`94d353e33`, specification v0.132), can firn's MISCONF suffix equal Redis
7.0.15's `strerror(aof_last_write_errno)` in every refusal?

Oracle, Redis 7.0.15 `src/`:

- `server.c:4324-4344` tests the write status before the fsync status; only
  the fsync branch copies `aof_bio_fsync_errno` into `aof_last_write_errno`,
  so a latched append error is described by its own errno.
- `aof.c:1024-1040`: `aofWrite` retries short writes and returns `-1` only
  when nothing was written; after a positive prefix a failing `write(2)`
  returns the prefix length. `aof.c:1157-1162` keeps `errno` for `-1`;
  `aof.c:1163-1184` sets `ENOSPC` for every short write, whether or not the
  truncate succeeds. Under RLIMIT_FSIZE a file at the limit therefore
  reports `File too large`, and a prefix followed by EFBIG reports
  `No space left on device`.
- `server.c:4346-4355` formats `-MISCONF Errors writing to the AOF file: %s`.
  Ordinary refusals send it as is (`server.c:3900-3904,3568-3577`); EXEC's
  discard wraps it as `-EXECABORT Transaction discarded because of: MISCONF
  ...` (`multi.c:136-140`).
- Legacy EVAL and EVALSHA (no shebang; firn rejects every shebang) skip
  `scriptPrepareForRun`'s "Writable scripts are blocked" text
  (`script.c:139-175`, only without `SCRIPT_FLAG_EVAL_COMPAT_MODE`). Their
  first write gets the same message from `scriptVerifyWriteCommandAllow`
  (`script.c:375-386`), stored without `-` and CR LF as the error table
  (`script_lua.c:546-570`); `redis.call` raises it with ` script: <sha>, on
  @user_script:<line>.` (`script_lua.c:1711-1722`), and `redis.pcall`
  returns the table.

Comparison and falsifier: the RLIMIT_FSIZE cases assert the complete reply
with glibc's C-locale text. The design is rejected if a zero-progress append
fault, its EXECABORT wrapping, or the script call and pcall paths give
anything but `File too large` after the exact prefix.

Design: `reply_aof_error` receives the text `aof_error_text` selects from
Meta (the saved host error, a short write, or nothing) and copies its description straight into the reply window. When
the window is too small the window grows by the length the host reports and
the copy runs once more. `DescriptionUnavailable` leaves the suffix empty;
glibc describes every code, so on Linux this needs a host failure.

Short write: Redis describes it as ENOSPC. Whitefoot's
`error_copy_class_description` (specification v0.136) copies the host's text
for the class of an error and ignores its code, so firn describes
`NoSpace(code: 0, origin: 0)` by class and holds no platform number; before
that release std named no host code for ENOSPC (Whitefoot item
`firn-req-enospc-text`), and the suffix was left empty.

Status: implemented on the work branch. The gate at `36ac0a5`
([run 38203897532](https://github.com/Ming-Research/Firn-wf/actions/runs/38203897532))
passed, including the four `aof_write_errors` cases on both routes with their
full `File too large` assertions; the short-write ENOSPC case was ignored,
citing `firn-req-enospc-text`, until the pin moved to the release with
`error_copy_class_description`, which un-ignored it.

## Completion review fixes

- A writer tick on which a failed append's retry is not yet due attempts no
  I/O. It used to report the flush as failed, so an active rewrite aborted as
  `writer_io` before any real failure; only an attempted append that does not
  drain, or a counted sync failure, now aborts it, and the rewrite reports the
  attempted append's `drain`. The case
  `aof_rewrite_during_latched_append_error_reports_the_attempted_drain`
  starts a rewrite while an append error is latched and requires `drain`.
- The append retry cadence follows Redis's event loop (every 100 ms) instead
  of the once-per-second cron retry; the idle sync retry stays once per second.
  No test pins the cadence: a timing bound tight enough to separate 100 ms from
  one second would be a CI timing assertion.
- Queued MULTI commands no longer take a second Meta hold: Redis reads the
  disk status once in `processCommand` before `queueMultiCommand`, and firn's
  admission check already refuses and dirties the queue; EXEC and each
  mutation recheck under their own hold.
- Scripts that hold only their declared keys (Firn-wf#47) check the AOF
  status in shared script admission, between arity and DENYOOM as
  `scriptCall` orders them (`script.c:530-557`), with the attempt's held Meta.
