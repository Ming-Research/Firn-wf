# Append and sync errors

What firn does when an append to or a sync of its active append-only file fails: the saved errors, retries, the sync schedule, recovery, and the MISCONF text.

### decision aof-errors-saved-in-meta
The AOF writer saves its append and sync errors in Meta; while either is saved, WRITE commands and PING get MISCONF, checked at admission and again under the mutation's Meta hold.
- rejected: an agent proposed keeping the AOF's health in a separate shared status object; the owner rejected it, because Meta already orders every mutation with the writer's publication, so another object adds no needed capability.

### decision aof-partial-append-rolls-back
A partial append is truncated back to the file's size before it and retried whole; if that truncation fails, the written prefix is counted and only the rest is retried.
- replaced: keeping the written prefix in the file and retrying only the rest, with no rollback, was on main; the owner changed it, because Redis 7.0.15 truncates back the same way, so the file holds exactly the bytes still to be written and no partial record.

### decision aof-append-retry-every-100-ms
The writer retries a failed append every 100 ms, whether or not clients write.
- replaced: a retry about once a second was on main; the owner changed it, because Redis retries a held buffer on every pass of its event loop, which runs at least 10 times a second when idle, so 100 ms is Redis's idle cadence.

### decision aof-sync-only-after-new-bytes
Outside shutdown the writer syncs at most once a second, and only when nothing is held back and bytes were appended since the last sync, or while a sync error is saved.
- replaced: syncing once a second whether or not anything was appended was on main; the owner changed it, because a device that failed while firn was idle failed that idle sync first, and firn then refused with MISCONF a write that Redis 7.2.16 acknowledges.

### decision aof-sync-error-cleared-only-by-durable-rewrite
After a failed sync, a later sync that succeeds starts a rewrite from memory; only a rewrite whose install is durable clears the sync error and lets writes in again.
- replaced: clearing the sync error when a retried sync succeeds was on main; the owner changed it, because Linux marks the pages whose writeback failed clean, so that sync does not write them, and a restart lost both a write acknowledged during the failure and one after.
- rejected: an agent proposed writing again, into the same file, the bytes appended since the last good sync; the owner rejected it, because it trusts that file's pages and size after an error, and in the faulty-device case even the file's length was not written.
- rejected: an agent proposed doing as Redis 7.2.16 does and never recovering without a restart; the owner rejected it, because an idle server then refuses writes until an operator restarts it, and the restart still loses the write acknowledged during the failure.

### decision aof-rotation-keeps-saved-errors
At a rewrite's switch to a new incremental file, the writer resets the size counts but keeps saved append and sync errors, even when the old file's last sync succeeds.
- rejected: an agent proposed clearing a saved sync error when the old file's sync before it is closed succeeds, as Redis 7.2.16 does; the owner rejected it, because that sync does not write the pages that failed, so a restart could lose writes already acknowledged.

### decision aof-misconf-ends-with-host-text
A MISCONF refusal ends with the host's text for the saved error: the failed append's own error when none of that append was written, and otherwise the sync error.
- rejected: an agent proposed a table in firn from error numbers to glibc's text; the owner rejected it, because it works around a Whitefoot gap, breaks on another libc or platform, and errors outside the table would differ from Redis.
- rejected: an agent proposed ending the reply with the error's class name, such as NoSpace, for now; the owner rejected it, because the reply would differ from Redis, which puts a known difference on main.

### decision aof-misconf-short-write-says-no-space
When an append was only partly written, the MISCONF refusal ends with the host's text for the no-space class, as Redis reports every short write as ENOSPC.
- replaced: an empty text after the MISCONF prefix was on main; the owner changed it, because Whitefoot's library now gives the host's text for an error class, so the reply equals Redis's word for word and firn's source holds no platform error number.
- rejected: an agent proposed describing the error the last write returned; the owner rejected it, because Redis reports disk full for a short write whatever the write returned.
- rejected: an agent proposed hard-coding the host's ENOSPC number; the owner rejected it, because that is the platform-dependent code the change removes.
- rejected: an agent proposed a fixed text; the owner rejected it, because the text can differ from one host to another.