# Rewrite sequence

The commit sequence that orders dataset changes for an AOF rewrite, and the last-write stamp each entry carries.

### decision rewrite-sequence-per-statement-and-inline-stamp
Each mutating atomic statement takes one u64 commit sequence, shared by all its command parts, and writes it as a u64 last-write stamp inline in each entry it changes.
- rejected: an agent proposed keeping the stamps in a separate map; the owner rejected it, because that map stores the keys and capacity again, which costs more, and adds lookups to the same entry observation.
- rejected: an agent proposed a sequence per key or per AOF record; the owner rejected it, because one EXEC or script can change many keys and append several records while it stays one execution unit.
- rejected: an agent proposed leaving the per-entry stamps out from the start; the owner rejected it, because the first version should be observable to show it correct, with the stamps' cost measured before deciding whether to keep them.

### decision rewrite-flush-aborts-the-scan
FLUSHALL and FLUSHDB take the statement's sequence and abort a running rewrite scan.
- rejected: an agent proposed keeping an image of the reset for the scan to reconcile against; the owner rejected it, because recording every key's deletion is not practical, and the scan would need reconciliation that knows about resets.

### decision rewrite-sequence-inert-without-aof
While AOF is off, a statement that holds no Meta takes an inert token: it takes no sequence, stamps zero and captures nothing, and INFO leaves out firn_aof_rewrite_commit_seq.
- replaced: a commit sequence that every write advanced in Meta was on main; the owner changed it, because then every write holds one shared object, which on the 14900K held 8-core SET at 1.13 M/s against GET's 25.6 (data: `research/investigations/write-meta/README.md`).