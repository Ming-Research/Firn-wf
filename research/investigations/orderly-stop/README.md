# Orderly stop

## The question

Redis 7.0.15's `SHUTDOWN`, and its handling of SIGTERM and SIGINT, append and
sync the append-only file, close every client and end the server at once.
firn ends only when its entry returns, and Whitefoot's spawns are structured:
the entry joins every context it started, so every client's context, the
accept loop and the writers must reach their end first
([kernel specification v0.94, WAIT-3's join rules](https://github.com/Ming-Research/Whitefoot/blob/0b7f5c5b98547dd27deb9691aed36281e350576b/spec/kernel-spec.md)).
A context waiting in `tcp_accept`, `receive_next` or `send_once` with no
deadline waits until the host produces an outcome, and v0.94 has no way to
cancel such a wait from another context. A stop request can therefore reach
every context in one of two ways:

- **Polling.** Every wait carries a deadline of at most a second, and each
  context reads the keyspace's `stopping` when its wait ends. `serve`
  (`firn/server/server.wf`) already does this while an idle limit is set.
- **A Whitefoot capability.** The language or its library lets a stop end
  the waits of other contexts, or lets the program end without joining them
  once its files are synced.

Polling needs no Whitefoot change. Its cost is a monotonic clock read and a
timer armed and removed on every receive that parks, which in an unpipelined
request and reply is every request (`wf_context_arm_deadline` in Whitefoot's
`compiler/src/backend/completion/bridge.c`). The question is whether that
cost is measurable on firn's throughput.

## The comparison, stated before measuring

- Workload: `redis-bench.yml` mode compare on the i9-14900K. Revisions:
  - `base`, Firn-wf main 936110104;
  - `head`, the same with a receive deadline of at most a second armed
    whether or not an idle limit is set;
  - `head-twin`, head's image again, as the noise control.
- Tests `set` and `get` at pipeline depths 1 and 16, on 1 and 2 server CPUs.
  A probe of 2 passes of 5 seconds sizes the run; the decision rests on the
  sized run.
- **Rejection rule.** Polling is rejected if head's median rate at depth 1,
  for either test on either CPU count, is lower than base's by more than 2%
  and by more than the difference between head and head-twin there. The
  alternative is then to ask Whitefoot for the capability above. Depth 16
  is reported, not decided on, since a deep pipeline parks once per batch.

## Results

None yet.
