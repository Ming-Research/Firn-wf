# Design tree change log

Newest first. One entry per approved change of the tree: a dated title,
`Nodes:` naming every node changed, `Owner-approved:` and `Summary:`; the
owner-wide instructions' design-tree part owns the form.

## 2026-10-08 Rewriting the append-only file in Redis 7's multi-part layout

Nodes: firn/aof-rewrite, firn/append-only-file

Owner-approved: In the Firn session of 2026-10-07, written in Chinese, after the report that presented keeping Redis 7.0.15's multi-part layout and rebuilding from the closed logs as Q213 option A, recommended: "213, what is AOF? But A looks reasonable" (translated), taken as approval; in the Firn session of 2026-10-08, after the completion report that presented refusing an append-only file behind a symbolic link as Q225 option A, refusing BGREWRITEAOF with appendonly off as Q226 option A, keeping the new manifest and every published file when the directory sync after its rename fails as Q227 option A, cutting an unfinished block from the current incremental file before a switch as Q231 option A, and quoting apostrophes in manifest filenames as Q232 option A, each recommended, with Firn-wf#29's design-tree text and the merge order as Q234: "Q225-Q234 all approved" (translated).

Summary: firn keeps the append-only log as Redis 7.0.15 does, a manifest naming a base file and incremental files below `appendonlydir`, upgrades an old single file by moving it into the directory, and answers BGREWRITEAOF and the automatic rewrite trigger by replaying the base and closed incremental files into a private keyspace and writing that dataset as a new base, while live commands go to a new incremental file, because firn has no fork and no snapshot to write from. Persisting the new manifest is the commit point; a failed directory sync after it keeps the new manifest and every published file in force, where Redis would delete the new base the manifest names. Before a switch the writer cuts an unfinished transaction block from the current incremental file, since the next start reads that file as one before the last, where an incomplete end is refused; `append-only-file`'s cut accordingly applies only to the last file. Manifest names with apostrophes are written in double quotes so Redis can read them back, BGREWRITEAOF with appendonly off is refused as Redis refuses a rewrite it cannot start, and a file behind a symbolic link is refused, provisionally, where Redis follows it ([remaining compatibility work](../docs/todo.md#server)).

## 2026-10-07 SHUTDOWN stops firn in order

Nodes: firn/orderly-stop, firn/transactions, firn/command-parts

Owner-approved: In the Firn session of 2026-10-07, written in Chinese, the owner answered the reports' ledgers with "Q209 agreed. Q210 agreed" (translated), approving Q209 A (a script that never ends keeps firn from stopping until the busy-script work) and Q210 A (merge Firn-wf#27 once this entry's gate and readiness pass); with "I think 206 is a hack; other ways of getting stuck cannot necessarily be solved like this. It must go into the TODO" (translated), and after the TODO entry and the stopgap wording, "Q206 agreed" (translated), approving Q206 A (polling as a stopgap); and with "208 agreed ... 207 approved" (translated), approving Q207 (SHUTDOWN SAVE takes Redis's failed-save path) and Q208 A (SHUTDOWN inside MULTI refused when sent).

Summary: SHUTDOWN records a request in the server's state, which every context reads by polling: each socket wait carries a deadline of at most a second, at a measured cost of at most 1.25% at pipeline depth 1, because Whitefoot v0.94 offers no way for one context to end another's wait; polling is a stopgap until that Whitefoot capability, recorded in docs/todo.md, replaces it. main stops the append-only file's writer only after every client has gone, so the commands clients ran before seeing the request are appended and synced. SHUTDOWN SAVE takes Redis 7.0.15's failed-save path because firn writes no snapshot; inside MULTI, SHUTDOWN is refused when sent and dirties the transaction, and scripts refuse it as noscript, as Redis 7.0.15 does ([investigation](../research/investigations/orderly-stop/README.md)).

## 2026-10-07 Enumeration through map_scan and RANDOMKEY's sampling

Nodes: firn/enumeration

Owner-approved: In the Firn session of 2026-10-07, written in Chinese, the owner answered the report's ledger with "approve all" (translated), approving Q200 (SCAN, KEYS and RANDOMKEY enumerate through Whitefoot's map_scan instead of an index of keys beside the map) and Q201 A (RANDOMKEY samples as Redis 7.0.15's dbRandomKey does), together with Q202 (merge Firn-wf#26) and Q203 A (merge Firn-wf#25 once this entry's gate and readiness pass).

Summary: firn implements SCAN, KEYS and RANDOMKEY over Whitefoot specification v0.94's map_scan, whose position cursor returns each key present throughout a scan exactly once while other clients write, instead of keeping an index of keys beside the keyspace map that every write would update. RANDOMKEY picks uniformly among the keys one map_scan step of count 15 gathers from a random position, wrapping at the table's end and drawing a fresh position when none is gathered, as Redis's dictGetFairRandomKey and dictGetRandomKey draw, instead of the first live key after a random position, whose chance grows with the empty run before it; firn_draws_random_keys_uniformly_as_redis_does separates the two (Firn-wf runs 37554610167 and 37554594356).

## 2026-10-06 Consumers, RESP3, transactions, scripts on Halo, command parts for every family, measurement sessions and known hangs

Nodes: firn, firn/command-parts, firn/consumers, firn/measurement-sessions, firn/protocol-versions, firn/reported-facts, firn/scripts, firn/transactions

Owner-approved: In the Firn session of 2026-10-06, written in Chinese, the owner answered the report's ledger with "all Qs agreed" (translated), approving on their recommendations Q61 (redis.call through Halo's resumable host call), Q63 A (a script's frozen time kept, on Redis 7.2's ground), Q65 (reported facts), Q66 (the six scripting decisions, one engine taken in turn among them), Q67 (measurement sessions), Q69 (the consumers), Q70 (RESP3 after HELLO 3), Q71 (transactions), Q72 (a body over a key set's entries) and Q73 (known hangs skipped by the gate).

Summary: The deployment milestone's consumers are Django, rate-limiter-flexible and connect-redis, their commands recorded against Redis 7.0.15. firn answers RESP3 after HELLO 3, runs MULTI/EXEC in one statement over every key, reports facts about itself with real or fixed values and leaves out what it cannot measure, and runs Lua scripts on Halo: in budgeted attempts in one statement, on one engine every script takes in turn so that Lua state persists as in Redis's one Lua state, with redis.call through Halo's resumable host call and SCRIPT KILL between attempts. A command's body may work over the entries of the key set it names, so RENAME, COPY, LMOVE, SMOVE and the set combinations share one implementation with EXEC and scripts. Measurements on the shared runner stop only the servers they registered. The suite gate skips tests already seen hanging, which the recording runs still try. A script's frozen time now rests on Redis 7.2's single snapshot per execution unit rather than on Redis 7.0.15, which freezes expiry checks alone.

## 2026-10-06 firn builds with a pinned release through Whitefoot-kit

Nodes: firn/building

Owner-approved: In the Firn session of 2026-10-05 and 2026-10-06, written in Chinese: Q44 B, a downstream downloads a published compiler; Q51, releases made on request and pinned by name, refined by the owner to compiled releases, never a Whitefoot submodule ("downstream uses the compiled release, not a submodule", translated); Q54 A, the shared rules kept in one repository taken as a submodule ("Q54 picks A"); Q55, its name Whitefoot-kit ("Q55 as recommended").

Summary: firn builds with the Whitefoot compiler release `whitefoot.pin` names by commit, a revision bound for main naming a release of a commit on Whitefoot's main, instead of a Whitefoot submodule or a specification-version pin, and takes the pin's checks, the download and the upgrade rules from the shared `whitefoot-kit` submodule, instead of a copy per project; shipping the shared rules in each release is refused because the downloader cannot come from what it downloads.

## 2026-10-06 Redis's own suite gates firn as a ratchet

Nodes: firn

Owner-approved: Q53 A, in the Firn session of 2026-10-06, written in Chinese: "52 53 both agreed" (translated), to the proposal that Redis 7.0.15's test suite run in every push's gate as a ratchet of the tests firn passes.

Summary: `design/firn.md` gains the decision that Redis 7.0.15's own suite gates firn as a ratchet of named passing tests, run without tolerating the framework's errors, so a lost pass fails the gate even when another test starts passing, instead of an aggregate count or a whole-suite requirement; repeated CI runs establish the passing set and tests passing only sometimes are listed apart with their frequency.

## 2026-10-06 Command parts, scripts' commands and the append-only file's blocks and cut

Nodes: firn/command-parts, firn/scripts, firn/append-only-file

Owner-approved: Approved in the firn session of Whitefoot PR #245 (2026-10-05) as the scripting groundwork plan S1-S6, its follow-ups 1-5, and rulings Q39 and Q40, and Q42 A in the Firn session of 2026-10-05, written in Chinese, where the owner answered "OK" to the proposal that Q41 and Q42 go with option A; Whitefoot PR #245's description records the first set, and lists Q42 as open because it predates that answer. Its code moved here as Firn-wf #3. The decision that a file that does not load stops firn is Q56 A, which the review of Firn-wf #3 raised, approved in the Firn session of 2026-10-06, written in Chinese: "agree".

Summary: A command is four parts, parsing, a body over its key's entry, a metadata step and a reply, shared by the network path and `script_command` (S5, follow-up 1). Scripts refuse Redis's noscript commands and answer a command not yet split with Redis's unknown-command text plus a firn note, instead of a second name table (S4, Q42 A). A script's commands see the clock as it was when the script began (S2); a key a script finds expired is removed at once with its `DEL` in the effects (follow-up 2); the caller wraps more than one appended record in `MULTI`/`EXEC` (Q39). Replay applies a block only once its `EXEC` is read (S3), and a file whose end did not load is cut as Redis cuts it, with `std::fs::truncate_file` (Q40). A file that does not load, a malformed record, a failed read or a block past the input window's ceiling, stops firn with status 4 before it listens, as Redis 7.0.15 exits, instead of serving part of the file (Q56 A).

## 2026-10-06 The firn tree moves here from Whitefoot

Nodes: firn

Owner-approved: Both decisions were approved in the Whitefoot session of 2026-10-03 as Q1 (complete standalone application workloads) and Q2 (a Lua interpreter written in Whitefoot, beginning with a vertical slice), recorded in Whitefoot's `design/log.md` entry "Firn's standalone deployment and Whitefoot Lua direction" ([Whitefoot at 648338c31](https://github.com/Ming-Research/Whitefoot/blob/648338c31240ba64ce13c314b1afca1749d44189/design/log.md)). The move itself follows the owner's ruling Q43, that firn leaves Whitefoot for this repository. In the session of 2026-10-06, written in Chinese, translated: "Decision 1 is right", and of decision 2: "I approved this long ago, otherwise Halo would not have been written up to now".

Summary: Whitefoot's node `language/firn` becomes this repository's root node `firn` (`design/firn.md`) with its two decisions unchanged: firn's first public deployment milestone is complete standalone Redis application workloads with existing clients and unchanged business logic, and its scripting milestone a Lua interpreter written in Whitefoot, tested first through a vertical slice against Redis's interpreter. Only the links to the deployment direction changed, to `research/investigations/firn/DESIGN.md` in this repository.
