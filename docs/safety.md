# Firn's safety guarantees

These claims describe Firn built with [its Whitefoot pin](../whitefoot.pin),
`wf-af7665be672d`: [specification v0.146 at the pinned commit][spec].
Whitefoot's [safety claims page][claims] states what its checker claims for an
accepted program, what it does not claim and what the claims trust; this
document links to those entries and adds what is Firn's: the boundaries as they
meet Firn, its resource failures, its evidence and its runtime source census.
All of it is conditional on the page's [trusted base][claims-tcb]
([SCOPE-3][scope]). It does not establish Redis compatibility, durability of
every acknowledged write, or continuous availability.

The page was added to Whitefoot after the pin, so it is linked at the commit
that merged it, `4b58ba6`. Compared rule by rule between
`spec/kernel-spec.md` at the pinned commit and at `4b58ba6`, the 35 rules the
page cites all exist at the pin; 29 read the same there, and ENT-1, SHARE-1,
SHARE-2, SHARE-3, STOR-3 and WAIT-2 differ only by clauses and wording the
later specification added. The five limits under the page's
[Not claimed][not-claimed] quote the pin's text verbatim.

## Guarantees and their boundaries

| Property | Where the claim is stated | What remains outside the claim |
| --- | --- | --- |
| **No data races in accepted Whitefoot code** | [No data races][claim-race] | General race conditions between separate transitions and application protocol mistakes remain possible ([not claimed][not-claimed]); native runtime synchronization is trusted ([trusted base][claims-tcb]). |
| **No use-after-free** | [No memory corruption][claim-mem], owners and references | Generated releases and native handle lifetimes are trusted ([trusted base][claims-tcb]). Resource termination has no cleanup guarantee ([not claimed][not-claimed]: resource availability). |
| **No out-of-bounds access** | [No memory corruption][claim-mem], subscripts and slices | Acceptance proves neither that available memory can serve an allocation nor that stack frames fit the execution environment ([not claimed][not-claimed]: resource availability; [STOR-8][storage]). |
| **No overflow of exact integer arithmetic** | [No silent overflow][claim-overflow] | This is not a claim that every counter is mathematically unbounded or that floating-point results never become infinity/NaN. [OP-3][arithmetic]. |
| **No source panic or proof-failure trap** | [No runtime traps][claim-trap] | Firn can deliberately return a nonzero entry status, encounter I/O errors, or be terminated by resource failure ([not claimed][not-claimed]: resource availability) or the host. [PRE-2][host]. |
| **Conditional progress; no general deadlock guarantee** | Not on the page, which lists [freedom from deadlock as not claimed][not-claimed]. This document's reading of two rules: an atomic guard/body cannot nest an atomic statement or call a waiting function, excluding those hold-and-wait forms. If every context reaches completion or a wait in finitely many steps from every point, WAIT-2 promises eventual next steps for runnable contexts or waits with a produced host outcome/completed joined context, and eventual unguarded or persistently enabled atomic transitions. [SHARE-2][shared], [WAIT-2][wait]. | False guards, joins on contexts that never complete, missing host outcomes or a context that never yields can still stall service ([not claimed][not-claimed]: deadlock, termination). No latency bound follows. [WAIT-3][wait]. |

Firn has concrete waits for its script engine and for all clients to leave;
scripts and guarded waits can delay shutdown, and filesystem calls have no
deadline/cancellation parameter. See [script engine acquisition](../firn/scripting/entry.wf#L54),
[shutdown drain](../firn/server/server.wf#L1128), [orderly-stop design](../design/firn/orderly-stop.md)
and [PRE-2][host].

Resource availability is [not claimed][not-claimed]. Heap exhaustion does not
return a Whitefoot allocation error or execute a language trap
([STOR-8][storage]); the pinned POSIX implementation attempts a resource record
and calls `abort()` (**SIGABRT**) on allocation refusal or a classified
stack-guard fault, and the OS can kill the process first.
Deep recursion and recursive release can exhaust the stack. The
[abnormal-stop audit](../research/investigations/firn/abnormal-stops.md)
links the implementation, counts Firn's resource operations and records its
deliberate exits; that audit is part of [PR #102][audit-pr].

Halo's Lua errors, logical heap accounting and execution budgets are
application mechanisms, not checker proofs of bounded process resources or
script termination. Its logical heap limit excludes reserve storage;
Firn constructs an engine with a maximum-`u64` limit and can
[resume a written script with a maximum budget](../firn/scripting/entry.wf#L580).
These mechanisms cannot catch physical
Whitefoot heap exhaustion. See [Halo's pinned embedding contract][halo-limit]
and [Firn's engine and attempts](../firn/scripting/entry.wf#L84).

## Evidence and known deviations

[PR #85][witness-pr] contains
[`research/experiments/safety-witnesses/`][witnesses]: **10 rejected/accepted
pairs** for four lifetime faults, two bounds faults, two data races and two
hold-and-wait forms. Its gate checks the expected rejection rule/location and
builds each accepted correction. The [recorded passing run][witness-run]
executed head `380cf742`; the later commit `a3253ee` (the linked tree) changed
only a comment in one accepted witness and the experiment README, and gate run
38424251158 passed on `a3253ee`. That run used Whitefoot `d75f318d5631`, an
earlier pin; accepted executables were **not run**.
These are specific source-admission examples, not a complete proof of Firn or
of the current compiler/runtime. The rules above are cited at Firn's current
pin, except for the claims page, which is linked at `4b58ba6`.

The audit also found an accept error that stopped service with status zero;
firn now logs it and keeps listening ([correction in PR #104][accept-pr]).
It found as well, read from source but not yet confirmed by a run, unknown AOF
commands whose error replies are discarded ([board item
`firn-bl-01-10`][board]). Both are operational and persisted-state
correctness gaps; memory safety does not establish their recovery policy or
Redis semantics. Their source evidence is in the
[audit's findings](../research/investigations/firn/abnormal-stops.md#5-gaps-uncertainty-and-disposition).

## Trusted base and source sizes

Whitefoot's page lists the [trusted base][claims-tcb] that the claims above
are conditional on and its checker does not verify. Firn and Halo's checked
Whitefoot code still need ordinary application-correctness evidence.

**Runtime census, 2026-10-13:** Linux x86-64, for both CI's ordinary cached
Firn build and its `--par` test build ([Makefile](../Makefile#L51-L60)). The pinned
[driver's source lists][driver-lists] and [selection function][driver-select]
select the same ordinary library, scheduler and completion units in both;
Firn's boxed storage also requires `heap.c`. There are **14 independent C
compiler inputs**, plus `concurrent_map.c`, text-included by
[`keyed_table.c:46`][keyed]. Each file was read from the pinned Whitefoot Git
object and counted once using:

```sh
git show af7665be672d29912d878dd5da8c3932bb6f296c:compiler/src/backend/<path> | wc -l
```

This counts physical lines, including comments, blanks and inactive
preprocessor branches. Paths below are relative to the pinned
[`compiler/src/backend/`][runtime-tree]. No local build/test was used to
produce the census; it measures the selected source files, not binary size or
verified code. The runtime figure in the page's [size table][claims-tcb]
counts the units of every platform, at a Whitefoot commit after the pin, so it
is not comparable with this Linux census at the pin.

| Participating C file | Physical lines |
| --- | ---: |
| `wf_floor.c` | 360 |
| `ordinary_values.c` | 2,084 |
| `heap.c` | 69 |
| `sched/core.c` | 1,478 |
| `sched/prim_host.c` | 348 |
| `sched/entry.c` | 173 |
| `completion/runtime.c` | 292 |
| `completion/wait_host.c` | 168 |
| `completion/file_adapter.c` | 1,120 |
| `completion/file_posix.c` | 791 |
| `completion/bridge.c` | 6,264 |
| `completion/stop_signals.c` | 500 |
| `completion/linux_io_uring.c` | 1,517 |
| `keyed_table.c` | 353 |
| `concurrent_map.c` (included once) | 3,226 |
| **C total: 15 files** | **18,743** |

| Staged runtime header | Physical lines | Included on Linux |
| --- | ---: | --- |
| `ordinary_values.h` | 285 | Yes |
| `sched/core.h` | 27 | Yes |
| `sched/prim.h` | 69 | Yes |
| `sched/entry.h` | 13 | Yes |
| `completion/contract.h` | 558 | Yes |
| `completion/file_adapter.h` | 490 | Yes |
| `completion/bridge.h` | 685 | Yes |
| `completion/file_posix.h` | 149 | Yes |
| `completion/socket_address.h` | 270 | Yes |
| `completion/linux_io_uring.h` | 228 | Yes |
| `concurrent_map.h` | 271 | Yes |
| `windows_runtime.h` | 232 | No; Windows branch |
| `completion/windows_iocp.h` | 179 | No; Windows branch |
| **Linux header total: 11 files** | **3,045** | |
| **All staged headers: 13 files** | **3,456** | |

C plus Linux headers totals **21,788 lines**; counting the two merely staged
Windows headers instead gives **22,199**. The separately compiled
`ordinary_values.ll` has **1,036 physical lines**, outside the C/header total.
Compiler/checker Rust, emitted Firn LLVM, system headers, libc, LLVM and the
OS are outside this runtime census.

[CI](../.github/workflows/check.yml#L21) selects **Ubuntu 24.04** and installs
**LLVM/clang/LLD major 22** through the pinned release's manifest
([release LLVM pin][llvm-pin], [kit toolchain target](../whitefoot-kit/whitefoot.mk#L77)).
A [passing PR #102 network job][ci-job] recorded **Ubuntu 24.04.5 LTS**,
runner image **20261004.327.1**, **clang/LLD 22.1.8**, and the Ubuntu
**glibc 2.39** series (`libc-bin 2.39-0ubuntu8.9`). These are observed versions,
not patch-version pins. The workflow does not fix the runner image, kernel or
libc patch; it prints clang/LLD versions but no exact linked-libc version.
That [runner image's manifest][runner-image] lists Linux kernel
**6.17.0-1022-azure**; the cited network job did not independently record `uname`.
This document assigns no line counts to these external tools/libraries or the
OS; the page's [size table][claims-tcb] gives snapshot sizes for the compiler
and LLVM.

[spec]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md
[scope]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L7-L21
[storage]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L825-L935
[arithmetic]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1013-L1075
[wait]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2268-L2287
[shared]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2321-L2360
[host]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2363-L3216
[claims]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md
[claim-mem]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#no-memory-corruption
[claim-race]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#no-data-races
[claim-overflow]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#no-silent-overflow
[claim-trap]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#no-runtime-traps
[not-claimed]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#not-claimed
[claims-tcb]: https://github.com/Ming-Research/Whitefoot/blob/4b58ba6dc219a19a5e8c98778faf3ce5b8b20982/docs/safety-claims.md#the-trusted-base
[halo-limit]: https://github.com/Ming-Research/Halo-wf/blob/58723031b1a6d58a932643d72c436026898417a1/lib/halo/embed/module.wfm#L39-L69
[audit-pr]: https://github.com/Ming-Research/Firn-wf/pull/102
[accept-pr]: https://github.com/Ming-Research/Firn-wf/pull/104
[board]: https://claude.ai/artifact/7tocXS3iUdthCLCQCMd3ip
[witness-pr]: https://github.com/Ming-Research/Firn-wf/pull/85
[witnesses]: https://github.com/Ming-Research/Firn-wf/tree/a3253ee93150db37958f60ca5c5e9e18037f3742/research/experiments/safety-witnesses
[witness-run]: https://github.com/Ming-Research/Firn-wf/actions/runs/38411761425
[driver-lists]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/bin/whitefootc.rs#L89-L190
[driver-select]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/bin/whitefootc.rs#L891-L923
[runtime-tree]: https://github.com/Ming-Research/Whitefoot/tree/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend
[keyed]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/compiler/src/backend/keyed_table.c#L46
[llvm-pin]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/.github/llvm-major
[ci-job]: https://github.com/Ming-Research/Firn-wf/actions/runs/38415303795/job/115358051890
[runner-image]: https://github.com/actions/runner-images/blob/ubuntu24/20261004.327/images/ubuntu/Ubuntu2404-Readme.md
