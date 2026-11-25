# Benchmarks

How a Redis benchmark runs on the shared self-hosted machine: what it stops when it ends, which CPUs servers and clients take, how each firn image is started, and how the competitor lines, their durability, the client's connections and the p99 verdict are measured.

### decision benchmark-session-stops-only-what-it-registered
A measurement stops its session whole while the session's leader runs, and after that only the servers it registered, each through a pidfd, TERM and then KILL.
- rejected: an agent proposed matching leftover processes by command name; the owner rejected it, because other projects run the same servers on the machine.
- rejected: an agent proposed a token in each process's environment; the owner rejected it, because Redis 7.0.15 and Valkey overwrite the environment the kernel shows when they set their process titles, so their leftover servers escape the match.
- rejected: an agent proposed matching every process of the recorded session started within hours of its leader; the owner rejected it, because once the session's processes have all ended, another session can take its number, and its processes would match.
- rejected: an agent proposed a PID namespace, a systemd scope or a cgroup per measurement; the owner rejected it, because the runner's account can make none of them: user namespaces are refused, no user service manager runs, sudo is barred and its cgroup is not delegated.

### decision benchmark-leftover-clients-are-left-to-the-runner
A process of a measurement's session that is not a registered server and outlives the leader, such as a client or a profiler, is left for the runner to stop.
- rejected: an agent proposed stopping every process that still carries the session's number; the owner rejected it, because once the leader has ended that number can name another session, and only a registered process is known to be the measurement's.

### decision benchmark-cpus-one-thread-per-physical-performance-core
Benchmark servers take one thread per physical performance core in topology order, skipping core_id 0, and clients take both threads of the remaining performance cores.
- replaced: placing servers and clients on consecutive logical CPUs was on main; the owner changed it, because on the native i9-14900K adjacent logical CPUs are hyperthread siblings and later numbers are efficiency cores, so a two-CPU server ran on one core.

### decision benchmark-firn-invocation-follows-each-revision-source
The benchmark starts each firn image in the startup format its own revision's source has, config if it holds `firn/server/config.wf` and positional if not, and records it.
- rejected: an agent proposed one invocation for every image; the owner rejected it, because one comparison can hold revisions of both startup formats.
- rejected: an agent proposed inferring the format from merge ancestry; the owner rejected it, because a branch revision can carry a format before it merges.

### decision benchmark-competitors-run-in-compare-mode
Competitor servers are measured in compare mode, with the firn images' server CPU list, reversed pass order and durability settings.
- rejected: an agent proposed extending scale mode's fixed-order measurement to the competitors; the owner rejected it, because compare mode already carries the twin and client-busy observations the baseline rules require (data: `research/investigations/perf-baseline/README.md`).

### decision benchmark-competitor-lines-check-version-and-provenance-strictly
Selecting a competitor line makes the run check its version and archive provenance strictly; only a probe may record unverified versions and omit an absent Dragonfly or Garnet.
- rejected: an agent proposed allowing missing executables or unverified provenance in a full run; the owner rejected it, because a full comparison needs identified competitors, while a probe must first establish which tools are installed.

### decision benchmark-competitor-provenance-comes-from-installer-receipts
A competitor's provenance is checked through a receipt the installer writes, binding the pinned version, URL and archive SHA-256 to the executable's real path and content hash.
- rejected: an agent proposed accepting a same-version executable, or an unbound archive digest, as provenance; the owner rejected it, because the installer verifies the downloaded bytes before recording the files, while an archive hash alone does not identify the executable PATH selects.

### decision benchmark-garnet-runs-its-net8-apphost
The Garnet baseline runs the pinned release's net8.0 apphost on a compatible .NET 8 runtime, both selected explicitly and bound by the receipt.
- rejected: an agent proposed using the first executable met while unpacking the archive; the owner rejected it, because the release holds two framework-dependent single-file variants and its publish profile lists net8.0 first with its default output directory.

### decision benchmark-dragonfly-is-refused-in-aof-cells
A comparison cell with the append-only file on refuses Dragonfly, because Dragonfly keeps snapshots and no append-only log.
- rejected: an agent proposed comparing Dragonfly's snapshot durability with firn's every-second append-only file; the owner rejected it, because the two persist differently and the cell would not compare equal durability (data: `research/investigations/perf-baseline/README.md`).

### decision benchmark-garnet-aof-results-stay-durability-not-equal
Garnet's append-only results are labelled durability not equal until its pinned commit path is shown to synchronize the disk.
- rejected: an agent proposed treating Garnet's millisecond commit setting as evidence of equal durability; the owner rejected it, because the instrument's committed-offset and file checks show that Garnet logs, not that it calls fsync (data: `research/investigations/perf-baseline/README.md`).

### decision benchmark-client-opens-exactly-50-connections
The compare client divides exactly 50 connections across its client CPUs, for every line.
- replaced: three connections per client CPU, 48 on 16 CPUs, was on main; the owner changed it, because the baseline rules fix 50 client connections for every line.

### decision benchmark-multi-process-quantiles-are-upper-bounds
Compare rows from several client processes label their quantiles as upper bounds, the largest of the processes' own values.
- replaced: keeping only the first process's p50 and p99 was on main; the owner changed it, because redis-benchmark's CSV percentiles cannot reconstruct a merged distribution, so one process's value does not stand for the combined p99.
- rejected: an agent proposed averaging the processes' percentiles; the owner rejected it, because the maximum of the processes' values only bounds the combined p99, and an average bounds nothing.

### decision benchmark-exact-p99-comes-from-one-threaded-pass
A competitor comparison runs one more single-process threaded pass for every line, and only its exact merged p99 decides the latency verdict.
- rejected: an agent proposed judging p99 from the multi-process upper bounds; the owner rejected it, because a bound cannot decide whether firn's p99 is above a competitor's (data: `research/investigations/perf-baseline/README.md`).
