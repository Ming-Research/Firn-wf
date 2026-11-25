# Building

How firn is built with Whitefoot: the compiler it pins, where the shared build rules come from, and the safety witnesses the gate compiles.

### decision firn-builds-with-a-pinned-whitefoot-compiler-release
firn builds with the published Whitefoot compiler release that `whitefoot.pin` names, `wf-` and 12 hex digits of a commit on Whitefoot's main.
- rejected: an agent proposed a submodule of Whitefoot built in firn's CI; the owner rejected it, because firn would then compile Whitefoot in its own CI and carry Whitefoot's whole tree for one executable.
- rejected: an agent proposed pinning a Whitefoot specification version; the owner rejected it, because the compiler changes within one specification version, and firn needs a compiler that changes only when firn chooses to adapt to it.

### decision firn-takes-whitefoot-rules-from-the-kit-submodule
firn takes the pin's checks, the compiler's download and the rules for upgrading Whitefoot from the shared `whitefoot-kit` submodule, as every project written in Whitefoot does.
- replaced: a copy of the pin plumbing and rules in each project was on main; the owner changed it, because the copies drifted apart, and one change had to be made again in every project.
- rejected: an agent proposed shipping the shared rules inside each Whitefoot release; the owner rejected it, because the code that downloads a release cannot itself come from that release.

### decision gate-compiles-paired-safety-witnesses
The gate compiles ten pairs of faulty and corrected forms of real Redis-family safety bugs, requiring each faulty form's named diagnostic and each correction's build.
- rejected: an agent proposed keeping the witnesses in the research record, run by hand; the owner rejected it, because no one would notice in time when a Whitefoot upgrade weakens a safety guarantee, and the evidence shown could go stale.
- rejected: an agent proposed checking only that each faulty form fails to compile; the owner rejected it, because a nonzero compiler exit can hide an unrelated error.
- rejected: an agent proposed checking only that the faulty forms are rejected; the owner rejected it, because that can hide a correct form that Whitefoot does not support.