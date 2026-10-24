# Recorded replies and AOF bytes from Redis 7.2.16

These files keep the replies, and where a case asks for them the append-only
bytes, that Redis 7.2.16 gave for requests that firn's network tests compare
against (`tests/network.rs`). They were
copied from the artifacts of the `oracle-replies` workflow, which ran the
reference server built from the pinned Redis 7.2.16 release. They are kept
here because GitHub deletes run artifacts after 90 days.

Each file is a JSON list of cases. A case has `name`, `resp` (the protocol
version the client used), `replies` (each request with the reply Redis gave)
and `file`. A case whose requests include `["__AOF__"]` also keeps the complete
append-only incremental file that Redis wrote for the requests before it, as
the reply's raw text.

## `set-spop-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/38406427583
- Run head commit: `351b3d4c842d0f1e5fa2b95a8a8b4180968a3a90`
- Case `aof-set-nx-pxat`: `SET` with `NX`, `XX GET`, and `PX`, whose AOF
  record is rewritten to `PXAT`. Used by
  `firn_records_set_pxat_arguments_and_rewrites_other_expiries`.
- Case `aof-spop-2500`: `SPOP` of 2500 and of 2 members from sets of 3000
  and 3 members. Its AOF shows the `SREM` batches of 1024, 1024 and 452
  inside `MULTI`/`EXEC`, and a single `SREM` of 2 without `MULTI`. Used by
  `firn_records_spop_count_in_redis_srem_batches`.

## `legacy-forms-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/38403444843
- Run head commit: `476dd4416dbfdd7f98b00f924d3f5ceafbae394a`
- Case `legacy-direct-propagation-forms`: 36 requests whose complete AOF is
  1418 bytes including `SELECT 0`. Used by
  `firn_records_its_writes_as_redis_propagates_them`.

## `72-scan-type-none-and-deep-glob.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/38406427583
- Run head commit: `351b3d4c842d0f1e5fa2b95a8a8b4180968a3a90`
- Case `scan-type-none-and-deep-glob`: an expired key that `SCAN TYPE none`
  removes is not returned and is gone afterwards (`EXISTS` gives 0); `KEYS`
  with 1001 nested `*a` groups returns an empty list, while 1000 and 999
  groups return the 1001-byte key; `SCAN` with `MATCH` at 1001 nested groups
  returns nothing. Used by
  `firn_scan_type_none_omits_a_key_expired_during_the_scan` and
  `firn_glob_matching_obeys_redis_star_nesting_limit`.

## `held-scan-type-none-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/39435954761
- Run head commit: `dfd20fecdd869b517d3236d3ef45241e91ff172a`
- Case `held-scan-type-none`: the requests of
  `firn_records_held_scan_type_none_as_redis_propagates_it`, with the two
  keys made expired by time (`PX 100`) and `DEBUG SLEEP 0.3` while active
  expiry is off, so no command removes them before the transaction. The AOF
  starts with `SELECT 0`, then three setup `SET`s; after them are the
  expected records of the test, which the replies match too.

## `blocking-expired-keys-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/39899773861
- Run head commit: `b00c4463a2d05b8fad0415ac024924f9f1d5ddc6`
- Three cases of list commands that meet an expired key, each made expired by
  `SET key v PXAT 1` in the same write as the command. A case has `name` and
  `steps`; a step is `pipe` (requests sent in one write with the `replies`
  Redis gave), `send` (a request whose reply is read later), `read` (that later
  reply) or `aof` (the complete incremental file at that point, `SELECT 0`
  included). Cases: `top-level-producers-over-expired-key`,
  `immediate-blocking-pops-over-expired-key` and
  `served-waiter-over-expired-destination`. Used by the scenarios
  `aof-expired-top-level-producers`, `aof-expired-immediate-blocking-pops` and
  `aof-expired-served-waiter` in `tests/blocking_lists_oracle.rs`.

## `blocking-waits-on-strings-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/40057972246
- Run head commit: `544cacde74c0c4e02df20d62082312527f848c89`
- Four cases of list writes while a client is blocked on a key that holds a
  string or on another key, in the step form of the previous file:
  `waiter-survives-an-expired-string-then-push` (`SET k v PXAT 1` and `LPUSH k`
  in one write), `waiter-survives-an-expired-string-then-move` (the same for
  the destination of an `LMOVE`), `waiter-survives-a-live-string-and-a-refused-push`
  (`LPUSH`, `LPUSHX` and `RPUSH` refused with WRONGTYPE leave the waiter
  blocked until `DEL` and `LPUSH` create the list) and
  `pushes-onto-a-live-list-with-a-waiter-elsewhere` (seven list writes onto a
  live list while a client waits on another key). Used by the scenarios
  `narrow-expired-string-then-push`, `narrow-expired-string-then-move`,
  `narrow-live-string-refused-pushes` and
  `narrow-live-list-writes-with-waiter-elsewhere` in
  `tests/blocking_lists_oracle.rs`.

## `lfu-rename-msetnx.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/38406427583
- Run head commit: `351b3d4c842d0f1e5fa2b95a8a8b4180968a3a90`
- Case `lfu-rename-msetnx`: under `allkeys-lfu` with `lfu-log-factor 0` and
  `lfu-decay-time 0`, the object frequencies after `RENAME`, `RENAMENX`, a
  same-name `RENAME`, and twenty duplicate-key `MSETNX` batches. Used by
  `firn_matches_the_lfu_rename_and_msetnx_oracle`.

## `score-text-recording.json`

- Source run: https://github.com/Ming-Research/Firn-wf/actions/runs/38394816029
- Run head commit: `9da04882f1e5202b27758d743b0f844a193d7edf`
- Case `existing-1955 reads_and_writes_scores`: ZSCORE and ZPOPMIN replies
  for the scores of the read and write cases. Used by
  `firn_reads_and_writes_scores_as_redis_does`.
- Case `new-2101 seed+rewrite+resp2` and `new-2101 resp3 readback (fresh seed)`:
  the twenty-one scores in RESP2 and RESP3, with ZSCORE, ZINCRBY, ZRANK WITHSCORE
  and ZRANGE WITHSCORES replies, and the records a BGREWRITEAOF writes after
  them. Used by `firn_writes_score_text_as_redis_7_2_16_does`.
- Case `existing-2929 replay`: the `0.1` score the replayed file holds. Used by
  `firn_replays_the_value_types_from_its_append_only_file`.
- Case `existing-9900 rewrite`: the score records of the rewritten file. Used by
  `firn_rewrites_all_value_types_with_expiries_and_redis_commands`.
