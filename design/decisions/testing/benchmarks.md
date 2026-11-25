# Benchmarks

How a Redis benchmark runs on the shared self-hosted machine: what it stops when it ends, which CPUs servers, clients and the profiler take, which passes and profiled runs it voids and runs again for other load on the machine, and how each firn image is started.

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

### decision benchmark-voids-passes-run-beside-other-jobs
A comparison pass or profiled run during which the CPUs outside the server, client and profiler CPUs averaged busier than the threshold its record states is void and is run again.
- rejected: an agent proposed that the owner keep the 14900K free of other jobs for each measurement; the owner rejected it, because it needs the owner at every measurement, stops other works for about 45 minutes, and protects no later measurement.
- rejected: an agent proposed a machine lock in Whitefoot-kit that every project's 14900K tests share and a measurement takes alone; the owner rejected it, because it needs a separate kit item this work would wait for, and every agent's tests would queue for up to an hour during a measurement.
