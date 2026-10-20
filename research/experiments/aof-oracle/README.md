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
  expiry is off, so no command removes them before the transaction. The
  first three records of the AOF are the setup `SET`s; the rest are the
  expected records of the test, which the replies match too.
