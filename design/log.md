# Design tree change log

Newest first. One entry per change of the tree: a dated title, `Nodes:`
naming every node changed and `Summary:`; the owner-wide instructions'
design-tree part owns the form. Older entries also carry an `Owner-approved:`
line, a field since retired.

## 2026-10-13 Limit concurrent clients with maxclients

Nodes: firn/startup

Summary: firn had no connection limit and rejected_connections stayed 0. The limit now follows Redis, with the one difference that the operator raises `ulimit -n` since firn cannot. Accept errors, descriptor exhaustion included, keep their existing handling, which ends the accept loop and stops firn; Redis logs them and keeps serving, which firn does not yet; that belongs to the separate accept-error change.

## 2026-10-13 Start firn the way redis-server does

Nodes: firn/startup, firn/measurement-sessions

Summary: firn read no config file, logged nothing and took four positional arguments; an operator replacing redis-server found it treating the config path as a port. It now starts and is configured as redis-server is, with two stated differences (one `bind` address; bare arguments refused) and a temporary path limit that Whitefoot's request for opening ordinary paths removes. The benchmark script chooses the old or new startup form per version so comparisons across the change still run.

## 2026-10-14 Gate the ten safety witnesses

Nodes: firn/building

Summary: Each witness is a pair of small programs written from a real defect in Redis, Valkey, KeyDB, Garnet or Dragonfly. Checking the diagnostic, not just a nonzero exit, avoids masking an unrelated error, and compiling the fixed form avoids masking a correct form Whitefoot does not support. A Whitefoot upgrade that stops rejecting a class, or stops accepting a fix, turns the gate red. Replaced: unchecked examples as safety evidence.

## 2026-10-21 Queue a promoted connection's INFO reply inside the lock interval

Nodes: firn/connection-ownership

Summary: Every admitted INFO form on a promoted (subscribed) connection commits its complete reply under the dataset, Meta and event-hub boundary, with the script-registry and server-state snapshots taken in that same non-waiting statement. Otherwise a key-count snapshot released before the reply is committed would let a later SET/PUBLISH transaction precede an INFO reply that reports the earlier empty database. Host heap and residency samples precede the statement, and INFO on a never-subscribed connection keeps its original snapshot path.

## 2026-10-13 Leave an AOF rewrite's memory out of maxmemory

Nodes: firn/aof-rewrite, firn/aof-rewrite/capture, firn/memory-limit

Summary: On the 14900K a cache with maxmemory about half its dataset and eviction on answered BGREWRITEAOF with "Can't execute an AOF background rewriting" where Redis 7.2.16 rewrites, so its append-only file would grow until the disk filled. Redis rewrites in a forked child whose memory is not the parent's used memory, and its eviction figure also excludes the AOF buffer. The new case rewriting under eviction and recovering the surviving data after a restart fails on main and passes here.

## 2026-10-14 Refuse writes after a failed directory sync at the rewrite's manifest swap

Nodes: firn/aof-rewrite

Owner-approved: On the shared status board on 2026-10-14, the owner chose option A (approve) on card firn-q-rw-dirsync-tree, which asked to approve PR #120's design change at head ca3d045 (the tree is unchanged at the current head): when the directory sync fails after the AOF rewrite renames the new manifest into place, the rewrite fails and firn records an AOF sync error (EINVAL excepted, as Redis treats it as success), then answers writes with MISCONF until a recovering rewrite installs durably; this deliberately differs from Redis 7.2.16, which reverts the switch but keeps acknowledging writes.

Summary: At the switch point the old manifest and old append file stay in force and the new incremental file is left in place, since both manifests list the old incremental file and data written to it survives whichever manifest a restart reads; at the install point the renamed new manifest stays in force and the sync error is recorded too. Continuing to switch to the new incremental file and acknowledge writes after a failed directory sync is rejected, because a power loss could then lose acknowledged writes (the new case failed on the code before the change). The rule matches firn's existing refusal of writes after a failed sync of the active file. EINVAL from the directory sync is not a failure: it means the filesystem cannot sync directories at all, so it counts as a successful sync, as Redis 7.2.16's fsyncFileDir treats it, and the rewrite completes with writes continuing on the new increment (the case switch-dirsync-einval shows both servers finishing the rewrite and acknowledging writes after it).

## 2026-10-14 Add publish/subscribe with promoted connections and an event hub

Nodes: firn, firn/connection-ownership, firn/pubsub-registries, firn/script-attempt-external-effects, firn/scripts, firn/transactions, firn/orderly-stop, firn/enumeration

Summary: A connection keeps the receive/execute/send owner until its first successful subscription, then moves once to independent receive, command and output owners. Membership changes and complete reply groups commit under one event hub, and one dispatcher publishes them in order to per-connection output inboxes. Channels, patterns and shard channels have separate indexes with monotonically allocated registration ids. A dataset command on a promoted connection holds the whole keyspace, Meta and hub through its reply commitment. Outstanding frames are charged to the connection's output limit, with Redis's pub/sub client thresholds. A script attempt that meets PUBLISH, SPUBLISH or PUBSUB retries in whole-map mode, stages its deliveries until the final reply, and publishes nothing if abandoned. EXEC adds the hub for pub/sub queues or promoted connections, and the reference suite's argument-free unsubscribe helpers are patched to drain the known acknowledgments. Pattern delivery and channel listing use the shared iterative glob matcher, whose nesting refusal the enumeration node now names for them.

## 2026-10-14 Match Redis 7.2.16's client-visible script behaviour

Nodes: firn/scripts, firn/command-parts

Owner-approved: On the shared status board on 2026-10-14, the owner chose option A (approve) on card firn-q-script-client-tree, which asked to approve PR #89's changes to the design nodes "scripts" and "command composition" at head 3a2c0ab (gate and fault-disk checks green, one review finding fixed).

Summary: `script_command` now also refuses EVAL, EVALSHA, EVAL_RO and EVALSHA_RO inside a script, with Redis 7.2.16's text after their arity checks, on top of the noscript commands it already refused. EVAL_RO and EVALSHA_RO share EVAL's source registry, compiled scripts, attempt protocol and command parts, with read-only authority set afresh on each invocation and kept through every retry; a script's write command is rejected after arity validation but before persistence and OOM checks, and redis.pcall can catch it.

## 2026-10-14 Keep firn's AOF rewrite diagnostics after INFO's Redis fields

Nodes: firn/info-diagnostics

Summary: INFO keeps firn's own append-only rewrite diagnostics (the firn_aof_rewrite_ fields that operators and the rewrite acceptance cases read) in the Persistence section, after every selected Redis 7.2.16 field. This keeps the real measurements and Redis's relative field order; interleaving them with Redis fields, deleting them or moving them to another section is rejected.

## 2026-10-14 Keep the list element ceiling and refuse before changing anything

Nodes: firn/lists

Owner-approved: On the shared status board on 2026-10-14, the owner chose option A (approve) on card firn-q-lists-ceiling-tree, which asked to approve PR #83's new design node at head 75133a9: firn lists keep the store-wide ceiling of 2^32 elements shared with hashes and sets, a deliberate difference from Redis 7.2.16 on 64-bit hosts; a command that would exceed it replies with an error naming the limit before changing any element or access metadata.

Summary: A push, LINSERT or LMOVE that would exceed the ceiling refuses whole, so a successful reply always means the elements were kept: a multi-element push is checked as one, LMOVE checks the destination before popping the source, a rotation within one list needs no room, and LINSERT with a missing pivot still returns -1. Dropping elements while replying success, partial multi-element pushes and popping the source before checking the destination are rejected. The ceiling is a storage policy, to be reopened when a selected workload needs more than 2^32 elements.

## 2026-10-14 Add SSCAN, HSCAN and ZSCAN as whole-collection scans

Nodes: firn/enumeration, firn/scripts

Summary: Until Whitefoot's standard-library hash map can resume a scan (board item coord-wfbl-02-01), SSCAN, HSCAN and ZSCAN return the whole MATCH-filtered collection in one held statement with cursor 0, in firn's element order; the temporary code is removed and replaced by bounded batches when the library delivers, and the item's acceptance waits for that. ZSCAN writes scores with Redis's d2string text when the whole sorted set has at most 128 members of at most 64 bytes each, and with the exact %.17g writer otherwise. The script rollback decision now rests on the scans having no cursor that spans an attempt, and reopens when bounded continuation arrives. Rejected: Redis encoding history, resuming at a numeric offset, dropping entries or returning cursor 0 for a partial result, one score formatter for all sizes.

## 2026-10-14 Write scores in replies and AOF rewrites byte for byte as Redis 7.2.16

Nodes: firn/scripts

Summary: Redis 7.2.16 selects the text rule by command and encoding, so one module now writes all of them: command-score replies and script double replies use d2string (nan, infinities and signed zeros first, an integral value up to 2^62 as an integer, otherwise fpconv_dtoa); a Lua number passed to redis.call or redis.pcall uses the integer rule and otherwise fpconv_dtoa; rewritten append-only scores use plain fpconv_dtoa; ZSCAN scores for skiplist sorted sets use ld2string(LD_STR_AUTO), implemented by the exact %.17g writer. The Grisu2 digits come from Halo-wf's port, checked against 18,235 doubles recorded on redis-server 7.2.16. Command-reply scores no longer keep %.17g.

## 2026-10-14 Keep a saved sync error through a successful close-sync of the old file

Nodes: firn/append-only-file

Owner-approved: On the shared status board on 2026-10-14, the owner chose option A on card firn-q-close-sync-recovery (keep firn's recovery rule; not clearing a saved sync error when the sealed old file's pre-close sync succeeds is a deliberate difference from Redis 7.2.16) and option A (approve) on card firn-q-close-sync-tree, which asked to approve PR #91's design change at head a6bbe8d: the decision on rotation preserving health cites Redis 7.2.16 and names its close-sync success rule as an intentional difference, and the rejected clearing on a successful retried sync now includes the pre-close sync.

Summary: A saved sync error survives the successful sync of the sealed old incremental file at a rewrite's switch, as it survives any later successful sync, and only a durable rewrite installation clears it; Redis 7.2.16's BIO_CLOSE_AOF success path clears it at once. A later successful sync does not write the pages whose writeback failed, so clearing would acknowledge writes before the hole is repaired and a restart would lose them. A faulty-device CI case with only the old incremental file on the failing device shows both servers refusing SET and PING with MISCONF after the failed close sync and accepting writes again after the device recovers ([close-fsync status](../research/investigations/aof-write-errors/README.md#close-fsync-status-in-redis-7216)).

## 2026-10-14 Initialize the access stamp at RENAME's destination and MSETNX stores like Redis 7.2.16

Nodes: firn/memory-limit

Summary: A successful RENAME or RENAMENX to a different name initializes the destination's LRU clock or LFU minute with counter 5 instead of carrying the source's stamp, because Redis 7.2.16 adds the moved value through dbAddInternal, which initializes it; same-name and refused renames keep the ordinary lookup refreshes. MSETNX keeps its ordered existence lookups but stores without refreshing a repeated key after each pair, as Redis 7.2.16's msetGenericCommand does, so the last pair wins with the first insertion's fresh stamp. The LFU outcomes are recorded in research/experiments/aof-oracle/lfu-rename-msetnx.json, from Firn-wf run 38406427583.

## 2026-10-14 Exclude expired keys from SCAN and cap glob nesting as Redis 7.2.16

Nodes: firn/enumeration

Summary: SCAN removes and records a matched key found expired before deciding whether its type belongs in the reply, so the removed key is excluded under every TYPE argument including none, as Redis 7.2.16's scanGenericCommand rejects a null lookup result. The shared glob matcher, used by KEYS, SCAN MATCH and CONFIG GET, obeys Redis 7.2.16's maximum nesting of 1000 while staying iterative, counting only entry into a nonterminal star suffix after adjacent stars collapse; a 1001st suffix entry returns no match and a trailing star can still succeed at depth 1000.

## 2026-10-14 Write SET with PXAT and effect groups as Redis 7.2.16 does

Nodes: firn/append-only-file

Summary: A successful SET supplied with PXAT appends its original argument vector; EX, PX and EXAT append SET key value PXAT with the absolute millisecond expiry, and GET is stripped only when there is no expiry, as Redis 7.2.16's setGenericCommand does. An ordinary top-level command remembers its initial AOF length and an effect counter, appends effects under its existing atomic hold, and inserts the 15-byte MULTI frame at that offset and appends the 14-byte EXEC frame only when it commits several effects; zero effects write nothing and one stays bare, as propagatePendingCommands groups the execution unit, including a command's own lazy-expiry DELs. The single-effect hot path adds only the offset and counter; the frame uses the rotation budget and is completed before sequence commit. Counted partial SPOP contributes one SREM per batch of at most 1024 members. Scripts and EXEC keep their outer grouping; active-expiry and eviction deletions are published independently.

## 2026-10-13 Stop the append-only replay on a stop signal and exit 0

Nodes: firn/orderly-stop

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-replay-stop-tree, which asked to approve PR #124's design change at head 0059406: the stop listener opens before the replay, so a host that refuses it stops firn with status 3 before the replay and ahead of status 4; a stop requested during the replay stops it at the next check (before the first command, then every 1024 commands, as Redis 7.2.16 does) and firn exits with status 0 without creating or opening an incremental file, truncating, deleting history or writing the manifest; an upgrade of an old single file stays before the replay as in Redis; having the replay poll a cancellation handle is rejected.

Summary: A SIGTERM or SIGINT during the startup replay used to end firn by the host default, where Redis 7.2.16 installs its handlers before loading and exits with status 0 leaving the append-only directory unchanged. The listener now opens first and the replay reads the server's stop request. A request arriving after the last check is taken as the ordinary orderly stop once firn listens. The first signal still closes the listener, so a later signal ends firn at once; matching Redis there needs a Whitefoot immediate exit, requested from Whitefoot.

## 2026-10-13 Check maxmemory admission without Meta while no AOF is kept

Nodes: firn/memory-limit

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-admit-tree, which asked to approve PR #67's design change at head bc2425d: while firn keeps no append-only file, a command's maxmemory admission check reads the heap and compares it with the limit without a Meta statement and takes Meta only once the reading is over the limit; with a file it keeps the Meta statement; the decision cites the 14900K measurement instead of being provisional.

Summary: Every command's admission check took the exclusive Meta statement whenever a limit was set, so with an 8 GiB limit far above use main ran depth-16 GET at 0.117 and SET at 0.145 of its unlimited throughput on 8 P-cores of the i9-14900K. Without an append-only file nothing is excluded and no rewrite can start, so the Meta-held check reduces to a heap comparison; with it the change ran at 0.941 and 0.948, and without a limit within the twin's range of main ([admission without Meta](../research/investigations/memory-limit/README.md#admission-without-meta-while-no-aof-is-kept)). Admission still raises the peak when its reading exceeds the peak the connection last saw recorded.
## 2026-10-13 Carry the AOF transaction boundary instead of rereading history

Nodes: firn/aof-rewrite, firn/aof-rewrite/capture

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-rw-boundary-state, which asked to approve PR #78's design change at head 17138bd: at the rewrite's file switch, the AOF writer uses the transaction-boundary state it carries from the startup parse and advances with every append, instead of rereading the whole old incremental file; the PR merges after the 14900K measures the per-append parse cost within the twin's noise.

Summary: On the 14900K a rewrite under 50 writing connections scanned the keyspace and then aborted with rotation_reserve, because the single AOF writer reread about 525 MB of incremental file at the switch while new writes filled the capture buffer past its 16 MiB reserve; Redis 7.2.16 does not reread at its switch. The writer now keeps the file length, parse position and first unclosed MULTI from the startup parse, advances them with the same parser after each successful append, and uses them at the switch; it invalidates the state and falls back to the reserve-charged, cancellable reread only when the boundary is uncertain (a failed append without rollback, a failed truncation, an unexpected form, or an unreplayed file opened at startup). Rereading the whole history at every switch is rejected because of that abort ([scan-pace](../research/investigations/aof-rewrite/scan-pace.md)). The capture decision's wording now matches where the writer checks for an abort.

## 2026-10-13 Write a conditional Meta hold as one statement

Nodes: firn/command-parts

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-one-stmt-design, which asked to approve PR #72's design change at head 0be3c57 (option A, in English: approve; merge #72, then write the other 39 sites as one statement and measure the whole change once on the 14900K).

Summary: A command that holds Meta only on some runs (AOF kept, or an expiry it may queue) is written as one atomic statement naming its keys and Meta, with every use of Meta under the run-time condition, instead of two statements around that condition. The pinned compiler acquires a named target at its first use on a path, so the path that never uses Meta never holds it; SET so written measured within the twin's noise at six passes on 8 P-cores with AOF off ([write-meta](../research/investigations/write-meta/README.md#one-statement-set)). The specification only permits holding less than a statement names, so the form rests on the compiler's behaviour, which Whitefoot's lock ledger (Whitefoot#421) is to check at each site; the owner kept the language without a conditional target.

## 2026-10-13 Describe a short AOF append by the host's no-space text

Nodes: firn/append-only-file

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-enospc-design, which asked to approve PR #70's design change at head 13ba7ed with the pin moved from wf-c52e867cae4f (specification v0.135) to wf-2a785316590a (specification v0.136): a partial AOF append's MISCONF reply ends with the host's description of the no-space class.

Summary: Redis 7.2.16 reports every short AOF write as ENOSPC. Whitefoot v0.136's error_copy_class_description gives the host's text for an error class, so firn's reply now equals Redis's word for word without naming a platform error number, which closes the gap the previous empty suffix recorded.

## 2026-10-13 Run Lua scripts as Redis 7.2.16 does

Nodes: firn/scripts

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-host72-design, which asked to approve PR #69's three new decisions at head 34c96c2: a script's numbers and double replies are written with Redis 7.2.16's Grisu2 text through Halo's port; the script environment is protected once through Halo's embedding interface as Redis's scriptingInit does; redis.set_repl accepts REPL_AOF and REPL_ALL and refuses the flags firn cannot honor.

Summary: Redis 7.2.16 writes script numbers with Grisu2, whose digits differ from the exact shortest form for 355 of 16,922 recorded values, so firn takes Halo's byte-checked port; command-reply scores keep %.17g until board item firn-score-text-72. Protection through the embedding gives Redis's refusal message at Redis's location. firn records a command's changes and its expiry deletions under one switch, so it refuses REPL_NONE, REPL_SLAVE and REPL_REPLICA rather than honor them partly; this is a deliberate difference from Redis.

## 2026-10-13 Let AOF-off writes that queue no expiry hold only their keys

Nodes: firn/command-parts, firn/aof-rewrite/sequence

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A (approve) on card firn-q-write-meta-design, which asked to approve PR #65's design-tree change at head d82c411: while AOF is off, a write that can queue no expiry and reads nothing else from Meta holds only its keys' entries, and admission's saved-error check skips Meta; such statements take an inert sequence token and INFO leaves firn_aof_rewrite_commit_seq out while AOF is off.

Summary: one exclusive Meta target on every write bounded all writes together; on the i9-14900K main's SET fell to 1.23 M/s on 8 P-cores, 1/17 of GET, with 43% of its time acquiring Meta. Without a file nothing a write records in Meta is read, so those statements now hold their entries alone; on 8 P-cores SET reached 21.4 M/s, 99.6% of GET, and SET and MSET grew 1.74 and 1.73 times from 4 to 8 cores. INFO omits the commit sequence while AOF is off because most writes no longer advance it.

## 2026-10-12 Bound each AOF rewrite scan step by the reserve and pace it by time slice

Nodes: firn/aof-rewrite/capture

Owner-approved: On the shared status board on 2026-10-12, the owner chose option A (approve) on card firn-q-approve-57-tree, which asked to approve PR #57's design-tree changes at head 27e7c34, carrying out the owner's earlier choice of option A on card firn-q-rw-pace; the ruling adds that the merge waits for a formal Whitefoot release fixing the 2-core MSET regression and a passing before-and-after comparison on the 14900K.

Summary: Each live scan step calls map_scan_within with the reserve left after the step buffer as its limit and charges the growth key_set_bytes reports, so a step never exceeds R; copies and buffer growth are charged before allocation, and a one-position step that does not fit is discarded, waits outside every hold and retries while it could fit, or aborts with reserve when it never can. The scan runs steps back to back and yields about every millisecond, with a per-step key hint starting at 64 that halves or doubles by the bytes a step writes and falls back to one position when several do not fit, because a 100-microsecond sleep per one-key step cost about a millisecond per key and a million-key rewrite could not finish under write load. CONFIG answers Redis 7.0.15's hidden rdb-key-save-delay so cases can keep a rewrite in progress without a firn-only hook. The post-call charging limitation and its rejection entry are replaced by three rejected alternatives ([scan pacing](../research/investigations/aof-rewrite/scan-pace.md#design)).

## 2026-10-13 Recover from an AOF sync failure only through a durable rewrite, and sync only after new bytes

Nodes: firn/append-only-file, firn/aof-rewrite/capture

Owner-approved: On the shared status board on 2026-10-13, the owner chose option A on card firn-q-sync-recover (after a failed sync, a retried sync that succeeds starts a rewrite of the AOF from memory, and only a durable installation clears the error and admits writes again, as implemented on PR #68 at head c183857) and option A on card firn-q-sync-schedule (sync once a second only after new bytes or while a sync error is saved, as Redis 7.2.16 does, instead of once a second while idle).

Summary: Linux marks pages whose writeback failed clean, so a retried fsync succeeds without writing them; on the faulty-device job a write acknowledged during the failure and one acknowledged after firn's automatic recovery were both lost after a restart while firn loaded the file silently. firn now keeps refusing writes until a rewrite from memory is durably installed; recovery rewrites run whatever the automatic-rewrite block, and append errors keep the clear-on-complete-append rule. Syncing only after new bytes makes firn acknowledge a write after an idle-time device fault as Redis does, instead of refusing it because an idle sync failed first.

## 2026-10-12 Take Redis 7.2.16 as the gate's suite and the reference server

Nodes: firn, firn/consumers

Owner-approved: On the shared status board on 2026-10-12, the owner chose option A (approve) on card firn-q-approve-64-tree, which asked to approve PR #64's four design-tree changes at head fb94ca4 (the tree is unchanged at d46597e): the gate's suite moves to the reference Redis 7.2.16; the recorder skips known hangs and reruns their units unskipped, a hang leaving the list only when it passes; every use of the reference server builds it from the pinned 7.2.16 release archive checked by SHA-256, with the system package, Redis's apt repository and the official container image rejected; the application comparison runs against 7.2.16 with AOF on both sides.

Summary: firn's reference moved from Redis 7.0.15 to 7.2.16 as the owner ruled on the reference-version card, and the tree now says so where the gate, the recorder, the reference build and the application comparison depend on it. The gate keeps its per-test ratchet over 7.2.16's suite. The recorder treats an error or timeout of a known hang, run unskipped, as no evidence it stopped hanging, since a hung test leaves its connection in a state such as an open MULTI that makes later tests fail at once. The reference server is built from the release archive because Ubuntu 24.04 ships 7.0.15 and an unpinned package or image can change between runs.

## 2026-10-12 Keep a raised script error's reply on one line

Nodes: firn/scripts

Owner-approved: On the shared status board on 2026-10-12, the owner chose option A (approve) on card firn-q-script-error-crlf-tree, which asked to approve PR #62's design-tree change at head 14841e0: a raised script error written as Redis 7.2.13 and later write it, every carriage return and line feed as a space, with writing it raw as Redis 7.0.15 does rejected.

Summary: A script that raises an error whose text holds CR or LF no longer splits its reply into replies the client reads as answers to its next commands. firn writes a raised error as Redis 7.2.13 and later do (addReplyErrorSdsExSafe), the reference Redis 7.2.16 included; 7.0.15 wrote the raw bytes, a defect Redis fixed as a security issue. Returned error tables and compile errors were already one line in both versions; firn's compile-error path, which wrote Halo's message raw, now matches them.

## 2026-10-12 Validate the first release on the usage survey's applications

Nodes: firn/consumers

Owner-approved: On the shared status board on 2026-10-12, the owner chose option A on card firn-q-production-scope, which asked whether firn's first release takes the scope the usage survey proposes; the ruling reads (translated) "The first release takes the survey's proposed scope ... Validation applications are added in the order the survey gives."

Summary: The owner set firn's first release to production drop-in use. The consumers node keeps the deployment milestone's three consumers and adds, after them and in the survey's order, Sidekiq, Celery, BullMQ, the Socket.IO and ActionCable pub/sub applications, Redisson's default lock and a smoke test of twelve mainstream clients, each run unchanged against Redis 7.2.16 and firn with the append-only file on. The first release's command scope is the survey's approved list, consumers' refusals still naming work. The rejection of BullMQ and Sidekiq as unselected scenarios is removed, since the ruling supersedes it ([survey](../research/investigations/usage-survey/README.md#validation-applications-and-clients)).

## 2026-10-12 Raise the heap peak only past a connection's last seen peak

Nodes: firn/memory-limit

Owner-approved: On the shared status board on 2026-10-12, the owner chose option A (approve) on card firn-q-approve-59-tree, which asked to approve PR #59's design-tree changes at head 550e4a0.

Summary: After each command a connection takes Meta to raise the recorded heap peak only when its heap reading exceeds the peak it last saw recorded, because the peak only rises, so a lower reading cannot change it, while a Meta statement after every command on every connection serialized all cores: on 8 P-cores of the i9-14900K depth-16 GET ran at 2.98M/s with it and 19.2M/s with the per-connection check. Recording once per read batch was not adopted because a later pipelined command can free what an earlier one allocated and hide a peak Redis records; the per-command Meta statement is retired as a rejected alternative ([peak recording](../research/investigations/memory-limit/README.md#peak-recording-across-cores)).

## 2026-10-12 Refuse writes while the append-only file cannot be written or synced

Nodes: firn/append-only-file, firn/orderly-stop, firn/scripts, firn/transactions

Owner-approved: On the shared status board on 2026-10-11, the owner chose option A (approve) on card firn-q-approve-55-tree, which asked to approve PR #55's design-tree changes at head 9a584fe; on 2026-10-12 the owner chose option A (approve) on card firn-q-approve-55-tree-2, which asked to approve the two decisions changed since then at head c06b0b4 (the MISCONF suffix and the 100 ms append retry). On 2026-10-12 the owner also chose option C on card firn-q-sync-fail-evidence: merge with the sync-failure path checked against Redis 7.0.15's source, and add a runtime sync-failure case before the Whitefoot release as a separate item.

Summary: The sole AOF writer publishes separate append and sync errors in Meta. WRITE commands and PING are refused with Redis's MISCONF reply while either error stands, checked at admission and again under the mutation's Meta hold; reads, expiry and eviction continue. The reply's suffix is the host's description of the saved error, taken from Whitefoot's std::io::error_copy_description, choosing the append error when nothing of the failed append was written and otherwise the sync error, as Redis 7.0.15 does. A partly written append is suffixed with no text until Whitefoot can describe ENOSPC without a platform number. An incomplete append is truncated back, or its written prefix accounted, so only the bytes still owed are retried, every 100 ms whether or not clients write, which is Redis's idle cadence. A rewrite's new incremental file keeps the error state. Scripts check before their first accepted write, MULTI and EXEC follow Redis's EXECABORT ordering, and shutdown makes one final append and sync attempt and then exits with status 0 even on failure. firn keeps its single everysec policy ([investigation](../research/investigations/aof-write-errors/README.md)).

## 2026-10-11 Scripts hold only their declared keys

Nodes: firn/aof-rewrite, firn/aof-rewrite/capture, firn/command-parts, firn/scripts

Owner-approved: On the shared status board on 2026-10-11, the owner chose option A (approve) on card firn-q-approve-47-tree, which asked to approve PR #47's design-tree changes at head 7e2fa43, carrying out the owner's earlier choice of option A on card firn-q-script-engine.

Summary: A script first holds the entries of its declared KEYS with Meta; an undeclared key or a whole-map command abandons the attempt before it touches any entry, restores each changed entry from a logical copy taken before its first change, and reruns the script holding the whole keyspace, because declared entries keep isolation while Redis 7.0.15 allows undeclared keys outside cluster mode. Restore is logical rather than layout-exact because firn offers no collection cursor or layout-dependent result. Meta stays held for the whole attempt, so scripts still run one at a time; one shared engine remains until the engine pool, sized by the Whitefoot driver count, lands. Script commands map their operands onto declared ranks through the shared command bodies. Scripts keep complete rewrite images even for one declared key; single-key byte capture stays limited to ordinary commands. The old whole-keyspace and single-engine decisions and the rejection of engine pools are retired ([investigation](../research/investigations/script-engines/README.md)).
## 2026-10-11 Keep the AOF rewrite journal on disk

Nodes: firn/aof-rewrite, firn/aof-rewrite/capture

Owner-approved: On the shared status board on 2026-10-11, the owner chose option A (approve) on card firn-q-approve-54-tree, which asked to approve PR #54's design-tree changes at head 3622c21, carrying out the owner's earlier choice of option A on card firn-q-rw-pace.

Summary: Statements still publish framed after-images or effects under Meta, but into bounded 64 KiB staging chunks charged to the reserve; the sole AOF writer drains them outside Meta to temp-F.journal and the rewrite reads the file back through one charged buffer after the cut, because a journal held in memory approached a second copy of the dataset under uniform writes and aborted every rewrite. A full staging queue aborts the rewrite with journal_backlog without blocking client writes. Startup serves even when a leftover journal cannot be removed, as Redis 7.0.15 ignores temporary-file unlink failures, and INFO reports cleanup failures and the leftover separately from the first abort cause. On the 14900K the rewrite no longer aborted and the reserve peaked at 8.3 MB of 16 MiB ([investigation](../research/investigations/aof-rewrite/journal-disk.md)).

## 2026-10-11 Report why an AOF rewrite stopped

Nodes: firn/aof-rewrite/capture

Owner-approved: On the shared status board on 2026-10-11, the owner chose option A (approve) on card firn-q-approve-51-tree, which asked to approve PR #51's design-tree change at head 7bd7c92: the new decision in design/firn/aof-rewrite/capture.md on rewrite diagnostics.

Summary: INFO reports the first cause that stopped a rewrite, latched under Meta before cleanup, with the reserve's use and peak, the journal's image and effect counts and the scan's progress, because an undifferentiated error cannot tell which resource or I/O condition stopped an attempt; these facts showed on the 14900K that rewrites aborted because the scan was slow and the journal grew in memory ([investigation](../research/investigations/aof-rewrite/README.md)).

## 2026-10-10 AOF rewrite from a reconciled live scan and an effect journal

Nodes: firn/aof-rewrite, firn/aof-rewrite/capture, firn/aof-rewrite/sequence

Owner-approved: On the shared status board on 2026-10-10, written in Chinese, the owner approved, with the board's approve answer ("agree"), the request on the item for the reconciled chunked export, which asked to approve PR #42's design nodes: the changes to design/firn/aof-rewrite.md and the new aof-rewrite/capture.md and aof-rewrite/sequence.md; on the same board the owner chose option A on cards firn-q-rw-cut, firn-q-rw-stamp, firn-q-rw-limits and firn-q-rw-startup-multi, and option C on card firn-q-rw-bigkey.

Summary: The rewrite no longer replays the closed log into a private keyspace, a second copy of the dataset. It scans the live map in bounded steps after S0 while every mutating statement takes a sequence number and stamps the entries it changes; a key's first write in the window journals its complete after-image and, for a statement holding only that key, later writes journal the statement's own append-only bytes; the sole writer rotates once at S1, cutting the sealed file before a startup-retained unfinished MULTI, and the base is the scan followed by the journal in sequence order. The rewrite's memory is a reserve of 5% of maxmemory (16 MiB without one) counted in process memory; exhausting it aborts the attempt with the old files authoritative and stops automatic rewrites until a manual one succeeds; FLUSHALL and FLUSHDB abort a scan. Evidence and reviews are in research/investigations/aof-rewrite/scan-log.md.

## 2026-10-10 Benchmark servers run on distinct physical performance cores

Nodes: firn/measurement-sessions

Owner-approved: On the shared status board on 2026-10-10, written in Chinese, the owner approved the item request on "the measurement script binds servers and clients to different physical performance cores" that asked to approve PR #43's placement rule in design node measurement-sessions (servers on distinct physical performance cores skipping core 0, clients on both threads of the remaining performance cores and never on a server's core, overridable lists), with "agree" (translated).

Summary: On the native i9-14900K adjacent logical CPUs are hyperthread siblings and CPUs 16-31 are efficiency cores, so the old placement ran a two-CPU server on one physical core and put client threads on efficiency cores. Placement now reads the Linux topology and the hybrid core lists, puts each server CPU on its own performance core, keeps core 0 for the OS and the runner, gives clients the remaining performance cores' threads, and records the chosen lists with every run.

## 2026-10-10 firn bounds memory as Redis's maxmemory does

Nodes: firn/memory-limit, firn/command-parts, firn/scripts, firn/transactions

Owner-approved: On the shared status board, written in Chinese, the owner chose option A on cards firn-maxmemory-policies, firn-access-stamp, firn-script-retry-access, firn-policy-read, firn-keyset-invariant, firn-evict-admission, firn-evict-sampling, firn-evict-config, firn-aof-exclusion and firn-stamp-lock, and on 2026-10-10 approved the item request on "memory accounting, maxmemory and eviction" asking to approve PR #35's design-tree changes not decided by a separate card (GET's stamp refresh in the entry's own statement, giving back the map reserve before evicting, MULTI OOM refusal and RESETSTAT, the volatile absence-scan invalidation, shared command identity and flags, script OOM before the first accepted write, explicit shebang refusal, and Redis's admission order) and to merge with the known limitation that an append-only rewrite exceeds the limit, with "agree" (translated).

Summary: firn implements all eight Redis 7.0.15 maxmemory policies. Every entry carries Redis's LRU clock or LFU stamp, refreshed on every lookup Redis refreshes; each command's own context checks the limit after Redis's earlier admission checks, first gives back the keyspace map's reserve, and evicts by sampling a random map_scan cursor into one shared pool of 16, revalidating, deleting, logging DEL and counting each victim in one statement; OOM refusal follows Redis inside MULTI, EXEC and scripts. used_memory is Whitefoot's heap_in_use less both pending append-only buffers. Rejected alternatives (relaxed stamp fields, a second stamp map, a dedicated evictor, serializing commands under a limit) and the i9-14900K results are in design/firn/memory-limit.md and research/investigations/memory-limit/README.md. The append-only rewrite still builds a private copy counted by the meter; the owner ruled on board card firn-maxmem-scope that the rewrite must stop doing so, tracked as firn-aofrw-meter.

## 2026-10-10 Shutdown and idle-limit changes end waits through cancellation

Nodes: firn/orderly-stop

Owner-approved: On the shared status board on 2026-10-10, written in Chinese, the owner approved the item request on "firn's stop moves to cross-context cancellation, removing the polling" that asked to approve PR #40's changes to design node orderly-stop (a shutdown request fires the wake generation and a separate shutdown cancellation; server state holds a replaceable cancellation source, and CONFIG SET timeout fires the old generation outside the atomic statement after replacing it; the signal context waits without a deadline; propagating shutdown only by polling and waking every second to observe configuration changes rejected) with "agree" (translated), after choosing option A on the board card firn-adopt-timeout-wake.

Summary: A shutdown request fires a shutdown cancellation that main, the signal context and the expiry context watch, and the current receive wake generation, so every wait ends without a polling deadline; a CONFIG SET timeout that changes the idle limit installs a fresh generation in the statement that changes it and fires the old one after it, because firing waits, so parked clients see a new limit at once and clients with no limit wait without a timer. The signal context waits without a deadline. Polling deadlines were rejected because they cannot end a wait without a deadline, arm a timer per parked receive and delay exit; on the i9-14900K the change showed no loss at the stated 1% rule, and main's polling failed the new 500 ms stop bound at 1.103 s (research/investigations/orderly-stop/README.md).

## 2026-10-09 Design nodes cite status-board keys instead of docs/todo.md

Nodes: firn/aof-rewrite, firn/orderly-stop, firn/scripts

Owner-approved: The owner decided on 2026-10-09, relayed by the coordinator session in Chinese and recorded as board item coord-todo-firn, that the shared status board is each project's only TODO, that repositories delete docs/todo.md, and that code, documents and checks citing unfinished work cite the board item's key (translated).

Summary: The three nodes' references to docs/todo.md entries now cite the board items that hold them (firn-bl-01-29, firn-bl-03-00, firn-wf-excl-create, firn-cancel, firn-bl-01-00). No decision, reason or rejected alternative changed.

## 2026-10-08 EVALSHA reuses the engine's compiled script; SCRIPT FLUSH waits for running scripts

Nodes: firn/scripts

Owner-approved: On the shared status board on 2026-10-08, written in Chinese, the owner ruled the decision card firn-evalsha-flush-race, which asked what firn does when another connection's SCRIPT FLUSH meets an EVALSHA in progress and presented option C, recommended, as SCRIPT FLUSH waiting until no script runs together with re-reading the registry on every cache miss, with "C: SCRIPT FLUSH takes effect once no script is running" (translated).

Summary: EVALSHA still answers NOSCRIPT from the registry before it takes the engine, but an attempt whose engine has already compiled the script copies nothing, and an attempt whose engine has not reads the registry again and copies the source from that read, because copying the source on every call took 2.8 to 3.8% of firn's CPU under rate-limiter-flexible's script ([profile](../research/investigations/deployment-performance/README.md#result)). SCRIPT FLUSH waits until no script is in progress, as Redis 7.0.15 does below its busy threshold, so it never interrupts a running script's later attempts; above the threshold Redis answers BUSY, which firn adds with its busy-script replies ([TODO](../docs/todo.md#server)).

## 2026-10-08 SIGTERM and SIGINT stop firn in order

Nodes: firn/orderly-stop

Owner-approved: On the shared status board of 2026-10-08, written in Chinese, after the card "firn's way of taking stop signals, and how to handle the Whitefoot gap" (translated), which recommended option A, approving the two decisions, merging Firn-wf #33 and recording a context ending the program with a status as a Whitefoot gap, the owner chose A.

Summary: SIGTERM and SIGINT request the orderly stop SHUTDOWN requests, as Redis 7.0.15 shuts down gracefully on either under its defaults, appending and syncing the append-only file. One stop listener, opened just before firn listens, is waited on by a context of its own that polls the shutdown request every tenth of a second, because nothing else can end its wait when firn stops for another reason; a host that refuses the listener stops firn with status 3. The context closes the listener on the first signal, restoring the host default, so a second signal ends firn at once, which keeps an escape from a stop that does not finish; a second SIGTERM thus ends firn where Redis ignores it, and a signal during the replay ends it where Redis exits with status 0. Matching those needs a Whitefoot program exit with a chosen status from one context, recorded under Whitefoot requirements in docs/todo.md.

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
