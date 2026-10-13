# Reply after the covering append

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
