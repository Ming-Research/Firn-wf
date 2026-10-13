Decision: Firn keeps a maximum of 4294967296 (2^32) elements per list, an intentional difference from Redis 7.2.16 on 64-bit, instead of promising Redis's full 64-bit list range. This is a provisional compatibility boundary; reopen it when a selected application workload requires a larger list.

Decision: A list command that would exceed the element limit returns an ERR reply naming the limit before changing any entry or access metadata, admitting an entire multi-element push at once and checking a move's destination before popping its source, because success must mean that every accepted element was stored and a refusal must preserve the dataset, instead of silently dropping elements, partially inserting a push, or losing a moved element. Moving within the same list is a rotation and needs no additional room. LINSERT with an absent pivot still returns -1 because it adds nothing. Network commands, transactions, scripts and append-only replay use the same command bodies ([shared command parts](command-parts.md)).

Rejected:
- Returning success while dropping an element at the ceiling: rejected because the reply would claim a mutation that did not preserve the accepted element.
- Popping a move's source and then testing its destination: rejected because a refused destination would already have removed the source element.
