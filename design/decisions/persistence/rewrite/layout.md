# Append-only layout

The files of firn's multi-part append-only layout: their names, the manifest's text, what a base holds, and how an old single file moves in.

### decision aof-multi-part-layout
For appendfilename F, firn keeps Redis's multi-part layout under appendonlydir: the manifest F.manifest selects a base F.<n>.base.aof and the incremental files F.<n>.incr.aof after it.
- rejected: an agent proposed one file whose replacement must also copy the writes made while it is rewritten; the owner rejected it, because a manifest can select a consistent base and the incremental files after it while later writes go on.

### decision aof-manifest-quotes-apostrophes
The manifest writer puts a filename that holds an apostrophe in double quotes, as sdscatrepr does, where Redis's writer leaves it bare.
- rejected: an agent proposed copying Redis's sdsneedsrepr, which does not quote apostrophes; the owner rejected it, because the loader, which reads names with sdssplitargs, then refuses the manifest.

### decision aof-rewrite-temporary-base-name
A rewrite writes its new base to temp-F.base inside appendonlydir, after Redis's naming of temporary append-only files.
- rejected: an agent proposed Redis's name for the rewrite child's file, made from its process ID, in the working directory; the owner rejected it, because the installation rename must stay on one file system.
- rejected: an agent proposed numbering temporary bases from a sequence shared by independent manifests; the owner rejected it, because servers with different appendfilenames need different temporary base names.

### decision aof-base-holds-commands
A base file holds commands in the append-only format, with no RDB preamble.
- rejected: an agent proposed Redis's default RDB preamble for the base; the owner rejected it, because firn reads and writes no RDB.

### decision aof-upgrade-moves-the-old-file
An old single file F is upgraded by first persisting a manifest that names it as base 1, then moving it into appendonlydir with std::fs::move_file.
- rejected: an agent proposed copying the old file into a second file; the owner rejected it, because persisting the manifest before the move already lets Redis's recovery finish an interrupted upgrade without copying the log.
- rejected: an agent proposed reading an unfinished move as an empty dataset; the owner rejected it, because with the manifest written first, a restart finds the old file and finishes the move, as Redis does.

### decision aof-files-behind-a-link-are-refused
firn refuses an append-only file or manifest whose name is a symbolic link, where Redis follows the link.
- rejected: an agent proposed adding to Whitefoot an operation that follows a link with the authority it reaches; the owner rejected it, because Whitefoot's operations keep a name below its root by refusing a link there, and no deployment is known to keep these files behind a link.