# Design tree change log

Newest first. One entry per approved change of the tree: a dated title,
`Nodes:` naming every node changed, `Owner-approved:` and `Summary:`;
`skill/SKILL.md` owns the form.

## 2026-10-06 The firn tree moves here from Whitefoot

Nodes: firn

Owner-approved: Both decisions were approved in the Whitefoot session of 2026-10-03 as Q1 (complete standalone application workloads) and Q2 (a Lua interpreter written in Whitefoot, beginning with a vertical slice), recorded in Whitefoot's `design/log.md` entry "Firn's standalone deployment and Whitefoot Lua direction" ([Whitefoot at 648338c31](https://github.com/Ming-Research/Whitefoot/blob/648338c31240ba64ce13c314b1afca1749d44189/design/log.md)). The move itself follows the owner's ruling Q43, that firn leaves Whitefoot for this repository. In the session of 2026-10-06, written in Chinese, translated: "Decision 1 is right", and of decision 2: "I approved this long ago, otherwise Halo would not have been written up to now".

Summary: Whitefoot's node `language/firn` becomes this repository's root node `firn` (`design/firn.md`) with its two decisions unchanged: firn's first public deployment milestone is complete standalone Redis application workloads with existing clients and unchanged business logic, and its scripting milestone a Lua interpreter written in Whitefoot, tested first through a vertical slice against Redis's interpreter. Only the links to the deployment direction changed, to `research/investigations/firn/DESIGN.md` in this repository.
