# Firn-wf

firn is a server of Redis's protocol written in
[Whitefoot](https://github.com/Ming-Research/Whitefoot), a systems language
whose compiler admits a program only once every partial operation in it is
proved safe. Redis clients talk to firn over TCP as they would to Redis
7.0.15: it keeps strings, lists, sets, hashes and sorted sets in one shared
keyspace, expires keys, and persists every change to an append-only file. It
is named for firn, snow that has lasted a season: stored and compacted.

firn exists to show what Whitefoot gives a concurrent, persistent network
server and to expose what it still lacks. It began as the subset of Redis
that `redis-benchmark`'s default suite needs and is growing toward
standalone application workloads: cache and session storage and scripted
conditional updates, used by existing applications through their ordinary
clients ([design](design/firn.md)).

## Status

- **Commands:** the keys, strings, lists, sets, hashes and sorted-set
  commands, the server and connection commands, and the append-only file
  that [firn/README.md](firn/README.md) lists, answered in RESP2 or inline,
  pipelined or not.
- **Compatibility:** judged against Redis 7.0.15 by differential runs and
  Redis's own test suite
  ([redis-compat](research/experiments/redis-compat/README.md)).
- **Performance:** measured with `redis-benchmark` against Redis, Valkey,
  Dragonfly and Garnet ([measurements](research/investigations/firn/DESIGN.md)).
- **Not yet:** scripting (`EVAL`, through the Halo engine), `WATCH` and
  transactions of commands beyond those `firn/README.md` lists,
  RESP3, memory limits and eviction, replication and clustering, among the
  gaps [docs/todo.md](docs/todo.md) lists. firn is a research server, not a
  production database.

## Build and run

On Linux x86-64 or macOS arm64, with `git`, `curl`, Python 3,
`/usr/bin/clang` and, on Linux, LLD:

```sh
git clone --recurse-submodules https://github.com/Ming-Research/Firn-wf.git
cd Firn-wf
make firn
./build/firn 6379 0 - 0
redis-cli -p 6379 PING
```

`make firn` downloads the Whitefoot compiler release that `whitefoot.pin`
names and builds the server; [firn/README.md](firn/README.md#build-and-run)
explains its arguments and options. `make check` is the gate every change
passes.

## Repository

- `firn/`: the server, one Whitefoot module program (`firn/modules.wfg`).
- `whitefoot.pin`: the Whitefoot compiler release firn builds with;
  `whitefoot-kit/`, a submodule of
  [Whitefoot-kit](https://github.com/Ming-Research/Whitefoot-kit), fetches
  and checks it.
- `design/`: the decisions firn is built on, and their approval log;
  `design/skill/`, a submodule of
  [Design-skill](https://github.com/Ming-Research/Design-skill), holds the
  tree's lint.
- `research/`: the server's design and measurements
  (`research/investigations/firn/`) and the instruments behind them
  (`research/experiments/`).
- `docs/todo.md`: known gaps and defects.
- [AGENTS.md](AGENTS.md): how work on firn proceeds.

firn's history before it moved here is in the Whitefoot repository, where it
lived as `apps/firn`.

## License

MIT; see [LICENSE](LICENSE).
