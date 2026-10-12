# Runtime contention witnesses

Small Whitefoot programs, independent of firn's code, that separate the
runtime paths suspected of making firn's throughput fall as server cores rise.

## Question

With pipelined clients (depth 16) on the i9-14900K, firn's GET peaks at 2-4
server CPUs (7.85 M/s on 4) and collapses after them (0.87 M/s on 16, 8
P-cores with both hyperthreads), the server's CPUs staying 99% busy and its
CPU per request rising from 0.26 µs to 18 µs; SET and MSET collapse earlier.
Whitefoot's runtime owner, reading the runtime, named three suspects:

1. an atomic statement that holds concurrent-map entries and then takes a
   shared object never suspends: it spins, calling `sched_yield` every 256
   turns (firn's write commands hold their keys, then the keyspace's
   metadata);
2. an ordinary atomic statement that finds its object taken spins 256 times
   before it suspends;
3. the runtime's internal spin locks, on each object and on each driver's
   ready and receive queues, with two hyperthreads of one core slowing each
   other.

Each witness runs `CONTEXTS` contexts for `MILLISECONDS` and prints one line,
`witness=NAME contexts=N drivers=D ops=OPS ns=ELAPSED ops_per_s=RATE
check=C`. Every context first waits at a gate, a shared object the root
opens once it has spawned them all, so that all contexts start together
whatever order the runtime starts them in; the gate carries the common
deadline. Each context then does the same loop: batches of 1024 operations,
each advancing a context-local 64-bit linear congruential number, then one
clock reading, until the deadline; at the end it adds its count to a totals
object once. The elapsed time runs from the gate's opening to after the last
join. The witnesses differ only in the atomic statement each
operation adds:

| witness | atomic statement per operation | isolates |
|---|---|---|
| `w_base` | none | the runtime's no-sharing baseline: contexts, drivers and the clock alone |
| `w_private` | adds one to a shared object the context made and alone names | the cost of an atomic statement that never meets another context, so per-object locking without contention |
| `w_shared` | adds one to one shared object every context names | suspect 2: an ordinary atomic statement on a contended object, with no map |
| `w_read` | reads and writes back the context's own entry of one shared concurrent hash map, under a key no other context uses | suspect 3: whether map-entry access alone, as firn's GET holds one entry, falls as drivers rise; no other shared object |
| `w_read_param` | `w_read`'s statement and body, its batches made by a function that receives the map as a reference parameter whose effect row only reads it, as firn's command functions receive the keyspace | the map's handle count: every other witness's loop reaches its objects through a value handle, so each atomic statement counts the object's handles up at its start and down at its end, and for the one map of `w_read` that counter is shared by all drivers; through a read-only reference parameter the compiler skips both updates |
| `w_heap` | `w_base`'s loop with one read of the process's live heap bytes per operation, each context through its own share of the memory meter | the heap reading firn takes after every command to record peak memory: it has no atomic statement, so any collapse against `w_base` is the runtime's own cost of reading the heap from every driver at once |
| `w_write` | `w_read`'s entry body, then adds one to one shared object every context names, both held in one statement, entry first | suspect 1: entries held, then a shared object taken, as firn's SET holds its key and then the metadata |

Reading the table across witnesses: `w_private` against `w_base` prices an
atomic statement; `w_read` against `w_read_param` isolates the handle count,
the two differing in nothing else; `w_read_param` against `w_private` shows
what map entries add when no two contexts touch one entry; `w_shared` shows
contention on one object; `w_write` against `w_shared` shows what holding an
entry first adds to that contention. A collapse in `w_read` that
`w_read_param` does not share points at the handle count, one in both at
suspect 3, one in `w_write` well beyond `w_shared`'s at suspect 1, and one in
`w_shared` at suspect 2.

The handle count is not in the specification: [SHARE-2] only requires that
"each object stays live until the statement completes". The skip is the
compiler's, at the pinned commit: an atomic target whose handle place is
rooted at a reference parameter of the enclosing function, with no `writes`
of that parameter in its effect row, is marked borrowed in the semantic check
(`compiler/src/semantic/check/control/atomic.rs`), and lowering then emits
no `wf__shared_share` for it (`compiler/src/lowering/builder/atomic.rs`).
Writing the held entry is still allowed through such a parameter, since a
statement's footprint keeps only its handle places, read, and drops every
path into the held state [SHARE-2], so `w_read_param` keeps `w_read`'s
write-back. firn's GET and SET reach the keyspace through `store: &Keyspace`
rows that only read it, so by that rule they already skip the count; the
witnesses `w_shared` and `w_write` do not, and so carry the count beside the
contention they measure. `run.sh` records how many `wf__shared_share` calls
the emitted programs of `w_read` and `w_read_param` hold, to confirm the two
differ by the one in the statement.

The entry bodies of `w_read` and `w_write` both read the value and write it
back, as firn's GET writes its access stamp and SET its value, so the two
differ only in the shared object `w_write` adds. Unlike firn's serving
contexts, which wait for their client's next requests, a witness context
waits only at its atomic statements. A `w_base` context therefore keeps its
driver until its deadline, and so does a `w_private` or `w_read` context if an
uncontended atomic statement does not suspend: then about as many contexts run
at once as there are drivers, and the rest run after the deadline and stop
after one batch. Where atomic statements suspend, more contexts take turns
through the ready queues. The per-driver counts `WF_SCHED_REPORT=2` prints
show which happened.

The standard library at the pinned compiler exposes no process CPU time
(`std::process` offers heap and resident bytes only), so the programs print
none; `run.sh` reads the user and system split of the CPUs it uses from
`/proc/stat`.

## Files

- `modules.wfg` and `module.wfm`: one module program with an entry per
  witness, `w_base`, `w_private`, `w_shared`, `w_read`, `w_read_param` and
  `w_write`.
- `harness.wf`: argument reading, the start gate, the totals object, the
  local operation, the context key and the report line, shared by every
  witness.
- `base.wf`, `private.wf`, `shared.wf`, `read.wf`, `read_param.wf`,
  `write.wf`: one witness each.
- `run.sh`: builds each witness with `--full-lto` and the compiler release
  `whitefoot.pin` names, runs each under `taskset -c` on the CPU lists `0,2`,
  `0,2,4,6`, `0,2,4,…,14` (one hyperthread of each of 1, 2, 4 and 8 P-cores)
  and `0-15` (8 P-cores, both hyperthreads), with `WF_DRIVERS` equal to the
  list's CPU count and `WF_SCHED_REPORT=2`, and writes `results.csv`
  (`witness,cpus,drivers,N,ops,seconds,ops_per_s,user_frac,sys_frac,pass`)
  and each run's standard output and error. `CONTEXTS`, `MILLIS`, `PASSES`,
  `WITNESSES` and `CPU_LISTS` in the environment change its settings.
- `run.env`: the settings of the temporary workflow
  `.github/workflows/runtime-contention.yml`, which runs `run.sh` on the
  14900K or a GitHub runner and uploads the results. A push that changes
  `run.env` is what starts it, because a workflow that exists only on a branch
  cannot be dispatched by hand; its `witnesses` line, not `run.sh`'s default,
  chooses the witnesses that run.

N defaults to 64 contexts: firn was measured with redis-benchmark's 50
connections, one serving context each, and 64 leaves at least four contexts
for each of the 16 drivers of the widest list.

## Purpose and lifetime

The witnesses serve the firn board item "firn 的吞吐随服务器核数上升" and the
Whitefoot request to be filed from it. When that request closes, this
directory is removed, or kept as the request's acceptance test if Whitefoot
adopts it as one.
