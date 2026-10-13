# Writes without one exclusive hold on Meta

Board item `firn-write-meta-free`, under `firn-core-scaling`. Every claim
below is labelled: **source-read** (read in this repository at `e4ec8bc`, in
Whitefoot at the pinned commit `94d353e33034` or in the named branch),
**measured** (with its run or record), or **judgment** (reasoning not yet
tested). Sections 1 to 3 and 5 to 7 were written before anything was built;
only step 1 (section 3(a)) has been built since, and its runs on 2, 4 and 8
cores are in "Step 1 results" in section 4.

## 1. The question

firn's writes get slower as cores are added. Why, and which design lets
SET, MSET and every other write command scale by at least 1.6 times per
core doubling, 2 to 4 to 8 P-cores, with the append-only file (AOF) off and
on, while keeping Redis's replies, the file's contents and its failure
behavior?

### Observation

**Measured** (reported by the session that opened the board item, 2026-10-12,
i9-14900K, native Linux, server on 1/2/4/8 P-cores, clients on the 16
E-cores, redis-benchmark pipeline depth 16, `-r 100000`, firn without AOF):

| Test, M requests/s | 1 core | 2 cores | 4 cores | 8 cores | 16 HT |
|---|---|---|---|---|---|
| SET | 3.33 | 4.71 | 3.45 | 1.13 | 0.34 |
| MSET | 1.01 | 1.61 | 2.07 | 1.10 | |
| GET | 4.0 | | | 25.6 | |

Server CPU per SET grows from 0.3 us on 1 core to 7 us on 8. perf at 8
cores: SET spends 43% in `set_key`'s resumed frame, 42% in
`wf__shared_acquire` and 6% in `wf__shared_unlock`. At 4 cores MSET spends
about 20% in `wf__shared_acquire`, 20% in `run_mset`, 6% in
`wf__table_hold_take` and 7% in `try_entry`; perf c2c puts 37% of all HITM
on one cache line, written by `wf__shared_acquire`/`wf__shared_unlock` and
`run_mset`. The board item records that a Whitefoot experiment release with
two runtime fixes gave the same result, so the cause is firn's design. The run
identifiers are not in this record; the falsifying comparison of section 4
produces its own.

**Source-read.** The contended object is `Meta`. SET's statement is
`atomic slot = &store^.keys[key], m = &store^.meta { ... }`
(`firn/commands/strings.wf:466`) and MSET's is
`atomic slots = &store^.keys[keys], m = &store^.meta { ... }`
(`firn/commands/strings.wf:796`). Whitefoot atomic statements give their
block exclusive access to every state they target (SHARE-3, kernel spec
v0.132 at the pin), and the runtime takes a write hold for every
whole-object target (Whitefoot `research/investigations/shared-counters`,
section 2.2, branch `claude/shared-counter-study`). So every write on every
core queues on one object for its whole command. 129 atomic statements in
`firn/` name Meta; about 50 of them are command statements in
`firn/commands/` and `firn/scripting/`.

### What one exclusive object costs on this machine

**Measured** (Whitefoot board item `firn-req-shared-max`, run
[38304337014](https://github.com/Ming-Research/Whitefoot/actions/runs/38304337014),
6 rounds, medians, M updates/s):

| Arm | 2 cores | 4 cores | 8 cores | 16 HT |
|---|---|---|---|---|
| 64 contexts, `+1` on one shared object | 11.1 | 4.5 | 2.8 | 0.7 |
| twin build of the same source | 10.4 | 4.9 | 2.7 | 0.7 |
| each context its own shared object | 89.4 | 171.8 | 346.0 | 437.9 |
| hand-built array of 16 shared objects | 58.9 | 82.8 | 175.1 | 134.9 |
| C hardware atomic add, one counter | 142.5 | 124.9 | 118.1 | 98.9 |
| C, each thread its own cache line | 498.5 | 997.0 | 1993.7 | 1669.5 |

**Deduction.** The shortest possible exclusive section, one `+1`, on one
object reaches 2.8 M/s at 8 P-cores and falls as cores are added. Any design
in which every write takes one exclusive hold on one shared object is
therefore bounded near 2.8 M writes/s at 8 P-cores on this runtime, however
short the section, and cannot reach 1.6 times per doubling. firn's 1.13 M/s
SET is under that bound, as expected for a longer section.

## 2. What a write takes Meta for, and what Redis guarantees

### 2.1 Meta's fields a write reaches

**Source-read**, `firn/store/module.wfm:164-207` (Meta) and the call sites
named.

| Field(s) | Written or read by a write | Purpose |
|---|---|---|
| `aof_append_error`, `aof_sync_error`, `aof_append_partial` | read by `aof_refuse` (`firn/commands/admission.wf:35-44`) before the change | MISCONF refusal |
| the same three fields, read by admission's own exclusive Meta statement | one statement for every write and every PING, before the command's statement (`disk_sensitive`, `firn/commands/transaction.wf:429-430`) | MISCONF refusal at admission; **found in the completion review, not in the first inventory** |
| `commit_seq`, `aof_rewrite_seq_exhausted` | read by `sequence_new` (`firn/store/sequence.wf:1-10`), written by `sequence_commit` (`:100-123`) | one dataset statement sequence; entry `last_write` stamps; rewrite cut S0/S1 (`firn/persistence/rewrite.wf:535,623`); INFO `firn_aof_rewrite_commit_seq` |
| `rewrite` (`active`, `start`, `limit`, `used`, `peak`, `journal`, abort state) | read at `sequence_new`, written at `sequence_commit` | rewrite capture: after-images or effect bytes staged in sequence order, charged to the reserve R |
| `log` | appended by the command's metadata step (e.g. `set_meta`, `strings.wf:553`) | the AOF collecting buffer, swapped out by the writer every 10 ms (`firn/persistence/rewrite.wf:276-300`) |
| `rewrite` via `LogBudget` | `log_budget_begin` / `sequence_commit` | rotation-time charge of collecting-buffer growth against R |
| `due` | `queue_due` for an expiry option (`set_meta`) | active-expiry queue and the volatile-eviction invalidation flag |
| `expired_keys` | lazy expiry in `remove_expired`/`expire_key` (`firn/store/store.wf:142-198`) | INFO counter |

Fields a write does not reach in its own statement: `memory_peak` (separate
statement after a command, only when the reading exceeds the connection's
last seen peak, `firn/commands/eviction.wf:41-51`), the eviction pool and
cursor (eviction's own statements), the rewrite controls the writer and
worker use, `clients`, `stopping` and `random`.

**Source-read: what a write needs when AOF is off.** `store^.logging` is
fixed at startup (`firn/server/server.wf:1021-1035`) and CONFIG SET refuses
a different `appendonly` (`firn/commands/server.wf`, CONFIG SET doc).
BGREWRITEAOF with AOF off answers the rewrite-start failure
(`design/firn/aof-rewrite.md`), so capture never becomes active. The writer
is not started, so no append or sync error is ever saved. With AOF off a
write therefore needs Meta neither at admission (the disk-error statement of
the table's second row) nor, in its own statement, for anything but `due`
(expiry options) and the firn-only counter `commit_seq`, which then serves only the INFO field
`firn_aof_rewrite_commit_seq` (`firn/README.md:481-492`) and stamps no
stamp anyone reads. maxmemory adds nothing to the write statement itself:
eviction runs in its own statements before the command
(`firn/commands/eviction.wf:314-449`).

### 2.2 The guarantees firn must keep

Redis executes commands on one thread. Redis 7.2.16 collects each execution
unit's propagated commands and appends them to the AOF buffer at the end of
the unit, wrapping them in MULTI/EXEC when the unit propagated more than one
(`propagatePendingCommands`, `server.also_propagate.numops > 1`, called from
`postExecutionUnitOperations`, `src/server.c`); the buffer is written in
`beforeSleep` before replies are sent. **Source-read** from the 7.2.16 tag by
quoted fragments; line numbers not re-read. firn's design tree cites the same
behavior at 7.0.15 lines (`design/firn/append-only-file.md`,
`design/firn/scripts.md`, `design/firn/transactions.md`).

From these, the guarantees a concurrent firn must keep (**deduction**):

- **P1, prefix states.** The file's records are in the order their
  statements took effect, so every whole-command prefix of the file, which is
  what a crash, an everysec loss or `aof-load-truncated` leaves, restores a
  state the server actually passed through, containing every write whose
  effect preceded the last one kept. Note what this rules out: an order that
  only keeps writes to the same key in order is not enough. Client C1 sets
  `a`; C2 reads `a` and then, because of it, sets `b`. The two writes touch
  different keys, but a file holding `b` before `a`, cut between them,
  restores `b` without `a`, a state that never existed. The read that
  ordered them is not in the file.
- **P2, units whole.** An EXEC or a script appends its effects as one block,
  applied whole or not at all on replay (`design/firn/scripts.md`,
  `transactions.md`, `append-only-file.md`).
- **P3, MISCONF.** Once the writer has published a failed append or sync, no
  later write takes effect. Redis checks `writeCommandsDeniedByDiskError` in
  `processCommand` before `call`. firn rechecks under the same hold as the
  mutation, because a snapshot taken before the hold could admit a mutation
  after publication (`design/firn/append-only-file.md`, fifth decision).
- **P4, eviction and expiry records.** An eviction's or expiry's DEL is
  ordered with the writes to its key like any other record (P1 covers it).
  Eviction runs in each command's own context
  (`design/firn/memory-limit.md`); `performEvictions` precedes the command in
  Redis.
- **P5, rewrite capture.** base(S1) plus the commands after S1 restores the
  dataset exactly. This needs one cut separating every statement before S1
  from every statement after, and the journal reconciled in an order
  consistent with P1 (`design/firn/aof-rewrite.md`, second and third
  decisions; `design/firn/aof-rewrite/sequence.md`).
- **P6, INFO counters.** A counter INFO reports is the exact total of the
  events that took effect before INFO's statement.

firn today keeps P1 to P6 by making Meta's one exclusive order the order of
every write.

## 3. Candidate designs

### (a) AOF off: writes hold no Meta unless they queue an expiry

**Change.** Each write statement gets a second form without the Meta
target, chosen at runtime when `store^.logging` is false and the command
cannot queue an expiry. The body takes an inert sequence token (no counter,
no capture, stamps zero, as loading does). The command-parts split already
separates the body from the metadata step
(`design/firn/command-parts.md`), so the second form calls the same body and
skips the metadata step.

Admission's disk-error check belongs to the same change. It takes one
exclusive Meta statement before the command's own statement for every write
and every PING, to read the saved append and sync errors. The first version
of step 1 missed it, so with AOF off every write still took one exclusive
Meta hold at admission; the completion review found it (F1), and the fix skips
that statement while `store^.logging` is false, because only the writer saves
such an error and the writer runs only with a file.

**Roster (what keeps Meta).** Commands that can queue or transfer an expiry
keep Meta in every configuration: SET and GETEX with an expiry option, SETEX,
PSETEX, the EXPIRE family, RENAME and COPY. So do RANDOMKEY, which draws from
Meta's random state, SCAN and the flushes, which hold the whole map, and EXEC
and scripts. A SET or GETEX without an expiry option, and every other write
whose statement reads nothing else from Meta, takes the Meta-free form. The
choice is made at run time from `store^.logging`, fixed at startup, and, for
SET and GETEX, from the parsed expiry option.

**Replay at startup (source-read).** AOF replay runs on the keyspace as
created with `logging` false (`firn/server/server.wf:1021-1031`); the handle
with `logging` true is made only after `aof_start` has replayed the file, so
replayed commands that queue no expiry take the Meta-free form. That is
equivalent to the Meta form: replay already used the inert loading token,
which takes no sequence and captures nothing, and replay is a single context,
so no other statement orders against it.

**Guarantees.** P1, P2, P4 and P5 have no file to protect. P3 cannot occur.
P6: `firn_aof_rewrite_commit_seq` would stop counting with AOF off; under the
reported-facts rule (`design/firn/reported-facts.md`) the field is then left
out when AOF is off, which changes the README's documented behavior and
needs the owner. Commands with an expiry option (SET EX, SETEX, EXPIRE, GETEX
and others) keep the Meta form, because `due` is in Meta.

**Whitefoot.** No new runtime capability is needed: with AOF off nothing
needs the order Meta provides. Writing it did expose a language limitation.
An atomic statement's targets are fixed syntax, so a statement cannot hold
Meta only when a run-time condition (AOF on, an expiry option) holds. Each
converted command was therefore first written as two atomic statements
around an ordinary `if`, one with the Meta target and one without (the
one-statement form below later replaced it). They share the body function but
duplicate 4 to 30 lines of statement scaffolding per command, and drift
between the two copies is the residual risk. The minimal witness is a
conditional target,

```whitefoot
atomic slot = &s^.keys[k], m = &s^.meta if want_meta { ... }
```

or a compiler that holds Meta only on the paths that touch it, which the
specification's rule that an implementation may hold less than a statement's
states already allows. Disposition: whether to file this with Whitefoot is an
owner decision, tracked on the status board.

**Expected effect (judgment).** SET and MSET without expiry options then
hold only their entries, as GET does. GET went from 4.0 to 25.6 M/s once
nothing shared remained on its path. SET adds an allocation and a stamp per
write; the prediction registered in section 4 is that SET and MSET reach
1.6 times per doubling, with SET at 8 cores at least half of GET's rate in
the same run. Session and cache workloads that set a TTL on every write
(Django's cache, express-session) keep Meta for `due` and gain nothing until
(b) moves `due` into shards.

### (b) Per-driver pending buffers, each record framed with a sequence, merged by the writer

**Change.** Meta's per-write parts move into a sharded state with one part
per driver: the collecting buffer, the expiry queue and its eligibility
flag, `expired_keys`, the rewrite staging chunks, and copies of what writes
only read (the saved AOF errors, the capture flags and window start). Each
write holds its entries plus its own part. Each record is framed with its
statement's sequence number and length, as the rewrite journal's staging
already frames units (`firn/store/module.wfm`, `RewriteCapture`). Every
10 ms the writer holds all parts at once in one statement, swaps each part's
buffer for an empty one, and outside the hold merges the buffers by
sequence (each part is already in increasing order, so a k-way merge) and
appends the unframed bytes. The writer publishes an append or sync error by
writing it into every part under that all-parts hold. S0, S1, scan steps,
EXEC, FLUSHALL and whole-map script retries hold every part, as they hold
Meta today. Meta keeps the rare controls (rewrite requests, thresholds,
eviction pool, peak, clients, stopping).

**Guarantees.** P1 holds when sequence numbers are increasing in the order
statements take effect (see (c)): a sequence is taken inside a statement
that holds its part, the writer's cut holds every part, so no statement is
between taking its number and appending its record at the cut, and the
merged stream is in effect order. P2: a unit is one framed record. P3: the
writer's publication and every write's check are ordered by the part both
hold. P4: an eviction or expiry deletion holds its entry and a part and
takes a sequence like a write. P5: S0 and S1 read the sequence under the
all-parts hold; every statement is wholly before or after the cut, so the
entry stamps and the `stamp > commit_seq` check
(`firn/persistence/rewrite-dataset.wf:81`) keep their meaning. P6: INFO sums
the parts under one all-parts hold.

**What it changes in approved decisions (source-read).**
`design/firn/memory-limit.md` (mem_not_counted_for_evict sums one collecting
buffer and one pending capacity; it would sum every part's),
`design/firn/aof-rewrite/capture.md` (staging chunks "in Meta commit order";
LogBudget and capture charges against one R), `design/firn/aof-rewrite.md`
(the sole writer seals collecting bytes through S1 "under Meta"),
`design/firn/aof-rewrite/sequence.md` (the sequence counter lives in Meta),
and `design/firn/scripts.md` (scripts hold Meta for the whole attempt; they
would hold their part). The reserve R would be either split into per-part
slices, which changes when capture aborts under skewed load, or charged
through a non-exclusive add.

**Whitefoot gaps.** (b) needs per-driver parts holding owned, non-copy
values (gap G1 below) and an ordering source, which is (c).

**Expected effect (judgment).** A write's part is written only by its own
driver, so its lines stay in that core's cache, as in the measured "own
object" arm (346 M/s at 8 cores against 2.8 for one object). The remaining
shared costs are the entries, the allocator and the sequence. The writer
must merge and write about 1 GB/s at 20 M SET/s of 50 bytes; that is within
one core's copy rate but must be checked.

#### Ordering sources for (b)

Three sources were considered.

1. **A global sequence taken without exclusive hold: (c).** Keeps P1.
2. **Lamport stamps from the entries.** Each statement's number is one more
   than the largest of its part's clock and the `last_write` stamps of the
   entries it holds; no global line is written. **Rejected (deduction)**:
   a deleted key leaves no entry and so no stamp. If driver A deletes `x`
   with stamp 1000 and driver B then creates `x` with stamp 6, both inside
   one writer cut, the merge emits B's SET before A's DEL and replay loses
   `x`. Fixing this needs tombstone entries, which every read, DBSIZE,
   SCAN, RANDOMKEY and eviction sampling would have to skip and something
   would have to collect. It also breaks P1 for writes ordered only through
   a read.
3. **Key stripes: (e1) below.**

### (c) Sequence numbers from a counter taken without exclusive hold

**Change.** `commit_seq` becomes a shared sequence that a statement takes
inside its block without holding it as an exclusive target; the take can be
implemented as one hardware fetch-and-add. Combined with (b) it is the
ordering source. Alone, with one shared buffer, it does not help: the buffer
would still be one exclusive object.

**Guarantees.** The take happens while the statement holds all its states.
Two statements that conflict, or that are ordered through a read, take their
numbers in the order they take effect. Statements that do not conflict take
numbers in some order consistent with a valid serial order. So the numbers
give a total order consistent with SHARE-3's one order, and P1 and P5 hold.
Saturation at u64's maximum, which `sequence.md` refuses to wrap, can be
kept by treating values near the top as exhausted (about 584 years at
10^9 takes/s).

**Whitefoot gap G2.** No way exists to take a value from a shared counter
inside an atomic statement without making the counter an exclusive target
for the whole statement. Whitefoot's own study says its recommended sharded
state "does not serve an update that needs the global value back at once,
such as allocating the next identifier"
(`research/investigations/shared-counters/DESIGN.md`, section 5.4). firn's
sequence is exactly that case. The owner's ruling on the Whitefoot card
"多个核同时更新同一个共享计数时，Whitefoot 该提供什么？" (option A, then the
measurement) set single-step integer cells aside for counters, because
hardware add reached only 6% of private lines at 8 cores. That criterion
compares counters with per-core counting, which a sequence cannot use; the
question for a sequence is whether one add per write fits firn's rate.

**Expected effect (judgment).** The C arm reaches 118 M adds/s on one
counter at 8 P-cores at full load. At 20 M writes/s the counter would run
below 20% of that capacity, costing roughly one contended line transfer per
write (tens of nanoseconds against a budget of about 400 ns per write per
core at that rate). Predicted AOF-on SET at 8 cores within 25% of the
same build with AOF off, unless the writer or the rewrite scan (section 6)
limits it.

### (d) Take Meta only for the append

**Change.** Hold Meta only around the metadata step, not the whole command.
Within one statement this needs the compiler to acquire a whole-object
target at first use and release it after last use, which SHARE-3's last
sentence permits (outcomes equal to exclusive execution; it is two-phase
locking). As two statements it breaks the order: the record would no longer
be appended in the hold that changed the key.

**Rejected (deduction from measurement).** The section shrinks but stays one
exclusive object per write. The one-object arm, whose section is a single
`+1`, already falls to 2.8 M/s at 8 cores and 0.7 at 16. MSET, which takes
Meta once per ten keys, still falls from 2.07 M/s at 4 cores to 1.10 at 8.

### (e) Others

**(e1) Key stripes.** Records go to one of P stripe buffers chosen by the
key's hash; a multi-key statement holds every stripe its keys reach (a
`KeySet` target on a map of stripes, expressible at the pin) and leaves one
marker in each; the writer emits a multi-stripe record when it heads every
stripe it marked. No global counter and no language change. **Not
recommended:** it orders only writes to the same keys, so it breaks P1 (the
`a`/`b` example in section 2.2); it redefines the approved one-sequence
rewrite design (`sequence.md`) as epochs per stripe; and each write moves
two stripe lines between cores, so its cost per write is no lower than (c)'s
one counter line (judgment).

**(e2) Key stripes plus one MULTI/EXEC per writer cut.** Wrapping each 10 ms
cut in MULTI/EXEC would make every loadable prefix a whole cut, a real
state, and restore P1 at cut granularity. **Not recommended:** the file no
longer has Redis's shape, inner EXEC and script blocks must be flattened, a
cut at 1 GB/s is a 10 MB block that firn's replay must hold before EXEC, and
the startup cut rules of `append-only-file.md` change.

**(e3) One statement per pipelined read.** A connection's batch of 16
commands runs in one statement holding all their keys and Meta, a schedule
Redis also allows. Meta is taken 16 times less often at depth 16 and no less
at depth 1, and MSET (ten keys per Meta hold) already shows that this form
collapses with cores. **Rejected.**

**(e4) Split Meta into several objects, or read Meta's read-only fields
through shared read holds.** Every write still writes the log object. Read
holds still write the object's lock line in the pinned runtime (Whitefoot
study, section 2.2). Useful for read-mostly fields only; (b)'s per-part
copies cover them. **Rejected as the fix.**

## 4. Recommendation and falsifier

**Recommendation.**

- **Step 1, now, no Whitefoot change:** (a). It removes Meta from AOF-off
  writes that queue no expiry.
- **Step 2:** (b) with (c). Per-driver parts carry the collecting buffer,
  expiry queue, expiry count and copies of the writer's published state.
  Each write takes one global sequence without an exclusive hold, and the
  writer merges by sequence. firn files G1 (parts holding owned values) and
  G2 (a sequence take) into Whitefoot with the witnesses below, and does not
  build (e1) to avoid them.

**Comparison for step 1.** Same source, interleaved, on the 14900K under
the measurement-session rules (`design/firn/measurement-sessions.md`):
builds base, base twin and (a). Server on 2, 4 and 8 P-cores (1 core and 16
hyperthreads recorded, not judged), clients on the E-cores, 50 connections,
`-r 100000`, depth 16 and depth 1, AOF off. Tests: SET, MSET, INCR, LPUSH,
SADD, HSET, ZADD, and GET as the control. 6 rounds, as the board item
states, after a pilot timing one round. Record INFO's peak statements
indirectly by running a fourth arm without peak recording, so that the
peak statement cannot be mistaken for the remaining cost.

- *Accept (a)* if SET and MSET at depth 16 each reach at least 1.6 times
  from 2 to 4 and from 4 to 8 cores in the (a) arm, with SET at 8 cores at
  least half of GET at 8 cores in the same run, and GET within the twin's
  spread of base.
- *Reject (a) as sufficient* if it misses 1.6 times while the base-to-twin
  spread is smaller than the miss: another serializer remains (allocator,
  table hold, peak statement), and perf c2c on the (a) arm locates it.
- *Reject (a)* if any network case or the Redis suite ratchet fails.

### Step 1 results

**Measured, 2, 4 and 8 P-cores.** Setup, the same for every run: i9-14900K
self-hosted runner, `redis-bench.yml` in compare mode. Builds: main at
`fd25006` (base), main-twin (the same revision, a second build, as the noise
control) and step1 at `d82c411`, the admission fix (`22b5318`) with its grammar repair. The server runs on P-cores: CPUs 2,4 for 2
cores, 2,4,6,8 for 4 cores, and 0,2,...,14 for 8 cores. The
redis-benchmark clients run on E-cores 16-31 with pipeline depth 16, in 6
interleaved passes of 10 s each; the table values are medians. AOF is off.
The client tools are the runner cache's redis-benchmark 7.0.15, used only as
the load generator.

Runs: [2 cores](https://github.com/Ming-Research/Firn-wf/actions/runs/38337775390),
[4 cores](https://github.com/Ming-Research/Firn-wf/actions/runs/38326520306),
[8 cores](https://github.com/Ming-Research/Firn-wf/actions/runs/38329020678).
The 8-core run also took a perf profile and a perf c2c profile of SET. An
earlier 2-core run, [38319226722](https://github.com/Ming-Research/Firn-wf/actions/runs/38319226722),
measured build `9294228` before the admission fix and is superseded by run
38337775390.

The criterion is the one in section 4: SET and MSET reach at least 1.6 times
per doubling of server cores, and SET at 8 cores is at least half of GET at 8
cores in the same run.

Medians in k requests/s. The last column is step1 against main.

2 cores:

| Test | main | main-twin | step1 | step1 vs main |
|---|---|---|---|---|
| SET | 4699 | 4717 | 6536 | +39.1% |
| MSET | 1629 | 1606 | 2028 | +24.5% |
| INCR | 4462 | 4471 | 6409 | +43.6% |
| LPUSH | 3351 | 3359 | 4333 | +29.3% |
| ZADD | 1150 | 1151 | 1179 | +2.6% |
| GET (control) | 6443 | 6444 | 6420 | -0.3% |

4 cores:

| Test | main | main-twin | step1 | step1 vs main |
|---|---|---|---|---|
| SET | 3380 | 3120 | 12308 | +264.2% |
| MSET | 2054 | 2006 | 3725 | +81.4% |
| INCR | 3578 | 3146 | 12023 | +236.0% |
| LPUSH | 3222 | 3149 | 4996 | +55.1% |
| ZADD | 1154 | 1159 | 1187 | +2.9% |
| GET (control) | 12204 | 12122 | 12126 | -0.6% |

8 cores:

| Test | main | main-twin | step1 | step1 vs main |
|---|---|---|---|---|
| SET | 1225 | 1209 | 21397 | +1646.2% |
| MSET | 1203 | 1196 | 6452 | +436.4% |
| INCR | 1290 | 1273 | 20934 | +1522.9% |
| LPUSH | 3070 | 3074 | 5164 | +68.2% |
| ZADD | 1150 | 1148 | 1192 | +3.6% |
| GET (control) | 21595 | 21622 | 21486 | -0.5% |

The main runs spread widely on contended writes: the 6-pass spread is 20.4%
for 4-core SET on main and 29.1% for its twin, and 62.1% for 8-core SET on the
twin. Main-against-twin differences on those writes are noise; the largest
of them is 4-core INCR, at 13.7%. The GET control differs from main by at
most 0.6% at every core count, inside the twin's GET spread (2.8%, 3.1% and
1.1% at 2, 4 and 8 cores).

Scaling, as the ratio of the median at the larger core count to the median at
the smaller one. The 2-to-4 ratio compares two runs, the 2-core and 4-core
runs, each with its own twin.

| Test | step1 2 to 4 | step1 4 to 8 | main 2 to 4 | main 4 to 8 |
|---|---|---|---|---|
| SET | 1.883 | 1.738 | 0.719 | 0.363 |
| MSET | 1.836 | 1.732 | 1.260 | 0.586 |
| LPUSH | 1.153 | 1.034 | 0.962 | 0.953 |
| ZADD | 1.007 | 1.004 | 1.004 | 0.997 |

**Verdict.** The prediction held for SET and MSET at every doubling measured
(2, 4 and 8 P-cores). SET reached 1.883 and 1.738 per doubling, and MSET
1.836 and 1.732. At 8 cores, step1 SET is 0.996 of GET in the same run
(21397 k against 21486 k); main is 0.057 (1225 k against 21595 k).

**Profile (8 cores, SET).** On main, `wf__shared_acquire` takes 43.3% of
cycles and `set_key.resume` 44.8%. One cache line takes 65.1% of all
cross-core cache-line hits (HITMs), at offset 0x10, touched by
`wf__shared_acquire` and `set_key`; the total is 232854 local HITMs. On step1
the top symbols are `try_entry` at 7.7% and `record_memory_peak.resume` at
4.5%. The total is 5975 local HITMs, 97% fewer than main. The hottest line
holds 2.5% of HITMs, and 96% of those are at `record_memory_peak`, the
heap-peak update, which the admission change on PR #67 addresses.

**Findings beyond the criterion.** LPUSH and ZADD do not scale on either
revision. ZADD stays at 1.15 to 1.19 M requests/s at 2, 4 and 8 cores (step1:
1179, 1187 and 1192 k). Step1 LPUSH grows 1.153 from 2 to 4 cores and 1.034
from 4 to 8. The cause is not yet profiled; it is tracked on the status board
as board item firn-list-zset-scaling.

**Comparison for step 2.** Builds base, twin, (b)+(c) on a Whitefoot
experiment release (`whitefoot-kit/downstream.md`, trying an unmerged
Whitefoot change), and a measurement-only arm N: (b) without any sequence,
whose file order is wrong and which is never merged. It isolates the
sequence's cost. N can be built at the pin with an array of 16
`Shared<Part>` chosen by connection number. Same cores and tests as step 1,
with AOF off and with AOF on (`./firn PORT 0 firn.aof`, everysec, Redis's
default rewrite thresholds), INFO `aof_rewrites` and the writer's CPU time
recorded per run.

- *Accept* if (b)+(c) reaches 1.6 times per doubling for SET and MSET with
  AOF on and off, and its 8-core AOF-on SET is at least 75% of arm N's.
- *Reject (c)* if arm N meets the scaling criterion and (b)+(c) does not:
  the global sequence is the limit, and (e1)/(e2) or a different ordering
  source must be weighed against P1 with the owner.
- *Reject (b)* if even arm N misses the criterion with AOF on: the limit is
  elsewhere (the writer, the rewrite scan of section 6, the allocator).

### One-statement SET

Question: does writing SET as one atomic statement that names its key and
Meta, with every use of Meta under `if held`, cost anything against step 1's
two statements when AOF is off? The pinned compiler acquires a named target
at its first use on a path (Whitefoot's runtime line, from the
specification's SHARE-3 and the lowering), so the AOF-off path should never
hold Meta. Prior criterion, written in Whitefoot's research before measuring
([Whitefoot#418](https://github.com/Ming-Research/Whitefoot/pull/418)):
the one-statement SET's throughput ratio to the two-statement form falls
within the twin's noise; a ratio below the twin's lower bound rejects it.

Run [38367914126](https://github.com/Ming-Research/Firn-wf/actions/runs/38367914126),
i9-14900K, Linux 7.0.0-38, release wf-d75f318d5631; server on P-cores
0,2,4,6,8,10,12,14, clients on E-cores 16-31; AOF off; redis-benchmark
depth 16; 6 interleaved passes of 10 s. Images: main 1d1657c (two
statements), its twin, and Firn-wf#72 at 73db747. Ratios are per pass
against main's run in the same pass.

| test | image | median rps | per-pass ratio, median | range |
|---|---|---|---|---|
| SET | main | 21,542,344 | 1 | |
| SET | twin | 21,655,571 | 1.001 | 0.994-1.025 |
| SET | one statement | 21,330,337 | 0.9965 | 0.984-1.001 |
| GET | main | 21,177,203 | 1 | |
| GET | twin | 21,297,132 | 1.000 | 0.998-1.027 |
| GET | one statement | 21,283,652 | 1.006 | 0.993-1.016 |

The one-statement SET's median ratio, 0.9965, lies inside the twin's range,
so the prediction holds at this resolution: no cost beyond the twin's noise.
The six passes' mean ratio, 0.994 (standard deviation 0.007), sits at the
twin's lower bound; a cost under 1% is not excluded by six passes. GET does
not run this statement and serves as a second control.

[Firn-wf#75](https://github.com/Ming-Research/Firn-wf/pull/75) writes the
other 39 conditional holds in the same one-statement form, so no
two-statement copy remains in firn. Run
[38376297156](https://github.com/Ming-Research/Firn-wf/actions/runs/38376297156),
with the setup above: main d3d25fa (two statements at 39 sites), its twin
(identical image) and Firn-wf#75 at 576df40; MSET at 50 connections like
SET and GET. The same prior criterion applies: each test's median per-pass
ratio to main within the twin's range.

| test | twin ratio, median (range) | one statement, median (range) | one statement, mean (sd) |
|---|---|---|---|
| SET | 1.005 (0.986-1.009) | 0.9962 (0.990-1.004) | 0.9965 (0.005) |
| MSET | 1.021 (1.000-1.045) | 1.004 (0.959-1.020) | 0.998 (0.021) |
| GET | 1.001 (0.978-1.003) | 0.9995 (0.988-1.008) | 0.999 (0.007) |

Each median lies inside the twin's range, so the criterion holds for all
three tests. One MSET pass (0.959) fell below the twin's lowest pass; MSET's
spread is the widest for both twin and change at six passes.

## 5. Whitefoot gaps and witnesses

Sketches in the specification's style, not compiled.

**G1. Per-driver parts that hold owned values.** Whitefoot's proposed
`Shards<T>` (`shared-counters/DESIGN.md`, section 5.4; Whitefoot board item
`firn-req-shared-max`, owner's ruling to build sharded state) requires
`T: copy`, so a part cannot hold a `Box<Slots<u8>>` buffer or a
`PriorityQueue`. firn needs parts made per shard by a constructor, and the
whole-run target to swap their owned contents.

```whitefoot
struct Part {
  log: Box<Slots<u8>>;
  refused: Bool;
}

fn part_new() -> made: Part pure {
  return Part(log: box_slots_new::<u8>(capacity: 65536_u64), refused: False());
}

// Requested: shards made by calling part_new once per shard.
fn write(keys: &Shared<ConcurrentHashMap<u64>>, parts: &Shared<Shards<Part>>, k: &[u8]) -> r: unit reads(keys), reads(parts), reads(k) waits {
  let mine = any_shard();
  atomic slot = &keys^[k], p = &parts^[mine] {
    if p^.refused {
      return unit;
    }
    set slot^ = Some<u64>(value: 1_u64);
    log_append_plain(log: &p^.log, bytes: k);
  }
  return unit;
}

fn drain(parts: &Shared<Shards<Part>>, out: &Box<Slots<Box<Slots<u8>>>>) -> r: unit reads(parts), writes(out) waits {
  atomic all = &parts^ {
    for @each (i in 0_u64..all^.len) {
      let fresh = box_slots_new::<u8>(capacity: 65536_u64);
      swap(first: &all^[i].log, second: &fresh);
      slots_push(slots: out, value: move fresh);
    }
  }
  return unit;
}
```

Acceptance: 64 contexts running `write` on random keys from 100 000 reach at
least 1.6 times per doubling on 2/4/8 P-cores of the 14900K, and `drain`
every 10 ms does not take that below the criterion.

**G2. A sequence taken inside a statement without holding it exclusively.**

```whitefoot
// Requested: a shared sequence and a take that is not an atomic target.
fn write(keys: &Shared<ConcurrentHashMap<u64>>, parts: &Shared<Shards<Part>>, seq: &Shared<Sequence>, k: &[u8]) -> r: unit reads(keys), reads(parts), reads(seq), reads(k) waits {
  let mine = any_shard();
  atomic slot = &keys^[k], p = &parts^[mine] {
    let n = sequence_take(seq: seq);   // n increases in SHARE-3's one order
    set slot^ = Some<u64>(value: n);
    frame_append(log: &p^.log, sequence: n, bytes: k);
  }
  return unit;
}

fn cut(parts: &Shared<Shards<Part>>, seq: &Shared<Sequence>) -> s: u64 reads(parts), reads(seq) waits {
  let s = 0_u64;
  atomic all = &parts^ {
    set s = sequence_peek(seq: seq);   // above every take before this statement, below every take after
  }
  return s;
}
```

Required meaning: a take is part of its statement's effect; if statement A
takes effect before B in SHARE-3's order and both take, A's value is
smaller; values are distinct; a take near u64's maximum reports exhaustion
rather than wrapping. On a target with one driver a take is a plain
increment; on a target with several drivers and no hardware read-modify-write
it needs a lock, which the relaxed-fields ruling on target capabilities
decides (refuse at composition or not). Acceptance: the witness at 8 P-cores
runs at least 75% of the same program without `sequence_take`, and its
`(key, n)` log, sorted by `n`, replays to the final map in every run.

## 6. Found along the way

- **With maxmemory set, every command takes Meta exclusively at
  admission.** `evict_before` runs `memory_over` in an exclusive Meta
  statement for each command when the limit is nonzero
  (`firn/commands/eviction.wf:314-323`, called from
  `firn/commands/transaction.wf:398`), and `memory_over` also raises
  `memory_peak` there (`eviction.wf:53-56`). So with a limit set, GET too
  queues on Meta, the pattern that held 8-core GET at 2.98 M/s before the
  per-connection peak check. **Inferred from source, not measured.** Cache
  deployments set a limit (`design/firn/memory-limit.md`). Disposition:
  (b)'s per-part copy of the AOF capacities, plus the per-connection peak
  check, removes it; until then, a board item under `firn-core-scaling`.
- **Lazy expiry found by a write does not count `expired_keys`.**
  `entry_kind` removes an expired value without incrementing the counter
  (`firn/commands/keys.wf:249-265`), while `remove_expired` and `expire_key`
  do (`firn/store/store.wf:163,190`). Redis's `expireIfNeeded` counts both.
  **Inferred from source;** check with `SET k v PX 1`, wait, `SET k w`, then
  INFO stats against Redis. Disposition: board item; (b)'s per-part counter
  is where it would be counted.
- **With AOF on, rewrites run during benchmarks and each scan step holds
  the whole map and Meta.** The benchmark starts firn's AOF with Redis's
  default thresholds (100%, 64 MB) (`research/experiments/redis-bench/redis-bench.sh:493-495`).
  At about 1 GB/s of AOF a rewrite would start every fraction of a second,
  and each scan step, count hint 1, takes the whole map
  (`design/firn/aof-rewrite/capture.md`). **Judgment;** step 2 records
  `aof_rewrites` per run to see it. Disposition: board item; batching scan
  steps needs resumable map iteration (status board item `coord-wfbl-02-01`).
- **The proposed `Shards<T>` excludes owned values (`T: copy`).** That is G1;
  disposition: filed into Whitefoot with the witness above.

## 7. Decisions the owner would need

1. With AOF off, should write commands that queue no expiry stop holding
   Meta, and `firn_aof_rewrite_commit_seq` be left out of INFO while AOF is
   off instead of counting statements? Recommended: yes.
2. Should firn ask Whitefoot for a sequence that a statement takes without
   holding it exclusively (G2), reopening for identifier allocation the
   single-step integer cell set aside for counters, rather than ordering the
   AOF by key stripes, which breaks the prefix-state guarantee? Recommended:
   yes.
3. Should firn ask Whitefoot for per-driver parts that hold owned values
   (G1), instead of only copy values? Recommended: yes.
4. Once G1 and G2 exist, should per-driver parts replace Meta for the
   collecting buffer, expiry queue, expiry count and copies of the writer's
   published state, with the writer merging by sequence? This changes the
   approved nodes named in (b). Recommended: yes.
5. Later, when step 2 starts: should the rewrite reserve R be split evenly
   across parts, a capture aborting when one part's slice runs out, instead
   of one shared charge? No recommendation yet; it depends on whether G2's
   take can also serve a non-exclusive add.
