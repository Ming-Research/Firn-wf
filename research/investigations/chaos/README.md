# Concurrent workload under faults

Board: `firn-safety-showcase`, `chaos-tool` and `chaos-run`.

## Question and rejection criterion (before measurement)

Does firn preserve a serial execution and every acknowledged effect while
many clients contend, connections disappear, storage fails, and the process
is killed and restarted? Run the same seeded workload separately against
firn, Redis 7.2.16, Valkey, Dragonfly, and Garnet. This is a correctness
experiment, not a throughput comparison or a power-loss simulation.

The owner's proposed acceptance is 86,400 seconds on the i9-14900K, zero
unrequested firn exits, zero inconsistencies, and zero lost acknowledged
writes. An injected SIGKILL is not an unrequested crash. A missing server,
skipped storage fault, incomplete history/check, failed restart, lack of
successful writes, or shorter run cannot establish that acceptance. First
run the 120-second hosted smoke mode, inspect its time and coverage, then
choose the long-run scale. No result has been measured yet.

**Known expected failure:** before [append before reply, Firn-wf#97](https://github.com/Ming-Research/Firn-wf/pull/97)
lands, firn can reply before its AOF record reaches the file. SIGKILL can
therefore lose approximately the last 10 ms of acknowledged writes. A run
explicitly identified as lacking that fix labels restart losses as the
known acknowledgment-before-append failure, not a newly discovered defect.
It still fails acceptance. Unknown fix status never silently assumes the
fix is present. Losses during a storage-failure cycle are not attributed
solely to that timing window.

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
remain outside the faulty filesystem. Recheck all closed epochs after each
restart and at the end; never reset the database between epochs. Histories
grow on disk, and rechecking accumulated data costs more as the run grows.
Record elapsed workload/check time and operation counts; this is no fixed
throughput promise. Provision space for both AOF and history before a day
run; the workflow's larger sparse fault device is not the existing 64 MiB
device's default. Unexpected exhaustion of artifact storage is a harness
failure, never a clean result.

Seeded epochs last 15–25 seconds of active traffic. One in sixteen requests
is sent and its connection closed before reading a reply; reconnect before
the next operation. Other clients continue. Every epoch ends with SIGKILL
while clients are writing, followed by joining writers and restarting from
the same files. Do not flush or wait for the AOF before the kill. Every
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
storage. Repeated identical snapshot replies reference an immutable file
containing the complete RESP bytes; changed replies receive a new file.
This lossless representation avoids storing the accumulated dataset again
after every restart. Each request still has its own timestamps and reply
reference. A complete successful APPEND or EXEC reply is an acknowledgment;
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
`CHAOS_FAULT_MOUNT`, `CHAOS_RESTART_SECONDS`, and
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
own 86,400-second requested duration, and a reserved 14900K. Inspect the
smoke artifacts before scheduling it. The optional device size is 32 GiB
in this workflow; the existing fault suite retains its 64 MiB default.

## Results

### Implementation review, 2026-10-13 (no experiment run)

The harness, shared checker, dispatch workflow, and proposed design node
are prepared in the worktree. No build, server, self-test, gate, or chaos
run was executed on the Mac. Compilation, fault behavior, competitor
startup, 120-second smoke results, and 86,400-second acceptance remain
unverified. The owner commits, pushes, and reads CI. No pin, submodule,
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

Every row below remains **unexecuted** until CI. The existing durability
negative test remains in place; the new pure tests run without starting a
database. Shared process-support tests also remain wired.

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
| `snapshot_reply_references_preserve_changes_and_reject_missing_or_stale_files` | Changed reply gets distinct bytes; a missing referenced file or stale index cannot silently overwrite history. |

The workflow also requires an invalid device size to fail before device
operations, verifies the saved ownership path, and reuses the device's
existing failure/writeback self-checks.

### Remaining experimental limits

Accumulated checkpoint reads and model work grow quadratically in the
number of epochs. Reply payloads are retained by reference to limit artifact
growth, but actual day-run resource use still needs the short-run evidence.
Distinct failure deduplication uses additional memory in an already-failed
run. The experiment measures process crashes and storage EIO, not power
loss, ENOSPC, arbitrary mixed-command linearizability, or a throughput target.
Garnet commit semantics remain unverified; absent competitors and missing
sudo leave explicit coverage gaps. No Whitefoot gap was identified by this
Rust-only implementation; only a real CI run can supply language/runtime
evidence.
