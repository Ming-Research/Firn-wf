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
The owner will decide how to treat the current-append difference after the
two-server observation is available.

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
this new interference. The contention attribution remains a hypothesis until
the unchanged regression passes in the isolated gate; no CPU or scheduler
trace establishes it from the first log alone.

The traced-stop repair leaves strace running until subject stops propagate,
then requires a stable second all-thread stopped-state witness with strace
stopped. It accepts only `T`, `t` or `Z`, keeps both rewrite-phase checks,
and still kills and drains the complete group before recovery. The two
failed cases are its before-fix evidence; the repaired cases await CI.

The separate read-only static review covered the original eight-file change,
affected helpers and rewrite design nodes/ancestors. The current repair
changes stop coordination, test scheduling and the exact diagnostic
expectation; runtime validation of these repairs remains pending. No local
build, test, check, device operation or measurement has run, and this repair
has not been committed. Pins and production behavior remain unchanged.

Record each CI result with its revision, command, witnessed fault/phase,
manifest selection before and after the fault, current-write and rewrite
statuses, refused replies, recovery comparison and duration. Take the
smallest new-case run first; use its duration and observed variation to
choose any repeats. Do not add repeats simply to mark acceptance complete.
