# Real server bug patterns at the Whitefoot boundary

These ten pairs are original, small transcriptions of faulty patterns reported
in Redis-family servers: four use-after-free cases, two buffer overruns, two
data races and two hold-and-wait forms behind reported deadlocks (Whitefoot
cannot express the original lock protocols, so these pairs show that their
closest forms are rejected). They implement board item
`firn-safety-showcase`'s bug witnesses. No upstream implementation is copied.
They do not change firn or reproduce complete server commands.

**Validation status: all ten pairs passed in [CI][validation-run] at
[the tested revision][validation-revision], with the compiler release
`wf-af7665be672d` that `whitefoot.pin` names.** Each rejected program received
its expected diagnostic and each correction built an executable.

## What the evidence means

A passing pair establishes that the pinned compiler refuses this particular
faulty source for the specified reason and builds its corrected counterpart.
It does not prove absence of an entire bug class, correctness of firn,
upstream compatibility, termination, or correctness of the compiler/runtime.
The accepted executables are built but not run; this check measures source
acceptance, not their application results or scheduling behavior.

The language claim belongs to the [pinned specification][spec] (v0.146), read
at Whitefoot commit `af7665be672d29912d878dd5da8c3932bb6f296c`, the revision
of the release `wf-af7665be672d` that [whitefoot.pin](../../../whitefoot.pin)
names:

- [SCOPE-2 and SCOPE-3][scope] require static discharge before lowering and
  specify no undefined behavior conditional on the compiler, checker, linked
  definitions, runtime, allocator and OS. Resource exhaustion, including
  stack exhaustion, remains outside the source outcome guarantee.
- [REF-2 and REF-3][references] invalidate references after owner replacement,
  consumption or scope exit, and forbid storing/returning references. A move
  does not retarget an existing reference to its new owner.
- [OP-4][bounds] requires a proved initialized-element bound at every indexed
  access. These two cases concern array overruns, not arithmetic overflow or
  exhaustion of an otherwise correctly sized stack. An unproved exact integer
  operation is separately a static [OP-2][arithmetic] obligation; this suite
  does not substitute wrapping arithmetic or a runtime trap for its proof.
- [WAIT-3, SHARE-1 and SHARE-2][contexts] transfer owned values to spawned
  contexts, retain shared state through handles, restrict state access to
  atomic statements, and prohibit writing through `SharedRead` authority.
  [SHARE-3][atomic] gives atomic state transitions exclusive, single-point
  semantics. It does not make a sequence of separate transitions one operation.
- [SHARE-2][atomic] forbids nested atomic statements and waiting calls inside
  their guards/bodies. This excludes the two transcribed hold-and-wait forms.
  **Whitefoot is not deadlock-free:** [WAIT-2][contexts] explicitly permits
  executions stuck on false guards or context joins. Its progress guarantee is
  conditional; an accepted source is not a proof that a producer will run.

## Cases and expected outcomes

Each rejected file is paired with the linked correction in its directory.
The `error[RULE]: Kind` spelling below corresponds to the structured `rule`
and `kind` fields checked by the runner, together with the precise source line
in [cases.tsv](cases.tsv). Verified coverage is limited to the validation
status above; the remaining outcomes are expectations.

| Bug | Project and source | Class | Faulty pattern | Rejected witness | Expected compiler diagnostic | Accepted correction |
| --- | --- | --- | --- | --- | --- | --- |
| Blocked-client iteration | Redis [report #15562](https://github.com/redis/redis/issues/15562), [fix #15594](https://github.com/redis/redis/pull/15594) | Use-after-free | Cache the next waiter, let a callback remove it, then dereference the cached node. | [rejected.wf](redis-blocked-client/rejected.wf) | `error[REF-2]: InvalidReferenceUse` after the callback writes the owning link | [accepted.wf](redis-blocked-client/accepted.wf): select the remaining next owner after the callback. |
| Pending TLS iteration during CLIENT KILL | Valkey [#4234](https://github.com/valkey-io/valkey/pull/4234) | Use-after-free | Cache the next pending connection before a reentrant handler can delete it. | [rejected.wf](valkey-pending-tls/rejected.wf) | `error[REF-2]: InvalidReferenceUse` after handler deletion | [accepted.wf](valkey-pending-tls/accepted.wf): borrow a surviving connection after the handler. |
| Deferred stream-ID reply | Dragonfly [#7874](https://github.com/dragonflydb/dragonfly/pull/7874) | Use-after-free | Defer copying reply bytes until after their temporary owner leaves scope. | [rejected.wf](dragonfly-reply-lifetime/rejected.wf) | `error[REF-2]: InvalidReferenceUse` after the temporary scope ends | [accepted.wf](dragonfly-reply-lifetime/accepted.wf): copy owned bytes while the temporary is live. |
| ACL LOAD with a deferred-free client | Valkey [#3800](https://github.com/valkey-io/valkey/pull/3800) | Use-after-free | Release the registry's user while a deferred client's alias still refers to its name. | [rejected.wf](valkey-acl-user/rejected.wf) | `error[REF-2]: InvalidReferenceUse` after the owning Box is consumed | [accepted.wf](valkey-acl-user/accepted.wf): retain an owned `Frozen<User>` handle for the client. |
| XACKDEL fixed ID vector, CVE-2025-62507 | Redis [advisory](https://github.com/redis/redis/security/advisories/GHSA-jhjx-x4cf-4vm8) | Out-of-bounds | Fill a fixed-capacity ID array after skipping the growth needed for a larger request. | [rejected.wf](redis-xackdel/rejected.wf) | `error[OP-4]: UndischargedBoundsObligation` at the fixed-array write | [accepted.wf](redis-xackdel/accepted.wf): allocate the initialized array for the actual ID count. |
| XAUTOCLAIM deleted-ID array, CVE-2022-31144 | Redis [#11002](https://github.com/redis/redis/pull/11002) | Out-of-bounds | Size the deleted-ID array by COUNT but advance the stopping counter only for claimed entries. | [rejected.wf](redis-xautoclaim/rejected.wf) | `error[OP-4]: UndischargedBoundsObligation` at the deleted-ID write | [accepted.wf](redis-xautoclaim/accepted.wf): count deleted and claimed entries toward the same limit. |
| ZTTL mutation under a read lock | Garnet [#1642](https://github.com/microsoft/garnet/pull/1642) | Data race | Give a query read authority even though its helper removes an expired sorted-set member. | [rejected.wf](garnet-zttl/rejected.wf) | `error[SHARE-2]: ReadonlyWriteTarget` at the helper's writing argument | [accepted.wf](garnet-zttl/accepted.wf): supply `Shared` mutation authority and perform removal atomically. |
| IO-thread script timeout observation | Redis [#15781](https://github.com/redis/redis/pull/15781) | Data race | Spawn a reader borrowing the main context's flag while the main context can write it. | [rejected.wf](redis-script-timeout/rejected.wf) | `error[WAIT-3]: InvalidSpawn`: spawned parameters must be values | [accepted.wf](redis-script-timeout/accepted.wf): pass an owned shared handle and access the flag atomically. |
| Reported work-queue/global-lock inversion | KeyDB [issue #883](https://github.com/Snapchat/KeyDB/issues/883) | Deadlock | Acquire global access while retaining queue access, opposite another worker's order. | [rejected.wf](keydb-lock-order/rejected.wf) | Original mutex protocol is not expressible; closest nested atomic attempt expects `error[SHARE-2]: WaitInsideAtomic` | [accepted.wf](keydb-lock-order/accepted.wf): name both objects in one atomic statement, including a caller with reversed header order. |
| Shutdown holds GIL while draining lazy-free workers | Valkey [#3029](https://github.com/valkey-io/valkey/pull/3029) | Deadlock | Wait for worker completion while retaining engine access the worker needs. | [rejected.wf](valkey-shutdown/rejected.wf) | Original GIL protocol is not expressible; closest waiting call inside atomic expects `error[SHARE-2]: WaitInsideAtomic` | [accepted.wf](valkey-shutdown/accepted.wf): finish engine access, then wait for worker completion outside it. |

## Fidelity and limits of the transcriptions

The two iterator examples retain the critical two-node deletion episode;
they omit queue scheduling and the callback's unrelated server operations.
The Dragonfly temporary is a 40-byte heap-owned byte array instead of a
`std::string`; the scope boundary and deferred reference use are preserved.
The ACL example models the client's pointer with a local reference alias:
putting that reference in a client field is already forbidden by TYPE-8 and
REF-3. Its correction retains lifetime, not the full ACL reload/revocation
policy. These are safety-pattern repairs, not ports of the upstream patches.

XACKDEL uses capacity four and a five-ID input to expose the same missing
size relationship. The command/advisory concerns Redis 8.2, not firn's
Redis 7.2.16 compatibility reference. XAUTOCLAIM bounds the experiment to
1,000 scanned/claimed entries in both versions to separate array capacity
from integer overflow; two deleted entries with COUNT one suffice to expose
the error. The fixed version's loop invariants prove the shared limit without
discarding deleted IDs solely to appease the checker.

The ZTTL example represents one expiring sorted-set member. Garnet #1642
describes the fault: ZTTL reads through Tsavorite's shared-lock Read path,
and the sorted-set layer then deletes expired members, mutating the shared
structure under that shared lock. `SharedRead` represents read authority, not an implementation
promise that Whitefoot uses a reader/writer lock. The script-timeout example
has no timing assertion: either flag value can be observed legally after
the repair, but a reference into the parent cannot be passed to a spawn.

Whitefoot's shared-state interface has no independently held raw mutex/GIL
token. The deadlock witnesses use its nearest state-access constructs,
not invented `lock()` functions whose missing names would trivially fail.
KeyDB's issue reports the inverse acquisition pattern; the discussion
disputes the precise pending-versus-held writer condition, so this is not a
claim that its complete root cause was independently reproduced. A single
atomic statement names its entire state transition; header order is not a
source-directed sequence of lock acquisitions. The Valkey correction lets
the worker acquire engine access before shutdown waits for its completion.
Neither pair establishes general termination or excludes guard/join cycles.

Whitefoot has no comments ([FORM-4][form]). Each program therefore names its
bug and URL in a `doc` field at the beginning of its first declaration body.

## Running and auditing the evidence

Run only in CI, following the repository's machine rule:

```sh
make compiler
make toolchain
make safety-witnesses
```

The Makefile uses `WHITEFOOTC` from `whitefoot-kit/whitefoot.mk` and depends on
the pin. The `safety-witnesses` job in `check.yml` is part of `make check`'s
target set, checked by `make gate-sync`. No Whitefoot pin or submodule is
moved by this experiment.

`check.sh` uses Python's standard JSON parser to inspect the compiler's
structured diagnostic rather than matching incidental text. It first tests
its own verdict handling with injected results: unexpected acceptance,
compiler signal, wrong rule/kind/category/source location, unstructured tool
failure, missing source, corrected-source failure, and missing executable
must all fail. These injected results are harness checks, not Whitefoot
evidence. `sh check.sh --self-test` runs only those checks, also in CI.
Injected traversal checks also require failure output to be printed and both
variants of later cases to be attempted after an earlier mismatch, timeout,
startup error or missing input. They inspect the aggregate JSON and CI summary.

For every row in `cases.tsv`, the runner requires both files, checks the
20–80-line limit, invokes the pinned compiler separately with `-o` and JSON
diagnostics, and reads its exit code directly. A rejected file passes only
with a source rejection matching the specified rule, kind, file and faulty
source line. An accepted file passes only with exit zero and a newly produced
executable. There must be at least ten pairs, at least two per class, and no
unlisted case directory. A newly accepted faulty pattern therefore fails the
gate rather than silently becoming a claimed success. An unrelated parse
error cannot satisfy a reference, bounds, spawn or atomic-access expectation.
The runner checks both variants independently, continues through every listed
case, prints captured stdout and stderr for every failed attempt, and exits
nonzero only after reporting all mismatches. Invalid global configuration,
such as an unusable compiler or malformed manifest, fails before traversal.

The job uploads `build/safety-witnesses/results.json` and each invocation's
stdout/stderr, including failures. The JSON records the pin, compiler hash,
both source hashes, observed diagnostics, per-variant outcomes and failure
reasons. A CI step summary lists every pair and its failures. The aggregate
JSON is written after traversal; an interrupted run retains only the
invocation logs already written and cannot count as completed coverage.
The runner removes an earlier JSON report before starting new attempts, so
interruption cannot leave that report appearing to describe the current run.
Each completed invocation reports its elapsed time.
These times describe validation cost, not server performance.

After CI, cite its exact revision and artifact when reporting verified
coverage. If the natural faulty form compiles, keep its minimal source and
report the possible Whitefoot gap under `firn-safety-showcase`; do not change
the expected outcome, add an unrelated error or hide it with a runtime check.
If a corrected form is rejected, inspect the diagnostic as a potential
language/compiler gap before changing its spelling. No gap has yet been
established; a correction's failed compilation needs its actual diagnostic
before it can be classified as a source defect or a language/compiler gap.

[spec]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md
[scope]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L9-L21
[references]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L695-L710
[bounds]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1076-L1092
[arithmetic]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1013
[contexts]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2270-L2348
[atomic]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2322-L2359
[form]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L100
[validation-run]: https://github.com/Ming-Research/Firn-wf/actions/runs/39570476159
[validation-revision]: https://github.com/Ming-Research/Firn-wf/commit/9ef54ba8f76996734dd7376b96b069df8db76b9c
