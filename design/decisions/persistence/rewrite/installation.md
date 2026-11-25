# Rewrite installation

How a finished rewrite is published: while appends to the old incremental file fail, and when the directory sync after a manifest rename fails.

### decision aof-rewrite-survives-a-failed-append
A failed append to the old incremental file does not abort a rewrite, which keeps its scan and journal; a saved sync error still aborts it.
- replaced: Abandoning the rewrite at the first failed append was on main; the owner changed it, because the scan and base stay valid while appends fail, no crash point is safer for abandoning them, and the failed attempt kept automatic rewrites off after the disk was repaired.

### decision aof-rewrite-direct-installation
When the base is complete while bytes for the old incremental file are still unwritten, firn publishes the base with a new empty incremental file, and adopting that manifest drops those bytes.
- rejected: an agent proposed writing the retained bytes into a fresh incremental file at the cut, so the old selection is whole again; the owner rejected it, because that needs two more manifest publications on the failing device, and a partial command left by a failed rollback stops startup.
- rejected: an agent proposed waiting for the disk to be repaired, then switching and installing; the owner rejected it, because nothing is published while the fault lasts, the reserve and the backlog after S1 stay held, and no space is freed when the fault is ENOSPC.
- rejected: an agent proposed releasing the retained bytes at the cut, since the base holds their effects; the owner rejected it, because an abort or crash before publication would then lose acknowledged commands from every selection.

### decision aof-rewrite-admits-writes-after-direct-installation
After a durable direct installation firn clears the append error and admits writes, where Redis 7.2.16 refuses them until its retained bytes are written.
- rejected: an agent proposed keeping the append error, as Redis does, until a later append succeeds; the owner rejected it, because firn's retained bytes are already in the new base, so nothing is left to append and no repair could clear the error.

### decision aof-rewrite-dirsync-failure-refuses-writes
If the directory sync after a rewrite's manifest rename fails other than with EINVAL, the attempt fails and saves the AOF sync error, so writes get MISCONF until a recovery rewrite is durable.
- rejected: an agent proposed doing as Redis 7.2.16 does, failing the rewrite but still acknowledging writes; the owner rejected it, because the device confirmed neither the rename nor the entries it names, and firn refuses writes after every other failed sync.

### decision aof-rewrite-failed-switch-keeps-the-old-file
After such a failure at the switch, the old manifest and append handle stay in force, and the new incremental file stays in place.
- replaced: Adopting the switch's new incremental file after the directory sync failed, with writes still acknowledged, was on main; the owner changed it, because a restart that reads the old manifest does not replay that file, so writes acknowledged into it are lost.

### decision aof-rewrite-einval-counts-as-synced
EINVAL from the directory sync after any manifest rename counts as a successful sync, as Redis 7.2.16's fsyncFileDir treats it, so the rewrite completes and writes go on.
- rejected: an agent proposed failing the rewrite on EINVAL without saving a sync error; the owner rejected it, because EINVAL means the filesystem can never sync directories, so every rewrite would fail and the append-only file would grow until the disk filled.
- rejected: an agent proposed treating EINVAL as a failed sync that refuses writes; the owner rejected it, because the recovery rewrite's sync returns EINVAL too, so firn could never accept writes again on that filesystem.

### decision aof-rewrite-install-keeps-the-renamed-manifest
At installation a renamed manifest stays in force with every file it names even when its directory sync fails, and only a durably synced manifest lets firn delete history files.
- rejected: an agent proposed doing as Redis 7.0.15's backgroundRewriteDoneHandler does and deleting the new base after that failure; the owner rejected it, because the manifest a restart sees may name that base, and could then not be replayed.