# Publishing a rewrite while the current increment cannot be written

Board item: `firn-rw-publish-during-append-error`. This record is written
before implementation; nothing below has been built or measured unless a
result is cited with its run.

## Question

The [rewrite fault investigation](rewrite-faults.md#current-append-failure-user-impact-and-policy-assessment)
observed, in [gate run 38426641490](https://github.com/Ming-Research/Firn-wf/actions/runs/38426641490),
that when every append to the current incremental file fails with EIO while
a rewrite runs, Redis 7.2.16 still installs its new base and reports
`aof_last_bgrewrite_status:ok`, while firn abandons the rewrite at the first
failed append (`firn_aof_rewrite_abort_reason:drain`), keeps the old base and
every increment, and blocks automatic rewrites until a manual one succeeds.
Both recover every acknowledged write after the disk is repaired.

Can firn keep the scanned work and publish base(S1) while the old increment
still refuses appends, so that the base and the files that follow it apply
every acknowledged command exactly once at every crash point, the bytes
retained for the failing file are discarded only after the new selection is
durable, and firn's existing recovery after a failed sync is unchanged?

## What Redis 7.2.16 does

All references are to `src/aof.c` and `src/server.c` at tag 7.2.16
(`REDIS_VERSION "7.2.16"` in `version.h`), read from
`https://raw.githubusercontent.com/redis/redis/7.2.16/src/`. The line ranges
the earlier fault record gives for the same functions (for example
2390–2495 for installation) do not match this file; the ranges below were
resolved against it. These are source readings, not measurements, except
where the gate run above is cited.

1. **Start.** `rewriteAppendOnlyFileBackground` (aof.c:2427–2493) calls
   `flushAppendOnlyFile(1)` (2442) and then `openNewIncrAofForAppend`
   (2443) before forking. `openNewIncrAofForAppend` (792–852) opens
   `F.<n+1>.incr.aof` with `O_TRUNC`, persists a manifest naming it after
   every old file (821), and points `aof_fd` at it (840). It leaves
   `aof_buf` untouched. The child writes the dataset as of the fork, S0.
2. **Append failure.** With everysec, a failed or short `write(2)` in
   `flushAppendOnlyFile` (1066–1260) keeps the unwritten bytes in `aof_buf`
   (1206–1213, a partial write trimmed only when `ftruncate` fails),
   records `aof_last_write_errno`, and sets `aof_last_write_status = C_ERR`
   (1204). Only a later complete write clears it (1215–1222). Retries run
   from `beforeSleep` on every event-loop iteration (server.c:1724) and once
   a second from `serverCron` while the status is err (server.c:1446–1456).
3. **Refusal.** `writeCommandsDeniedByDiskError` (server.c:4483–4500)
   returns `DISK_ERROR_TYPE_AOF` while `aof_last_write_status` is err, and
   `processCommand` refuses write commands and PING with MISCONF
   (server.c:4033). INFO reports `aof_last_write_status:err`
   (server.c:5812).
4. **Completion.** `backgroundRewriteDoneHandler` (aof.c:2584–2700) checks
   only the child's exit status. It renames the child's file to the new
   base, marks every increment except the current one as history
   (`markRewrittenIncrAofAsHistory`, 494–520, which skips the last
   increment while `aof_fd != -1`), persists the manifest (2667), deletes
   history (2693) and sets `aof_lastbgrewrite_status = C_OK` (2695). It
   never reads `aof_last_write_status` and never touches `aof_buf`.
5. **In the observed case**, the fault began after admission, so the failing
   file is the post-S0 increment `F.<n+1>.incr.aof`. Redis publishes
   base(S0) plus that increment, keeps refusing writes, and keeps the
   post-S0 bytes in `aof_buf` (59 bytes in the gate run). After repair the
   next flush writes them to the same increment, the status returns to ok,
   and restart loads base(S0) and the increment: every command once.
6. **Restart while the fault persists** loses the retained `aof_buf`, as any
   everysec crash does; the gate run did not measure this window.

A source-derived hazard in Redis, not measured: when a rewrite starts while
bytes are already retained (the error predates admission), the forced flush
at 2442 fails, `aof_fd` moves to the new increment at 840 with those bytes
still in `aof_buf`, and the forked base already holds their effects. After
repair they are written into the new increment, so a successful rewrite's
selection, base(S0) plus that increment, applies them twice. Nothing in
`bgrewriteaofCommand` (2497–2512) or the automatic trigger refuses a rewrite
while the status is err. firn must not copy this; the protocol below
partitions retained bytes at the cut.

## What firn does now

`firn/persistence/rewrite.wf` at the branch base:

- The ordinary flush inside the rewrite loop calls `aof_rw_drain`
  (rewrite.wf:319). A failed append saves the append error, keeps the
  unwritten bytes and records the abort cause Drain (927–930); the loop
  then records WriterIo (800–804), which the first-cause rule hides.
- The cut is refused while private bytes are retained: `aof_rw_cut` allows
  it only when the private buffer is empty (682), and the loop first tries
  to drain it (765). After the cut the sealed prefix must drain (769), the
  journal is drained to its file (771) and only then does `aof_rw_switch`
  (773) rotate.
- The worker reconciles only after a successful rotation: it waits for
  `cut_done` and requires `rotated` (619–623).
- Installation (`aof_rw_install`, 424–477) publishes the new base with only
  the newest increment active (440–449).
- `aof_rw_complete` sets `aof_rewrite_retry_blocked` after any failure
  (171).

So an append error at any time between admission and rotation abandons the
attempt. The [S1 rotation decision](../../../design/firn/aof-rewrite.md)
requires draining and syncing the ordinary prefix before publishing the new
increment beside every old file; that requirement exists because a manifest
naming the old files followed by a post-S1 increment would skip the unwritten
prefix. It does not require discarding the scan.

## Proposed protocol

Terms: **P** is the private buffer of bytes retained for the old increment
`I_n` (all of them committed at or before S1 once the cut has run); **L** is
the collecting buffer in Meta; **T** is `temp-F.base`; **J** is
`temp-F.journal`; `I_{n+1}` is the next incremental name; **B'** is the next
base name. "Old selection" is the current manifest: old base plus
`I_1..I_n`.

### Normal path (unchanged)

When P drains and `I_n` syncs before the base is complete, the existing
rotation runs: publish old files plus `I_{n+1}`, adopt it, drain L into it,
reconcile, install base(S1) plus `I_{n+1}`. Nothing below changes this path
or its crash analysis in [scan-log.md](scan-log.md#cut-and-authoritative-files).

### Append error before S1

1. A failed append keeps the existing append-error rules (retained bytes,
   rollback, saved error, MISCONF refusal, 100 ms retry). It no longer
   aborts the rewrite. A counted sync failure still aborts it, unchanged,
   because the sync-error recovery rests on every sync error saved during an
   attempt aborting that attempt.
2. The scan continues; journal staging continues to drain to J, which is a
   different file.

### The cut with bytes retained

3. When the scan is done the writer cuts even if P is not empty. In one Meta
   statement it reads S1, disables capture, freezes the journal and appends
   L's bytes to the end of P, leaving L empty for commands after S1. The
   growth of P is charged to the reserve R through the rotation budget
   before it is allocated; if it does not fit, the attempt aborts with
   `rotation_reserve` and P keeps its order. Every byte in P is now from a
   statement at or before S1; every later statement collects in L. This is
   the single correspondence the existing cut already guarantees, extended
   to bytes that were retained before the cut.
4. The writer tries to drain P once. If it drains, the normal path follows.
   Either way it drains the frozen journal completely to J and then marks
   the cut done, so the worker may reconcile. The rewrite stays in its
   rotating state, so post-S1 growth of L keeps consuming R through the
   existing rotation budget.

### Waiting state W1 (cut, P retained, not rotated)

5. While in W1 the writer does not run the ordinary flush, which would move
   L into P and send post-S1 bytes to `I_n`. Every 100 ms it retries P alone
   against `I_n`. If P drains, the writer runs the existing switch (boundary
   check, sync `I_n`, publish old files plus `I_{n+1}`) and the attempt
   continues on the normal path.
6. Clients stay refused because the append error is saved. L can grow only
   from internal expiry or eviction and from statements committed between
   the cut and the failed drain being published. Exhausting R aborts as
   today.

### Direct installation (base complete while still in W1)

7. The worker reconciles J after S1, syncs and closes T, and reports a
   complete base. Installation commits under Meta with `aof_installing` as
   today; shutdown or pressure that wins that hold refuses it.
8. Open `I_{n+1}` (truncated, unlisted).
9. Rename T to B'.
10. Build the manifest with B' as base, `I_{n+1}` as the only active
    increment, and the old base and `I_1..I_n` as history; write
    `temp-F.manifest`, sync it, rename it over `F.manifest` (the
    publication point), sync `appendonlydir`.
11. Published and durable: in one Meta statement adopt `I_{n+1}` (incremental
    and synced sizes zero, empty boundary), release P, clear the saved append
    error and its partial flag, end the rotating state and record success.
    Then close the old handle (a close failure is counted in
    `firn_aof_rewrite_cleanup_failed`; it cannot undo the publication),
    remove history, and drain L into `I_{n+1}` by the ordinary flush.
12. Published but the directory sync failed: adopt `I_{n+1}`, release P and
    clear the append error as in step 11, but save the directory sync's
    error as the sync error, keep every history file and entry, and report
    `aof_last_bgrewrite_status:err`. Writes stay refused with MISCONF and the
    existing sync-error recovery rewrite re-establishes a durable selection.
    The normal path does not need this, because there the old selection
    already holds every acknowledged byte; here the old selection lacks P, so
    a selection whose rename may not survive power loss must not take new
    acknowledged writes.
13. Not published (open, rename or manifest write failed): remove B' and
    `I_{n+1}` when they exist, abort with `install_io`, end the rotating
    state, and keep the old handle. The ordinary flush then drains P and
    afterwards L to `I_n` in order once the disk is repaired, as the
    existing pre-publication failure rule requires.

Abort in W1 for any other reason (shutdown, reserve, journal error) takes
step 13's fall-back: the old handle, P then L in order. Shutdown's single
forced append of P and L is unchanged.

### What is reported

| Moment | `aof_rewrite_in_progress` | `aof_last_bgrewrite_status` | `aof_last_write_status` | `aof_buffer_length` | Writes |
| --- | --- | --- | --- | --- | --- |
| Fault during scan, and W1 | 1 | previous value | err | P plus L | MISCONF |
| After durable direct installation | 0 | ok | ok, until an append to `I_{n+1}` fails | L | admitted |
| After a published installation whose directory sync failed | 0 | err | err (saved sync error) | L | MISCONF until a recovery rewrite is durable |
| After a refused or failed direct installation | 0 | err | err until P drains | P plus L | MISCONF until P drains |

`firn_aof_rewrite_abort_reason` is empty after success. The Drain cause can no
longer be recorded, because a failed append no longer aborts an attempt;
the variant and its INFO text are removed with it.

This deliberately differs from Redis after a durable direct installation:
Redis keeps its retained bytes, because they belong to its current increment,
and refuses writes until the disk is repaired; firn has no byte left to write,
because every retained byte is in the new base and the failing file is no
longer selected. A real failing device fails the first append to `I_{n+1}`
and refuses writes again. Keeping the error with nothing to retry would
refuse writes that no repair could ever clear.

### Crash points

Q_d denotes the last command whose bytes reached `I_n` before the fault. A
process kill leaves page-cache contents; power loss leaves only synced data
and durable directory entries. "Lost window" means acknowledged commands that
were never written, which everysec and Redis also lose.

| Crash after | Selection replayed | Recovered state | Exactly once because |
| --- | --- | --- | --- |
| Fault, during scan or W1 | old | dataset(Q_d); P and L lost (lost window) | old files unchanged; T, J, `I_{n+1}` unlisted and ignored |
| Partial append of P with failed rollback | old | `I_n` ends inside a command, cut after its last whole command at startup | existing last-file truncation |
| T synced and closed; `I_{n+1}` opened | old | as the first row | `I_{n+1}` unlisted; the next rotation truncates it |
| T renamed to B' | old | as the first row | B' unlisted; the next installation renames over it |
| Temporary manifest synced, before rename | old | as the first row | existing temporary-manifest recovery |
| Manifest rename, before directory sync (process kill) | new | dataset(S1) | B' holds every statement through S1, P's included; `I_{n+1}` is empty |
| The same, power loss | old or new | dataset(Q_d) or dataset(S1) | each selection is one of the rows above |
| Directory sync, before P is released | new | dataset(S1) | P is never written to `I_{n+1}` |
| L partly drained to `I_{n+1}` | new | dataset(S1) then a prefix of L | L holds only statements after S1 |
| History removal partly done | new | as above | startup syncs the directory and removes listed history |
| Failed direct installation, P and L draining to `I_n` after repair | old | dataset(Q) for the drained prefix | P precedes L on one handle |

In every row the recovered state is the dataset after some prefix of the
committed statements: no statement is applied twice and none is applied
without every earlier one. Direct installation shortens the lost window
rather than widening it: after step 10 is durable, P's commands survive
power loss inside B', where before they were unwritten.

The new file `I_{n+1}` is created before the manifest that names it and
becomes durable with the directory sync in step 10, the same ordering the
existing rotation uses. A power loss that persists the manifest rename
without the earlier creation would select a missing increment; that ordering
is the same assumption the existing rotation and Redis's
`openNewIncrAofForAppend` make, and is not tested here.

## Alternatives

- **Abandon at the first append error (current).** Safe, but it discards a
  valid base, leaves automatic rewrites blocked after repair, and keeps every
  old file. No crash point above is safer under it. Rejected.
- **Wait for repair, then rotate and install.** Keeps the scan and the
  journal and satisfies the existing barrier unchanged. It never publishes
  while the fault lasts, so it fails the acceptance case below, keeps R and
  the post-S1 backlog held for as long as the device is broken, and cannot
  reclaim space when the fault is ENOSPC. Rejected as the whole answer; it
  remains the path taken when repair comes before the base is complete.
- **Move the retained bytes into a fresh increment at the cut.** Write P to a
  new file published after the old files, so the old selection becomes
  complete again, then rotate a second time for post-S1 bytes. It needs two
  more manifest publications on the failing device; and when a failed
  rollback left a partial command at the end of `I_n`, a selection naming a
  later file after it stops startup with status 4 unless `I_n` is first
  truncated, which the failing device may also refuse. Rejected.
- **Rotate at S0, as Redis does.** The failing file would then be the
  post-S0 increment, as in Redis, but base(S1) paired with an increment from
  S0 applies scan-window writes twice; the tree already rejects rotation at
  S0.
- **Release P at the cut, because base(S1) holds it.** An abort or a crash
  before durable installation would then lose acknowledged commands from
  every selection. Rejected.
- **Keep the append error after direct installation until a later append
  succeeds.** With L empty nothing is ever appended, so writes stay refused
  with no repair able to clear them. Rejected.

## Acceptance, written before implementation

The proposal is wrong if any of these fails:

1. `aof_rewrite_current_append_failure_observes_publication` in
   `tests/aof_sync_fault.rs`, in the `aof-sync-fault` job: with append
   injection on the current increment active through completion, both
   Redis 7.2.16 and firn publish a new base (`published` true for both) and
   report `aof_last_bgrewrite_status:ok`; the failing file's bytes stay
   unchanged; firn's new manifest names the new base and exactly one active
   increment, which is not the failing file; Redis still reports
   `aof_last_write_status:err` with a nonzero buffer, firn reports ok with
   an empty `firn_aof_rewrite_abort_reason`. After repair, crash, remount and
   restart, both recover the 513 expected keys with the counter equal to its
   number of acknowledged INCRs and the list in order, and the refused key
   is absent. A firn write acknowledged after publication and before repair
   is present after restart.
2. The two existing sync-failure cases and the two temporary-base cases in
   the same job still pass unchanged: a sync error saved during an attempt
   still aborts it (`aof_last_bgrewrite_status:err`), and recovery still
   needs a durable installation.
3. `aof_rewrite_during_latched_append_error_reports_the_attempted_drain` in
   `tests/aof_write_errors.rs` changes meaning: a rewrite admitted while the
   append error is latched now publishes with `aof_last_bgrewrite_status:ok`.
   Its retained bytes must include an INCR and an RPUSH, and after restart
   each must appear once, which detects the double application Redis's
   source predicts in the same situation. Renamed to say so.
4. A new ordinary network case: with an append error latched, a directory
   at `appendonly.aof.2.base.aof` makes direct installation fail before
   publication; the attempt reports `install_io`, the old manifest is
   unchanged, and after the limit is lifted P and L drain in order and a
   restart replays each INCR once.
5. `firn_rewrite_host_failures_keep_a_replayable_manifest` and the rest of
   the network and crash suites pass unchanged; in particular the
   `new-incremental` obstacle still aborts with `writer_io`, because there
   the prefix drained and the normal rotation failed.

## Implementation outline

In order; each step keeps the gate green.

1. `firn/store/module.wfm`, `firn/store/rewrite-capture.wf`: keep `cut_done`
   meaning "S1 taken and the frozen journal fully in J"; keep `rotated`
   separate; let `rotating` stay set through W1 and end at installation or
   abort.
2. `firn/persistence/rewrite.wf` `aof_rw_drain`: stop recording Drain on a
   failed append. `aof_rw_cycle`: stop turning an append failure into
   WriterIo; `aof_rw_flush` already aborts on a counted sync failure.
   Remove `RewriteAbortReason::Drain` and its INFO text in
   `firn/commands/info.wf` once unused.
3. `aof_rw_cut`: allow a nonempty private buffer, appending L to it under
   Meta with the growth charged to R before allocation, as the rotation
   budget charges collecting-buffer growth.
4. `aof_rw_cycle`: after the cut, try P once, drain the journal fully, set
   `cut_done`; on success call `aof_rw_switch`; otherwise enter W1, where
   the loop retries P alone at the 100 ms cadence and calls `aof_rw_switch`
   when it drains, and skips `aof_rw_flush`.
5. `aof_rw_worker`: reconcile after `cut_done` when not aborted, without
   requiring `rotated`.
6. A direct-installation function beside `aof_rw_install` in
   `rewrite.wf` (sharing its rename and manifest code): steps 8–13. It needs
   the directory sync's error from publication, so `AofPublication` in
   `firn/persistence/manifest.wf` carries it. Its size accounting closes
   the whole current size into history, so `aof_current_size` becomes the
   base size.
7. `aof_rw_complete`: unchanged; success clears the automatic-rewrite block
   as it does now.
8. Tests as listed under acceptance; `docs` and `firn/README.md` where they
   describe rewrite status during append errors; `rewrite-faults.md`'s
   current-append conclusion pointed here.

## Whitefoot

Every step uses forms the code already uses: an allocation and byte append
inside a Meta statement (`aof_rw_flush`'s forced path), a waiting atomic on a
Meta flag (`aof_rw_worker`), moving and swapping owned buffers and file
handles, and the existing file operations. No new language or library form
is needed, so no Whitefoot gap is expected. Whether the W1 loop's extra
state fits `aof_rw_cycle` without the function exceeding what the checker
handles comfortably is a code-size question, not a language gap.

Status: proposal; awaiting the owner's approval of the design-tree change.
