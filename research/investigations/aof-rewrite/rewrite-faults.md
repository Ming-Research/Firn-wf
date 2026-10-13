# AOF rewrite faults

Board item: `firn-bl-01-02-rest`, plan row `rewrite-faults`.

## Question and rejection criteria

Does a process crash or a failed temporary-base write leave a recoverable
manifest-selected AOF, preserving the independently recorded acknowledged
commands exactly? Does the old selection remain usable until a complete new
base is published? When the **current** increment append fails, what happens
to rewrite completion, publication, write refusal and eventual recovery in
Firn and Redis 7.2.16?

Reject recovery if any acknowledged value is absent or stale, a non-idempotent
operation is applied twice, a refused or incomplete command creates state,
a manifest selects a missing or malformed active file, or the old active
files are removed before publication. A successful restart alone is
insufficient. Temporary files not named by the active manifest are allowed;
they must not contribute recovered state.

This investigation changes tests and their CI wiring, not Firn's behavior.
The current-append policy remains an owner decision. The observations and
recommendation below do not change the current implementation or design.

## Redis source expectations

All links refer to [`src/aof.c` at Redis 7.2.16](https://github.com/redis/redis/blob/7.2.16/src/aof.c).
These are source-derived expectations, not measured results.

| Case | Redis expectation and source |
| --- | --- |
| Kill during scan | Old base plus ordered active increments recover; rewrite admission adds an increment before forking. [Admission, lines 733–770](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L733-L770), [fork, lines 2258–2292](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2258-L2292). |
| Complete base, before publication | Completing the child file does not select it. The parent renames the base, then persists the new manifest, then removes history. [Child completion, lines 2202–2227](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2202-L2227), [installation, lines 2390–2495](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2390-L2495). |
| Manifest rename boundary | A synced temporary manifest replaces the active manifest by rename; directory sync follows. Recovery uses the selected base and increments. [Publication, lines 489–554](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L489-L554), [loading, lines 1523–1619](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1523-L1619). |
| Temporary-base ENOSPC / EIO | Write/flush/sync failure removes the temporary file and returns failure; unsuccessful child completion reports rewrite error without installing that base. [Temporary output, lines 2172–2227](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2172-L2227), [failure handler, lines 2508–2525](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2508-L2525). |
| Current increment append failure | With everysec, failed bytes remain buffered and writes become refused. Successful child installation has no append-status prerequisite and retains the current increment. [Append error, lines 1063–1142](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1063-L1142), [increment retention, lines 458–485](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L458-L485), [installation, lines 2390–2495](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2390-L2495). |

The board's earlier description that Redis waits for the current append before
enabling the new file is therefore a hypothesis to measure, not a requirement
to encode. Record manifest publication and current-write recovery separately.

## Method

Use the Redis 7.2.16 executable built by
`tests/redis-suite/reference.sh` from its SHA-256-pinned release archive.
Every new case runs the same workload on that oracle and Firn. The expected
state comes from command inputs and successful replies, not a dump of the
server's own pre-crash state. Counters and lists distinguish duplicated replay
from idempotent replacement. Check full values and key count after restart.

The kill cases run in the network target, after the ordinary suite, with
one test thread. The ordinary invocation excludes only `rewrite_faults::`;
the second invocation requires all three of that module's cases to pass.
This separates ptrace and process-stop experiments from continuous-write
rewrite regressions without changing their workloads or assertions.
Device cases remain
ignored there and run serially through `make aof-sync-fault`, in the existing
root-enabled CI job. That target requires the mount, the pinned oracle and
five passing cases: the two retained sync-failure cases and three rewrite
fault cases. `make check` and `check.yml` retain the same gate target set;
`make gate-sync` checks correspondence in CI.

Fault attribution must identify the pathname and actual syscall error.
dm-flakey writeback failure alone cannot establish that a temporary-file
write, rather than the current increment's sync, failed. The new temporary
EIO and current-append cases use strace pathname filtering and syscall
injection. The [strace manual](https://github.com/strace/strace/blob/master/doc/strace.1.in)
documents combining `-P` with `--inject`. The temporary ENOSPC scenario fills
the fixed-size filesystem and uses the syscall trace to witness ENOSPC on
the temporary base. Current appends must remain healthy in temporary-file
cases. The injector must not silently select zero calls.

Path-filtered write injection covers Firn's file adapter; native io_uring
submission is not a traced write syscall. This limitation must accompany
the results. A process kill is not a power-loss experiment: page-cache
survival and flush policy delimit the claim. Disk-error cases heal and drain
the retained current bytes before killing for the recovery check; they do
not claim that everysec preserves an append that is still failing at kill.

### Implemented scenarios and falsifiers

`tests/rewrite_faults.rs` supplies three ordinary network cases. Each uses
4,000 seed keys, overwrites and new keys during the rewrite, independently
counted INCR and ordered RPUSH effects. It consumes all complete-command
replies and waits for the last acknowledged SET record in a manifest-selected
increment. An intentionally incomplete RESP SET cannot execute and must not
appear. The exact recovery oracle is 4,036 keys, full expected string values,
counter 33 and the 33-element list in order. It starts both the crash
directory and a copy of the frozen old authoritative selection; neither
may lose, repeat or invent an effect.

| Case | Required phase/fault witness | What a wrong recovery fails |
| --- | --- | --- |
| Early-scan SIGKILL | Stop the whole process group; the temporary base contains 16–999 of the 4,000 seed keys and the old base remains selected. Runs Redis and both Firn runtime routes. | Full values, counter/list multiplicity, exact key count, absent incomplete SET and independent old-selection replay. |
| Complete-base SIGKILL before manifest publication | A complete new named base exists, contains every seed key, and the frozen manifest still selects the old base. Redis and Firn's file adapter run under a one-second syscall-exit delay on rename variants. | The same recovery oracle; a prematurely deleted old file fails copying or old-selection replay. |
| SIGKILL after atomic manifest publication | The frozen manifest selects the new base while the saved old selection can still be copied. The rename-return delay holds the publisher before cleanup. | New-selection recovery and independent old-selection replay must both reproduce the same acknowledged effects. |
| Temporary-base ENOSPC | Fill the 64-MiB filesystem while the rewrite worker is stopped, resume it, and demand a traced write returning real ENOSPC on that base. | Rewrite must report error, current append remain healthy, and old manifest plus every old active byte remain unchanged. Old-selection replay and healed crash/remount/restart check all 512 full seed values and exact INCR effects. |
| Temporary-base EIO | Inject EIO only for write variants to the stopped worker's discovered temporary-base pathname, then resume. | The same temporary-failure checks; absent injection, wrong pathname or a sync-only failure cannot satisfy the syscall witness. |
| Current increment EIO | Keep path-selected append injection active through rewrite completion; require retained bytes, unchanged current-file bytes and refusal of a new SET. | Compare base selection and rewrite status before repair: Redis is source-expected to publish with rewrite ok; Firn to abort with rewrite err. After repair, drain retained bytes and verify exact acknowledged values/counter and absence of the refused key after crash/remount/restart. |

Every SIGKILL stops the complete server group, including Redis's child and
all threads. For traced runs, stop the subject processes while strace still
handles their SIGSTOP delivery, then stop strace and recheck every thread.
A raced observation resumes only the tracer to finish pending stop events.
The [ptrace group-stop semantics](https://man7.org/linux/man-pages/man2/ptrace.2.html)
require tracer delivery before a traced process's siblings all stop.
The kill cases recheck their phase after stopping and require
that no live group member can continue modifying files before restart.
There is no observable partial state inside the atomic manifest rename;
the cases cover its two sides. Rename-delay cases cover the file adapter,
not native-ring publication.

Temporary-failure cases let the normal everysec writer settle the
acknowledged pre-fault increment before filling the filesystem. Independent
replay uses only the saved old active bytes and manifest; a later rewrite
cannot rescue an invalid old selection. For the current-append case this copy represents
the acknowledged prefix before the deliberately failing append; the live
server's retained acknowledged append is checked after repair.

## Results

### Passing gate, 2026-10-13

[Gate run 38426641490](https://github.com/Ming-Research/Firn-wf/actions/runs/38426641490)
tested [revision c27b414](https://github.com/Ming-Research/Firn-wf/commit/c27b414414d04e249fe4630ce90e2253bb364a26).
All four jobs passed, including the network and device jobs and gate-sync.
The job logs identify the same checkout revision. The test-invocation
durations below come from their libtest result lines, not whole-job timing.

| Invocation | Result | Duration |
| --- | --- | --- |
| Ordinary network suite, excluding the crash module | 201 passed, zero failed, five ignored, three filtered out | 55.70 seconds |
| Three crash cases, serially after the ordinary suite | Three passed, zero failed or ignored | 27.44 seconds |
| Device cases: two existing sync cases and three rewrite cases | Five passed, zero failed or ignored | 106.39 seconds |

All three crash scenarios now recovered the 4,036 expected keys, exact
counter/list effects and no incomplete-command key, from both the copied old
selection and the crash directory. This includes both sides of the atomic
manifest switch on Redis and Firn's file adapter; early-scan kills also cover
Firn's native ring. Both temporary-base fault scenarios recovered exactly
513 expected keys on both servers, after verifying unchanged old active
files and manifest and a healthy current append.

The current-append case completed on both servers. Redis reported rewrite
ok and published before append repair, with append err and 59 bytes retained;
Firn reported rewrite err, retained the old base and 36 bytes, and named the
first cause `drain`. Both refused the next SET, replayed the old selection
alone, and recovered exactly 513 expected keys after repair, drain and
crash/remount/restart. This is recovery after repair, not proof that buffered
acknowledged writes survive a crash while append failure persists.

The unchanged long-history scan case passed in the ordinary invocation,
before the serialized crash cases ran. This is consistent with the extra
parallel load hypothesis from the first gate, but does not prove it: stop
coordination and scheduling were repaired together, and there was no
interleaved load control or scheduler trace. Its original workload, reserve
and completion/recovery assertions remain unchanged.

### First gate, 2026-10-13

[Gate run 38424318643](https://github.com/Ming-Research/Firn-wf/actions/runs/38424318643)
tested [revision 1851bc1](https://github.com/Ming-Research/Firn-wf/commit/1851bc12643b6d4c32e457bb42d43833fc50cae8).
Compilation succeeded. The network invocation reported 201 passed, three
failed and five ignored in 64.24 seconds; the serial device invocation
reported four passed and one failed in 103.65 seconds. These are test-invocation
durations, not performance comparisons.

| Scenario | Observed result on that revision |
| --- | --- |
| Early-scan kill | Passed on Redis and both Firn routes. Both the copied old selection and crash directory recovered all 4,036 expected keys, counter/list effects and no incomplete-command key. |
| Complete base before manifest / after manifest switch | Both failed on Redis before the kill/restart check. Strace was stopped (`T`), some tracee threads were in ptrace-stop (`t`), and others remained sleeping (`S`). The assertion already accepted `t`; stopping strace prevented completion of SIGSTOP delivery to all siblings. These cases produced no Firn or recovery result. |
| Temporary-base ENOSPC | Passed on Redis and Firn's file adapter: the trace attributed actual ENOSPC to the base write; rewrite error, healthy current append and unchanged old files/manifest were observed. Independent old-only replay and healed crash/remount/restart each recovered exactly 513 keys with expected values and counter. |
| Temporary-base EIO | Passed on both servers through the same preservation and recovery checks, with EIO attributed to the injected temporary-base write. |
| Current increment EIO | Redis published the new base with rewrite ok while append remained err and 59 bytes remained buffered; the next SET was refused. Old-only replay and repaired restart each recovered exactly 513 keys. Firn retained its old base and 36 buffered bytes, with rewrite err, append err, cut 0 and scan incomplete. Its abort reason was `drain`; the erroneous `writer_io` assertion prevented its recovery checks from running. |

The first gate confirms that Redis need not repair its current append before
publishing the new base in this scenario. It does not settle the owner's
choice about Firn's intended difference. Firn's `drain` diagnostic follows
the design's first-cause rule: the actual failed append records Drain before
the outer rewrite loop reports WriterIo. The existing latched-append-error
regression required `drain` and passed this run. The new expectation now
requires exactly `drain`. Production behavior and the refusal, publication,
retained-byte and recovery assertions remain unchanged.

The unchanged long-history regression also failed during the network run.
Its scan had finished and S1 had occurred, but it reported `reserve` with
16,721,192 of 16,777,216 bytes used, leaving 56,024 bytes. The post-cut journal
reader requires 65,560 bytes, which exceeds that remaining allowance by
9,536 bytes. This is consistent with backlog pressure delaying reconciliation,
not evidence of an old-history reread (`rotation_reserve`). The revision
changed no production source, pin or existing workload. The two traced-stop
tests spent their ten-second waits repeatedly scanning `/proc` alongside the
ordinary tests; all were in one parallel invocation. There is no shared
heavy-test lock in this checkout. Separate required invocations now remove
this new interference. The unchanged regression subsequently passed in the
passing gate above. That result supports, but does not prove, the contention
attribution; no CPU or scheduler trace establishes it from these logs.

The traced-stop repair leaves strace running until subject stops propagate,
then requires a stable second all-thread stopped-state witness with strace
stopped. It accepts only `T`, `t` or `Z`, keeps both rewrite-phase checks,
and still kills and drains the complete group before recovery. The two
failed cases are its before-fix evidence; all three repaired crash cases
passed in run 38426641490.

The separate read-only static review covered the original eight-file change,
affected helpers and rewrite design nodes/ancestors. The current repair
changed stop coordination, test scheduling and the exact diagnostic
expectation; those repairs are revision c27b414 and passed the gate above.
No local build, test, check, device operation or measurement ran. Pins and
production behavior remained unchanged.

## Current-append failure: user impact and policy assessment

### What the observed difference means

Firn leaves the old base and every manifest-selected increment authoritative.
It preserves the running dataset and the bytes awaiting append; the failure
does not replace the old selection with an incomplete new base. The tested
durable prefix remains replayable, and after repair the retained acknowledged
write drains and restart recovers all expected values. There is no compaction
from the cancelled attempt. A later rewrite must succeed before the obsolete
history can be reclaimed.

Disk occupancy therefore remains at the uncompressed old history. While a
known append error persists, client writes are refused; a continuously failed
write syscall cannot itself keep growing its file. Disk growth requires
successful partial or complete appends. Permitted internal expiry/eviction
can add buffered bytes even while persistently failing appends prevent file
growth; intermittent successful appends can put those bytes on disk.
More importantly, after append repair client writes resume but the failed
attempt has disabled ordinary automatic rewrites until a manual rewrite
succeeds, under the [capture retry decision](../../../design/firn/aof-rewrite/capture.md).
The AOF can then keep growing without automatic compaction. Sync-error
recovery rewrites are an explicit exception to that automatic-retry block.

Redis gains a completed compaction despite the independent current-append
error: it can retire old history, reduce retained storage and recovery work,
and, when enough space is reclaimed, potentially help an ENOSPC condition.
Publication does not repair an EIO or make the retained current bytes durable.
Redis still refuses writes and retries that buffer. The current increment
remains active; publication does not discard its suffix. A crash before a
post-fork acknowledged command is successfully appended can lose it, since
the fork's base cannot contain it. This is the existing everysec/failed-append
window, not additional loss caused by replacing the base; retaining the old
base would not supply that unwritten command either. The new tests repaired
the append before crashing and do not measure that crash-during-error window.
[Current-increment retention](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L458-L485),
[failed-buffer retention](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L1100-L1130),
[publication and history cleanup](https://github.com/redis/redis/blob/7.2.16/src/aof.c#L2390-L2495).

### Why Firn cancels

The governing [rewrite decision](../../../design/firn/aof-rewrite.md) requires
the sole writer to rotate at S1, then:

> drains the frozen journal to disk and drains and syncs the ordinary AOF prefix before publishing the new increment while keeping all old files named

Its reason is:

> base(S1) must be paired only with commands after S1 to apply non-idempotent writes exactly once

An append failure cannot satisfy that existing publication barrier. The
implementation goes further: `aof_rw_drain` in `firn/persistence/rewrite.wf`
immediately records Drain on a failed append, retaining the unwritten bytes.
`aof_rw_cycle` subsequently reports WriterIo, but the
[first-cause decision](../../../design/firn/aof-rewrite/capture.md) preserves
Drain. That decision says diagnostics report "the first failure cause".

No live design decision explicitly requires abandoning the scan at the first
ordinary append error. The quoted barrier prohibits skipping the old-prefix
drain in the current protocol; it does not distinguish cancelling from
retaining the rewrite and waiting for repair. Immediate cancellation is the
implementation's policy, not a safety theorem or an already settled permanent
compatibility difference. The [append error decisions](../../../design/firn/append-only-file.md)
require retaining failed bytes and state that "only a complete append clears
its saved error"; they also permit periodic retries.

### What matching Redis would require

Simply deleting the Drain/WriterIo aborts would not permit safe publication.
Firn's base represents S1, after the scan and journal reconciliation; Redis's
forked base represents the admission cut, S0, and its current increment holds
the commands after that cut. A failed Firn append from before S1 may already
be represented in its replacement base. Writing those same commands into a
post-S1 increment would duplicate INCR or list effects. Dropping them before
the replacement is durably selected could instead lose acknowledged state.

Keeping the scanned work until the old-prefix retry succeeds is compatible
with the present barrier, but does not match Redis's publication while the
append still fails. Matching that completion behavior requires an independent
durable replacement protocol: freeze S1 and its journal, reconcile a complete
base without requiring prior rotation, partition retained bytes at S1, and
publish that base with only the post-S1 increment. Keep the old authoritative
selection until the new one is complete; release any pending prefix already
covered by the base only after the required durable publication. A partial
append, failed rollback, incomplete transaction and a failure after manifest
rename must each retain one correctly replayable selection.

The affected code would be `aof_rw_drain`, `aof_rw_cut`, `aof_rw_cycle`,
`aof_rw_worker`, `aof_rw_switch`, `aof_rw_install` and `aof_rw_complete` in
`firn/persistence/rewrite.wf`; the journal handoff in `rewrite-journal.wf`;
boundary handling in `rewrite-boundary.wf`; and pending-byte/recovery state
in `firn/store/module.wfm` and `firn/persistence/persistence.wf`. The worker
currently waits for completed rotation before reconciling. The owner would
also need to decide when a durable replacement may clear an append error,
replacing the current complete-append-only rule where appropriate.

This must preserve Firn's separate sync-error recovery guarantee: a successful
retry of fsync does not reconstruct pages lost to failed writeback; only a
durably installed replacement can clear that saved error. Append EIO and
sync EIO must remain distinct. A rewrite that misses damaged post-cut data
cannot clear a sync error merely because its base and manifest were written.
The protocol above is a direction for design and fault validation, not a
proven implementation or an approved design-tree change.

### Recommendation for the owner

Recommend matching the useful completion behavior: a current-append error
should not by itself discard a separately valid compaction result. The
ordinary append failure does not establish that the scan or reconciliation
is invalid, and healthy replacement storage can reclaim history and reduce
dependence on the failing old file. Immediate cancellation also leaves the
operator with disabled automatic compaction after append repair.

Keep the present behavior until the owner approves and fault tests validate
the replacement protocol. Match compaction progress without copying Redis's
fork implementation or weakening Firn's sync-error recovery. Redis's behavior
is not intrinsically an extra acknowledged-write-loss hazard, so safety alone
does not justify keeping immediate cancellation as a permanent difference.

## Switch manifest whose directory sync fails

Board item: `firn-rw-switch-dirsync`.

### Problem, found by reading the code

A rewrite's switch publishes a manifest naming a new increment by syncing a
temporary manifest, renaming it over the active one and syncing the
directory. Until this change firn adopted the new increment whenever the
rename succeeded, even if the directory sync then failed, and only marked
the rewrite failed. Writes stayed admitted and went to the new increment. If
the device then loses the rename, a restart reads the old manifest, which
does not name the new increment, so writes acknowledged, and even synced,
after the switch are lost. No run has shown this yet; the case below shows
the admitted write and where it lands.

### What Redis 7.2.16 does

Source: [`src/aof.c`](https://github.com/redis/redis/blob/7.2.16/src/aof.c),
[`src/util.c`](https://github.com/redis/redis/blob/7.2.16/src/util.c) and
[`src/config.h`](https://github.com/redis/redis/blob/7.2.16/src/config.h) at
7.2.16; a source reading, observed by the case below.

- Redis does sync the directory. `writeAofManifestFile` writes and syncs
  `temp-F.manifest`, renames it over `F.manifest`, then calls
  `fsyncFileDir` (aof.c:527-597, rename at 572-579, directory sync at
  581-588). `fsyncFileDir` opens the directory and calls `redis_fsync`,
  which is `fdatasync` on Linux (util.c:1100-1138, config.h:117); it treats
  EBADF and EINVAL as success and any other error as failure.
- On that failure `persistAofManifest` returns C_ERR without clearing
  `dirty` (aof.c:600-610). Redis's switch is `openNewIncrAofForAppend`,
  called by `rewriteAppendOnlyFileBackground` before it forks
  (aof.c:2443-2446). On C_ERR it goes to `cleanup` (aof.c:821-823,850-855):
  it closes the new file's descriptor without unlinking the file and frees
  the candidate manifest, so `server.aof_fd` and `server.aof_manifest` stay
  the old ones (they change only at aof.c:832-847). The caller sets
  `aof_lastbgrewrite_status = C_ERR` and BGREWRITEAOF answers "Can't
  execute an AOF background rewriting" (aof.c:2443-2446,2506-2510).
- Nothing sets `aof_last_write_status` or `aof_bio_fsync_status`, so
  `writeCommandsDeniedByDiskError` (server.c:4483-4503) admits writes. They
  are appended to the old increment, which both the old manifest and the
  renamed one name; the renamed one also names the empty new increment.
- At installation, `backgroundRewriteDoneHandler` treats the same failure by
  unlinking the new base and the renamed temporary increment the visible
  manifest may name (aof.c:2666-2678); firn's installation keeps the
  renamed manifest in force instead ([design](../../../design/firn/aof-rewrite.md)).

So Redis keeps every acknowledged write in whichever manifest a restart
reads, because it keeps appending to the old increment, but it does not
treat the failed directory sync as a persistence failure.

### Reasoning about the handle

After the failed directory sync either manifest may be the one on the device
at the next restart. The old increment is named by both, and the new one only
by the renamed manifest. So the handle that keeps every write in both
selections is the old increment's, and the in-memory manifest stays the old
one. Adopting the new increment is never required for later recovery: the
recovery rewrite's switch builds its manifest from the old one, reopens the
same new increment name empty, and its base is written from memory, so it
holds every write acknowledged before its cut, wherever those writes were
appended. The new increment's file stays in place because the renamed
manifest may name it, as Redis leaves it.

firn also saves the directory sync's error as its AOF sync error. The device
did not confirm the directory entries either selection depends on, and the
page-end case above showed that Linux can mark failed writeback clean, so a
later sync that succeeds proves nothing about them. Refusing writes with
MISCONF until a recovery rewrite's installation is durable is the rule firn
already applies to a failed sync of the active file, and the recovery
rewrite is the step that writes a new rename and syncs the directory again.
This deliberately differs from Redis, which keeps acknowledging. The same
rule covers a failed directory sync of the installation manifest; there the
renamed manifest stays in force, because both manifests name the active
increment.

### Case and rejection criteria

`aof_rewrite_switch_directory_sync_failure_refuses_until_recovery` in
`tests/aof_sync_fault.rs`, in the aof-sync-fault job. strace, attached to
the server with `-P appendonlydir`, makes every `fsync` and `fdatasync` of
the directory itself fail with EIO; the rename and file syncs are not
touched. firn's switch comes at S1 after a scan paced at 20 ms a key, so the
tracer is attached during the scan; Redis's comes at BGREWRITEAOF, so the
tracer is attached before it. Both servers: the trace shows the injected
EIO on the directory, the rewrite is reported failed, the manifest on disk
is the renamed one naming the new increment, and no base was installed.
Then:

- Redis: a write is acknowledged; asserted, as the source predicts.
- firn: INFO reports `aof_last_write_status:err` and abort reason
  `install_io`, and SET and PING are refused with MISCONF and the host's
  "Input/output error". Before this change firn acknowledged the SET, which
  fails this assertion.
- Both: the old manifest with its files' present bytes, and the renamed one
  with theirs, each load exactly the acknowledged state in a fresh server.
  Before this change the write acknowledged after the failure was only in
  the new increment, so the old selection would lack it.
- firn, after the tracer is detached: the recovery rewrite installs a new
  base and writes are accepted again, never while a rewrite is still
  running.
- Both: after one more write, a kill, a remount and a restart, the state is
  exactly what was acknowledged.

Rejected if firn acknowledges a write between the failed directory sync and
a durable recovery installation, if either selection lacks an acknowledged
write or holds a refused one, or if the restart differs. Results: pending
the aof-sync-fault job on this change.
