# Reply after the covering append

This record came from the branch of Firn-wf#97, which was closed unmerged:
the owner chose, on decision card `firn-q-reply-after-append`, to reply after
the covering append through group commit (board item `firn-group-commit`)
instead of #97's dedicated writer. The sections up to
[Measured result of the batch overhead reduction](#measured-result-of-the-batch-overhead-reduction-2026-10-13)
describe #97 and its measurements; [Group commit](#group-commit-firn-group-commit)
is the design that replaces it. Line references in the earlier sections name
#97's revisions, not main's.

## Question and prior criterion

Proposal for board item `firn-bl-01-02-rest`, decision card
`firn-q-reply-after-append`, option A. Can firn stop acknowledging healthy AOF
records before their write completes, while retaining Redis's reply ordering,
everysec policy and write-error replies, and batching concurrent connections'
records? This record precedes validation of the proposal. The task reports
eight acknowledged writes missing after SIGKILL in
[CI run 38396907594](https://github.com/Ming-Research/Firn-wf/actions/runs/38396907594).
Before this proposal the writer slept 10 ms between drains, and connections
sent replies without observing a drain. That schedule permitted the reported loss.

Reject the proposal if the existing kill case loses any acknowledged write
after restart with healthy appends; if pipelined replies change order; if a
command that creates no record waits for another connection's writes; if the
writer marks bytes written before a complete successful append; or if an
append fault changes already-executed commands' replies or prevents the
existing autonomous retry and shutdown cases from completing.

## Redis 7.2.16 reference

Read from the verified release source, not inferred from firn:

- `src/server.c:1721-1739` flushes the AOF before sending pending replies.
- `src/aof.c:1100-1117` postpones a non-forced everysec append while a
  background fsync is running, first recording the current second and
  returning, then writing once the integer-second difference reaches two.
  `beforeSleep` still sends replies when that flush returns without writing.
- `src/aof.c:1191-1213` exits for an always-policy write failure, but for
  everysec saves the append error, keeps unappended bytes and returns.
  `beforeSleep` still sends the replies it already produced. They are not
  rewritten as MISCONF. Later write admission refuses commands while the
  saved error remains (`src/server.c:4483-4502`).
- `src/aof.c:1215-1222` clears the saved append error after a complete write.

Firn has no background fsync queue: its single writer performs sync itself.
Keep that implementation and its everysec schedule. A batch appended before
a sync can release its replies as soon as its append is published. New
records collected during the sync wait for the writer to append them when
it returns. There is no two-second bypass. This strengthens the healthy
process-kill guarantee at the possible cost of latency during slow fsync;
it does not promise survival of power loss or disk failure.

## Proposal and ordering argument

Before, the execution path was:

```text
atomic keys and Meta: mutate; append command bytes; commit sequence
connection: send buffered replies
writer's next 10 ms wake: take all collecting bytes; write; later sync
```

After, it is:

```text
atomic keys and Meta:
    mutate; append command bytes; commit sequence
    if this statement appended bytes: retain its sequence in this client
writer wakes on collecting bytes:
    atomic Meta: exchange the batch and its covering sequence
    write the entire batch, retrying positive short writes
    atomic Meta: publish written-through only after complete success
connection:
    retain reply byte ranges with their preceding appended sequence
    for each remaining range, in connection order:
        if the sequence is zero: send without reading Meta
        atomic Meta when written-through >= this range's sequence
                         or failure-release >= this range's sequence:
            include all consecutive ranges already ready
        send that ready prefix together
```

The actual sequence is captured inside the command's existing Meta hold.
A post-command snapshot of the global sequence would wrongly make a read
wait for unrelated writes. Compare the collecting length with the local
sequence token's initial length to distinguish appending commands from
no-effect commands. A propagated metadata command such as FUNCTION FLUSH
takes a sequence even though it changes no dataset key. All committed
records, including expiry DELs and eviction, wake the writer through the
collecting state. A coalesced timer notification retains idle housekeeping;
it is not required for healthy append wakeup.

Every buffer exchange carries its final sequence, including the sealed
prefix at a rewrite's end cut and the final shutdown combination. Partial
append/rollback keeps the same covering sequence until the suffix is fully
written. The writer is the sole owner of the active file, so successful
publication covers every earlier batch. Rotation never resets the
written-through sequence. Script retries publish no token for discarded
effects; accepted script effects, including effects before a Lua error,
and a complete EXEC block publish at their outer commit.

Reply boundaries distinguish successive sequences within one pipeline. For
`SET first; GET first; SET second`, the GET waits for the first SET only;
the later SET must not extend its barrier. A first implementation held the
entire suffix behind the pipeline's maximum sequence. Static review found
that over-delay and the implementation was revised to retain sequence
boundaries while coalescing consecutive ready replies into one send. A
range with sequence zero has no AOF dependency.

Failure publication releases unchanged replies for commands that have
already committed, including records accumulated while the append was in
flight. It records a distinct failure-release watermark; it never advances
written-through. Internal expiry and eviction remain allowed during saved
failure and publish their own failure release, so reads cannot wait forever
for a disk that is still failing. Later WRITE commands and PING preserve
their current MISCONF admission. A sync error is not an append failure and
does not release records the writer has not written.

The ordinary barrier is a global sequence comparison, independent of which
connection or future driver supplied the bytes. Per-driver buffers can
therefore retain it if their merge publishes only a contiguous written
prefix of the global sequence.

## Sequence exhaustion

The existing dataset counter saturates at u64's maximum and then refuses
new rewrite captures while continuing service. Reusing that maximum in a
plain `written-through >= commit` test would falsely release every later
write. The proposal retains the current counter policy and conservatively
waits for both collecting and private pending buffers to become empty at
that boundary, or releases under a currently saved append error. It ignores
old maximum-valued success and failure watermarks. This cannot falsely
acknowledge new bytes, but continuous later traffic can prevent a global
drain; it is not a throughput or bounded-latency claim at exhaustion.

## Validation and performance comparison

The existing SIGKILL case is the before-change failure witness. The new
pipeline case must compare the complete reply stream against independently
constructed RESP, mixing writes, reads and no-effect commands. Immediate
AOF inspection after replies must find the complete expected records,
including EXEC and script effect blocks, without sleeping or querying INFO
first. Reply-order checks can pass before this change; they guard against
new overtaking bugs. Immediate-file assertions detect the old race when a
reply wins the writer's tick. Existing write-error cases retain their
original successful reply, rollback, retry and MISCONF expectations.

Run the network suite and complete project gate in CI, including the
separate AOF sync-fault job; inspect failures rather than weakening expected
replies. No build, compiler or test is run on the owner's MacBook. CI
acceptance, fault results and the performance comparison remain pending.

Another session measures performance on the i9-14900K through CI, using
`make firn-lto` builds, the same compiler and source except this proposal,
and identical AOF-on SET and MSET workloads at 2 and 8 cores. Record worker
and driver counts, client concurrency, pipeline depth, dataset, payloads,
machine and compiler revisions, and all other settings. Start with a timed
small sample to choose duration; interleave main, a twin of main and the
change. Proposed rejection criterion, separately for each command/core
combination: change/main throughput ratio below the lowest twin/main ratio
minus 0.05 (five percentage points of main's throughput). Retain every
sample and its pairing. Do not claim a performance result before this
comparison, or attribute a change using unequal settings.

## Implementation handoff

The proposal is implemented in the worktree, awaiting CI compilation and
runtime validation. No pin or submodule moved and no approval log was
written. The source review also found FUNCTION FLUSH's previously untaken
sequence and the saturated-counter comparison problem; both are handled
as described above, with the sequence design node updated in the same
proposal. No compiler rejection or new Whitefoot gap has been observed.

- `firn/store/sequence.wf`, its interface and initialization return an
  actual appended sequence, while the command callers, scripting entry
  and lazy-expiry helper carry that sequence to the originating client.
- `firn/protocol/` retains the client's maximum appended sequence, and
  `firn/server/server.wf` keeps connection-local reply boundaries whenever
  that sequence changes. Flush validates the buffer extents, waits on each
  range's own sequence in a guarded atomic statement and sends consecutive
  ready ranges together, preserving the existing output batching.
- `firn/persistence/` carries a covering sequence with each batch and
  publishes separate success and failure release. The ordinary writer and
  rewrite servicing loop wake from collecting state; a coalesced timer
  provides idle housekeeping. The existing single writer still waits on
  its filesystem calls during sync, rewrite switching and installation.
  If collecting records are empty after a bounded drain of 16 rewrite
  journal chunks, remaining journal chunks wait for the 10 ms housekeeping
  notification rather than the previous requested 100 microsecond pause.
  This can retain rewrite budget longer under intermittent traffic; it is
  an unmeasured pacing change. Live AOF records, a ready end cut and worker
  completion wake directly, and the end cut drains the entire sealed
  journal, so it is not a new condition blocking completion.
- `tests/durability.rs` adds
  `aof_pipeline_replies_stay_in_order_after_their_covering_append`, which
  covers an independent leading read, interleaved writes/reads/no-effect
  commands and a standalone FUNCTION FLUSH, and
  `aof_exec_and_script_replies_wait_for_all_effect_records_on_both_routes`,
  which checks complete EXEC, declared EVALSHA and fallback EVAL effect
  blocks. Both run on both runtime routes in CI. Immediate file checks
  expose reply-before-append races; ordering and no-extra-record checks
  are regression checks and are not claimed to fail before this proposal.
  These tests do not distinguish a read released after its earlier write
  from the same read unnecessarily held behind a later write. The
  per-range dependency inspection supports that requirement; its temporal
  selectivity has no runtime witness in this change.
  The existing kill case and all append-error expectations are unchanged.

`git diff --check` passed. This is whitespace evidence only: the pinned
compiler, new tests, existing durability and write-error cases, rewrite
interactions, full gate and performance criterion remain unverified until
the primary session's CI and 14900K runs. In particular, source inspection
does not establish the scheduler's progress or the reply-range bounds' compiler
acceptance.

## Independent static review

A fresh read-only GPT-6.1-Sol review examined the complete diff from
`9e3a3a0a73fb94ad834ad108e336cfbd8e7fd44c` to the final uncommitted worktree,
including this investigation, the changed code in context and its direct
consumers. It checked the project checklist and G1-G3/DC1-DC4 against the
relevant AOF, rewrite, sequence, capture, script, transaction, shutdown,
command-parts and memory-limit design nodes. It confirmed the correction
from a batch-maximum barrier to per-sequence reply ranges, including the
zero-sequence range that now sends without accessing Meta. No additional
finding remained within static scope. This review is not owner approval.

A1, C1, D1, G1-G3 and DC1-DC3 passed within that source-only scope; T2 was
not applicable because gate selection and wiring were unchanged. C2, T1,
T3, R1 and DC4 remain unverified where compiler acceptance, CI ratchet
results, runtime behavior or measurements are required. The reviewer ran
only small git/read/search commands and independently observed
`git diff --check` exit 0; no green suite was rerun.

Outstanding evidence is the pinned compiler's acceptance of reply-range
index/slice and effect proofs; single- and multiple-driver progress,
retry/shutdown/rewrite interaction; runtime selectivity of a read's barrier
against later or unrelated writes; and the 14900K throughput and rewrite
pacing comparison. The existing failing kill run establishes the original
bug, while all new and retained cases still require CI on this revision.

## Measured cost and a cheaper protocol proposal (2026-10-13)

This section is analysis and a proposal, not an implementation or an
approved replacement of the AOF design. The inspected Firn revision is
`5ded2213758d6d2ba2d20af909074e8ad934d3bc` (#97). The language and runtime
sources inspected are the pinned Whitefoot commit
`af7665be672d29912d878dd5da8c3932bb6f296c`. All source locations below refer
to those revisions, or to Redis 7.2.16. No local compiler, build, test or
benchmark was run.

### Observation before attribution

The owner supplied these results from
[14900K run 38418748888](https://github.com/Ming-Research/Firn-wf/actions/runs/38418748888):
AOF everysec, server on two cores, clients on E-cores, four rounds of five
seconds. The main rates are rounded; the ratios are measurements supplied
with the task, not independently re-extracted from the run here.

| Command | Pipeline | Main ops/s | #97/main | Twin/main range | Minimum accepted ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| SET | 1 | 775,000 | 0.613 | 0.979–1.010 | 0.929 |
| SET | 16 | 3,330,000 | 0.657 | 0.990–1.035 | 0.940 |
| MSET | 1 | 492,000 | 0.597 | 0.943–1.027 | 0.893 |
| MSET | 16 | 893,000 | 0.673 | 0.945–1.063 | 0.895 |

SET p99 at pipeline 1 rose from 0.077 ms to about 0.17 ms. All four
throughput results fail the previously proposed twin-minimum-minus-0.05
criterion. Eight-core and Redis AOF-on comparisons remain pending.

The source establishes added coordination and more frequent small appends.
It does **not** establish what fraction of this loss comes from each
mechanism. In particular, the result is not evidence of a healthy 10 ms
sleep, nor of exactly two operating-system context switches per command.

### Trace of one command and one append batch

1. **Execute and collect.** SET holds its key and Meta at
   `firn/commands/strings.wf:468`, appends and captures its sequence at
   `:488-491`. MSET holds its key set and Meta at `:826`, appends at
   `:863` and captures its sequence at `:866-867`. The new token is
   returned under the existing hold, not by another post-command lock.
   `firn/store/sequence.wf:111-114,146` distinguishes actual appended
   records from a command that produced no record. Existing admission
   and mutation holds remain part of each command's cost.
2. **Wake the writer.** Publishing collecting bytes makes
   `aof_writer_ready` true (`firn/persistence/persistence.wf:61-73`);
   the writer waits in `atomic ... when` at `:128`. Healthy bytes wake
   it directly. The separate timer at `:88-109` sets a coalesced 10 ms
   housekeeping flag; it is not a healthy batching deadline. Failed
   appends still retry at 100 ms (`firn/persistence/rewrite.wf:313-318`).
3. **Seal a batch.** The writer exchanges all bytes currently collected
   with its private empty buffer and snapshots their covering sequence
   in one Meta hold (`firn/persistence/rewrite.wf:288-304`). It does not
   wait for all clients, a pipeline to finish, or a particular batch size.
4. **Append.** `append_all` passes the entire remaining byte range to
   `append_once` (`firn/persistence/persistence.wf:33-58`, call at `:42`).
   A full successful batch uses one host append operation. Positive
   short writes require more operations; the host may retry EINTR.
   Thus one `write(2)` per full successful batch is the ordinary case,
   not a guaranteed syscall count for every outcome.
5. **Publish completion.** After the host append returns, the writer
   parses the batch to maintain its rewrite boundary
   (`firn/persistence/rewrite.wf:926`;
   `firn/persistence/rewrite-boundary.wf:33-74`). Only then does it publish
   written-through (`rewrite.wf:931-940`). It also allocates a fresh
   65,536-byte pending buffer inside that Meta hold (`:943`). Parsing
   and allocation predate #97, but now run more often and precede reply
   release. They cannot be attributed to a newly added parser.
6. **Release replies.** A connection waits at
   `firn/server/server.wf:211` for its reply run's sequence to be covered
   by written-through, or by the separately documented append-failure
   exception (`:103-119`). Consecutive ready runs coalesce under that
   hold (`:212-245`) and send at `:249`. An already-covered guard can
   complete without parking. A zero-sequence prefix bypasses Meta.

A healthy iteration with no rewrite and no sync due takes **seven Meta
atomic statements per append batch**: writer readiness (`persistence.wf:128`),
buffer exchange (`rewrite.wf:288`), pre-append size (`:905`), completion
(`:931`), sync eligibility (`:330`), cancellation (`:16`) and rewrite
scheduling (`persistence.wf:150`). Cancellation also reads ServerState
(`firn/store/store.wf:441`). Much of this machinery used to run at the
10 ms cadence; eager draining raises its frequency with the append count.
It is seven entries, not seven necessarily contended locks or seven
necessarily suspended contexts. Sync, error and rewrite paths add work.

For a batch of B commands releasing G connection contexts, the useful
coordination is one writer wake/activation and up to G completion
notifications: a fan-in to one writer, then a fan-out to the connections.
An uncovered connection experiences a connection-to-writer-to-connection
source-context dependency. The writer can already be running, several
connections can share its batch, and true guards need not suspend.
Therefore `(1 + G) / B` is a count of useful notification edges per
command in this model, not an operating-system switch rate. Guard retries
and host I/O completion can add further handoffs.

There is also a host-I/O completion path. Pinned Whitefoot's
`compiler/src/backend/ordinary_values.c:1190-1198` submits append as a
completion operation. `completion/bridge.c:5531-5564` dispatches it; the
Linux ring backend has no APPEND case
(`completion/linux_io_uring.c:459-513`), so this path uses the file adapter
queue (`completion/file_adapter.c:930-951`). Its POSIX implementation
calls `write` (`completion/file_posix.c:310-317`). A joining thread can
claim its own queued operation (`file_adapter.c:954-1001`), so a helper
thread switch is possible, not mandatory on every append. Moving the
Firn append into the connection does not itself remove this runtime path.

### Batching that exists, and batching that is unknown

The connection already executes every complete command in its received
input before flushing (`firn/server/server.wf:556-607`), except when reply
bytes exceed 16,384 (`:599-600`). It records sequence boundaries
(`:573-584`), not a wait after each execution. A complete 16-command
write-only pipeline whose records are all written can use one guard and
one send. If the writer takes cuts inside that pipeline, it can instead
require several prefix waits and sends, up to one per dependent command.
It cannot blindly replace mixed-pipeline boundaries with a maximum token:
`W1; R1; W2` must let R1 depend on W1 rather than W2.

For approximately 50 closed-loop clients, pipeline 1 gives roughly
1–50 commands available/in flight, while pipeline 16 permits roughly
800. These are concurrency envelopes, not measured batch sizes. The
benchmark's client count must be taken from its exact mode: threaded
clients use `-c 50` (`redis-bench.sh:980-982`), while compare-mode process
clients use three connections per client CPU (`:1086-1087`). Receive
fragmentation and individual command Meta releases mean a pipeline-16
append can still contain fewer than 16 commands. With no batching epoch,
it is reasonable to expect small pipeline-1 batches and larger but
variable pipeline-16 batches; claiming an average of 50 or 800 would be
unsupported.

By contrast, main could collect many thousands over 10 ms while replying
independently: 775,000 SET/s corresponds arithmetically to about 7,750
commands per 10 ms. That is an illustration using the supplied rate, not
a recorded main batch size. #97 removes that unacknowledged-write window
and exposes per-batch write submission, seven metadata entries, parser
work, allocation and completion notification much more frequently.

### Redis's execution structure

Redis normally processes complete buffered commands for ready clients on
its main event loop (`src/networking.c:2545-2605`). `beforeSleep` finishes
the pending-read path (`src/server.c:1651`), flushes the collected AOF
(`:1720-1724`), then sends pending replies (`:1739`).
`src/aof.c:1131` passes the whole buffer to `aofWrite`; its loop at
`:1028-1044` normally makes one `write(2)` and continues for short writes
or EINTR. The loop's iteration provides a batching boundary without
moving the append to a separate Firn-style context.

Redis defaults to one I/O thread and threaded reads off
(`src/config.c:3172,3074`). Even when network I/O threads are enabled,
they defer command execution to main
(`src/networking.c:2589-2595,4506-4535`); AOF append still occurs on main.
Optional network or background-fsync threads are distinct from a
cross-context command-to-AOF-writer handoff. Redis everysec's fsync
postponement and failed-append exceptions remain those documented above;
the proposed healthy guarantee does not use its two-second bypass.

### Whitefoot constraints and two concrete gaps

File I/O cannot run while an ordinary Whitefoot atomic statement holds
the file state. Pinned [SHARE-2](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2342)
(`spec/kernel-spec.md:2342-2347`) counts atomic as waiting but forbids
waiting calls in its guard or body; `std::fs::append_once` and `sync_file`
explicitly wait (`lib/std/fs/module.wfm:78-85`). Atomic-bound references
end with the block (`spec/kernel-spec.md:2341`), so borrowing the file
out of an atomic block is not a lease. Spawn takes owned values
([WAIT-3], `:2282`), not a parent file reference shared among connections.

**Gap 1: shared exclusive ownership of a linear file.** A correct
leader would take ownership under a brief atomic statement, append outside
it, then return the resource and publish completion. But
[`Shared<T: drop>`](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2396)
and `shared_new<T: drop>` (`:2575`) exclude
[`WriteFile`, an opaque nodrop type](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/lib/std/fs/module.wfm#L23).
PROV-6 (`spec/kernel-spec.md:773-784`) propagates linearity through
Option, structs and boxes, regardless of the current variant. Thus
`Shared<Option<AofLog>>` cannot hold the proposed available/taken state.

Minimal complete witness function, normatively inadmissible at the drop
bound; no compiler diagnostic was obtained because no compiler was run:

```whitefoot
alias WriteFile = std::fs::WriteFile;

fn park(file: WriteFile) -> slot: Shared<Option<WriteFile>> pure {
  let present = Some<WriteFile>(value: move file);
  return shared_new::<Option<WriteFile>>(value: move present);
}
```

The requested capability is scoped exclusive leasing of a linear resource
with mandatory final recovery. Its owning scope must close admission,
wait for every lease/operation, extract the resource exactly once and
explicitly close it. Each error, disconnect and shutdown path must return
the lease; only one lease can exist. This is a Whitefoot design request,
not a proposal to remove `nodrop`, weaken `drop`, reopen a file per
connection, leak handles or pass raw descriptor integers.

**Gap 2: helper guards lose selective wake filtering.** #97 naturally
uses the helpers at `server.wf:211` and `persistence.wf:128`. Pinned
[`compiler/src/lowering/builder/wake.rs:297-357`](https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/lowering/builder/wake.rs#L297)
recognizes direct Boolean/integer conditions, but an ordinary helper call
falls through to `Condition::True`; `:371-375` then emits no selective
wake test. `compiler/src/backend/completion/bridge.c:3164` treats an
empty test as true. Meta writes can therefore select a connection whose
actual completion guard remains false.

This is not a simultaneous broadcast to all clients. The runtime selects
the oldest eligible watch and passes a reservation through candidates
(`bridge.c:3388-3415,3431-3455,4110-4118`). A read-only release does not
start a fresh write notification (`:4147`), but can forward an existing
sweep. False resumes and Meta reacquisitions are consequently a concrete
possible amplifier; their frequency and share of the regression have not
been measured. SHARE-3 permits repeated guard evaluation (`spec:2358`),
so this is a compiler performance gap, not evidence of wrong replies.

A minimal source witness for the compiler request is:

```whitefoot
struct Progress {
  written: u64;
  unrelated: u64;
}

fn covered(progress: &Progress, target: u64) -> ready: Bool reads(progress.written) {
  return progress^.written >= target;
}

fn wait_helper(state: Shared<Progress>, target: u64) -> result: unit pure waits {
  atomic p = &state when covered(progress: p, target: target) {
  }
  return unit;
}
```

While written remains below target, update unrelated. Compare the emitted
wake predicate and false-resume counters with an equivalent direct
comparison in a Whitefoot CI witness. Acceptance requires the natural
helper form to retain an effective safe completion filter, including its
call boundary. Firn must not inline/re-spell the guard to hide the gap.

### Candidate protocols and estimated coordination cost

These estimates count operations and dependencies, not measured cycles.
Let B be commands per append, G the connections sharing it and K the
ready prefixes a connection sends for its available input.

| Proposal | Useful append coordination | Batch/syscall consequence | Constraint and likely cost |
| --- | --- | --- | --- |
| Connection leader/follower group commit | One connection claims the exclusive resource; leader has no separate Firn writer round trip; at most G−1 followers need completion notifications | One full host append for all bytes committed at the claim; still one write per full batch, with partial-write retries | Needs the linear-resource lease gap closed. Approximately two coordinated ownership/publication transitions per append can replace the seven-entry writer cycle if accounting is captured at claim and completion. Runtime file submission remains. |
| One highest-sequence wait for an available write-only pipeline | One useful connection completion wait/send for P available writes instead of up to K | Does not stop another appender cutting between those P commands; may leave write syscall count unchanged | Pipeline processing is already implemented. Potential savings are prefix guard entries, boundary scanning and sends, predominantly at depth 16. Mixed read/write pipelines must retain their selective boundaries. Almost no depth-1 saving. |
| Dedicated writer, eager wake and a brief spin | Keeps producer→writer→connection dependency; spin may avoid a park if work arrives during it | May enlarge a batch during a deliberately bounded gather period, or shrink it if it repeatedly flushes immediately | Eager wake already exists. Spinning can occupy one of the two available cores, contend on Meta and delay producers; zero spin is the baseline, not a 10 ms wait. No throughput improvement is established. |
| Driver/event-loop epoch owns execution, append and reply sends | Redis-like zero separate append-context handoff for clients handled in that epoch | One append after the epoch's ready command groups | Requires a design for ready-request multiplexing, ownership of host handles, per-driver sequence merging and global publication. The current spawn-per-connection loop has no such epoch. This is broader than the next isolated comparison. |

Separate improvements apply to either writer architecture: take size and
health alongside the buffer exchange rather than reading them again;
move rewrite/stop/sync scheduling back to actual control notifications and
periodic maintenance rather than polling everything after every small
append; and reuse storage rather than allocating 64 KiB under the
completion hold. Reuse needs an actual cheap, valid initialized-prefix
representation, not a claim that a per-byte clearing loop is free.
Boundary correctness must remain: removing the parser without an encoder
summary and startup/partial-write proof would retire a required check.
Keeping parsing initially permits a clean attribution of coordination
savings; a later boundary-summary change needs its own comparison.

### Recommended protocol, conditional on Whitefoot support

Recommend **connection-side leader/follower group commit with scoped
linear-resource ownership**, while preserving input batching and selective
reply ranges. Fix the helper-guard filtering gap first in Whitefoot and
measure that change alone; it could remove a substantial part of the
present cost without changing Firn's protocol. The recommendation is not
that the current pin can already implement the lease, nor that source
inspection proves group commit will pass the throughput line.

Proposed flow:

1. A connection executes its complete available input using the existing
   mutation/commit boundary. It retains reply dependencies, including
   EXEC/script units and expiry/eviction records. A write-only run may
   use its highest token; an intervening read must not gain a later token.
2. At reply release, if the required sequence is already written, send
   without taking the file. Otherwise elect one leader, atomically claim
   the complete active-file/pending-buffer/boundary/health bundle and
   seal all currently committed collecting bytes with their final sequence.
   The election and cut must be one coherent protocol, not independent
   observations followed by an unchecked claim.
3. Release atomic exclusion and append as the resource's unique owner.
   Other connections keep executing and collecting while it performs
   host I/O. A follower parks only while its required prefix is uncovered
   and someone else owns the resource; when ownership returns, it either
   sees completion or can become the next leader. Independent reads still
   send without a file lease or completion wait.
4. Return the bundle and publish full-append completion coherently.
   Keep a compact completion object separate from mutable keyspace
   bookkeeping for followers' observation; this separates responsibilities
   even after compiler wake filtering is fixed. Update Meta byte/error
   accounting in the same publication transition. Release the lease
   **before** network sends, so a slow receiver cannot hold the AOF file.
5. The maintenance context uses the same lease for sync, retained-byte
   retries and rotation; it does not compete as an eagerly woken ordinary
   writer for every healthy client append. It still drains expiry-only
   records promptly: internal producers must request maintenance service,
   or another capable connection must claim their batch. Background work
   cannot depend on a future client write. Shutdown closes lease admission,
   lets in-progress accepted work finish and performs the existing final
   append/sync attempt before explicit file/directory closes.

The resource protocol must order retained failed bytes before new bytes,
give file rotation exclusive ownership, preserve the sealed S1 prefix and
rotation-reserve charges, and bound/fairly hand off an appender's lease.
The rewrite scanner keeps its own temporary file and must not monopolize
the active-file lease during scanning or worker waits; active-file appends
must continue between the finite sealing/switching operations. Combining
election, accounting and publication into roughly two transitions per
append is a design target for the new facility, not a proved count of
locks or waiting calls in an implementation that does not yet exist.
It must not hold Meta across host I/O. Written-through advances only after
the entire batch is appended. The existing separately recorded everysec
failed-append reply exception remains explicit; failure is never called
written. Slow fsync gives no successful-reply bypass. Saturated sequence
handling retains the conservative rule until a separately approved
sequence-reinitialization design exists. A future per-driver buffer merge
must publish a contiguous global written prefix, not merely the largest
sequence seen on one driver.

The leader eliminates the additional Firn source-context round trip for
itself, but not for G−1 followers. For a large shared group, that direct
benefit alone is small. The fuller proposal also removes repeated writer
control polling and reduces the seven-entry batch cycle; fixing false
wakes matters independently. Group commit without these distinctions
could still be slow, or make more tiny appends. There is no deliberate
sleep or unbounded busy spin in the recommended common path. Redis-like
epoch batching remains a possible later direction if append histograms
show that election still seals batches too early.

Expected direction and the hurdle for the four measured rows are:

| Workload | Expected benefit and remaining cost | Minimum improvement over current #97 to reach supplied line |
| --- | --- | ---: |
| SET, pipeline 1 | Less leader coordination latency; fewer batch bookkeeping holds and false wakes. Followers still wait and small appends still cost host I/O. Aim to reverse the p99 increase, not promise 0.077 ms. | about 51.5% more throughput; ratio at least 0.929, about 720k ops/s using rounded main |
| SET, pipeline 16 | Input grouping plus fewer prefix waits/sends for write-only runs; fewer writer control transitions per append. Multiple leaders can still cut during another connection's pipeline. | about 43.1% more; ratio at least 0.940, about 3.13M ops/s |
| MSET, pipeline 1 | Same coordination reduction; key-set acquisition, encoding and larger-record processing remain. No basis to assume SET's cycle saving transfers unchanged. | about 49.6% more; ratio at least 0.893, about 439k ops/s |
| MSET, pipeline 16 | Amortized coordination and sends; large records and mutation work limit gains, while batching can reduce submissions per command. | about 33.0% more; ratio at least 0.895, about 799k ops/s |

Those percentages are algebraic acceptance hurdles
(`limit / current_ratio - 1`), **not forecasts**. The prediction is a
directional reduction in coordination demand, strongest for latency at
depth 1 and repeated prefix work at depth 16. The actual ratio may still
miss every line; claiming a numerical forecast requires the missing
batch/wake/CPU evidence.

### Measurements that distinguish the hypotheses

Before implementing Firn changes, retain the pending eight-core and
Redis 7.2.16 AOF-on results. Resolve the exact main/twin heads, compiler,
native/fallback route, workers/drivers, affinity, client mode and count,
key/value sizes, fsync and rewrite settings from the artifacts; two server
cores are not automatically two drivers, and about 50 clients is not an
append batch size. Redis's single command/AOF thread and optional network
threads must be stated with their settings. Record postponed Redis appends
so that its exception is not mistaken for a stronger healthy guarantee.

On the 14900K through CI, use these independent comparisons:

- **Wake-filter attribution:** identical Firn source, current pinned
  compiler versus a Whitefoot revision fixing helper guards, with its own
  twin control. Check emitted predicates and existing guard-counter
  instrumentation for true/false handoffs, reservations, notification
  sweeps and re-parks. This isolates the compiler gap from a new protocol.
- **Coordination and batching attribution:** after the language facility
  has its own correctness evidence, same Firn source baseline versus
  leader/follower changes, same compiler and settings, retaining parser
  behavior for the first comparison. Report commands/bytes/connections
  per append, full/short writes, appends per command, completion waits
  that actually park, wake-to-append and append-to-send delay, lease
  elections and follower count. Sampling/counters must not add a global
  lock per command; measure their overhead separately from throughput.
- **Cost breakdown:** sampled user/system CPU and call stacks for runtime
  wake reservation/queueing, Meta holds, file adapter, boundary parsing
  and allocation; syscall/context-switch tracing in separate diagnostic
  runs. Do not infer CPU service time from reciprocal throughput alone.
- **Acceptance:** interleave main, its twin, #97 and the candidate for
  SET/MSET, pipeline 1/16, at both two and eight cores. Reuse the observed
  five-second scale initially, extending only if spread prevents a
  decision. Apply `candidate/main >= min(twin/main) - 0.05` separately
  to each command/depth/core row, with p50/p99 and server/client CPU
  reported. The new eight-core line comes from its own twin, not the
  two-core limits above. Reject a supposed scheduling improvement if
  its parked waits/false handoffs do not decrease, or it only increases
  CPU occupancy while throughput remains below the line.

Retain the kill, immediate-AOF, ordered pipeline, script/EXEC, append-error,
sync-fault, rotation-backlog and shutdown checks. The proposed lease also
needs contention/fairness, leader failure/disconnect, background-expiry-only
progress, partial append, rotation racing append and mandatory final
resource recovery cases. Existing timing/order tests alone cannot prove
that reads avoid later or unrelated writes; that temporal witness remains
required. Neither improved throughput nor an error-release watermark is
evidence that bytes were written before successful healthy replies.

This analysis names two Whitefoot requests, with witnesses and acceptance
above. Their language/runtime changes belong to Whitefoot, and the
resource facility requires an owner decision. No workaround, Firn
implementation, design-tree ruling, approval log or pin movement is made
by this proposal.

An independent read-only GPT-6.1-Sol review checked this section against
the measured Firn revision, pinned Whitefoot sources and Redis 7.2.16.
It found no defect within proposal scope. It verified the consequential
source traces and acceptance arithmetic, while leaving exact measurement
provenance, batch/wake attribution and the proposed resource facility's
soundness and implementation evidence unverified. Only source reads and
small git commands ran; `git diff --check` passed. This is review of an
analysis, not approval or validation of the proposed protocol.

## Firn-only batch overhead reduction

This separate, uncommitted change is intended to be measured against
`5ded2213758d6d2ba2d20af909074e8ad934d3bc`, retaining its single writer,
reply barrier, boundary parser and compiler pin. It does not implement
leader/follower election or bypass either filed P0 Whitefoot request:
`firn-req-helper-guard-wake-filter` and `firn-req-linear-resource-lease`.
The earlier proposal remains conditional on those language/runtime changes.

### Eight-core evidence and scope

The owner supplied the following additional results from
[run 38419950603](https://github.com/Ming-Research/Firn-wf/actions/runs/38419950603):
server on eight P-cores, clients on E-cores, AOF everysec, four five-second
rounds, revision `5ded221` (#97).

| Workload | Main ops/s | #97/main | Main → #97 p99 |
| --- | ---: | ---: | --- |
| SET, pipeline 1 | 1.76M | 0.201 | 0.039 → approximately 0.21 ms |
| SET, pipeline 16 | 1.05M | 0.606 | not supplied |
| MSET, pipeline 1 | 861k | 0.319 | not supplied |
| MSET, pipeline 16 | 825k | 0.592 | not supplied |

Depth-one throughput retains only 20–32% of main on eight cores. A single
global collecting buffer can merge records from different drivers; these
ratios alone do not prove that it never merges them. They do show that the
current coordination/batching cost is much worse there. Batch density and
runtime wake evidence must separate tiny groups from false wakes and
contention. The supplied eight-core data lacks twin ranges, so no eight-core
numerical veto line can yet be calculated. Main's SET pipeline-16 scaling
regression from two to eight cores is a separate baseline issue.

### Changes and invariant

For an ordinary healthy nonempty append outside rewrite, control and
maintenance wakes, the source now enters Meta twice rather than seven times:

1. The guarded readiness statement also exchanges collecting/pending
   buffers and snapshots incremental size, synced size, append/sync health,
   covering sequence and outer-execution count into owned `AofWork`
   (`firn/persistence/persistence.wf:132–140`,
   `firn/persistence/rewrite.wf:285–342`).
2. After append and the unchanged boundary parser, one completion statement
   publishes byte accounting, health, written-through and counters
   (`firn/persistence/rewrite.wf:969–1001`).

Only the sole writer changes active-file size and persistence health.
Commands may add collecting records during I/O, but do not invalidate this
file snapshot. The pending covering sequence and count travel together;
completion uses that snapshot, not the later collecting commit sequence.
On append failure the existing failure-release sequence still covers the
then-current commit sequence without claiming it was written. Retained
bytes/counts survive retry and partial rollback; their complete drain
counts once. Saturated sequences retain the previous conservative barrier.
There is no Meta hold across append, truncate, sync or network I/O.

Sync eligibility, cancellation and automatic/recovery rewrite scheduling
run on the existing coalesced 10-ms maintenance tick or actual control
notifications, rather than after every append. Shutdown and changes to
automatic-rewrite thresholds publish a control pulse in their existing
holds; a manual rewrite request is itself a readiness notification.
A sync attempt adds one combined error/size/recovery publication. The
everysec eligibility/new-byte rule, 100-ms append retry rule and final
forced append/sync remain. Maintenance scheduling can add up to one tick
to the previous eager eligibility check; it does not delay healthy appends
or their replies until a tick.

Rewrite wakes likewise combine their state snapshot and ordinary buffer
exchange. Cut/rotation paths take a fresh snapshot after file identity and
size can change. Journal-drain coordination and cold cancellation/scheduling
holds remain; this is not a claim that an entire rewrite iteration takes
two holds. Cancellation still publishes an abort through the existing
`aof_rw_cancelled` helper. Background expiry uses the same eager collecting
notification. The helper-guard compiler gap remains visible and unfixed.

### Counters and their limits

INFO persistence reports three saturating lifetime counters when AOF is on:

- `firn_aof_append_batches`: nonempty `append_all` attempts, including
  failures and retries; one attempt can make multiple host writes after
  short writes, so this is not a syscall counter.
- `firn_aof_written_batches`: attempts that completely drain their retained
  batch, including a finally successful retry.
- `firn_aof_appended_commands`: outer AOF-producing executions covered by
  those complete drains. SET and MSET each contribute one; a script or
  EXEC contributes one for its complete effect block; an internal expiry
  execution also contributes one. No-effect commands contribute zero.

Collecting counts advance within the existing sequence-commit hold, and
batch counters publish within completion. INFO reads its existing snapshot.
No additional global lock is taken per command. Before saturation,
`Δappended_commands / Δwritten_batches` measures complete-drain group
density for the proposed SET/MSET workloads. Attempts minus complete drains
identifies retries/failures; it does not measure parked contexts. Counter
publication overhead is included in this candidate and must be reported
as part of the attribution, not assumed zero.

### Two additional Whitefoot gaps

**Constant-cost pending-buffer reset.** The representation remains
`Box<Slots<u8>>`: initialized prefix length plus backing capacity. After a
complete append it still replaces pending with a fresh 64-KiB allocation
inside completion so its memory accounting remains consistent. At pin
`af7665be672d29912d878dd5da8c3932bb6f296c`, WIN-1
(`spec/kernel-spec.md:831–835`) forbids assigning the initialized length
directly. OP-10 (`:1198–1207`) and the published window operations
(`:2518–2555`) have no native reset. `take_back` removes one element, and
stdlib vector truncate (`lib/std/collections/vector/grow-vector.wf:117–150`)
loops per element. No constant-cost guarantee follows from that loop.

The missing capability has this minimal interface witness (a required
primitive, not an implemented or compiler-tested function):

```whitefoot
fn clear(bytes: &Box<Slots<u8>>) -> result: unit writes(bytes) contract {
  ensures bytes^.inner.len == 0_u64;
  ensures bytes^.inner.cap == entry(bytes)^.inner.cap;
}
```

For byte slots it must preserve backing without allocation, copying or
traversing each byte. Switching encoders to `Box<Array<u8>>` plus a used
length just to escape this restriction would hide the gap and is not done.
Reuse therefore remains blocked; no per-byte clearing loop is presented
as free.

**Actual connection-park observation.** The pinned `std::process` interface
(`lib/std/process/module.wfm:29–57`) offers no context scheduling counter,
and guarded atomic completion offers no suspension outcome. SHARE-3
(`spec/kernel-spec.md:2350–2359`) makes guard evaluation count unobservable.
The minimal semantic witness is:

```whitefoot
fn wait_written(mark: Shared<u64>, target: u64) -> result: unit pure waits {
  atomic value = &mark when value^ >= target {
  }
  return unit;
}
```

The caller needs to distinguish immediate completion from this statement
suspending at least once because its guard was false. A separate failed
readiness check can race with publication before the atomic, falsely
counting a park that never occurred. Repeated false wake/repark events and
lock-contention suspension also need distinct definitions. A context-local
runtime observation or wait outcome would permit this without a per-command
global lock. The requested actual-park INFO field is omitted until that
capability exists; no fabricated zero or attempt counter substitutes for it.
These are source/interface gaps, not observed compiler rejections: no
compiler was run locally.

### Expected direction and next comparison

These are directional hypotheses for this Firn-only change, not numerical
predictions or claims of passing the veto line:

| Cores | Workload | Current #97/main | Expected direction; remaining constraint |
| ---: | --- | ---: | --- |
| 2 | SET p1 | 0.613 | Throughput up, p99 down from fewer Meta transitions; writer round trip and fresh allocation remain. |
| 2 | SET p16 | 0.657 | Throughput up as batch overhead falls; parser, input grouping and send boundaries unchanged. |
| 2 | MSET p1 | 0.597 | Throughput up; larger encoding and key acquisition limit relative benefit. |
| 2 | MSET p16 | 0.673 | Throughput up; amortized holds save less if record work dominates. |
| 8 | SET p1 | 0.201 | Strongest opportunity to reduce shared Meta/wake demand and latency; separate writer and broken wake filtering can still dominate. |
| 8 | SET p16 | 0.606 | Throughput up if Meta coordination dominates; separate main scaling regression remains outside this change. |
| 8 | MSET p1 | 0.319 | Throughput up and latency down if coordination dominates; no promise of recovery to the two-core ratio. |
| 8 | MSET p16 | 0.592 | Throughput up from fewer holds; larger records and cross-driver coordination remain. |

Interleave this candidate and `5ded221` on the same pinned compiler,
LTO build and 14900K settings, alongside main and its twin. Retain four
five-second rounds initially, two/eight server P-cores, E-core clients,
approximately 50 clients (confirm the exact count in the artifacts),
SET/MSET and pipelines 1/16. Record exact affinities and
heads, AOF everysec, no rewrite during the throughput window, key/value
sizes, per-round ops/s and p50/p99, user/system CPU. Read counters before
and after each round. Compare complete drains per second, outer commands
per drain and append attempts per drain; exclude saturation. A faster
writer may make smaller groups, so improved throughput alone does not
establish better grouping. Use separate runtime/profile diagnostics for
actual parks, false wakes and syscall counts until the API gap is closed.

The attribution is falsified if fewer source holds do not reduce relevant
CPU/coordination cost or candidate throughput is indistinguishable from or
worse than `5ded221` within twin noise. Overall acceptance still requires
`candidate/main >= min(twin/main) - 0.05` independently for every row;
the supplied two-core lines remain 0.929, 0.940, 0.893 and 0.895. Eight-core
lines require its own twin data. Neither this partial optimization nor
its two-hold count promises those lines. Preserve kill durability, ordered
pipeline, script/EXEC, failed-write replies/recovery, sync-fault, rewrite
backlog and shutdown cases.

The added counter case checks both socket routes, zero initialization,
SET/MSET versus key count, no-effect/read exclusion, one EXEC unit and
16 pipeline executions with 1–16 possible complete batches. It checks
acknowledged file bytes before INFO so polling cannot conceal an early
reply. The pre-change revision lacks these fields and fails this case.
Existing failed-append cases additionally distinguish failed attempts
from complete drains and require retained execution counts to publish
once on recovery. Existing coverage, not changed expected replies, guards
the scheduling/refactoring semantics. All compilation, gate, fault and
performance evidence for this worktree remains pending CI; no local
build, test suite or benchmark ran.

An independent read-only GPT-6.1-Sol review inspected all 13 changed files
against `b4d1f79bd82930480233464da5998aff7debe067`, including affected
writer/rotation paths, interfaces, sequence consumers, counter cases and
design correspondence. It found no remaining defect within scope; one
source-line citation was corrected. Checklist source checks passed or were
not applicable; compiler acceptance, CI correctness, measurement provenance
and candidate performance remain unverified. Pins and submodules did not
move. Only reads/searches and `git diff --check` ran.

## Measured result of the batch overhead reduction (2026-10-13)

Two 14900K `redis-bench.yml` runs measured #97 at `3d0f8a8` (the batch
overhead reduction above) against main, a twin of main as the noise
control, Redis 7.2.16 with the same AOF settings, and `22956c5`
(`tmp/inline-guards`, measurement only, never to merge: `3d0f8a8` with the
AOF hot-path wait guards written inline instead of through helper calls,
which estimates what selective wake filtering of helper-call guards,
`firn-req-helper-guard-wake-filter`, could gain). Every side used the
pinned release `wf-af7665be672d`, AOF everysec, the server on the P-cores
named below and the clients on the sixteen E-cores 16–31, four interleaved
rounds. Rates are the median of the four rounds; a ratio range is the
smallest and largest per-round ratio against main in the same round. The
veto line is `#97/main >= min(twin/main) - 0.05`. These are the numbers
the Whitefoot requests `firn-req-linear-resource-lease` and
`firn-req-helper-guard-wake-filter` cite.

[Run 38430979567](https://github.com/Ming-Research/Firn-wf/actions/runs/38430979567),
two P-cores (CPUs 2 and 4), main `f12587f`:

| Workload | Main ops/s | Twin/main | #97/main | Veto line | Inline guards/main | Redis ops/s | #97/Redis | p99 ms main → #97 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| SET, pipeline 1 | 777k | 0.986–1.001 | 0.607 (0.600–0.615) | 0.936 | 0.491 | 375k | 1.26 | 0.083 → 0.167 |
| SET, pipeline 16 | 3.30M | 0.962–1.029 | 0.693 (0.658–0.708) | 0.912 | 0.554 | 1.61M | 1.42 | 0.439 → 0.483 |
| MSET, pipeline 1 | 490k | 0.994–1.013 | 0.592 (0.584–0.613) | 0.944 | 0.513 | 220k | 1.32 | 0.159 → 0.219 |
| MSET, pipeline 16 | 896k | 0.940–1.072 | 0.662 (0.631–0.706) | 0.890 | 0.623 | 404k | 1.46 | 2.627 → 1.423 |

[Run 38433211566](https://github.com/Ming-Research/Firn-wf/actions/runs/38433211566),
eight P-cores (CPUs 0, 2, …, 14), main `372db37`:

| Workload | Main ops/s | Twin/main | #97/main | Veto line | Inline guards/main | Redis ops/s | #97/Redis | p99 ms main → #97 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| SET, pipeline 1 | 1.71M | 0.960–1.156 | 0.211 (0.211–0.222) | 0.910 | 0.206 | 384k | 0.95 | 0.039 → 0.215 |
| SET, pipeline 16 | 1.10M | 0.812–1.366 | 0.597 (0.540–0.666) | 0.762 | 0.582 | 1.62M | 0.37 | 1.231 → 2.507 |
| MSET, pipeline 1 | 866k | 1.000–1.092 | 0.314 (0.309–0.322) | 0.950 | 0.304 | 224k | 1.21 | 0.087 → 0.239 |
| MSET, pipeline 16 | 859k | 0.920–1.111 | 0.565 (0.544–0.602) | 0.870 | 0.540 | 413k | 1.14 | 2.791 → 2.287 |

`#97/Redis` is the median of per-round ratios; on eight cores SET pipeline
16 ranges 0.37–0.44 over the four rounds.

What this shows, no wider than these two runs:

- #97 fails the veto line in every row on both core counts. The batch
  overhead reduction did not move the two-core ratios materially from
  `5ded221`'s 0.613, 0.657, 0.597 and 0.673 in run 38418748888, a
  different run; this pair has no interleaved `5ded221` side, so the
  attribution test written above ("indistinguishable from `5ded221` within
  twin noise") is not settled by it.
- On eight cores #97 at SET pipeline 16 runs at 0.37 of Redis, and at SET
  pipeline 1 at 0.95 of Redis, where main leads Redis 4.5 times. Main
  itself is at 0.68 of Redis at SET pipeline 16 on eight cores, the
  separate scaling regression of board item `firn-write-meta-aof`.
- Writing the hot-path guards inline did not raise throughput: it is below
  #97 in every row, by 0.04–0.14 of main on two cores and 0.005–0.025 on
  eight. So these runs give no measured estimate of a gain from selective
  wake filtering of helper-call guards; any gain the request expects rests
  on the analysis above, not on this measurement.

## Group commit (firn-group-commit)

Design for board item `firn-group-commit`, decision card
`firn-group-commit-design`. Firn revision read: main at `7f8b4f8`; Whitefoot
main after Whitefoot#510 (linear states in shared objects, `shared_close`)
and Whitefoot#517 (contexts taking turns with a parked file, the program
`tests/programs/linear_shared_file_turns.wf`). The owner ruled option A on
Whitefoot's card `lease-direction`: no lease construct; the file sits in a
one-slot window that is a shared object's state, a context takes it out in an
atomic statement, uses it outside every statement and puts it back.

### Question and prior criterion

Can firn reply only after the record of the command and of every earlier
command is in the file, as Redis 7.2.16 does
([beforeSleep](https://github.com/redis/redis/blob/7.2.16/src/server.c#L1721-L1739)),
without #97's writer handoff on every batch, and keep AOF-on throughput within
noise of main? Reject the design if the kill case (acknowledged writes after
SIGKILL and restart) loses any write; if pipelined replies change order; if bytes are marked written before a complete
successful append; if the aof-sync-fault job, the write-error cases or the
rewrite cases change an expected result; or if the throughput criterion below
fails.

### Design

Collection is unchanged: a command appends its record to Meta's collecting
buffer under its existing hold and commits its sequence. What changes:

1. **Dependency.** Output waits for the commit sequence it could have
   observed: before a connection sends the replies it holds, and before a
   subscribed connection's output context sends a frame, it reads the
   commit sequence and waits until it is written. Redis 7.2.16 sends every
   reply and message of an event-loop iteration after that iteration's AOF
   write, so after a process kill no client has seen a value the file lacks;
   #97's per-command sequences and [reply
   boundaries](#proposal-and-ordering-argument) let a read show another
   client's unwritten write, and are dropped (decision card
   `firn-group-commit-reads`, option A). Every collected record takes a
   sequence, an AOF-only record included, so the commit sequence covers
   it. A reply that observed nothing still waits for earlier records: the
   cost is one Meta read per send with an append-only file, and a read
   sent while writes are in flight waits for the next group, as in Redis.
2. **Desk.** The active `AofLog` (file, boundary, manifest, directories) and
   the pending buffer move from the writer's locals into a shared object, the
   desk: a one-slot window holding them, the written-through sequence and the
   failure-release sequence. The desk is linear; each context that holds a
   handle ends it with `shared_close`.
3. **Leader.** At reply time, if the run's sequence is covered, send. If not,
   one atomic statement on the desk either finds it covered, or takes the log
   out of the slot (leader), or waits until one of the two holds (follower).
   The leader, in one Meta statement, exchanges the collecting buffer with the
   empty pending buffer and reads the commit sequence that covers it; appends
   outside every statement with the existing `aof_rw_drain` (rollback,
   boundary, byte accounting and health published in Meta as now); then, in
   one desk statement, puts the log back and publishes written-through, or on
   failure the failure-release sequence. No Meta hold spans host I/O, and no
   statement holds Meta and the desk together.
4. **Failure.** While an append error is saved or bytes are retained, a
   leader writes nothing: it publishes failure-release at the current commit
   sequence and puts the log back, releasing replies of commands already
   executed, as Redis's everysec failure path does
   ([aof.c](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1191-L1213));
   later writes are refused at admission as now. Retries stay with the
   maintenance context every 100 ms.
5. **Maintenance.** The writer context becomes the maintenance context. Its
   10 ms tick takes the log only for the step that needs it: draining records
   no connection waits for (active expiry and eviction), the failed-append
   retry, rewrite cut, rotation, direct install and the final forced append
   at shutdown; the rewrite's scan, journal drain and worker waits run with
   the log on the desk.
6. **Sync.** The everysec sync runs through a second write handle opened on
   the same incremental file, held by the maintenance context, so appends
   continue during fsync, as Redis's background fsync does. Linux writes back
   a file's dirty pages whichever descriptor calls fsync, and reports a
   writeback error once to each open file description; the synced size is
   the incremental size published before the sync starts. Rotation reopens
   the second handle with the new file; the sealed file's pre-switch sync
   keeps using the log's own handle, which the maintenance context holds
   then.
7. **Shutdown.** After every client context has ended, the maintenance
   context takes the log out, makes the forced append and sync, closes its
   desk handle (the last, which returns the empty slot) and closes the files.
8. **Reuse.** After a complete append the pending buffer is emptied with
   Whitefoot's `clear` (board item `firn-req-slots-clear-keep-capacity`)
   instead of a fresh 64 KiB allocation under the completion hold.

The follower's guard is a helper call; Whitefoot's selective wake for
helper-call guards (board item `firn-req-helper-guard-wake-filter`) keeps it
from waking on unrelated desk writes.

### Alternatives

| Approach | Handoffs per batch | Sync blocks appends | Status |
| --- | --- | --- | --- |
| Connection leader, sync through a second handle | none for the leader; followers wait for its publication | no | recommended |
| Connection leader, sync holds the one handle | same | yes, once a second for the fsync | simpler error handling; every slow fsync stalls replies |
| Dedicated writer woken per batch (#97) | connection to writer to connection | yes | measured 0.21-0.69 of main, every row below its line |
| Per-driver event-loop epoch, as Redis's loop | none | no | firn has no per-driver loop; a redesign of connection scheduling |

### Measurement and acceptance criterion

Written before measuring. On the 14900K through `redis-bench.yml`, with
`make firn-lto`, AOF everysec, no rewrite during the window, servers on two
and on eight P-cores and clients on E-cores 16-31: interleave main, a twin of
main and the candidate, plus Redis 7.2.16 with the same AOF settings for
context, for SET, MSET and GET (whose replies now read the commit
sequence) at pipeline 1 and 16, four rounds initially. For each command,
pipeline and core count, reject when candidate/main is below
the lowest twin/main ratio minus 0.05. Report per-round rates, p50/p99 and
the noise. If the release exposes the runtime's guard counters, also report
follower wakes that found their guard false; otherwise report them as
unavailable. A failing row is located by the batch density (commands per
complete append) and, if it is the sync, by a run with option B's sync as an
extra side.

