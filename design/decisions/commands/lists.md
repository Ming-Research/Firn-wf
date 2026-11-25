# Lists

Firn's lists: the most elements one list may hold, and what a command does when it would add past that.

### decision lists-share-the-store-ceiling
A list holds at most 4294967296 (2^32) elements, the store-wide ceiling `pkg::store::ceiling` that hashes and sets share.
- rejected: an agent proposed lists with no element ceiling on 64-bit hosts, as in Redis 7.2.16; the owner rejected it, because one capacity rule covers every collection, no selected workload needs so many elements, and it would change how lists, hashes and sets are stored.

### decision a-list-command-past-the-ceiling-changes-nothing
A list command that would pass the ceiling replies with an ERR naming the limit before it changes any element or access metadata; a multi-element push is checked whole.
- rejected: an agent proposed inserting part of a multi-element push and then replying with an error; the owner rejected it, because a retry would write the inserted part twice, and the error does not tell the client how many went in.
- rejected: an agent proposed that LMOVE pop its source first and put the element back if the destination is full; the owner rejected it, because the element would have to go back to its old place, and the source key may already be deleted once empty.