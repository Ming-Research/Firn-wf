# Rewriting the append-only file

## The question

firn appends every change to its append-only file and never shortens it. A
cache or session store that runs for weeks therefore fills its disk, and its
replay at start grows with every write it ever took. Redis 7.0.15 bounds
both with `BGREWRITEAOF` and its automatic trigger
(`auto-aof-rewrite-percentage` 100 and `auto-aof-rewrite-min-size` 64 MB),
which replace the history with a file holding only the current dataset. The
deployment milestone names the rewrite among what persistence needs
(docs/todo.md, "Complete firn's standalone deployment workloads").

The question is how firn writes a dataset that is consistent with the
writes that keep arriving, without stopping its clients, and how it switches
files so that a failure at any point leaves a log that replays to the right
state.

## How Redis 7.0.15 does it

`src/aof.c`:

- **Layout.** A directory, `appendonlydir`, holds a base file, one or more
  incremental files and a manifest naming them. Replay loads the base and
  then each incremental file in order.
- **Start** (`rewriteAppendOnlyFileBackground`). It flushes the current file
  and opens a new incremental file, which every later write goes to
  (`openNewIncrAofForAppend`), then forks.
- **The child** writes the dataset as of the fork, the same instant the new
  incremental file began, to a temporary file.
- **Finish** (`backgroundRewriteDoneHandler`).
  - The temporary file is renamed as the new base.
  - The manifest is rewritten to name the new base and the incremental files
    from the new one on; it is written through a temporary file, a rename
    and a directory sync.
  - The files no longer named are deleted.
- **Failure.** Before manifest publication, failure leaves the old manifest,
  whose base and incremental files, the new one included, still replay to
  the whole state. After rename, a directory-sync failure can leave Redis
  naming a new base that its failure handler then unlinks; firn retains it
  as described below.

The base's consistency comes from `fork`: the child sees the dataset exactly
as it was when the new incremental file began.

## Why firn cannot copy this directly

firn has no `fork`. A scan of the live keyspace with `map_scan` returns each
key once, but each key in the state of the moment the scan reaches it.

Replaying such a base and then every write since the scan began applies, a
second time, the writes made to keys before the scan reached them. `INCR`,
`APPEND` and `LPUSH` are not idempotent, so the replayed state would be
wrong.

## Candidates

- **A. Rewrite from the log.**
  - At the start, as Redis does, the current incremental file is closed and
    a new one opened for every later write.
  - A context of its own replays the closed files, the old base and old
    incremental files, which no longer change, into a private keyspace, and
    writes that keyspace as the new base.
  - The base is then exactly the state at the instant the new incremental
    file began, as Redis's is.
  - Costs: the replay's processor time on other cores, and, during the
    rewrite, a second copy of the dataset's memory, which is also the worst
    case of Redis's copy-on-write fork.
  - Needs: no stop, and no new Whitefoot capability beyond file replacement
    (Whitefoot PR #267).
- **B. Stop the keyspace and write it.** Hold the whole map, write or copy
  the dataset, and release it. This is simple and consistent, but every
  client waits for the whole dataset to be written.
- **C. Split the writes by scan position.** A write goes to the base's
  stream or the new incremental file according to whether the scan has
  passed its key. This needs a key's scan position, which Whitefoot does not
  expose, and, so that a failed rewrite still leaves a whole log, two
  streams of writes.
- **D. A snapshot of the keyspace kept by the runtime**, copy-on-write as a
  fork gives. This is a new runtime capability of large scope.

Layout: Redis 7.0.15's multi-part directory, base, incremental files and a
manifest, rather than one file. With it, the new incremental file begins at
the rewrite's start and nothing written during the rewrite has to be copied
into the new base. It also replays Redis's own directories written without
an RDB preamble (`aof-use-rdb-preamble no`).

## Proposal

A, in the multi-part layout.

Whitefoot v0.98 supplies `open_directory_write` to create or open
`appendonlydir` and `move_file` to move the old log into it. The
[writable-subdirectories investigation](https://github.com/Ming-Research/Whitefoot/blob/1ca213242/research/investigations/writable-subdirectories/README.md)
records the grounds for these interfaces.

Validation, stated before implementing:
- **A failure at each step.** A rewrite stopped at each of its steps (before
  the base is renamed, after, before the manifest is renamed, after, and
  before the old files are removed) leaves a directory that replays to the
  state of every acknowledged write.
- **Concurrent writes.** Clients writing `INCR`, `APPEND` and `LPUSH`
  throughout a rewrite replay, after a restart, to exactly the values they
  were answered.
- **Redis's own directory.** firn replays a directory Redis 7.0.15 wrote
  with `aof-use-rdb-preamble no`.
- **Cost.** On the 14900K, measure the rewrite's effect on throughput and
  p99 of the session workload, and its peak memory.

## Design

Names follow Redis 7.0.15 with `appendfilename` `F`, `appendonly.aof` by
default, below the directory `appendonlydir`:
- base `F.<n>.base.aof`, in the format of the commands, as Redis writes it
  with `aof-use-rdb-preamble no`, since firn reads and writes no RDB;
- incremental `F.<n>.incr.aof`;
- manifest `F.manifest`, one line per file, `file <name> seq <n> type <b|i|h>`,
  written as a temporary `temp-F.manifest`, synced, renamed over the
  manifest, and the directory synced.

Start:
- the directory is opened for writing, created when missing;
- a manifest is read and its base and incremental files replayed in order,
  every file but the last required to end on a whole command, the last cut
  as a single file is cut today;
- with no manifest and no files, an empty base `F.1.base.aof` is written, as
  Redis forces a base on an empty start, then `F.1.incr.aof` is opened and
  the manifest persisted;
- an old-style single file `F` beside the directory with no manifest is
  upgraded as Redis upgrades it: a manifest naming `F` as the base is
  persisted, then `F` is moved into the directory with `move_file` between
  the working directory's and append-only directory's write halves.

A rewrite, started by `BGREWRITEAOF` or automatically when the files have
grown by `auto-aof-rewrite-percentage` (100) over the base written by the
last rewrite and exceed `auto-aof-rewrite-min-size` (64 MB):
1. **Switch.** After draining any earlier partial append, the writer takes
   pending bytes in one atomic statement and appends and syncs them to the
   current incremental file. It opens the next incremental file and persists
   a manifest naming the old base, every incremental file and the new one,
   retaining the old handle until publication. It then selects the new handle
   and closes the old one. Records enqueued after that atomic statement go
   only to the new file if switching succeeds.
2. **Rebuild.** A context spawned for the rewrite replays the old base and
   the closed incremental files, which no longer change, into a keyspace of
   its own, then writes that keyspace to `temp-F.base` inside
   the directory as Redis's `rewriteAppendOnlyFileRio` writes a dataset:
   `SET`, and `RPUSH`, `SADD`, `ZADD` and `HMSET` of at most 64 elements a
   command, each key followed by `PEXPIREAT` when it has an expiry. Redis
   7.0.15's `rewriteHashObject` uses `HMSET` for hashes. It syncs
   the file. Redis writes its temporary file in the working directory and
   moves it in; firn writes it in the directory, where the rename cannot
   cross a file system.
3. **Install.** The temporary file is renamed to the next base name and a
   manifest naming the new base and the incremental files from the switch on
   is persisted; the old base and closed incremental files are then removed.
4. **Failure.** Manifest rename publishes the new selection. A failure before
   it leaves the manifest of step 1, removes the temporary file and any
   renamed base the manifest does not name, and records failure for `INFO`'s
   `aof_last_bgrewrite_status`. If rename succeeds but the directory sync
   fails, the new manifest stays in force, every published file and history
   entry is retained, and the status reports failure. The persistence result
   carries publication and durability separately, so the writer also adopts
   a published switch manifest and its new append handle on that failure.
   Redis 7.0.15's `writeAofManifestFile` can return failure after rename,
   while `backgroundRewriteDoneHandler` then unlinks the new base; retaining
   it avoids leaving a visible manifest that names a missing file.
   Only successful directory sync permits history cleanup. After that,
   cleanup cannot roll the base back or fail the rebuild, as Redis's handler
   ignores `aofDelHistoryFiles`' result. Startup or a later rewrite can retry
   cleanup for history still listed on disk; an unlink failure followed by
   a successful cleanup manifest can leave an unlisted orphan, as in Redis.

`INFO persistence` reports `aof_rewrite_in_progress`, `aof_rewrites`,
`aof_last_bgrewrite_status`, `aof_current_size` and `aof_base_size` from
the rewrite's state; `BGREWRITEAOF` answers as Redis does when a rewrite is
already running.

## Implementation notes

With appendonly off there is no closed log to replay and no snapshot.
`BGREWRITEAOF` therefore returns Redis's generic rewrite-start failure:
`ERR Can't execute an AOF background rewriting. Please check the server logs for more information.`
Redis's `bgrewriteaofCommand` allows that state; the compatibility work to
remove firn's difference is in [the TODO](../../../docs/todo.md#server).

The temporary base follows Redis's temporary append-only naming convention:
`TEMP_FILE_NAME_PREFIX` (`temp-`), appendfilename `F`, and suffix `.base`.
It sits beside `temp-F.manifest`, so servers with distinct appendfilenames
in one directory have distinct temporary files. Two servers with the same
appendfilename already share a manifest, which Redis does not support.
This rewrite consequently needs no exclusive temporary-file creation API.

The writer keeps its old append handle until the switch manifest is
published, as `openNewIncrAofForAppend` does, so an open or pre-publication
manifest failure leaves an appendable old file. It syncs that old file
before preparing the switch and closes it before spawning the rebuild.
The temporary base is buffered one key at a time outside host IO; map
statements touch only the private keyspace. Collection commands hold at
most 64 elements. The existing score formatter supplies round-trippable
score text without a second formatting implementation. Script caches have
no AOF records and no functions are persisted.

A metadata flag serializes accepting SHUTDOWN with the install handler,
as Redis's event loop does. The writer acquires it in the same atomic
statement that reads the shutdown request. A request accepted first prevents
installation; a request arriving during installation waits for it to end.
Main also waits for the flag before marking the writer stopped. Filesystem
IO stays outside atomic statements.

## Validation still needed

The network cases cover the disabled-AOF refusal, independent appendfilenames
sharing a directory, interrupted directory states, host refusals before
publication, concurrent writes, shutdown and successful installation. They
have not been run for this implementation; compilation and all checks await
CI after push. Interrupted-state fixtures establish restart expectations,
not execution of a failing directory sync. That path needs a CI fault
experiment in which manifest rename succeeds and the following directory
sync returns an error, for both switching and installation. It must observe
the selected files, retained history, later writes and restart, and require
`aof_last_bgrewrite_status:err`. No performance result is claimed.
