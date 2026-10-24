# Reproducing the performance comparison on Linux

This guide runs the redis-benchmark grid on your own Linux x86-64 machine,
without GitHub Actions. Use the Firn-wf revision attached to the published
comparison, including its submodule revisions and `whitefoot.pin`. The
[baseline rules](../research/investigations/perf-baseline/README.md) own the
comparison's workloads, sizing and verdicts; this guide supplies the commands.
The same method lets you compare ratios on different hardware; both ratios
and absolute throughput can change with the machine.

## Prerequisites and builds

CI runs this method on ubuntu-24.04 hosted runners and on the i9-14900K
(Ubuntu, kernel 7.0). The package list below is checked against Ubuntu 24.04
package names. The commands assume Ubuntu 24.04 and a user who can run
`sudo`. Install these packages:

<!-- reproduce: packages -->
```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential git curl ca-certificates python3 coreutils util-linux procps \
  iproute2 xz-utils tar pkg-config libicu-dev libssl-dev zlib1g \
  libgssapi-krb5-2 tzdata \
  sudo perl lsb-release wget gnupg software-properties-common
```

These provide the C build tools, archive tools, GNU core utilities, `taskset`,
`lscpu`, Python and the native libraries for Garnet's .NET runtime. No Rust
toolchain is needed for this prebuilt-image `compare` grid: `make firn-lto`
runs the downloaded `whitefootc`, and the compiler is a downloaded release, not
built from source. The separate Rust application-workload client (the
`workloads` mode) requires Cargo and Rust; it is outside the grid below.
`make toolchain` additionally needs `sudo`, `lsb-release`, `wget`, `gnupg` and
`software-properties-common` (apt.llvm.org's installer uses them), and `perl`
provides `shasum` for the compiler download's checksum. Optional `perf` profiling is disabled in these commands.

Set `FIRN_REVISION` to the published comparison's Firn-wf commit before this
block. Run the rest of this guide in Bash from the checkout root:

<!-- reproduce: build -->
```bash
: "${FIRN_REVISION:?Set FIRN_REVISION to the published comparison commit}"
git clone https://github.com/Ming-Research/Firn-wf.git
cd Firn-wf
git checkout "$FIRN_REVISION"
git submodule update --init --recursive
git rev-parse HEAD
git submodule status
cat whitefoot.pin

# Fetch the pinned Whitefoot compiler.
make compiler
# Install the LLVM major this pin's compiler was built with (clang and LLD).
# On a host that keeps its own LLVM, run `make toolchain-check` instead.
make toolchain
# Build the full-LTO server.
make firn-lto

# Build the pinned Redis 7.2.16 tools from their checked release archive.
REFERENCE_REDIS_BIN=$(bash tests/redis-suite/reference.sh bin)
export REFERENCE_REDIS_BIN

# An optional custom prefix must also be used for PATH and receipt lookup.
export COMPETITOR_PREFIX="$HOME/.local"
bash research/experiments/redis-bench/install-competitors.sh "$COMPETITOR_PREFIX"
export PATH="$REFERENCE_REDIS_BIN:$COMPETITOR_PREFIX/bin:$PATH"
```

`make toolchain` downloads apt.llvm.org's installer and, with sudo, installs
the LLVM major the pin's release names and points `/usr/bin/clang` and
`/usr/bin/ld.lld` at it. `make toolchain-check` only verifies the host's
clang major and fails on a mismatch. `make compiler` obtains the release named by `whitefoot.pin` through
Whitefoot-kit. `make firn-lto` produces `build/firn-lto`.

`reference.sh` builds Redis once into
`${REDIS_COMPAT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/whitefoot-redis-compat}`
and prints its tools' directory. Put it first on PATH: packaged Redis, or
another same-version executable, does not establish the reference provenance.

The competitor installer defaults to `~/.local` when called without an
argument. It installs the pinned Valkey 9.1.2, Dragonfly 2.0.2 and Garnet 2.2.1
archives, verifies their SHA-256 pins, and writes executable-bound receipts
under `PREFIX/opt/<name>-<version>`, with launchers in `PREFIX/bin`. It requires
no sudo. Garnet selects its net8.0 apphost and reuses a compatible .NET 8 runtime
or obtains one in the prefix. The receipt binds the runtime executable; the
runtime acquisition itself is not an archive pin.

## CPU placement and host conditions

Keep the host idle apart from the benchmark. Arrange exclusive access for the
whole invocation; the standalone script does not acquire a host-wide lock.
Record the CPU topology before choosing lists:

<!-- reproduce: topology -->
```bash
lscpu
lscpu -e=CPU,SOCKET,CORE,ONLINE
```

On a hybrid CPU, put servers on performance cores and clients on separate
efficiency cores, following the published placement's shape. CPU numbering is
machine-specific: inspect sibling sets and the performance/efficiency class
lists under `/sys/devices/` before choosing numbers. On an all-big-core CPU,
reserve distinct physical cores for the clients. Keep the client list fixed
across every server-count cell, preferably 16 logical CPUs if the host has
room. A server count means logical CPUs: the 1/2/4/8 cells use one thread of
each of 1/2/4/8 physical cores; the 16 cell uses both threads of those eight
server cores. A machine without that topology cannot reproduce every cell
with the same placement.

`SERVER_CPU_LIST` and `CLIENT_CPU_LIST` accept comma-separated CPU numbers
and ranges. The script requires exactly the requested number of server CPUs,
online topology for every selected CPU, and physical-core separation between
servers and clients, including hyperthread siblings. Explicit lists may use
either core class and core 0; the script records their classes but does not
enforce the reference machine's class choice. A missing independent client
core skips the cell. Invalid or overlapping lists fail.

Without explicit lists, automatic placement selects one thread per physical
performance core in topology order, reserves `core_id` 0, and puts clients
on remaining eligible cores, using their sibling threads too. Without hybrid
information it uses all physical cores. It warns and caps the client count
when fewer CPUs remain, and skips server counts that need too many physical
cores. Automatic placement therefore does not recreate the eight-core and
16-thread reference cells: use explicit lists for this grid.

`host.txt` records the kernel command line only through an allow-list of
performance-relevant parameters (isolcpus, nohz_full, rcu_nocbs, irqaffinity,
mitigations, nosmt, intel_pstate, amd_pstate, idle, the C-state limits,
transparent_hugepage, hugepages, default_hugepagesz, threadirqs, preempt,
nmi_watchdog, skew_tick), or `none`; the raw command line, which can hold disk
identifiers or secrets, and the host name are never recorded.

The reference i9-14900K's lists below are **not portable CPU numbers**. Edit
the server-list variables and the client list for your topology before
running them. Smaller reference cells leave core 0 to the OS; the largest
two use every performance core.

| Logical server CPUs | Reference server list | Reference client list |
|---|---|---|
| 1 | `2` | `16-31` |
| 2 | `2,4` | `16-31` |
| 4 | `2,4,6,8` | `16-31` |
| 8 | `0,2,4,6,8,10,12,14` | `16-31` |
| 16 | `0-15` | `16-31` |

Host settings are recorded, never changed by the instrument. Compare your
`host.txt` with the reference table below, including governors, turbo, SMT,
hugepages and kernel boot options. Report differences rather than silently
assuming the hosts are configured alike.

## Probe, then run the grid

This block defines a reusable command with all comparison settings explicit.
Each call gets a separate output directory. Use fresh labels if repeating a
call: measurement CSVs are overwritten on another invocation with the same
`OUT`. Paths in `IMAGES` must not contain whitespace because image pairs are
space-separated.

<!-- reproduce: probe -->
```bash
# Edit these lists for your host; these defaults belong to the reference 14900K.
SERVER_1=2
SERVER_2=2,4
SERVER_4=2,4,6,8
SERVER_8=0,2,4,6,8,10,12,14
SERVER_16=0-15
export CLIENT_CPU_LIST=16-31
export IMAGES="firn=$PWD/build/firn-lto firn-twin=$PWD/build/firn-lto"
export COMPARE_CLIENT=procs
export COMPARE_DEPTHS="1 16"
export COMPARE_AOF=false
export COMPARE_REFERENCE=false
export COMPARE_MAXMEMORY=
export IMAGE_DRIVERS=
export PERF=
unset BASELINE_ROOT WF_SCHED_REPORT FIRN_REDIS_BENCH_RECORD
BASELINE_LINES="reference reference-io valkey valkey-io dragonfly garnet"

run_cell() {
  local label=$1 count=$2 servers=$3 tests=$4 passes=$5 seconds=$6 lines=$7
  local cell_out="$PWD/build/benchmark-results/$label-$count"
  mkdir -p "$cell_out"
  OUT="$cell_out" SERVER_CPU_LIST="$servers" \
    COMPARE_CPUS="$count" COMPARE_TESTS="$tests" \
    COMPARE_PASSES="$passes" COMPARE_SECONDS="$seconds" \
    COMPARE_LINES="$lines" \
    sh research/experiments/redis-bench/redis-bench.sh compare \
    2>&1 | tee "$cell_out/run.log"
}

# Two server-count probes, two depths, two numbered passes of about five seconds.
run_cell probe 2 "$SERVER_2" "set get mset" 2 5 "$BASELINE_LINES probe"
run_cell probe 8 "$SERVER_8" "set get mset" 2 5 "$BASELINE_LINES probe"
```

The `probe` token permits recording unverified versions and missing Dragonfly
or Garnet. A full run omits that token and refuses absent executables, version
mismatches or invalid provenance receipts. Repair those before publishing a
comparison. Redis and Valkey's I/O-thread lines are omitted at one CPU because
they equal their plain lines there; at higher counts the I/O thread count
equals the server CPU count. Every line receives the same server affinity.
firn's driver count equals that count; the empty `IMAGE_DRIVERS` preserves it.

Evaluate the probe under [rule 5](../research/investigations/perf-baseline/README.md#rule-5-run-sizing).
With twin spread at most 3% and line spread at most 5% in every probe cell,
use three passes of five seconds. With twin spread above 3% but at most 6%,
or line spread above 5% but at most 10%, use five passes of ten seconds.
Above those bounds, investigate the noise before a full run. Then run each
server-count cell separately:

<!-- reproduce: skip: the full grid takes hours; its run_cell is the probe's -->
```bash
# Choose this pair from the probe evidence:
GRID_PASSES=3
GRID_SECONDS=5
# For the noisier allowed case, use GRID_PASSES=5 and GRID_SECONDS=10 instead.
run_cell baseline 1 "$SERVER_1" SUITE_DEFAULT "$GRID_PASSES" "$GRID_SECONDS" "$BASELINE_LINES"
run_cell baseline 2 "$SERVER_2" SUITE_DEFAULT "$GRID_PASSES" "$GRID_SECONDS" "$BASELINE_LINES"
run_cell baseline 4 "$SERVER_4" SUITE_DEFAULT "$GRID_PASSES" "$GRID_SECONDS" "$BASELINE_LINES"
run_cell baseline 8 "$SERVER_8" SUITE_DEFAULT "$GRID_PASSES" "$GRID_SECONDS" "$BASELINE_LINES"
run_cell baseline 16 "$SERVER_16" SUITE_DEFAULT "$GRID_PASSES" "$GRID_SECONDS" "$BASELINE_LINES"
```

`SUITE_DEFAULT` selects the 19 tests named in the baseline record, at depths
1 and 16, 50 connections, 100,000 random keys and 3-byte values. The
instrument verifies each line, then sizes fixed request counts from pilots
using the fastest line's rate times `COMPARE_SECONDS`. That value is a sizing
target, not a wall-time limit for each line: slower servers take longer.
Pass order reverses on even numbered passes. No line uses persistence in
this grid.

Each invocation with competitor lines **automatically adds one single-process
threaded exact-p99 pass** for every line, test, depth and CPU count. It uses
`COMPARE_CLIENT=threads` internally and writes `pass=exact`; no second command
is needed. It reuses the sized request counts, so its duration can exceed
the requested seconds. Its throughput is diagnostic only; the numbered
multi-process passes determine throughput medians. Only its exact merged
latencies decide the p99 verdict.

The baseline record also defines application workloads with every-second AOF
and a session-memory comparison. The existing `workloads` selector runs
competitors without AOF; it does not yet provide the full application baseline's
persistence parity. The commands here reproduce the redis-benchmark grid and
its exact-p99 pass, and do not claim to complete those application/AOF cells.

## Results and comparison

The commands above write to `build/benchmark-results/<label>-<count>/`.
Without `OUT`, the script defaults to `/tmp/redis-bench`. Keep the complete
directory and the checkout revision with the report:

| File | Meaning |
|---|---|
| `host.txt` | System settings, tool/image identities, pin/provenance checks, placements and comparison settings; unavailable host values are `unknown`. |
| `run.log` | Verification, pilots, skips, warnings and measured rows. |
| `compare.csv` | One row per line/pass/server-count/test/depth. |
| `compare-percentiles.csv` | Per-process percentiles and the combined row, with connection/request allocations and quantile kind. |
| `compare-cpu.csv` | Per-CPU busy/user/system fractions and server/client role. |
| `compare-limited.csv` | Mean client busy fraction and the client-limited flag, set above 0.90. |
| `compare-memory.csv` | Used memory, RSS and process high-water RSS after each server pass, in bytes. |
| `compare-server-*.txt` | The server log for each line, CPU count and pass. |

`compare.csv` has columns
`line,pass,cpus,test,depth,requests,rps,server_cpu_us_per_request,p50_ms,p99_ms,quantiles`.
`rps` is the combined request rate over client wall time;
`server_cpu_us_per_request` is server user plus system CPU time per request.
Latencies are milliseconds. `quantiles=upper_bound` means the maximum of the
process percentiles, which bounds but does not reconstruct a merged
distribution. `quantiles=exact` means a single-process merged histogram (also
possible with one process in `procs` mode). Use `pass=exact` for the latency
comparison and numeric passes for throughput, without mixing their rates.

For each cell, choose the competitor with the highest median throughput over
the numeric passes, then divide firn's median by that competitor's median.
Compare p99 against that same competitor; also report the lowest competitor
p99. The twin is the same executable measured again, and never a competitor.
Its throughput spread is the largest paired difference between firn and
firn-twin over the numbered passes, divided by firn's rate. Use the equivalent
paired difference on the exact pass for p99 noise.

A reproduced ratio agrees within the noise evidence when its difference from
the published ratio is within the **sum of the two runs' twin spreads** in
that cell. Report both spreads. Ratios outside that range need investigation;
different host settings and client saturation can change them. Apply the
[baseline verdict rules](../research/investigations/perf-baseline/README.md#rule-6-verdicts-and-what-rejects-firn-leads),
including the client-limited restriction; this guide does not supply an
automatic verdict analyzer.

## Standalone operation

GitHub Actions normally prepares the pinned builds, installed competitors,
PATH, images and `OUT`. The commands above supply those directly. The
instrument requires no `GITHUB_*` variable or runner checkout path. The
installer's optional `RUNNER_TEMP` uses `/tmp` when unset. Reference caching
uses the home-directory default described above. `host.txt` is created by
the standalone instrument without a workflow's earlier inventory step.

The workflow's persistent `session.py` record/cleanup wrapper protects its
shared runner after a cancelled job; a normal standalone run on a dedicated
host does not need it. The script traps normal exit, INT and TERM to stop its
current server. SIGKILL or host failure can bypass those traps: inspect and
stop leftovers from your own run before another measurement. Do not set
`FIRN_REDIS_BENCH_RECORD` to a runner record when running independently.

## Checking this guide

Every `bash` block of this guide sits under a marker comment: a step CI runs
(`<!-- reproduce: NAME -->`, the steps being `packages`, `build`, `topology`
and `probe`) or a block it does not (`<!-- reproduce: skip: WHY -->`, here the
hours-long grid, whose `run_cell` is the probe's). The `reproduce` mode of the
`redis-bench.yml` workflow starts a fresh Ubuntu 24.04 container (docker) with
an administrator user, runs the marked blocks in order as one Bash script that
stops at the first failure, with `FIRN_REVISION` set to the dispatched commit,
and fails unless both probe cells have a measured rate for firn, its twin,
Redis, Valkey, Dragonfly and Garnet, with their I/O-thread lines. The
commands are those above, extracted by `research/experiments/redis-bench/guide-blocks.py`,
so a change to a block is checked as it stands and a block without a marker
fails `make guide-check`.

## Reference machine

Fill this table from the next native i9-14900K run's `host.txt`, keeping its
complete per-CPU records alongside the published results. No setting values
are assumed here.

| Recorded setting | Reference value |
|---|---|
| CPU model | TO-FILL |
| Microcode revision(s) | TO-FILL |
| Sockets, physical cores and logical threads | TO-FILL |
| Per-CPU topology and performance/efficiency core class | TO-FILL |
| Total memory | TO-FILL |
| Scaling governor per CPU class, including any per-CPU differences | TO-FILL |
| Energy-performance preference per CPU class, including any differences | TO-FILL |
| `intel_pstate` status and `no_turbo` | TO-FILL |
| `cpufreq` boost, when available | TO-FILL |
| SMT active/control | TO-FILL |
| Transparent hugepages enabled/defrag | TO-FILL |
| `vm.overcommit_memory` | TO-FILL |
| `vm.swappiness` | TO-FILL |
| Kernel command line, including `isolcpus`, `nohz_full` and mitigations | TO-FILL |
| irqbalance service active state | TO-FILL |
| `perf_event_paranoid` | TO-FILL |
| Kernel release | TO-FILL |
| Distribution release | TO-FILL |
| Loopback MTU | TO-FILL |
