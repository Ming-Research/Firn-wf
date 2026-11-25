# Commands

How a command is written as parts that every caller shares, and which objects its statement holds.

### decision command-written-as-shared-parts
A command is its parsing, a body over its key's entry or its key set's entries, a metadata step and a reply, and the network path, EXEC and scripts all call these same parts.
- rejected: an agent proposed a second, script-only copy of each command; the owner rejected it, because a client and a script must run one implementation of a command's semantics.
- rejected: an agent proposed a body per key for commands such as RENAME and COPY; the owner rejected it, because those commands move or compare values between keys, so the work on one key depends on the other.

### decision script-command-refusal-text
`script_command` refuses Redis 7.2.16's noscript commands and the EVAL family with Redis's text, and answers any other command it does not run with Redis's unknown-command text and a firn note.
- rejected: an agent proposed a reply of its own for a command firn identifies but does not run from scripts; the owner rejected it, because the extended unknown-command text keeps the boundary firn already had and the words Redis's own tests match.

### decision command-identity-and-flags-shared
`command_kind` identifies every command firn knows, and admission, MULTI and scripts read Redis's DENYOOM and WRITE flags from one byte map per flag, indexed by that identity.
- replaced: naming commands only in the dispatcher's rows, so that nothing told an unknown name from a command without parts, was on main; the owner changed it, because memory admission must tell unknown names apart before it evicts.
- rejected: an agent proposed separate identity or flag tables for each execution path; the owner rejected it, because memory admission must tell unknown names apart before eviction, and a script's write acceptance follows the WRITE flag, not what a command changed.
- replaced: scanning a list of kinds for each flag was on main; the owner changed it, because a command on neither list scanned both to the end, about 30 ns and 11% of a GET's server CPU, and the cost moved with the code around the scans (data: `research/investigations/firn/pubsub-command-cost.md`).

### decision pubsub-names-compared-last
`command_kind` and `execute` compare a name with the nine publish/subscribe names last, and `admission_arity` tests the publish/subscribe kinds after the kinds that have held parts.
- replaced: comparing the publish/subscribe names first was on main; the owner changed it, because every other command then paid for them: 7% of a GET's server CPU before the flag maps and 0.9% with them (data: `research/investigations/firn/pubsub-command-cost.md`).

### decision script-command-operand-planning
A script's command is planned as keyless, as finite ordered operands mapped onto the script's declared keys, or as a whole-map command, and keeps the shared parsing, bodies and replies.
- rejected: an agent proposed taking the order of the declared keys as the command's operand order; the owner rejected it, because set algebra and multi-key commands must keep their operands' order and duplicates.
- rejected: an agent proposed handing a command a reference to the whole map under a hold of declared entries; the owner rejected it, because a command under that hold must touch only the entries it holds.

### decision aof-off-writes-hold-only-their-keys
While no append-only file is kept, a write that can queue no expiry and reads nothing else from Meta holds only its keys' entries, with an inert sequence token and no metadata step.
- replaced: holding Meta in every write statement whatever the configuration was on main; the owner changed it, because one exclusive Meta target on every write bounds all writes together near 2.8 M holds/s on 8 P-cores (data: `research/investigations/write-meta/README.md`).

### decision conditional-meta-hold-in-one-statement
A command that holds Meta only on some runs is one atomic statement naming its keys and Meta, with every use of Meta under the run-time condition, so a run that never uses Meta never holds it.
- replaced: two atomic statements around the condition, one naming Meta and one not, was on main; the owner changed it, because they repeat each command's shell at about 40 sites, and SET as one statement ran within the twin's noise (data: `research/investigations/write-meta/README.md`).
- rejected: an agent proposed asking Whitefoot for an atomic target held only when a run-time condition holds; the owner rejected it, because no program needs what it adds: refusing a use outside the condition, and holding by condition a type earlier in the lock order.