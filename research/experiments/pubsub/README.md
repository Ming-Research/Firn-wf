# Pub/sub implementation and reference batches

This record belongs to board item `firn-scope-pubsub`, steps 4 and 5 of
[connection waits](../../investigations/firn/connection-waits.md#independently-reviewable-implementation-order).
The branch implements the shared connection lifecycle and direct, pattern
and shard pub/sub. Compilation, the project gate, network behavior and
performance remain unverified: the owner drives every build and test in CI.
No local compiler, suite, Redis server or benchmark was run.

The implementation starts from worktree revision
`84e755290c66aa81272af5e48a49c1d63c618ed0`, the connection-waits study on
local main's `c6f6058fb17277d4d44d364a45efad1eae01a22b`. The existing local
origin/main ref is newer (`eacae888b67c1a7250fd40854052dd68962771ae`) and uses
`wf-af7665be672d`; this worktree retains its original `wf-d75f318d5631` pin.
No fetch, rebase, dependency adoption or commit was performed. The review and
all source-level acceptance statements below apply to this worktree, not to
the newer main tree; combining with it needs CI at the resulting revision.

The owner selected growing hash maps and monotonically allocated u64
registration ids that are never reused within a process. Their supported
lifetime contains fewer than 2^64 allocations, as the interface documents.
The former slab-reservation concern was filed separately as Whitefoot request
`firn-req-slab-reserve` at P2; it does not block this representation. The
standalone capacity witness was removed, and the finding remains in
[the research note](../../investigations/firn/connection-waits.md#registration-identities).

## Changes by file

Paths below are relative to the repository root. All source behavior is a
draft pending compilation and owner-driven CI.

| File | Change |
|---|---|
| `firn/modules.wfg` | Registers the event module and its affected dependencies. |
| `firn/connection_events/module.wfm` | Defines hub, frame, stage and connection interfaces, lifetime bound, count snapshots and authoritative lifecycle checks. |
| `firn/connection_events/owners.wf` | Growing identities, independent queues, ordered dispatcher, cancellation, pending capacity accounting, soft deadlines and cleanup. |
| `firn/connection_events/registry.wf` | Three membership indexes, reverse names, duplicate handling and network-client-id index. |
| `firn/connection_events/commands.wf` | Direct, pattern and shard semantics plus PUBSUB introspection. |
| `firn/connection_events/frames.wf` | Subscription replies, shared immutable message bodies and ordered reply/publication commit. |
| `firn/protocol/module.wfm` | Adds registration/count fields and shared push/frame writer interfaces. |
| `firn/protocol/protocol.wf` | Initializes the added client fields. |
| `firn/protocol/frame.wf` | Shared logical-prefix writers and push headers. |
| `firn/store/module.wfm` | Adds the retained hub to Keyspace's setup representation. |
| `firn/store/store.wf` | Constructs and shares that hub at setup, without new GET/SET atomic targets. |
| `firn/commands/pubsub.wf` | Command identities, canonical admission errors, RESP2 subscriber restrictions and hub dispatch. |
| `firn/commands/admission.wf` | Reference arities and DENYOOM classification. |
| `firn/commands/dispatch.wf` | Direct pub/sub command dispatch. |
| `firn/commands/strings.wf` | RESP2 subscriber PING array. |
| `firn/commands/connection.wf` | Real CLIENT INFO membership counts and flag. |
| `firn/commands/info.wf` | Real distinct registry counts in INFO stats. |
| `firn/commands/transaction.wf` | Subscriber-mode admission and quiet RESET cleanup; conditional EXEC hub hold, complete outer reply and staged publications. |
| `firn/commands/script.wf` | Script command identification, NOSCRIPT/arity ordering, widening and held publication callback. |
| `firn/scripting/module.wfm` | Carries held hub and private publication stage through ScriptCommands. |
| `firn/scripting/entry.wf` | Whole-map hub hold, actual-delivery continuation and final attempt commit. |
| `firn/server/module.wfm` | Documents subscriber idle exemption. |
| `firn/server/server.wf` | One-time phase handoff; starts and terminates hub contexts. |
| `firn/server/promoted.wf` | Independent socket owners, cancellation forwarding, parser handoff, live admission and ordered finish. |
| `firn/bytes/bytes.wf` | Ports Redis's bounded recursive glob matching behind the existing interface. |
| `firn/bytes/redis-license.txt` | Redis source's BSD notice beside the port. |
| `tests/network.rs` | Registers the new pub/sub network module in the existing gate. |
| `tests/pubsub.rs` | Moves the former acceptance witnesses into normal network tests and expands coverage. |
| `firn/README.md` | Documents commands, framing, lifetime and actual accounting limits. |
| `research/investigations/firn/connection-waits.md` | Records the chosen growing identity representation and separate slab request. |
| `research/experiments/pubsub/README.md` | This implementation, acceptance, oracle and limitation record. |
| `research/experiments/pubsub/oracle-batches.json` | Single-connection recorder batches; temporary copy in `$TMPDIR/pubsub-batches.json`. |
| `design/firn/connection-ownership.md` | Proposed promotion, ordering, accounting and cancellation decisions. |
| `design/firn/pubsub-registries.md` | Proposed growing dictionary and separate registry decisions; explicit slab rejection. |
| `design/firn/script-attempt-external-effects.md` | Proposed widening and accepted external-effect publication decisions. |
| `design/firn/transactions.md` | Qualifies EXEC's hub hold and publication boundary. |
| `design/firn/scripts.md` | Qualifies budget continuation and whole-map hub lifetime. |
| `design/firn/orderly-stop.md` | Distinguishes promoted cancellable output from the retained ordinary send path. |

## Implementation boundaries

A first successful subscription creates a connection record and commits the
existing reply prefix and subscription acknowledgment under the same hub
hold. The original server loops then end and transfer both TCP halves,
client/parser buffers and transaction state once. The receiver owns input
and EOF observation, the command owner runs one command at a time, and the
output owner alone sends complete frames, retaining its partial-send offset.
An independent hub dispatcher publishes committed actions to retained
inboxes in order and performs no network I/O. Publication bodies use frozen
shared storage; recipient headers remain separate. Disconnect, send failure,
query overflow and pressure cleanup remove all memberships and cancel every
owner. An ordered finish marker allows QUIT and protocol errors to drain
preceding replies before cleanup.

Query bytes include both queued chunks and the command owner's retained
incomplete input until consumption is reported; a receiver closes above one
GiB rather than waiting for private queue space and stranding EOF. Active
subscriptions exempt the connection from the idle timeout. Membership,
configuration and completed I/O activity changes wake receives directly. Final
idle expiry checks authoritative hub membership and latest activity together
with registration removal; a delayed dispatcher notification cannot expire
an already subscribed client. Each parsed promoted command also checks its
live registration and cancellation together before execution, so queued
pressure cancellation cannot admit a later command on an invalidated id.
An admitted command may finish if removal happens after that boundary.
A guarded forwarding context combines those cancellation generations with
connection shutdown into the single watch accepted by the host receive;
its owner disarms it before joining after every receive.

A never-subscribed GET/SET still receives, executes and sends in the original
context. Its dataset atomic statements retain their existing key and optional
Meta targets. Added work is local command classification/admission, a local
promotion check, a larger client record and one retained hub handle created
at connection setup; there is no hub target or queue transfer on these
commands. RESP3 subscribers' ordinary commands use committed queued output
and pay the promoted-path cost. No unused-path timing result is claimed.

Channel and pattern registries are server-wide and separate from shard
channels. Duplicate membership acknowledges without adding another path.
NUMSUB/SHARDNUMSUB preserve operand order and duplicates; NUMPAT counts unique
patterns. RESP2 subscriber admission permits the six subscription commands,
PING, QUIT and RESET, after the reference's earlier admission checks. RESP3
allows ordinary commands. Shared protocol writers own framing, including
push headers; the event module owns membership and delivery, not sockets.

The shared glob matcher now follows Redis 7.2.16's recursive algorithm,
including signed-char range handling, malformed classes, empty-name behavior,
shared suffix-search pruning and refusal above depth 1000. Its existing
public interface remains unchanged. This also corrects the former greedy
matcher's missing reference depth refusal for other glob consumers. The
source's BSD notice is retained beside the port. Stack-ledger evidence is
still required for the maximum 1002 active calls including a rejected call.

## Script attempts

A declared-entry attempt encountering PUBLISH, SPUBLISH or PUBSUB introspection
abandons through the existing uncatchable whole-map retry before observing
membership. Its existing journal restores preceding writes and access stamps.
Every whole-map attempt holds the hub from its start together with keys and
Meta, including nonpublishing whole-map scripts, because those attempts have
no general rollback boundary for a later wider hold. This adds possible hub
contention only on that broad path; its cost is unmeasured.

Publication counts and recipients use the held membership snapshot. Frames
remain in an attempt-local stage until the final Lua result or error reply
is formed; accepted errors retain preceding publications. Actual staged
recipient deliveries prevent interpreter-budget abandonment; a zero-recipient
publication has no external delivery to duplicate and can retry. Publications
add no AOF dataset records and do not set script WRITE_DIRTY. The complete
outer script or EXEC reply commits before self-publication frames. Script
callbacks retain the real subscriber protocol in the hub while using Lua's
selected response protocol, and ordinary callbacks temporarily clear the
network client's membership counts to model Redis's unsubscribed script
client for PING.

The existing busy-script limitation remains: once an attempt must continue
without releasing its holds, SCRIPT KILL cannot observe it through the pool
between attempts. This branch does not introduce an in-attempt publication
channel to solve the already-recorded Whitefoot requirement. Its expanded
external-effect path needs that limitation included in acceptance reporting.
A minimal three-session example is A: `SUBSCRIBE news`; B: `EVAL
"redis.call('PUBLISH','news','once'); while true do end" 0`; C: `SCRIPT KILL`.
Redis's dataset-unwritten publisher remains killable after its busy boundary;
Firn's selected delivery prevents budget abandonment, so it cannot reach its
between-attempt kill check. This part remains dependent on the existing
Whitefoot request `firn-req-attempt-publish`, not claimed as reference-complete.
A zero-recipient publication remains retryable. No new compiler diagnosis was
produced on this machine.

## Limits and remaining evidence

Active pub/sub connections enforce the reference numeric defaults: hard at
at least 33,554,432 charged bytes and soft at at least 8,388,608 charged bytes
continuously for more than 60 observed Unix seconds. Falling below the soft
threshold, or leaving subscriber mode, clears the interval. A deadline queue
and cancellable timer generation drive closure without periodic polling.
Each frame charges its prefix and body reserved byte capacities until the
whole frame owner is released, including hub, dispatcher, inbox and partially
sent ownership. A shared body is charged in full to each recipient's limit;
physical process heap accounting retains one allocation while references
share it.

This accounting excludes allocator rounding, reference-control and queue
storage overhead, whereas Redis includes dynamic reply-allocation and
block/list overhead. Exact allocation values and the publication number at
which a slow client closes therefore differ; no allocator-equivalence claim
is made. The numerical thresholds, interval reset and independent output
progress still require CI observations. The slow-client network case checks
progress only. A controlled accounting witness must hold a backlog above the
soft threshold without reaching the hard threshold, then exercise a drop
below and re-entry. Compare the reference and Firn without assuming identical
kernel socket buffering.

The timer reads Unix time and arms a monotonic sleep for the next observed
threshold. Strict response to a forward calendar jump without another hub
wake retains the research note's missing host-notification limitation; no
periodic poll was introduced. Cancellation/partial-send races, old-id retained
envelopes after cleanup, query overflow and idle exemption also need runtime
evidence. No compiler was invoked, so no new compiler rejection or diagnosed language
gap was observed. The existing host calendar-jump notification and script
attempt-publication requirements remain open; this branch does not hide them.

The client-handshake branch was read without adopting it. CLIENT INFO now
shows the local sub/psub/ssub counts. The hub exports a nonwaiting
`client_counts` observation keyed by the network client id; CLIENT LIST can
hold its existing table and the hub together and obtain consistent counts
and subscriber classification. This avoids delayed client-table count
publication and adds no table/hub target to ordinary GET/SET. The other
branch's table formatting still needs that integration when the branches
are combined.

## Network acceptance cases

The former experiment cases now live in `tests/pubsub.rs`, registered by
`tests/network.rs` and included in the existing network CI job. No old test
or ratchet entry was removed or narrowed. There are 22 registered pub/sub cases;
none has yet been executed on either this revision or the old server. Each
listed guard names the incorrect behavior that would fail its assertion. Expectations derive from Redis
7.2.16 sources and tests, not Firn's previous output.

| Case | Failure it detects |
|---|---|
| `binary_channels_and_messages_preserve_every_byte` | NUL/high-byte payload corruption or wrong RESP2/RESP3 frame type. |
| `promotion_keeps_fragmented_pipeline_suffix` | Losing an incomplete request suffix at promotion. |
| `queued_classic_subscription_promotes_during_exec` | Failing first promotion or reply-prefix handoff inside EXEC. |
| `direct_channels_multiple_subscribers_and_introspection` | Duplicate delivery, missing unsolicited output, wrong counts or cleanup. |
| `resp2_subscriber_restrictions_error_precedence_ping_and_reset` | Wrong allowed commands, canonical error text/precedence, PING shape or RESET. |
| `promotion_preserves_pipeline_prefix_suffix_and_unsubscribe_all` | Lost earlier replies or complete pipeline suffix; missing null acknowledgment. |
| `unsubscribe_all_returns_every_membership_and_empty_acknowledgment` | Missing reverse cleanup, count progression or repeated empty reply. |
| `pubsub_admission_errors_abort_transaction_without_publication` | Executing a dirty transaction, unknown subcommand or wrong-arity admission. |
| `resp3_self_publication_follows_complete_command_transaction_and_script_reply` | Pushes splitting outer replies; wrong ordinary/script PING or PUBSUB query. |
| `script_retry_stages_publications_and_final_error_keeps_them` | Abandoned-attempt duplicates, lost pre-error effects or subscription allowed in scripts. |
| `script_hub_restart_rollback_whole_attempt_and_budget_preserve_once` | Double INCR before widening, unsafe late whole-map restart, budget replay or zero-recipient noncompletion. |
| `every_subscription_family_is_noscript_after_arity_validation` | Allowing any subscription family from Lua or reversing error precedence. |
| `disconnect_cleanup_and_new_connection_do_not_keep_old_membership` | Lingering EOF membership, stale output to another connection or failed QUIT closure. |
| `active_subscription_families_exempt_idle_timeout_then_last_unsubscribe_closes` | Wrong idle exemption or failure to restore normal timeout for either protocol and all three membership families. |
| `promoted_quit_and_protocol_error_drain_pipeline_and_leave_other_subscriber_live` | Lost earlier push/reply, executing commands after QUIT/protocol error, or cancellation crossing connections. |
| `slow_subscriber_does_not_delay_other_subscribers` | One unread socket stalling publication or a healthy output owner on one driver. |
| `info_stats_counts_unique_pubsub_registries_and_last_membership_cleanup` | Hardcoded zero or membership rather than distinct-name INFO counts, swapped registries, or incorrect last-member cleanup in RESP2/RESP3. |
| `patterns_count_each_delivery_and_unique_registry_patterns` | Counting clients instead of paths, duplicate patterns or nonunique NUMPAT. |
| `patterns_and_channel_filter_use_redis_stringmatchlen` | Case, class, escaping, malformed-pattern or empty-name mismatches. |
| `pattern_recursion_depth_matches_redis_boundary` | Missing depth-1000/1001 refusal or counting a final trailing star as recursion. |
| `shard_channels_are_separate_with_independent_counts_and_mode_lifetime` | Mixed namespaces/counts, wrong smessage or premature subscriber-mode exit. |
| `shard_subscribe_inside_exec_refuses_and_shard_publication_composes` | SSUBSCRIBE accepted in EXEC; SPUBLISH refused in EXEC/Lua; misplaced self-push. |

Start owner-driven CI with the smallest direct-channel case after the normal
compiler/toolchain/build setup and record its duration before scaling to the
complete filter. The existing gate runs all registered cases without staging
or copying a research fixture. No compile/run result is recorded yet.

## Recorder input and ORACLE placeholders

[oracle-batches.json](oracle-batches.json) retains the 12 original recorder
batches, also written to `$TMPDIR/pubsub-batches.json`. Each object has exactly
`name`, `resp`, and `commands`. Use a fresh connection per object; for RESP3,
negotiate HELLO 3 before its batch and record negotiation separately. Capture
all raw bytes, including pushes and multiple acknowledgments. Send promotion
batches as a single pipeline.

Three expected raw transcripts remain **ORACLE**:

| Batch | Commands |
|---|---|
| `ORACLE-step4-multichannel-subscription-exec-resp2` | RESP2: MULTI; SUBSCRIBE first second; PING tail; EXEC; UNSUBSCRIBE. |
| `ORACLE-step4-multichannel-subscription-exec-resp3` | RESP3: MULTI; SUBSCRIBE first second; PING tail; EXEC; UNSUBSCRIBE. |
| `ORACLE-step4-unsubscribe-all-exec-resp3` | RESP3: SUBSCRIBE first second; MULTI; UNSUBSCRIBE; PING tail; EXEC. |

The implementation writes each acknowledgment in the reference command's
position, including multiple acknowledgments inside EXEC; it does not repair
an irregular aggregate. The reference's own tests document that historical
framing. Unsubscribe-all dictionary order must be compared as membership
names and count progression, not fixed cross-implementation ordering. The
other nine batches independently confirm source-derived expectations.

## Multi-connection recorder scenarios

Use fresh connections and a fresh standalone server. Wait for each specified
acknowledgment/reply before its next step. Repeat subscriber scenarios with
RESP2 and RESP3; controllers use RESP2 unless stated otherwise.

1. A subscribes to `news news` and reads two count-1 acknowledgments; B joins
   `news` once. C checks NUMSUB news absent news = 2,0,2, publishes hello with
   count 2, and A/B each read one message. Both unsubscribe twice and read two
   count-0 acknowledgments; C's next publication returns zero.
2. A joins news. B runs the EVAL strings in the script-retry case, then
   publishes barrier. A must read once, accepted, barrier in that order with
   no duplicate. The accepted Lua error must precede B's next reply. Repeat
   with declared INCR-before-publication and DBSIZE-before-INCR to distinguish
   the two attempt boundaries; both counters finish at 1.
3. A joins news and closes both TCP directions. C waits, bounded by five
   seconds, until NUMSUB is zero, then publishes stale with count zero. B
   connects afterward, joins news and receives only C's fresh publication.
   This black-box case does not force a paused in-flight envelope; a retained
   old-inbox cleanup witness is still required for that race.
4. A joins news directly and pattern n* twice; B joins n*. C observes NUMPAT
   1 and publish count 3. A receives direct then pattern, B one pattern.
   PUNSUBSCRIBE leaves A's direct count 1, B's count 0, and a repeated removal
   replies with a null pattern. Use the separate glob/depth pairs exactly as
   written in the cases, removing each pattern before the next pair.
5. A joins ordinary news and shard news twice. C checks both counts and
   publishes global then shard, each count 1. A receives message then smessage.
   Removing ordinary membership leaves RESP2 subscriber PING until shard
   removal; empty SUNSUBSCRIBE still acknowledges.
6. A/B join large; stop reading A after its acknowledgment. C publishes forty
   one-MiB x payloads, reading each count (1 or 2) and B's complete frame before
   the next send. Each operation has the harness's ten-second timeout. This
   workload checks independent progress only, not a particular buffer cutoff.
7. Use a RESP3 controller subscribed to __timeout_control__, so timeout cannot
   expire it. A uses each protocol and each of SUBSCRIBE/PSUBSCRIBE/SSUBSCRIBE
   for news. Set timeout 1, send nothing for 2.2 seconds, then publish news;
   count is 1 and A receives the appropriate message. A's last unsubscribe
   acknowledges zero; its bounded five-second read then reaches EOF. Controller
   PING remains PONG. Repeat each family on a fresh server.
8. A is RESP3 and B is RESP2, both subscribed to news; C publishes before-close.
   Read B's message but leave A's pending. Pipeline ECHO earlier-reply, QUIT,
   ECHO must-not-execute to A. A reads its prior push, ECHO bulk reply, OK and
   EOF; C next publishes after-close with count 1 and B receives it. Repeat
   replacing QUIT with raw *1\r\n!0\r\n: A's final frame is the source's exact
   protocol error, not the following ECHO. See the network case for raw bytes.
9. A/B each subscribe twice to ordinary news, pattern n* and shard news. C uses
   INFO stats: distinct counts are 1/1/1 despite duplicates. Remove A's three
   memberships: counts stay 1/1/1. Remove B's ordinary, pattern and shard
   memberships in order: counts are 0/1/1, 0/0/1 and 0/0/0. Repeat RESP2 and
   RESP3, retaining INFO's bulk versus verbatim wire type.

## Reference units expected after CI

No new suite pass is claimed before a run. Direct-channel candidates in
[unit/pubsub.tcl](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsub.tcl)
have these exact source titles (PING expands to both protocol versions):

- `Pub/Sub PING on RESP2` and `Pub/Sub PING on RESP3`
- `PUBLISH/SUBSCRIBE basics`
- `PUBLISH/SUBSCRIBE with two clients`
- `PUBLISH/SUBSCRIBE after UNSUBSCRIBE without arguments`
- `SUBSCRIBE to one channel more than once`
- `UNSUBSCRIBE from non-subscribed channels`
- `PUBLISH/PSUBSCRIBE basics`
- `PUBLISH/PSUBSCRIBE with two clients`
- `PUBLISH/PSUBSCRIBE after PUNSUBSCRIBE without arguments`
- `PUNSUBSCRIBE from non-subscribed channels`
- `NUMSUB returns numbers, not strings (#1561)`
- `NUMPATs returns the number of unique patterns`
- `Mix SUBSCRIBE and PSUBSCRIBE`
- `PUNSUBSCRIBE and UNSUBSCRIBE should always reply`
- `publish to self inside multi`
- `publish to self inside script`
- `unsubscribe inside multi, and publish to self`

Shard candidates in
[unit/pubsubshard.tcl](https://github.com/redis/redis/blob/7.2.16/tests/unit/pubsubshard.tcl)
have these exact source titles:

- `SPUBLISH/SSUBSCRIBE basics`
- `SPUBLISH/SSUBSCRIBE with two clients` (both source occurrences)
- `SPUBLISH/SSUBSCRIBE after UNSUBSCRIBE without arguments`
- `SSUBSCRIBE to one channel more than once`
- `SUNSUBSCRIBE from non-subscribed channels`
- `PUBSUB command basics`
- `SPUBLISH/SSUBSCRIBE with PUBLISH/SUBSCRIBE`

Keyspace notifications and replication remain
outside this task. CLIENT REPLY OFF and complete OOM cases depend on their
broader existing command support; COMMAND's empty descriptor behavior remains
unchanged. Update the ratchet only from the implementation's actual CI evidence.

## Proposed design and validation

Three new nodes are proposed: connection ownership/output ordering, pub/sub
registries, and script-attempt external effects. The existing transactions,
scripts and orderly-stop nodes now state the conditional hub hold, external
delivery continuation and promoted-send cancellation. Their owner approval and
readiness evidence remain pending. No approval log, commit, pin movement or
submodule update was made. The prior read-only review of capacity preparation
is not an implementation review; a fresh completion review covers the full
new source and registered cases. Every compile/run, timing, memory-accounting
comparison and race result remains unverified until owner-driven CI.

The fresh read-only completion review used `gpt-6.1-sol` at high effort and
covered the starting revision through the complete final working tree,
including all untracked files, 22 registered cases, relevant existing design
nodes and the client-handshake branch. It read the pinned interfaces and Redis
7.2.16 source; it ran only reading/searching and Git inspection. Final
`git diff --check` passed. No suite or lint was run.

| Review group | Result |
|---|---|
| A1, C1, T1, D1 | Pass on source/reference inspection. |
| C2, T3, DC4 | Unverified: no compiler, project gate or aof-sync-fault result. |
| T2 | Pass on static gate wiring; gate-sync was not run. |
| R1 | Not applicable: no performance attribution. |
| G1, G2, G3, DC1, DC2, DC3 | Pass on source/design inspection after the findings below were corrected. |

The design tree has 19 nodes instead of the base's 16, its maximum depth
remains 3, and its Decision count is 116 instead of 108. These are read-only
counts, not a lint result. Owner approval remains outstanding.

The primary agent checked each review finding and the reviewer re-read its
repair. Idle expiry formerly trusted the delayed Membership notification;
it now checks committed membership and latest activity together with removal.
Pressure removal formerly left later command admission dependent on delayed
Close delivery; live registration and cancellation now jointly define admission
before execution. A nested call introduced during the first repair was
rewritten to the writing guide's atom operands, without a compiler diagnostic.
The existing SCRIPT KILL node was also qualified for selected publications
and the known dataset-unwritten-publisher difference. INFO's nonzero distinct
counts and the corrected per-file ownership descriptions were checked.
No confirmed finding remains within this read-only scope. The controlled
Membership/Close race witnesses, partial sends/cancellation, old-id delivery,
query overflow, hard/soft accounting/reset, lock/stack ledgers and timing
comparisons still need CI; broad source coverage does not constitute those
observations.
