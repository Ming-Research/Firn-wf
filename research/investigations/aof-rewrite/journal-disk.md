# Streaming the rewrite journal to disk

Status: implementation draft accepted by the check-only compiler; runtime
verification is pending. The investigation was written before source changes.
No runtime result is claimed and the detailed design edits remain unapproved.

## Question and prior evidence

Can a rewrite retain its ordered after-images and subsequent one-key effects
on disk, so total journal size no longer consumes the memory reserve R,
while preserving the S1 restore boundary and service-first abort policy?
This investigation is registered before implementation or measurement.
The owner selected both changes in board card `firn-q-rw-pace`, option A:
disk journaling here, and time-sliced scanning separately. The latter waits
for a Whitefoot interface and is not implemented by this change.

The 14900K observations (redis-bench rewrite-during, firn built from
PR #51 at 7bd7c92, Redis 7.0.15, one server CPU; runs
[38142167582](https://github.com/Ming-Research/Firn-wf/actions/runs/38142167582)
and [38142529829](https://github.com/Ming-Research/Firn-wf/actions/runs/38142529829))
used 1,000,000 keys, 64-byte values and 50 writing connections. R = 16 MiB aborted with reason `reserve` after
107,699 image units, 6,301 effect units and 2,266 scan steps. R = 429 MB
aborted with the same reason after 967,491 image units, 2,459,059 effect
units and 67,854 scan steps. Redis completed its rewrite in about 1.6 s in both. Uniform writes make the retained
journal approach another dataset copy. Redis 7.0.15 instead writes commands
arriving during its rewrite to the new incremental file on disk, without
this in-memory journal limit.

## Proposed design

Keep statement sequencing, complete first after-images and subsequent
one-key effects. Each statement publishes a complete framed record under
Meta. Atomic statements cannot wait or perform I/O: the existing fixed
directory of 64-KiB chunks becomes a bounded staging queue, charged to R
together with statement scratch, scan buffers, one in-flight draining chunk,
reconciliation buffers and the existing rotation backlog. Disk bytes do not
consume that memory allowance. Reserve remains floor(maxmemory / 20), or
16 MiB with maxmemory zero; the scan allocation exception remains explicit.

The sole AOF writer drains a bounded batch of staged chunks into
`temp-F.journal` in `appendonlydir`, outside Meta, alternating with ordinary
AOF draining so a continuously replenished queue cannot monopolize it.
Removing a chunk from the queue does not refund it: refund follows completion
of its append and release of its allocation. No borrowed buffer escapes its
owning context. Read/write handle transfer destructures the owned pair.
An append failure aborts the rewrite; the ordinary AOF remains
authoritative and its partial-append recovery is unchanged.

If staging cannot admit an entire record, abort with distinct reason
`journal_backlog`; never wait for disk under Meta, block a client on journal
space, or weaken ordinary command/AOF publication. Large statement scratch
can still fail with `capture`, and a scan can still fail with `reserve`.
This follows `firn-q-snap-budget`'s service-first policy. The existing retry
block after a failed rewrite remains.

At S1 the writer disables capture, fixes the end sequence and seals the
ordinary AOF prefix in the same Meta statement. It drains the finite staging
suffix before announcing a successful cut to the worker. The rotation backlog
budget remains active throughout this extra drain and rotation. The worker
reads the completed journal using one charged 64-KiB buffer, retaining frame
decoder state across short reads and record boundaries. It strips internal
framing and appends chronological payloads to the scanned base. Each record
must have an increasing sequence in (S0, S1] and a bounded payload; EOF must
be at a record boundary. A recorded file-byte length also detects a truncated
whole-record suffix. No replay keyspace or whole-file buffer is allocated.

The internal frame includes sequence, payload length and image/effect kind.
PR #51's `journal_images` and `journal_effects` counters count complete file
records successfully drained, rather than retained RAM units; incomplete or
failed writes do not count as complete records. The first abort cause and
its reserve snapshot remain latched before cleanup. Journal host failures
use `journal_io`; framing failures retain `journal_invalid`.

The journal needs no fsync: it is never named by a manifest or used for crash
recovery. Successful appends must be visible to subsequent reads; the finished
base is synced before publication, and existing increment/manifest/directory
sync rules remain authoritative. A crash before installation recovers the old
base and all named increments; after publication it uses the installed base
and suffix. Neither outcome needs the temporary journal to survive.

Close both journal handles before removing its name after success or abort,
including FLUSHALL, FLUSHDB, shutdown and open/append/read failures. FLUSH
still aborts under its statement's Meta hold; the drainer cannot turn that
abort into an install. Cleanup is bounded by chunks and completion waits for
all owners. A crash can leave the file, so startup removes this dataset's
unlisted `temp-F.journal` after loading the authoritative manifest; never
remove a file actually named by that manifest. Removal failure must be
reported rather than claimed as cleanup. Use the existing Redis `temp-`
naming convention, with the same unsupported cross-dataset name collision
as `temp-F.base` pending exclusive creation (`firn-wf-excl-create`). Stalled
host I/O still lacks a cancellation deadline; moving storage does not resolve
that previously recorded Whitefoot limitation.

## Alternatives

- Retain the full journal in RAM: rejected for this experiment because
  raising R did not resolve the supplied uniform-write failures.
- Reuse the ordinary incremental AOF alone: it cannot replace first
  after-images. A fuzzy scanned image may already include later commands,
  so replaying INCR or list operations can apply them twice; see
  [the restore argument](scan-log.md#cut-and-authoritative-files) and
  [capture semantics](scan-log.md#retention-emission-and-progress).
- A dedicated drainer: separates ordinary AOF scheduling from journal I/O,
  but adds another lifecycle and S1 acknowledgement. Start with the sole
  writer, which already owns rotation and authoritative append ordering;
  reconsider if the comparison finds journal draining harms ordinary AOF
  progress. The chosen queue never blocks service waiting for the drainer.
- Synchronously write each statement's journal: impossible inside an atomic
  statement, and outside it would lose commit order without staging.

## Validation and rejection criteria

Network tests must observe a physical journal larger than admitted R while
capture is active, then require rewrite success and identical explicit
logical values after restarting from the installed files. Pace bounded
writes by observed file growth so this checks total retention, separately
from producer pressure. Require temporary-file removal after success, abort
and restart, and a deterministic oversized atomic effect burst to report
`journal_backlog` while its ordinary write survives restart. Existing
FLUSH, shutdown, transaction, expiry and rotation-backlog cases stay wired.
Any missing/doubled effect, partial execution unit, published aborted base,
early refund, unbounded readback, or silent loss of a temporary-file failure
rejects the implementation. CI must also run the existing Redis ratchet.

On the idle 14900K through CI, first time a small useful sample, then repeat
the supplied rewrite-during workload at R = 16 MiB. Compare interleaved
same-source before/after runs with a base twin as noise control, and Redis
7.0.15 under identical data, connections, CPU, AOF and fsync settings. Record
revisions, settings, journal bytes, completion, rewrite duration, throughput
and latency during rewrite, plus requested heap and RSS separately. A correct
completion with disk journal larger than R is the intended result; recurring
staging pressure, worse service beyond control noise, or inability to complete
the selected workload rejects this implementation as sufficient. The existing
100-us sleep per scan step remains: completion of the million-key workload
may also require the pending time-slice interface. No completion or performance
claim is made before those runs.
