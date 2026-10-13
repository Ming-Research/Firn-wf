# Firn's safety guarantees

These claims describe Firn built with [its Whitefoot pin](../whitefoot.pin),
`wf-af7665be672d`: [specification v0.146 at the pinned commit][spec]. They are
conditional on a correct compiler, checker, linked definitions, runtime,
allocator and OS ([SCOPE-3][scope]). They do not establish Redis compatibility,
durability of every acknowledged write, or continuous availability.

## Guarantees and their boundaries

| Property | What the checker and specification provide | What remains outside the claim |
| --- | --- | --- |
| **No data races in accepted Whitefoot code** | Spawn transfers owned arguments; shared state is accessed through atomic statements with exclusive, single-point semantics. Automatic overlap must satisfy the interference and reduction rules of PAR-1/PAR-2. [WAIT-3][wait], [SHARE-2, SHARE-3][shared], [EFF-5][effects], [PAR-1, PAR-2][parallel]. | General race conditions between separate transitions and application protocol mistakes remain possible; native runtime synchronization is trusted. [CAP-1][parallel], [SCOPE-3][scope]. |
| **No use-after-free** | Consuming an owner kills its binding. References are invalidated by owner replacement, move, release or scope exit; invalid uses reject compilation. References cannot escape into stored values or return values. [OWN-1, REF-2, REF-3][ownership], [STOR-3][storage]. | Correct implementation of generated releases and native handle lifetimes is trusted. Resource termination has no cleanup guarantee. [SCOPE-3][scope], [STOR-3][storage]. |
| **No out-of-bounds access** | Each element index must be proved below the initialized length; slices require proved endpoints. Unproved accesses reject compilation; accepted accesses need no runtime bounds fallback. Target layout/address arithmetic must also qualify. [OP-4][bounds], [REF-4][ownership], [STOR-6][storage]. | Acceptance proves neither that available memory can serve an allocation nor that stack frames fit the execution environment. [STOR-6, STOR-8][storage]. |
| **No overflow of exact integer arithmetic** | Plain `+`, `-`, `*` require a representable result. Plain `/`, `%` also exclude zero divisors and signed minimum/−1; unproved domains reject compilation. [OP-2][arithmetic]. | Explicit wrap and saturating operations have their specified semantics; checked forms return errors. This is not a claim that every counter is mathematically unbounded or that floating-point results never become infinity/NaN. [OP-2, OP-3][arithmetic]. |
| **No source panic or proof-failure trap** | No writer-reachable abort effect, exception, unwinding edge or hidden proof fallback exists; proof failure is a compile-time rejection. Expected errors are values. [SCOPE-2][scope], [EFF-4][effects], [ERR-1][errors]. | Firn can deliberately return a nonzero entry status, encounter I/O errors, or be terminated by resource failure or the host. [PRE-2][host], [SCOPE-3][scope]. |
| **Conditional progress; no general deadlock guarantee** | An atomic guard/body cannot nest an atomic statement or call a waiting function, excluding those hold-and-wait forms. If every context reaches completion or a wait in finitely many steps from every point, WAIT-2 promises eventual next steps for runnable contexts or waits with a produced host outcome/completed joined context, and eventual unguarded or persistently enabled atomic transitions. [SHARE-2][shared], [WAIT-2][wait]. | False guards, joins on contexts that never complete, missing host outcomes or a context that never yields can still stall service. When all remaining contexts wait on false guards or other contexts and no host operation is outstanding, execution does not complete; an implementation may report and stop it. No latency bound or termination proof follows. [WAIT-2, WAIT-3][wait]. |

Firn has concrete waits for its script engine and for all clients to leave;
scripts and guarded waits can delay shutdown, and filesystem calls have no
deadline/cancellation parameter. See [script engine acquisition](../firn/scripting/entry.wf#L54),
[shutdown drain](../firn/server/server.wf#L1128), [orderly-stop design](../design/firn/orderly-stop.md)
and [PRE-2][host].

Heap exhaustion does not return a Whitefoot allocation error or execute a
language trap ([STOR-8][storage]). The pinned POSIX implementation attempts a
resource record and calls `abort()` (**SIGABRT**) on allocation refusal or a
classified stack-guard fault. The OS can kill the process first, and the
language specifies no exhaustion status, signal or cleanup guarantee.
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
builds each accepted correction. The [recorded passing run][witness-run] used
Whitefoot `d75f318d5631`, an earlier pin; accepted executables were **not run**.
These are specific source-admission examples, not a complete proof of Firn or
of the current compiler/runtime. The rules above are cited at Firn's current
pin.

The audit also found an accept error that can stop service with status zero
([pending correction in PR #104][accept-pr]), and unknown AOF commands whose error replies are
discarded ([board item `firn-bl-01-10`][board]). These are operational and
persisted-state correctness gaps; memory safety does not establish their
recovery policy or Redis semantics. Their source evidence is in the
[audit's findings](../research/investigations/firn/abnormal-stops.md#5-gaps-uncertainty-and-disposition).

## Trusted base and source sizes

The checker does not prove itself, the Whitefoot compiler/backend, the native
runtime or linked host functions correct. LLVM/clang/LLD must preserve the
generated semantics; libc's allocator, thread/signal functions and the Linux
kernel must honor their interfaces ([SCOPE-3][scope], [PRE-1, PRE-2][host],
[Whitefoot's trusted-base description][tcb]). Firn and Halo's checked Whitefoot
code still need ordinary application-correctness evidence.

**Runtime census, 2026-10-13:** Linux x86-64, for both CI's ordinary cached
Firn build and its `--par` test build ([Makefile](../Makefile#L49)). The pinned
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
verified code.

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
No line counts are assigned to these external tools/libraries or the OS.

[spec]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md
[scope]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L7-L21
[ownership]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L663-L740
[storage]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L825-L935
[bounds]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1076-L1099
[arithmetic]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1013-L1075
[effects]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1616-L1635
[errors]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L1638-L1658
[parallel]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2168-L2261
[wait]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2270-L2290
[shared]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2321-L2360
[host]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/spec/kernel-spec.md#L2363-L3216
[tcb]: https://github.com/Ming-Research/Whitefoot/blob/af7665be672d29912d878dd5da8c3932bb6f296c/README.md#L52-L56
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
