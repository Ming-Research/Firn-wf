Decision: firn builds with a published Whitefoot compiler release that `whitefoot.pin` names by its commit, `wf-<12 hex digits>`, a revision bound for main naming a release of a commit on Whitefoot's main, because firn needs a compiler that changes only when firn chooses to adapt to it and Whitefoot's specification version changes with every approved rule while its compiler changes within one version, instead of a submodule of Whitefoot built in firn's CI or a pin to a specification version.

Decision: firn takes the pin's checks, the compiler's download and the rules for upgrading and trying an unmerged Whitefoot change from the shared `whitefoot-kit` submodule, as every project written in Whitefoot does, and adopts a change to them by moving that submodule, because those rules are the same for every such project and a copy in each made one change a pull request per project, instead of keeping firn's own copy beside the others'.

Decision: Firn's gate compiles paired faulty and corrected transcriptions of real Redis-family safety bugs with its pinned Whitefoot compiler, requiring each faulty program's specified source diagnostic and each corrected program's successful build, because a nonzero compiler exit alone can conceal an unrelated error and a one-sided rejection can conceal an unsupported valid pattern, instead of treating unchecked examples as safety evidence ([witness scope and checks](../../research/experiments/safety-witnesses/README.md)).

Rejected:
- A copy of the pin plumbing and rules in each project: rejected because the copies drifted apart and one change took a pull request per project.
- Shipping the shared rules inside each Whitefoot release: rejected because the code that downloads a release cannot itself come from that release.
