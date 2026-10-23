#!/bin/sh
# Measurements of firn (firn/) against Redis and its competitors, driven by
# redis-benchmark: Experiments 7 and 8 of the io-model investigation (SHARED.md
# and TIME-AND-FILES.md), run then on the subset firn grew from, and the
# criteria of the firn investigation (research/investigations/firn/DESIGN.md).
# It builds firn with the worktree's compiler, checks every line for
# correctness, then measures the lines interleaved and prints one CSV line per
# run.
#
#   sh redis-bench.sh             the correctness pass and Experiments 7 and 8
#   sh redis-bench.sh verify      only the correctness pass
#   sh redis-bench.sh invocation-selftest
#                                 check both recorded argument forms without
#                                 building or starting any server
#   sh redis-bench.sh placement-selftest
#                                 check placement against fake Linux topology
#                                 and the per-CPU busy reading against a fake
#                                 /proc/stat, without building or starting any
#                                 server
#   sh redis-bench.sh suite       the correctness pass and the firn criteria:
#                                 Redis 7.0's redis-benchmark default suite
#                                 (SUITE_DEFAULT) on every line
#   sh redis-bench.sh scale       the suite's lines on each server CPU count in
#                                 SCALE (default 2 4 8 16), the client on the
#                                 remaining physical performance cores, for
#                                 SCALE_TESTS at each
#                                 depth in SCALE_PIPELINES (default 16)
#   sh redis-bench.sh quick       firn (or firn-base with QUICK_LINE=firn-base)
#                                 on QUICK_CPUS server CPUs (default 4) against
#                                 the fastest of Garnet and Dragonfly, in under
#                                 two minutes: QUICK_TESTS (default all ten)
#                                 for QUICK_RUNS runs of QUICK_SECONDS each,
#                                 printing a table and marking each test below
#                                 QUICK_TARGET times the best other server;
#                                 QUICK_CLIENTS client processes (default one
#                                 per available client core up to 16), each
#                                 count with
#                                 its own kept reference
#   sh redis-bench.sh compare     prebuilt firn images against each other:
#                                 IMAGES lists name=path pairs, one image may
#                                 appear under two names as a noise control;
#                                 each pass starts every image in turn, the
#                                 order reversed on even passes, and runs
#                                 COMPARE_TESTS (default mset set get; any
#                                 test of the default suite, and linsert and
#                                 lmove, which compare_client defines) at each
#                                 depth in COMPARE_DEPTHS (default 16 1) for
#                                 COMPARE_SECONDS (default 10) each, on
#                                 each server CPU count in COMPARE_CPUS (default 1 2), for
#                                 COMPARE_PASSES passes (default 6); with PERF
#                                 naming a perf executable it then records a
#                                 flat profile of each image under
#                                 COMPARE_PROFILE (default mset at depth 16),
#                                 and with PERF_C2C set a perf c2c report of
#                                 the cache lines cores contend for;
#                                 on Linux it records each server and client
#                                 CPU's busy fraction per sample; each image
#                                 runs with as many Whitefoot drivers as server
#                                 CPUs, or IMAGE_DRIVERS when it is set
#                                 with COMPARE_MAXMEMORY set to a byte count
#                                 (a nonnegative integer without leading
#                                 zeros), every server of the run, twins
#                                 included, gets CONFIG SET maxmemory of that
#                                 value as soon as it answers PING, which must
#                                 reply OK and read back by CONFIG GET; the
#                                 run records it as setting,maxmemory=<value>
#                                 in its output and in host.txt. Unset or
#                                 empty sets nothing
#                                 with COMPARE_AOF=true every server starts
#                                 with the append-only file on (firn.aof,
#                                 whose only policy is
#                                 appendfsync everysec) in a fresh directory
#                                 that stop removes, has its automatic
#                                 rewrites switched off by CONFIG SET
#                                 auto-aof-rewrite-percentage 0 (read back),
#                                 and must report aof_enabled:1 in INFO
#                                 persistence, printed as aof,<line>,
#                                 aof_enabled=<n>, or the run fails; the run
#                                 records setting,aof=true (false otherwise)
#                                 in its output and in host.txt
#                                 with COMPARE_REFERENCE=true Redis 7.2.16
#                                 (redis-server, whose version is checked) is
#                                 measured as one more line named reference,
#                                 started after the images, with the append-only
#                                 file on (appendfsync everysec, a fresh
#                                 directory stop removes) when COMPARE_AOF=true;
#                                 the run records setting,reference=true (false
#                                 otherwise) in its output and in host.txt
#                                 the lrange_* tests fill their list outside the
#                                 timed span and send LRANGE themselves, since
#                                 redis-benchmark's own fill would be timed;
#                                 summary.py ratios compares two images' passes
#                                 in compare.csv, with a twin as the noise
#   sh redis-bench.sh workloads   the deployment-performance investigation's
#                                 std-only Rust client, built offline with
#                                 cargo in release mode; Redis and firn, each
#                                 without and with AOF, at depth 1 with
#                                 WORKLOAD_CONNECTIONS connections (default
#                                 50; several values measure each in turn)
#                                 and one thread per client CPU (up to 16
#                                 CPUs, as in scale); WORKLOAD_CPUS
#                                 (default 1 2), WORKLOAD_PASSES (default 3),
#                                 WORKLOAD_SECONDS (default 10), WORKLOADS
#                                 (default limiter-script limiter-tx setmany-tx
#                                 session-set session-get). Each workload gets
#                                 a fresh server; line order reverses on even
#                                 passes. Before session-get, fill 1,000,000
#                                 sessions and record VmRSS in KiB. With PERF
#                                 set, in pass 1 only, each firn line's server,
#                                 after all its measured runs of a workload,
#                                 runs that workload once more at each
#                                 connection count in turn under perf record,
#                                 unmeasured, and keeps each flat profile as
#                                 profile-<line>-<cpus>-<workload>-
#                                 <connections>.txt; Redis lines are not
#                                 profiled. Probe with
#                                 WORKLOAD_PASSES=2 WORKLOAD_SECONDS=5 first.
#                                 With WORKLOAD_IMAGES set, the lines are
#                                 Redis and each image IMAGES names, both
#                                 without AOF, instead of this checkout's
#                                 firn: a comparison of builds on the
#                                 consumer workloads, whose twins are noise
#                                 controls.
#                                 Opt-in evict-zipf measures cache-aside hits
#                                 under allkeys-lru at half the filled heap;
#                                 session-set-limits/session-get-limits compare
#                                 unlimited and 64 GiB on the same images,
#                                 with a twin of each. These start fresh for
#                                 every connection count and variant, and do
#                                 not run the optional perf pass. WORKLOAD_OPTIONS
#                                 passes client options as whitespace-separated
#                                 words (no shell quoting/evaluation); defaults:
#                                 --keys 1000000 --value-size 64 --zipf-s 0.99
#                                 for evict-zipf, --keys 100000 --value-size 200
#                                 for session limits; --warmup-seconds 5 and
#                                 --sample-ms 10 for both. Seed is the pass.
#                                 evict-zipf.csv keeps memory/hit observations;
#                                 all rates and latencies enter workloads.csv.
#                                 Opt-in handshake counts one fresh TCP
#                                 connection, HELLO 3, CLIENT SETINFO LIB-NAME
#                                 firn-workload, CLIENT SETINFO LIB-VER 1.0 and
#                                 PING as one operation, verifying every reply.
#                                 client-list keeps WORKLOAD_CONNECTIONS open
#                                 (default 50), sets LIB-NAME once,
#                                 then counts each depth-1 CLIENT LIST as one
#                                 operation; its bulk reply must contain at
#                                 least that many clients. Both use Redis
#                                 7.2.16, omit AOF and the optional perf pass,
#                                 and accept only --warmup-seconds (default 5)
#                                 and --sample-ms (default 10) in WORKLOAD_OPTIONS.
#                                 --sample-ms is accepted as a shared option;
#                                 no auxiliary sampler runs for these workloads,
#                                 and every completed operation is recorded.
#                                 Opt-in pubsub-channel and pubsub-pattern
#                                 measure publish/subscribe delivery: one
#                                 publisher and WORKLOAD_CONNECTIONS
#                                 subscribers (default 50), all on one channel,
#                                 each subscribed to it (channel) or to a pattern
#                                 that matches it (pattern). The publisher
#                                 keeps each depth of WORKLOAD_DEPTHS (default
#                                 16 1) PUBLISH commands unanswered and never
#                                 runs more than --window messages (default
#                                 1024) ahead of the slowest subscriber. Each
#                                 sample runs --warmup-seconds (default 5) and
#                                 then WORKLOAD_SECONDS; pubsub.csv has
#                                 line,pass,cpus,workload,connections,pipeline,
#                                 published,delivered,seconds,rate,p50_ms,p99_ms
#                                 where rate is the messages delivered to
#                                 subscribers a second and the latencies run from
#                                 the publisher's write to the last subscriber's
#                                 read, and pubsub-limited.csv has the
#                                 client_work_fraction and client_limited of each
#                                 sample as above, except that a subscriber
#                                 thread's work is its reads that returned data
#                                 and their parsing, since one scan polls many
#                                 sockets. A line whose server answers
#                                 SUBSCRIBE with an unknown-command error is
#                                 skipped for the workload, except firn, which
#                                 must support it. With PERF naming a perf
#                                 executable, each firn line's server, after
#                                 all its measured runs of a workload in pass 1,
#                                 runs the workload once more at each depth
#                                 under perf record, unmeasured, and keeps each
#                                 flat profile as profile-<line>-<cpus>-
#                                 <workload>-<connections>-<depth>.txt.
#                                 Redis 7.2.16 is required,
#                                 WORKLOAD_OPTIONS accepts --warmup-seconds,
#                                 --value-size (default 64) and --window, and the
#                                 default lines are Redis and firn.
#                                 summary.py reads these files.
#                                 Opt-in blocking-handoff measures the hand-off
#                                 of list elements to blocked clients:
#                                 WORKLOAD_CONNECTIONS consumers (default 50)
#                                 each hold BLPOP on a key of their own, and one
#                                 producer LPUSHes each depth of WORKLOAD_DEPTHS
#                                 (default 16 1) elements at a time, each to a
#                                 consumer that has received its previous one,
#                                 so that no key holds more than one element. It
#                                 runs as the pub/sub workloads do and writes
#                                 the same files, with rate the elements
#                                 received a second and the latencies running
#                                 from the producer's write to the consumer's
#                                 read. It accepts --warmup-seconds and
#                                 --value-size in WORKLOAD_OPTIONS, not
#                                 --window, skips a server that answers BLPOP
#                                 with an unknown-command error, and keeps each
#                                 firn line's server log as workload-server-
#                                 <line>-<cpus>-<pass>-<workload>.txt, which
#                                 ends with the Whitefoot runtime's scheduling
#                                 counts when WF_SCHED_REPORT is set.
#                                 Where /proc/stat exists, workload-limited.csv has
#                                 line,pass,cpus,workload,connections,
#                                 client_busy_mean,client_work_fraction,
#                                 client_limited per sample. client_work_fraction
#                                 is the client's own accounting (loop time in
#                                 scans that moved bytes or finished a request,
#                                 over all loop time, from --client-work-file) and
#                                 alone sets client_limited (1 above 0.90).
#                                 client_busy_mean is the /proc/stat busy mean
#                                 over the client CPUs, recorded only: the
#                                 spinning client keeps its CPUs busy anyway.
#                                 Nonempty WORKLOAD_SERVERS selects, in order,
#                                 reference redis-latest firn valkey valkey-io
#                                 dragonfly garnet valkey-latest valkey-io-latest
#                                 dragonfly-latest garnet-latest (no duplicates);
#                                 every selected line omits AOF and absent
#                                 competitors are skipped. A name ending in
#                                 -latest is the newest stable release of its
#                                 project, REDIS_LATEST, VALKEY_LATEST,
#                                 DRAGONFLY_LATEST and GARNET_LATEST naming the
#                                 servers (latest.sh builds or extracts them).
#                                 firn/dragonfly/garnet use the current CPU
#                                 count. With WORKLOAD_IMAGES, selecting firn
#                                 expands every IMAGES entry, including twins,
#                                 beside the selected competitors. Without a
#                                 selector, image behavior stays as above;
#                                 without images, handshake/client-list default
#                                 to reference and firn at the current count.
#
# firn is built with the options FIRN_LINK names, --full-lto when it is unset;
# the records before the quick mode built it with none. The redis-bench
# workflow (.github/workflows/redis-bench.yml) runs compare, scale and workloads
# modes on the owner's i9-14900K.
#
# BASELINE_ROOT, when set, is a worktree of the revision before expiry with its
# compiler built; its subset is measured as the baseline lines of Experiment 8.
# DRAGONFLY and GARNET name those servers' executables; the suite skips a line
# whose executable is absent and says so. FIRN_BASELINE, when set, names
# another firn executable, such as one built from an earlier revision, which
# the suite measures as the firn-base lines beside firn. Its base row, and
# every IMAGES stem (a -twin uses its stem), must have exactly one row in
# REVISIONS_FILE (default OUT/revisions.csv): revision,name,commit,pin,invocation.
# invocation is config when the built source has firn/server/config.wf, otherwise
# positional; an optional sixth column holds kept-image provenance. This
# checkout records its own built source as firn; standalone
# callers supplying images or FIRN_BASELINE must provide their source records.
#
# Placement reads Linux topology under SYSFS_ROOT (default /sys), preferring
# performance cores from Intel hybrid CPU lists or the highest cpu_capacity.
# Without hybrid information it uses all cores. In CPU-number order it takes
# one thread per physical core, leaving core_id 0 for the OS and runner.
# Servers take the first n cores and clients the next cores, capped to those
# remaining; neither side shares a physical core with the other. On the
# native 14900K this gives server 2,4 and client 6,8,10,12,14 for n=2 with
# up to 16 client threads requested. SERVER_CPU_LIST and CLIENT_CPU_LIST,
# when set and not empty, override the respective lists (CPU numbers or
# ranges, comma-separated), which may then name any online CPU, core_id 0
# included. The server list must contain exactly n CPUs and may name both
# threads of a physical core, so n counts logical CPUs; no client CPU may
# share a physical core with a server CPU. The client thread count is the
# client list's length. Each selected placement is recorded once per mode,
# also in host.txt when present, with each CPU's core class (cpu:P for a
# performance core, cpu:E for an efficiency core, cpu:- when the host has no
# hybrid topology). The class is recorded, never enforced: keeping the shape of
# the owner's placement (servers on P-cores, clients on E-cores) is the
# dispatcher's job through the CPU lists, because hosts without a hybrid
# topology must still run. Nothing else should run on the host meanwhile.
set -e
# One argument constructor for both source generations. The remaining arguments
# are a command prefix, including exec at production call sites: the registered
# subshell becomes the server itself, with no intervening shell process.
firn_command() {
    firn_invocation=$1 firn_port=$2 firn_aof=$3 firn_filename=$4
    firn_clients=$5 firn_idle=$6 firn_bind=$7
    shift 7
    case $firn_aof in yes|no) ;; *) echo "invalid firn AOF setting: $firn_aof" >&2; return 1 ;; esac
    case $firn_invocation in
        positional)
            if [ "$firn_aof" = no ]; then firn_filename=-; fi
            set -- "$@" "$firn_port" "$firn_clients" "$firn_filename" "$firn_idle"
            ;;
        config)
            set -- "$@" --port "$firn_port" --firn-clients "$firn_clients" --appendonly "$firn_aof"
            if [ "$firn_aof" = yes ]; then set -- "$@" --appendfilename "$firn_filename"; fi
            set -- "$@" --timeout "$firn_idle"
            ;;
        *) echo "invalid firn invocation: $firn_invocation" >&2; return 1 ;;
    esac
    if [ -n "$firn_bind" ]; then set -- "$@" --bind "$firn_bind"; fi
    "$@"
}

invocation_selftest() (
    # Render each argument as a shell literal, preserving argument boundaries.
    render_command() {
        render_separator=
        for render_arg do
            printf "%s'" "$render_separator"
            while :; do
                case $render_arg in
                    *"'"*)
                        printf "%s'\\''" "${render_arg%%\'*}"
                        render_arg=${render_arg#*\'}
                        ;;
                    *) printf "%s'" "$render_arg"; break ;;
                esac
            done
            render_separator=' '
        done
        printf '\n'
    }
    check_command() {
        expected=$1
        shift
        actual=$(firn_command "$@" render_command exec taskset -c 2,4 '/tmp/firn image')
        if [ "$actual" != "$expected" ]; then
            printf 'invocation-selftest: expected %s, got %s\n' "$expected" "$actual" >&2
            exit 1
        fi
        printf '%s\n' "$actual"
    }
    check_command "'exec' 'taskset' '-c' '2,4' '/tmp/firn image' '6380' '17' '-' '23'" \
        positional 6380 no 'file with spaces.aof' 17 23 ''
    check_command "'exec' 'taskset' '-c' '2,4' '/tmp/firn image' '6380' '17' 'file with spaces.aof' '23' '--bind' '127.0.0.1'" \
        positional 6380 yes 'file with spaces.aof' 17 23 127.0.0.1
    check_command "'exec' 'taskset' '-c' '2,4' '/tmp/firn image' '--port' '6380' '--firn-clients' '17' '--appendonly' 'no' '--timeout' '23' '--bind' '127.0.0.1'" \
        config 6380 no 'file with spaces.aof' 17 23 127.0.0.1
    check_command "'exec' 'taskset' '-c' '2,4' '/tmp/firn image' '--port' '6380' '--firn-clients' '17' '--appendonly' 'yes' '--appendfilename' 'file with spaces.aof' '--timeout' '23'" \
        config 6380 yes 'file with spaces.aof' 17 23 ''
    if firn_command unknown 6380 no ignored 17 23 '' render_command >/dev/null 2>&1; then
        echo 'invocation-selftest: accepted unknown invocation' >&2
        exit 1
    fi
    if firn_command config 6380 unknown ignored 17 23 '' render_command >/dev/null 2>&1; then
        echo 'invocation-selftest: accepted unknown AOF setting' >&2
        exit 1
    fi
    echo 'invocation-selftest: passed'
)

if [ "${1:-}" = invocation-selftest ]; then
    invocation_selftest
    exit 0
fi

# The workflow passes an empty WF_SCHED_REPORT when its input is off; unset
# it so firn runs exactly as it does without the variable.
[ -n "${WF_SCHED_REPORT:-}" ] || unset WF_SCHED_REPORT

# Python already serves the workload checks and measurement session helpers;
# use it here to parse sysfs CPU ranges and sibling sets without shell eval.
# Exit 2 means this CPU count leaves too few physical cores; callers that
# sweep counts skip it. Invalid topology or overrides fail with exit 1.
placement() {
    python3 - "${SYSFS_ROOT:-/sys}" "$1" "$2" <<'PY'
import os
from pathlib import Path
import re
import sys

def cpu_list(text):
    result = []
    for part in text.strip().split(","):
        if not re.fullmatch(r"[0-9]+(?:-[0-9]+)?", part):
            raise ValueError(f"invalid CPU list: {text!r}")
        bounds = [int(x) for x in part.split("-")]
        first, last = bounds[0], bounds[-1]
        if last < first:
            raise ValueError(f"invalid CPU range: {part}")
        result.extend(range(first, last + 1))
    if len(set(result)) != len(result):
        raise ValueError(f"duplicate CPU in list: {text!r}")
    return result

def choose():
    root = Path(sys.argv[1])
    n, requested = map(int, sys.argv[2:])
    if n < 1 or requested < 1:
        raise ValueError("server and client CPU counts must be positive")
    # An empty override, as the workflow passes for an empty input, is unset.
    overrides = {key: cpu_list(os.environ[key]) for key in
                 ("SERVER_CPU_LIST", "CLIENT_CPU_LIST") if os.environ.get(key)}
    if "SERVER_CPU_LIST" in overrides and len(overrides["SERVER_CPU_LIST"]) != n:
        raise ValueError(f"SERVER_CPU_LIST must contain exactly {n} CPUs")

    base = root / "devices/system/cpu"
    online_path = base / "online"
    online = set(cpu_list(online_path.read_text())) if online_path.exists() else None
    cores, core_ids, capacities = {}, {}, {}
    for path in base.glob("cpu[0-9]*"):
        if not re.fullmatch(r"cpu[0-9]+", path.name):
            continue
        cpu = int(path.name[3:])
        if online is not None and cpu not in online:
            continue
        if (path / "online").exists() and (path / "online").read_text().strip() == "0":
            continue
        topology = path / "topology"
        core_ids[cpu] = int((topology / "core_id").read_text())
        siblings = topology / "thread_siblings_list"
        if not siblings.exists():
            siblings = topology / "core_cpus_list"
        if siblings.exists():
            cores[cpu] = ("siblings", tuple(sorted(cpu_list(siblings.read_text()))))
        else:
            # core_id is scoped to a package on multi-socket hosts.
            cores[cpu] = ("core", int((topology / "physical_package_id").read_text()), core_ids[cpu])
        capacity = path / "cpu_capacity"
        if capacity.exists():
            capacities[cpu] = int(capacity.read_text())
    if not cores:
        raise ValueError(f"no online CPU topology under {base}")

    source = "fallback"
    candidates = set(cores)
    performance = root / "devices/cpu_core/cpus"
    efficiency = root / "devices/cpu_atom/cpus"
    # Core class of every online CPU for the record: P, E, or - when the host
    # has no hybrid topology. It is recorded, never enforced (see the header).
    core_class = {cpu: "-" for cpu in cores}
    if performance.exists():
        performance_cpus = set(cpu_list(performance.read_text()))
        candidates &= performance_cpus
        source = "hybrid"
        core_class = {cpu: "P" if cpu in performance_cpus else "E" for cpu in cores}
    elif efficiency.exists():
        efficiency_cpus = set(cpu_list(efficiency.read_text()))
        candidates -= efficiency_cpus
        source = "hybrid"
        core_class = {cpu: "E" if cpu in efficiency_cpus else "P" for cpu in cores}
    elif len(capacities) == len(cores) and len(set(capacities.values())) > 1:
        candidates = {cpu for cpu in cores if capacities[cpu] == max(capacities.values())}
        source = "hybrid"
        core_class = {cpu: "P" if cpu in candidates else "E" for cpu in cores}
    # Representatives are ordered by logical CPU number, one per physical
    # core. Do not fill a large request with second threads or efficiency cores.
    representatives, seen = [], set()
    for cpu in sorted(candidates):
        if core_ids[cpu] != 0 and cores[cpu] not in seen:
            representatives.append(cpu)
            seen.add(cores[cpu])

    # Automatic servers are one thread per physical core; an explicit server
    # list may name both threads of a core, so that n counts logical CPUs.
    servers = overrides.get("SERVER_CPU_LIST", representatives[:n])
    if any(cpu not in cores for cpu in servers):
        raise ValueError("SERVER_CPU_LIST names a CPU without online topology")
    if len(servers) < n:
        print(f"skip,{n} server CPUs,only {len(representatives)} eligible physical cores", file=sys.stderr)
        return 2
    occupied = {cores[cpu] for cpu in servers}
    # Clients may use both threads of every eligible core the servers do not
    # occupy: a client thread never shares a physical core with a server.
    remaining = [cpu for cpu in sorted(candidates)
                 if core_ids[cpu] != 0 and cores[cpu] not in occupied]
    clients = overrides.get("CLIENT_CPU_LIST", remaining[:requested])
    if any(cpu not in cores for cpu in clients):
        raise ValueError("CLIENT_CPU_LIST names a CPU without online topology")
    if any(cores[cpu] in occupied for cpu in clients):
        raise ValueError("CLIENT_CPU_LIST overlaps a server physical core (including siblings)")
    if not clients:
        print(f"skip,{n} server CPUs,no distinct client core remains", file=sys.stderr)
        return 2
    if "CLIENT_CPU_LIST" not in overrides and len(clients) < requested:
        print(f"warning,placement,requested {requested} client threads,only {len(clients)} client CPUs remain; using {len(clients)}", file=sys.stderr)
    if overrides:
        source = "override"
    classes = [",".join(f"{cpu}:{core_class[cpu]}" for cpu in cpus) for cpus in (servers, clients)]
    print(",".join(map(str, servers)), ",".join(map(str, clients)), len(clients), source, *classes)
    return 0

try:
    sys.exit(choose())
except (OSError, ValueError) as error:
    print(f"placement: {error}", file=sys.stderr)
    sys.exit(1)
PY
}

# Assign lists in the calling shell; print each configuration once, even when
# suite switches between the same counts on successive passes.
placements_seen=
select_placement() {
    if placement_result=$(placement "$1" "$2"); then
        set -- $placement_result
        SERVER_CPUS=$1
        CLIENT_CPUS=$2
        CLIENT_THREADS=$3
        placement_record="placement,server=$1,client=$2,threads=$3,source=$4,server_class=$5,client_class=$6"
        case "|$placements_seen|" in
            *"|$placement_record|"*) ;;
            *)
                echo "$placement_record"
                if [ -f "$OUT/host.txt" ]; then
                    echo "$placement_record" >>"$OUT/host.txt"
                fi
                placements_seen="${placements_seen:+$placements_seen|}$placement_record"
                ;;
        esac
    else
        return "$?"
    fi
}

# A sweep skips unsupported counts, but must never hide a bad override.
placement_for_count() {
    placement_status=0
    select_placement "$1" "$2" || placement_status=$?
    case $placement_status in
        0) return 0 ;;
        2) return 1 ;;
        *) exit "$placement_status" ;;
    esac
}

# The busy fraction of each server and client CPU between two copies of
# /proc/stat, <before> <after> <server CPUs> <client CPUs>: of the ticks a
# CPU's line counts from user through steal (guest time is already inside
# user and nice), those neither idle nor iowait, and of them the user share
# (user and nice) and the kernel share (system, irq and softirq), since a
# thread spinning through sched_yield shows as kernel time. Prints
# cpu,role,busy,user,system for each CPU of the two comma-separated lists in
# turn.
cpu_busy() {
    python3 - "$@" <<'PY'
import sys

def ticks(path):
    result = {}
    with open(path) as stat:
        for line in stat:
            fields = line.split()
            if fields and fields[0].startswith("cpu") and fields[0] != "cpu":
                values = [int(x) for x in fields[1:9]]
                result[int(fields[0][3:])] = values + [0] * (8 - len(values))
    return result

try:
    before, after = ticks(sys.argv[1]), ticks(sys.argv[2])
    for role, cpus in (("server", sys.argv[3]), ("client", sys.argv[4])):
        for cpu in map(int, cpus.split(",")):
            if cpu not in before or cpu not in after:
                raise ValueError(f"/proc/stat has no line for cpu{cpu}")
            delta = [a - b for a, b in zip(after[cpu], before[cpu])]
            total = sum(delta)
            if total <= 0:
                raise ValueError(f"cpu{cpu} counted no ticks between the two reads")
            busy = (total - delta[3] - delta[4]) / total
            user = (delta[0] + delta[1]) / total
            system = (delta[2] + delta[5] + delta[6]) / total
            print(f"{cpu},{role},{busy:.4f},{user:.4f},{system:.4f}")
except (OSError, ValueError) as error:
    print(f"cpu-busy: {error}", file=sys.stderr)
    sys.exit(1)
PY
}

# Whether a sample was limited by its client, from cpu_busy's output in <file>
# (the OS busy fraction; workloads mode uses work_limited instead):
# prints the mean busy fraction over the client CPUs and 1 when it is above
# 0.90 (the owner's client-limited line, and a sample at exactly 0.90 is not
# limited), else 0. Such a cell only requires firn not to fall behind.
client_limited() {
    awk -F, '$2 == "client" { sum += $3; count++ }
        END {
            if (count == 0) { print "client_limited: no client CPU" > "/dev/stderr"; exit 1 }
            mean = sum / count
            printf "%.4f,%d\n", mean, (sprintf("%.4f", mean) + 0 > 0.9) ? 1 : 0
        }' "$1"
}

# Whether a workload sample was limited by its client, from the client's own
# accounting: <file> holds client_work_fraction, the share of the workload
# client's loop time spent in scans that moved bytes or finished a request. The
# workload client spins between scans, so /proc/stat shows its CPUs fully busy
# whatever the server does. Prints the fraction and 1 when it is above 0.90 (a
# sample at exactly 0.90 is not limited), else 0.
work_limited() {
    awk 'NR == 1 && NF == 1 && $1 ~ /^[0-9.]+$/ {
            printf "%.4f,%d\n", $1, (sprintf("%.4f", $1) + 0 > 0.9) ? 1 : 0; ok = 1 }
        END { if (!ok) { print "work_limited: no client_work_fraction" > "/dev/stderr"; exit 1 } }' "$1"
}

placement_selftest() (
    fixture=$(mktemp -d)
    trap 'rm -rf "$fixture"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    export SYSFS_ROOT="$fixture/sys"
    unset SERVER_CPU_LIST CLIENT_CPU_LIST
    fixture_cpu() {
        topology="$SYSFS_ROOT/devices/system/cpu/cpu$1/topology"
        mkdir -p "$topology"
        echo "$2" >"$topology/core_id"
        echo "$3" >"$topology/thread_siblings_list"
    }
    expect_placement() {
        actual=$(placement "$1" "$2" 2>"$fixture/stderr" | cut -d' ' -f1-4)
        [ "$actual" = "$3" ] || {
            echo "placement-selftest: expected '$3', got '$actual'" >&2
            exit 1
        }
    }
    # The core classes, fields 5 and 6 of the placement output.
    expect_classes() {
        actual=$(placement "$1" "$2" 2>"$fixture/stderr" | cut -d' ' -f5-6)
        [ "$actual" = "$3" ] || {
            echo "placement-selftest: expected classes '$3', got '$actual'" >&2
            exit 1
        }
    }
    expect_failure() {
        status=0
        placement "$1" "$2" >"$fixture/stdout" 2>"$fixture/stderr" || status=$?
        [ "$status" = "$3" ] && grep -q "$4" "$fixture/stderr" || {
            echo "placement-selftest: expected exit $3 and '$4'" >&2
            exit 1
        }
    }
    # Native 14900K: adjacent SMT pairs on eight P-cores, then 16 E-cores.
    cpu=0
    while [ "$cpu" -lt 32 ]; do
        if [ "$cpu" -lt 16 ]; then
            first=$((cpu / 2 * 2))
            fixture_cpu "$cpu" "$((cpu / 2))" "$first,$((first + 1))"
        else
            fixture_cpu "$cpu" "$((cpu - 8))" "$cpu"
        fi
        cpu=$((cpu + 1))
    done
    mkdir -p "$SYSFS_ROOT/devices/cpu_core" "$SYSFS_ROOT/devices/cpu_atom"
    echo 0-15 >"$SYSFS_ROOT/devices/cpu_core/cpus"
    echo 16-31 >"$SYSFS_ROOT/devices/cpu_atom/cpus"
    expect_placement 1 16 '2 4,5,6,7,8,9,10,11,12,13,14,15 12 hybrid'
    grep -q 'requested 16 client threads,only 12' "$fixture/stderr"
    expect_placement 2 16 '2,4 6,7,8,9,10,11,12,13,14,15 10 hybrid'
    expect_placement 4 16 '2,4,6,8 10,11,12,13,14,15 6 hybrid'
    expect_classes 2 16 '2:P,4:P 6:P,7:P,8:P,9:P,10:P,11:P,12:P,13:P,14:P,15:P'
    expect_failure 7 16 2 'no distinct client core'
    expect_failure 8 16 2 'only 7 eligible physical cores'
    SERVER_CPU_LIST=2,4 CLIENT_CPU_LIST=10,12 expect_placement 2 16 '2,4 10,12 2 override'
    # Empty overrides, as the workflow passes for empty inputs, are unset.
    SERVER_CPU_LIST= CLIENT_CPU_LIST= expect_placement 2 16 '2,4 6,7,8,9,10,11,12,13,14,15 10 hybrid'
    SERVER_CPU_LIST=2 expect_failure 2 16 1 'exactly 2 CPUs'
    SERVER_CPU_LIST=0-15 CLIENT_CPU_LIST=16-31 expect_failure 8 16 1 'exactly 8 CPUs'
    SERVER_CPU_LIST=32 expect_failure 1 16 1 'SERVER_CPU_LIST names a CPU without'
    CLIENT_CPU_LIST=32 expect_failure 1 16 1 'CLIENT_CPU_LIST names a CPU without'
    # A CPU twice in one list would pin two workers to one CPU.
    SERVER_CPU_LIST=2,2 expect_failure 2 16 1 'duplicate CPU in list'
    SERVER_CPU_LIST=2,4 CLIENT_CPU_LIST=10,10 expect_failure 2 16 1 'duplicate CPU in list'
    # The record carries each CPU's class, so a swapped placement shows.
    SERVER_CPU_LIST=0,2,4,6,8,10,12,14 CLIENT_CPU_LIST=16-19 expect_classes 8 4 \
        '0:P,2:P,4:P,6:P,8:P,10:P,12:P,14:P 16:E,17:E,18:E,19:E'
    SERVER_CPU_LIST=16,17 CLIENT_CPU_LIST=2,4 expect_classes 2 2 '16:E,17:E 2:P,4:P'
    # An explicit server list may name core_id 0 and both threads of a core.
    SERVER_CPU_LIST=2,3 expect_placement 2 16 '2,3 4,5,6,7,8,9,10,11,12,13,14,15 12 override'
    SERVER_CPU_LIST=0,2,4,6,8,10,12,14 CLIENT_CPU_LIST=16-31 expect_placement 8 16 \
        '0,2,4,6,8,10,12,14 16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31 16 override'
    SERVER_CPU_LIST=0-15 CLIENT_CPU_LIST=16-31 expect_placement 16 16 \
        '0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15 16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31 16 override'
    SERVER_CPU_LIST=2 CLIENT_CPU_LIST=3 expect_failure 1 16 1 'including siblings'
    SERVER_CPU_LIST=0,2,4,6,8,10,12,14 CLIENT_CPU_LIST=15-31 expect_failure 8 16 1 'including siblings'
    SERVER_CPU_LIST=2 expect_placement 1 2 '2 4,5 2 override'
    CLIENT_CPU_LIST=10,12 expect_placement 2 16 '2,4 10,12 2 override'
    # cpu_capacity supplies the same hybrid classification without PMU lists.
    rm -rf "$SYSFS_ROOT/devices/cpu_core" "$SYSFS_ROOT/devices/cpu_atom"
    cpu=0
    while [ "$cpu" -lt 32 ]; do
        capacity=512
        [ "$cpu" -ge 16 ] || capacity=1024
        echo "$capacity" >"$SYSFS_ROOT/devices/system/cpu/cpu$cpu/cpu_capacity"
        cpu=$((cpu + 1))
    done
    expect_placement 2 16 '2,4 6,7,8,9,10,11,12,13,14,15 10 hybrid'
    CLIENT_CPU_LIST=16,17 expect_classes 2 2 '2:P,4:P 16:E,17:E'
    # Non-hybrid four-core SMT host: second threads are numbered after firsts.
    rm -rf "$SYSFS_ROOT"
    cpu=0
    while [ "$cpu" -lt 8 ]; do
        core=$((cpu % 4))
        fixture_cpu "$cpu" "$core" "$core,$((core + 4))"
        cpu=$((cpu + 1))
    done
    expect_placement 1 16 '1 2,3,6,7 4 fallback'
    expect_placement 2 16 '1,2 3,7 2 fallback'
    expect_failure 3 16 2 'no distinct client core'
    expect_classes 2 16 '1:-,2:- 3:-,7:-'
    # Equal capacities are not hybrid information; core_cpus_list is accepted.
    cpu=0
    while [ "$cpu" -lt 8 ]; do
        path="$SYSFS_ROOT/devices/system/cpu/cpu$cpu"
        mv "$path/topology/thread_siblings_list" "$path/topology/core_cpus_list"
        echo 1024 >"$path/cpu_capacity"
        cpu=$((cpu + 1))
    done
    expect_placement 2 16 '1,2 3,7 2 fallback'
    echo 0-2,4-6 >"$SYSFS_ROOT/devices/system/cpu/online"
    expect_placement 1 16 '1 2,6 2 fallback'
    # cpu_busy: cpu0 300 user and 100 system of 1000 ticks; cpu1 idle; cpu2
    # 50 user (40 of them guest) and 40 iowait of 100.
    printf 'cpu  110 0 50 1930 10 0 0 0 0 0\ncpu0 100 0 50 850 0 0 0 0 0 0\ncpu1 0 0 0 1000 0 0 0 0 0 0\ncpu2 10 0 0 80 10 0 0 0 0 0\n' >"$fixture/stat-before"
    printf 'cpu  460 0 150 3540 50 0 0 0 40 0\ncpu0 400 0 150 1450 0 0 0 0 0 0\ncpu1 0 0 0 2000 0 0 0 0 0 0\ncpu2 60 0 0 90 50 0 0 0 40 0\n' >"$fixture/stat-after"
    actual=$(cpu_busy "$fixture/stat-before" "$fixture/stat-after" 0 1,2 | tr '\n' ' ')
    [ "$actual" = '0,server,0.4000,0.3000,0.1000 1,client,0.0000,0.0000,0.0000 2,client,0.5000,0.5000,0.0000 ' ] || {
        echo "placement-selftest: cpu_busy gave '$actual'" >&2
        exit 1
    }
    # client_limited: two client CPUs of 1000 ticks each, busy 905 and 900
    # (mean 0.9025, limited), 895 and 900 (0.8975, not), 900 and 900 (exactly
    # 0.90, not). The server CPU is idle and must not count.
    printf 'cpu0 0 0 0 0 0 0 0 0\ncpu1 0 0 0 0 0 0 0 0\ncpu2 0 0 0 0 0 0 0 0\n' >"$fixture/limit-before"
    for pair in '905 900 0.9025,1' '895 900 0.8975,0' '900 900 0.9000,0'; do
        set -- $pair
        printf 'cpu0 0 0 0 1000 0 0 0 0\ncpu1 %s 0 0 %s 0 0 0 0\ncpu2 %s 0 0 %s 0 0 0 0\n' \
            "$1" "$((1000 - $1))" "$2" "$((1000 - $2))" >"$fixture/limit-after"
        cpu_busy "$fixture/limit-before" "$fixture/limit-after" 0 1,2 >"$fixture/limit-busy"
        actual=$(client_limited "$fixture/limit-busy")
        [ "$actual" = "$3" ] || {
            echo "placement-selftest: client_limited gave '$actual' for busy $1 $2, expected '$3'" >&2
            exit 1
        }
    done
    # work_limited: the fraction decides, above 0.90 only; a missing file or a
    # non-number is an error.
    for pair in '0.9100 0.9100,1' '0.9000 0.9000,0' '0.2500 0.2500,0'; do
        set -- $pair
        echo "$1" >"$fixture/work"
        actual=$(work_limited "$fixture/work")
        [ "$actual" = "$2" ] || {
            echo "placement-selftest: work_limited gave '$actual' for $1, expected '$2'" >&2
            exit 1
        }
    done
    echo junk >"$fixture/work"
    if work_limited "$fixture/work" >/dev/null 2>&1; then
        echo "placement-selftest: work_limited accepted a non-number" >&2
        exit 1
    fi
    status=0
    cpu_busy "$fixture/stat-before" "$fixture/stat-after" 0 3 >/dev/null 2>"$fixture/stderr" || status=$?
    [ "$status" = 1 ] && grep -q 'no line for cpu3' "$fixture/stderr" || {
        echo "placement-selftest: cpu_busy accepted a CPU /proc/stat lacks" >&2
        exit 1
    }
    echo 'placement-selftest: passed'
)

if [ "${1:-}" = placement-selftest ]; then
    placement_selftest
    exit 0
fi

ROOT=${ROOT:-$(cd "$(dirname "$0")/../../.." && pwd)}
RELEASE=${RELEASE:-$(sed -n -E 's/^release = (wf-(exp-)?[0-9a-f]{12})$/\1/p' "$ROOT/whitefoot.pin")}
OUT=${OUT:-/tmp/redis-bench}
REVISIONS_FILE=${REVISIONS_FILE:-$OUT/revisions.csv}
WHITEFOOTC=${WHITEFOOTC:-$ROOT/build/whitefoot/$RELEASE/whitefootc}
BASELINE_ROOT=${BASELINE_ROOT:-}
DRAGONFLY=${DRAGONFLY:-dragonfly}
GARNET=${GARNET:-garnet-server}
# The newest stable releases of the four competitors (latest.sh), for the lines
# redis-latest, valkey-latest, valkey-io-latest, dragonfly-latest-N and
# garnet-latest-N; the lines reference and reference-aof stay Redis 7.2.16, and
# valkey, valkey-io, dragonfly-N and garnet-N the host's own.
REDIS_LATEST=${REDIS_LATEST:-}
VALKEY_LATEST=${VALKEY_LATEST:-}
DRAGONFLY_LATEST=${DRAGONFLY_LATEST:-}
GARNET_LATEST=${GARNET_LATEST:-}
FIRN_BASELINE=${FIRN_BASELINE:-}
DEFAULT_CLIENT_THREADS=${CLIENT_THREADS:-2}
REQUESTS=${REQUESTS:-1000000}
ROUNDS=${ROUNDS:-2}
PASSES=${PASSES:-3}
SECONDS_PER_RUN=${SECONDS_PER_RUN:-12}
# A server that closes an idle client first holds that port in TIME_WAIT for
# a minute, and a server that does not set SO_REUSEADDR cannot listen on it
# meanwhile, so each run starts its ports from its own process number rather
# than from one fixed port a run a minute earlier may still hold.
PORT=${PORT:-$((10000 + $$ % 400 * 50))}
MODE=${1:-bench}

case $MODE in
    compare|scale|workloads|quick) ;;
    *) select_placement 2 "$DEFAULT_CLIENT_THREADS" ;;
esac

mkdir -p "$OUT"
# firn is linked as a server would be, its module and the runtime's units
# optimized together; FIRN_LINK names other link options, or none.
if [ "$MODE" != compare ] && { [ "$MODE" != workloads ] || [ -z "${WORKLOAD_IMAGES:-}" ]; }; then
    "$WHITEFOOTC" ${FIRN_LINK---full-lto} --graph "$ROOT/firn/modules.wfg" --entry firn -o "$OUT/firn"
    # Record the source actually built, never infer its invocation from ancestry.
    invocation=positional
    if [ -f "$ROOT/firn/server/config.wf" ]; then invocation=config; fi
    python3 - "$REVISIONS_FILE" "$ROOT" "$invocation" <<'PYRECORD'
import csv
from pathlib import Path
import subprocess
import sys
path, root, invocation = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
commit = subprocess.check_output(["git", "-C", root, "rev-parse", "HEAD"], text=True).strip()
pin = (Path(root) / "whitefoot.pin").read_text().strip()
row = ["revision", "firn", commit, pin, invocation]
rows = list(csv.reader(path.open())) if path.exists() else []
matching = [r for r in rows if len(r) > 1 and r[:2] == row[:2]]
if matching and (len(matching) != 1 or matching[0][:5] != row):
    raise SystemExit("current firn source conflicts with its revisions.csv record")
if not matching:
    with path.open("a", newline="") as out:
        csv.writer(out).writerow(row)
PYRECORD
fi
baselines=
if [ -n "$BASELINE_ROOT" ]; then
    "$BASELINE_ROOT/compiler/target/gate/whitefootc" -o "$OUT/redis_baseline" \
        "$BASELINE_ROOT/tests/programs/redis_subset.wf"
    baselines="baseline-2 baseline-1"
fi

server=
# IDLE, when set, is the idle limit in seconds a start gives the server; KEEP,
# when set, keeps the append-only file a previous start left.
IDLE=
KEEP=
# Whether a line's server can run on this host.
available() {
    case $1 in
        dragonfly-latest-*) test -n "$DRAGONFLY_LATEST" && test -x "$DRAGONFLY_LATEST" ;;
        dragonfly-*) command -v "$DRAGONFLY" >/dev/null 2>&1 ;;
        garnet-latest-*) test -n "$GARNET_LATEST" && test -x "$GARNET_LATEST" ;;
        garnet-*) command -v "$GARNET" >/dev/null 2>&1 ;;
        redis-latest) test -n "$REDIS_LATEST" && test -x "$REDIS_LATEST" ;;
        valkey-latest|valkey-io-latest) test -n "$VALKEY_LATEST" && test -x "$VALKEY_LATEST" ;;
        firn-base-*) test -n "$FIRN_BASELINE" && test -x "$FIRN_BASELINE" ;;
        valkey*) command -v valkey-server >/dev/null 2>&1 ;;
        *) true ;;
    esac
}
# The number of CPUs in a taskset list such as 0,1.
cpu_count() {
    echo "$1" | tr ',' '\n' | wc -l
}
# Each start takes a fresh port: a stopped server's accepted connections wait
# out TIME_WAIT on its port, which a server without SO_REUSEADDR cannot bind.
# Registers the subshell that calls it, which then becomes a server by exec,
# in redis-bench.yml's record on the shared machine (session.py), so that
# every server a cancelled run leaves is named before it runs.
registered() {
    if [ -n "${FIRN_REDIS_BENCH_RECORD:-}" ]; then
        python3 "$ROOT/research/experiments/redis-bench/session.py" register \
            "$FIRN_REDIS_BENCH_RECORD" parent || exit 1
    fi
}

# A kept image's alias may differ from its recorded stem; metadata belongs to
# the source, while image_path still returns the alias's actual executable.
recorded_invocation() {
    python3 - "$REVISIONS_FILE" "$1" <<'PYINVOCATION'
import csv
from pathlib import Path
import sys
path, name = Path(sys.argv[1]), sys.argv[2]
if not path.is_file():
    raise SystemExit(f"missing revision metadata: {path}")
with path.open(newline="") as source:
    rows = [r for r in csv.reader(source) if len(r) > 1 and r[:2] == ["revision", name]]
if len(rows) != 1:
    raise SystemExit(f"expected exactly one revision record for {name}, found {len(rows)}")
row = rows[0]
if len(row) < 5 or not row[2] or not row[3] or row[4] not in ("config", "positional"):
    raise SystemExit(f"invalid revision metadata for {name}: requires commit, pin and invocation")
print(row[4])
PYINVOCATION
}

start() {
    # The original correctness/measurement modes include both one- and
    # two-driver lines. Give each its own server count, as suite already does.
    case $MODE in
        compare|scale|workloads|quick|suite) ;;
        *)
            case $1 in
                firn-*|baseline-*) default_server_count=${1##*-} ;;
                *) default_server_count=2 ;;
            esac
            select_placement "$default_server_count" "$DEFAULT_CLIENT_THREADS"
            ;;
    esac
    case $1 in
        firn-base-*) invocation=$(recorded_invocation base) ;;
        firn-*) invocation=$(recorded_invocation firn) ;;
        image-*) image_name=${1#image-}; invocation=$(recorded_invocation "${image_name%-twin}") ;;
    esac
    PORT=$((PORT + 1))
    cpus=$(cpu_count "$SERVER_CPUS")
    if [ -z "$KEEP" ]; then
        rm -rf "$OUT/appendonlydir" "$OUT/firn.aof"
    fi
    case $1 in
        reference)
            (registered; exec taskset -c "$SERVER_CPUS" redis-server --port "$PORT" --save "" \
                --appendonly no --timeout "${IDLE:-0}" --daemonize no \
                ) >"$OUT/server.log" 2>&1 &
            ;;
        reference-aof)
            ref_dir=$OUT
            if [ "$MODE" = compare ]; then
                # As for the images, a directory of its own that stop removes.
                AOF_DIR="$OUT/aof-$PORT"
                rm -rf "$AOF_DIR"
                mkdir -p "$AOF_DIR"
                ref_dir=$AOF_DIR
            fi
            (registered; exec taskset -c "$SERVER_CPUS" redis-server --port "$PORT" --save "" \
                --appendonly yes --appendfsync everysec --dir "$ref_dir" \
                --timeout "${IDLE:-0}" --daemonize no \
                ) >"$OUT/server.log" 2>&1 &
            ;;
        redis-latest)
            (registered; exec taskset -c "$SERVER_CPUS" "$REDIS_LATEST" --port "$PORT" --save "" \
                --appendonly no --timeout "${IDLE:-0}" --daemonize no \
                ) >"$OUT/server.log" 2>&1 &
            ;;
        valkey-latest)
            (registered; exec taskset -c "$SERVER_CPUS" "$VALKEY_LATEST" --port "$PORT" --save "" \
                --appendonly no --daemonize no ) >"$OUT/server.log" 2>&1 &
            ;;
        valkey-io-latest)
            (registered; exec taskset -c "$SERVER_CPUS" "$VALKEY_LATEST" --port "$PORT" --save "" \
                --appendonly no --io-threads "$cpus" --io-threads-do-reads yes \
                --daemonize no ) >"$OUT/server.log" 2>&1 &
            ;;
        valkey)
            (registered; exec taskset -c "$SERVER_CPUS" valkey-server --port "$PORT" --save "" \
                --appendonly no --daemonize no ) >"$OUT/server.log" 2>&1 &
            ;;
        valkey-io)
            (registered; exec taskset -c "$SERVER_CPUS" valkey-server --port "$PORT" --save "" \
                --appendonly no --io-threads "$cpus" --io-threads-do-reads yes \
                --daemonize no ) >"$OUT/server.log" 2>&1 &
            ;;
        dragonfly-latest-*)
            (registered; exec taskset -c "$SERVER_CPUS" "$DRAGONFLY_LATEST" --port="$PORT" \
                --proactor_threads="${1#dragonfly-latest-}" --dbfilename= \
                --logtostderr ) >"$OUT/server.log" 2>&1 &
            ;;
        dragonfly-*)
            (registered; exec taskset -c "$SERVER_CPUS" "$DRAGONFLY" --port="$PORT" \
                --proactor_threads="${1#dragonfly-}" --dbfilename= \
                --logtostderr ) >"$OUT/server.log" 2>&1 &
            ;;
        garnet-latest-*)
            (registered; exec taskset -c "$SERVER_CPUS" "$GARNET_LATEST" --port "$PORT" \
                --bind 127.0.0.1 ) >"$OUT/server.log" 2>&1 &
            ;;
        garnet-*)
            (registered; exec taskset -c "$SERVER_CPUS" "$GARNET" --port "$PORT" \
                --bind 127.0.0.1 ) >"$OUT/server.log" 2>&1 &
            ;;
        firn-base-*)
            (registered; WF_DRIVERS=${1#firn-base-} firn_command "$invocation" "$PORT" no firn.aof 0 "${IDLE:-0}" '' \
                exec taskset -c "$SERVER_CPUS" "$FIRN_BASELINE") >"$OUT/server.log" 2>&1 &
            ;;
        firn-aof-*)
            (registered && cd "$OUT" && WF_DRIVERS=${1##*-} \
                firn_command "$invocation" "$PORT" yes firn.aof 0 "${IDLE:-0}" '' \
                exec taskset -c "$SERVER_CPUS" ./firn) \
                >"$OUT/server.log" 2>&1 &
            ;;
        firn-*)
            (registered; WF_DRIVERS=${1#firn-} firn_command "$invocation" "$PORT" no firn.aof 0 "${IDLE:-0}" '' \
                exec taskset -c "$SERVER_CPUS" "$OUT/firn") >"$OUT/server.log" 2>&1 &
            ;;
        image-*)
            if [ "$MODE" = compare ] && [ "${COMPARE_AOF:-}" = true ]; then
                # Each AOF server starts in a directory of its own, which stop
                # removes, so no append-only file outlives or joins another run.
                img=$(image_path "${1#image-}")
                case $img in /*) ;; *) img="$PWD/$img" ;; esac
                AOF_DIR="$OUT/aof-$PORT"
                rm -rf "$AOF_DIR"
                mkdir -p "$AOF_DIR"
                (registered && cd "$AOF_DIR" && \
                    WF_DRIVERS=${IMAGE_DRIVERS:-$(cpu_count "$SERVER_CPUS")} \
                    firn_command "$invocation" "$PORT" yes firn.aof 0 "${IDLE:-0}" '' \
                    exec taskset -c "$SERVER_CPUS" "$img") >"$OUT/server.log" 2>&1 &
            else
            (registered; WF_DRIVERS=${IMAGE_DRIVERS:-$(cpu_count "$SERVER_CPUS")} \
                firn_command "$invocation" "$PORT" no firn.aof 0 "${IDLE:-0}" '' \
                exec taskset -c "$SERVER_CPUS" "$(image_path "${1#image-}")") >"$OUT/server.log" 2>&1 &
            fi
            ;;
        baseline-*)
            (registered; WF_DRIVERS=${1#baseline-} exec taskset -c "$SERVER_CPUS" \
                "$OUT/redis_baseline" "$PORT" 0 ) >"$OUT/server.log" 2>&1 &
            ;;
    esac
    server=$!
    tries=0
    until redis-cli -p "$PORT" PING 2>/dev/null | grep -q PONG; do
        tries=$((tries + 1))
        if [ "$tries" -gt 400 ]; then
            echo "$1 never answered on $PORT" >&2
            exit 1
        fi
        sleep 0.05
    done
    # Compare mode: every server, twins included, runs under the same memory
    # limit, set after it starts and read back before anything is measured.
    if [ "$MODE" = compare ] && [ -n "${COMPARE_MAXMEMORY:-}" ]; then
        reply=$(redis-cli -p "$PORT" CONFIG SET maxmemory "$COMPARE_MAXMEMORY") || reply=
        if [ "$reply" != OK ]; then
            stop
            echo "$1: CONFIG SET maxmemory $COMPARE_MAXMEMORY replied '$reply', not OK" >&2
            exit 1
        fi
        reply=$(redis-cli -p "$PORT" CONFIG GET maxmemory | sed -n 2p) || reply=
        if [ "$reply" != "$COMPARE_MAXMEMORY" ]; then
            stop
            echo "$1: CONFIG GET maxmemory read '$reply', not $COMPARE_MAXMEMORY" >&2
            exit 1
        fi
    fi
    # Compare mode with COMPARE_AOF=true: the server must report AOF on, and its
    # automatic rewrites are switched off so that the run measures the
    # steady-state append path (appendfsync everysec is firn's only policy).
    if [ "$MODE" = compare ] && [ "${COMPARE_AOF:-}" = true ]; then
        enabled=$(redis-cli -p "$PORT" INFO persistence | tr -d '\r' | sed -n 's/^aof_enabled://p') || enabled=
        echo "aof,$1,aof_enabled=$enabled"
        if [ "$enabled" != 1 ]; then
            stop
            echo "$1: COMPARE_AOF=true but INFO persistence reads aof_enabled='$enabled', not 1" >&2
            exit 1
        fi
        reply=$(redis-cli -p "$PORT" CONFIG SET auto-aof-rewrite-percentage 0) || reply=
        if [ "$reply" != OK ]; then
            stop
            echo "$1: CONFIG SET auto-aof-rewrite-percentage 0 replied '$reply', not OK" >&2
            exit 1
        fi
        reply=$(redis-cli -p "$PORT" CONFIG GET auto-aof-rewrite-percentage | sed -n 2p) || reply=
        if [ "$reply" != 0 ]; then
            stop
            echo "$1: CONFIG GET auto-aof-rewrite-percentage read '$reply', not 0" >&2
            exit 1
        fi
    fi
}

AOF_DIR=
# The start and verify argument of a compare line: the reference is Redis,
# with the append-only file on when COMPARE_AOF=true; any other name is an image.
compare_arg() {
    if [ "$1" = reference ]; then
        if [ "${COMPARE_AOF:-}" = true ]; then
            echo reference-aof
        else
            echo reference
        fi
    else
        echo "image-$1"
    fi
}
stop() {
    kill "$server"
    wait "$server" 2>/dev/null || true
    if [ -n "$AOF_DIR" ]; then
        rm -rf "$AOF_DIR"
        AOF_DIR=
    fi
}

fail() {
    echo "$1 failed the correctness pass: $2" >&2
    exit 1
}

# Correct: no error reply in a mixed run, and every one of 100,000 increments
# from 50 clients reaches the one counter.
verify() {
    start "$1"
    taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" -t incr -n 100000 \
        -c 50 -q >"$OUT/verify-$1.txt" 2>&1
    counted=$(redis-cli -p "$PORT" GET counter:__rand_int__)
    replies=$(printf 'SET a 1\nGET a\nINCR a\nDEL a\nGET a\n' |
        redis-cli -p "$PORT" | tr '\n' ' ')
    stop
    echo "verify,$1,counter=$counted,replies=$replies"
    if [ "$counted" != 100000 ] || [ "$replies" != "OK 1 2 1  " ]; then
        fail "$1" "counter or replies"
    fi
}

# Experiment 8: a key set with PX 100 answers a positive PTTL and is absent
# 200 milliseconds later; TTL answers -1 without an expiry and -2 for a
# missing key; PERSIST removes an expiry once.
verify_expiry() {
    start "$1"
    replies=$(printf 'SET kept 1\nSET brief hello PX 100\nTTL kept\nTTL absent\nEXPIRE kept 100\nTTL kept\nPERSIST kept\nTTL kept\nPERSIST kept\n' |
        redis-cli -p "$PORT" | tr '\n' ' ')
    left=$(redis-cli -p "$PORT" PTTL brief)
    sleep 0.2
    after=$(printf 'GET brief\nPTTL brief\n' | redis-cli -p "$PORT" | tr '\n' ' ')
    stop
    echo "verify-expiry,$1,replies=$replies,left=$left,after=$after"
    if [ "$replies" != "OK OK -1 -2 1 100 1 -1 0 " ] ||
        [ "$left" -le 0 ] || [ "$left" -gt 100 ] || [ "$after" != " -2 " ]; then
        fail "$1" "expiry replies"
    fi
}

# Experiment 8: after a stop and a restart on the file, a key set and not
# expired holds its value and a key whose expiry passed meanwhile is absent;
# a key made persistent before its expiry holds its value, and one incremented
# before its expiry passed is absent, as a replay that expires nothing while it
# loads leaves them.
# firn stops by being killed, so the check waits 100 milliseconds for its
# writer, which appends every 10, before stopping it; Redis flushes its file
# when it is stopped.
verify_restart() {
    start "$1"
    printf 'SET k1 v\nSET k2 v PX 300\nSET k3 v EX 100\nINCR n\nINCR n\nDEL k1\nSET k4 v\nSET k5 v PX 300\nPERSIST k5\nSET n2 5 PX 300\nINCR n2\n' |
        redis-cli -p "$PORT" >/dev/null
    sleep 0.1
    stop
    sleep 0.5
    KEEP=1
    start "$1"
    KEEP=
    got=$(printf 'GET k1\nGET k2\nGET k3\nGET n\nGET k4\nGET k5\nGET n2\n' |
        redis-cli -p "$PORT" | tr '\n' ' ')
    left=$(redis-cli -p "$PORT" TTL k3)
    stop
    echo "verify-restart,$1,got=$got,left=$left"
    if [ "$got" != "  v 2 v v  " ] || [ "$left" -lt 98 ]; then
        fail "$1" "replayed keys"
    fi
}

# Experiment 8: a connection silent past a one-second limit is closed within
# two seconds.
verify_idle() {
    IDLE=1
    start "$1"
    IDLE=
    silent=$(python3 - "$PORT" <<'EOF'
import socket, sys, time
connection = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
connection.sendall(b"*1\r\n$4\r\nPING\r\n")
connection.recv(64)
started = time.monotonic()
connection.settimeout(10)
connection.recv(64)
print("%.2f" % (time.monotonic() - started))
EOF
)
    stop
    echo "verify-idle,$1,closed after $silent s"
    if ! awk "BEGIN { exit !($silent < 2) }"; then
        fail "$1" "idle limit"
    fi
}

# Experiment 8: after 100,000 keys set with PX 1000 and no further reads,
# DBSIZE falls below 1 percent of them within ten seconds of the last set.
verify_active() {
    start "$1"
    removed=$(python3 - "$PORT" <<'EOF'
import socket, sys, time
KEYS = 100000
connection = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
batch = bytearray()
for index in range(KEYS):
    key = b"active:%d" % index
    batch += b"*5\r\n$3\r\nSET\r\n$%d\r\n%s\r\n$1\r\nv\r\n$2\r\nPX\r\n$4\r\n1000\r\n" % (len(key), key)
connection.sendall(batch)
expected = b"+OK\r\n" * KEYS
held = bytearray()
while len(held) < len(expected):
    held += connection.recv(1 << 16)
assert held == expected, held[:64]
started = time.monotonic()
reader = connection.makefile("rb")
while True:
    connection.sendall(b"*1\r\n$6\r\nDBSIZE\r\n")
    size = int(reader.readline()[1:])
    elapsed = time.monotonic() - started
    if size < KEYS // 100 or elapsed > 10:
        print("%d keys left after %.2f s" % (size, elapsed))
        break
    time.sleep(0.1)
EOF
)
    stop
    echo "verify-active,$1,$removed"
    case $removed in
        *" after "*) left=${removed%% keys*} ;;
        *) left=100000 ;;
    esac
    if [ "$left" -ge 1000 ]; then
        fail "$1" "active expiry"
    fi
}

# The firn criteria: every test of the suite completes on the line with no
# error reply. The suite is redis-benchmark's default of Redis 7.0, named
# explicitly: 7.2's default adds XADD, which firn lacks until its minimal
# streams land (board item firn-scope-min-streams), so the default would fail
# firn before anything is measured; XADD joins the list when they land. Its
# 19 tests print 20 rows, LRANGE's LPUSH fill among them.
SUITE_DEFAULT=ping_inline,ping_mbulk,set,get,incr,lpush,rpush,lpop,rpop,sadd,hset,spop,zadd,zpopmin,lrange_100,lrange_300,lrange_500,lrange_600,mset
verify_suite() {
    start "$1"
    taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" -c 50 -n 20000 \
        -r 100000 --threads "$CLIENT_THREADS" -t "$SUITE_DEFAULT" --csv >"$OUT/suite-$1.csv" \
        2>"$OUT/suite-$1.err"
    stop
    tests=$(grep -c -v '^"test"' "$OUT/suite-$1.csv")
    echo "verify-suite,$1,$tests tests"
    # redis-benchmark reads the server's CONFIG only to report it; Dragonfly
    # does not answer it as Redis does, which the client warns about and
    # which changes nothing it measures. Any other message is a failure.
    grep -v '^WARNING: Could not fetch server CONFIG$' "$OUT/suite-$1.err" \
        >"$OUT/suite-$1.problems" || true
    if [ "$tests" != 20 ] || [ -s "$OUT/suite-$1.problems" ]; then
        fail "$1" "the default suite"
    fi
}

measure() {
    start "$1"
    for pipeline in 1 16; do
        taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" \
            --threads "$CLIENT_THREADS" -c 50 -n "$REQUESTS" -r 100000 -d 16 \
            -t set,get -P "$pipeline" --csv 2>/dev/null | grep -v '^"test"' |
            sed "s/^/$1,round $2,pipeline $pipeline,/"
    done
    stop
}

SUITE_TESTS=${SUITE_TESTS:-"ping_inline ping_mbulk set get incr lpush rpush lpop rpop sadd hset spop zadd zpopmin lrange_100 lrange_300 lrange_500 lrange_600 mset"}
# The depths the suite and its pilot run each test at.
PIPELINES=${PIPELINES:-"1 16"}

# One test's rate on the running server at one depth after a given number of
# requests, the list refill of an LRANGE test left out.
# The scale run's client: by default one single-threaded redis-benchmark per
# client CPU (quick_client), since one threaded process stops near 6.7
# million requests a second, below firn on 16 server CPUs (21 million with
# the processes, measured on the i9-14900K); SCALE_CLIENT=threads keeps the
# threaded process, whose rate and latencies redis-benchmark reports itself.
procs_client() {
    test "${SCALE_CLIENT:-procs}" != threads && test "$MODE" = scale && test "$2" = 16
}
pilot_rate() {
    if procs_client "$1" "$2"; then
        quick_client "$3" "$1"
        return
    fi
    taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" \
        --threads "$CLIENT_THREADS" -c 50 -n "$3" -r 100000 -t "$1" -P "$2" \
        --csv 2>/dev/null | grep -v '^"test"' | grep -v '^"LPUSH (needed' |
        head -1 | cut -d, -f2 | tr -d '"'
}

# The requests a suite run of one test at one depth sends: SECONDS_PER_RUN
# seconds at the faster of Redis's and firn's rate, so that every line runs at
# least that long at the rate of the faster of them and redis-benchmark's
# quarter-second clock reads its rate to within about 2 percent. Each rate is
# read in two steps, 100,000 requests and then about two seconds' worth, since
# the clock cannot read a shorter run. The pilot's rates are printed as pilot
# lines.
pilot() {
    rm -f "$OUT/pilot.csv"
    for line in reference "firn-$(cpu_count "$SERVER_CPUS")"; do
        start "$line"
        for pipeline in $PIPELINES; do
            for test in $SUITE_TESTS; do
                first=$(pilot_rate "$test" "$pipeline" 100000)
                requests=$(awk -v rate="$first" 'BEGIN {
                    n = int(rate * 2); if (n < 100000) n = 100000; print n }')
                rate=$(pilot_rate "$test" "$pipeline" "$requests")
                echo "$line,$pipeline,$test,$rate" >>"$OUT/pilot.csv"
            done
        done
        stop
    done
    sed 's/^/pilot,/' "$OUT/pilot.csv"
}

requests_for() {
    awk -F, -v test="$1" -v pipeline="$2" -v seconds="$SECONDS_PER_RUN" '
        $2 == pipeline && $3 == test { if ($4 + 0 > rate) rate = $4 + 0 }
        END { printf "%d\n", rate * seconds + 1 }' "$OUT/pilot.csv"
}

suite_run() {
    start "$1"
    for pipeline in $PIPELINES; do
        for test in $SUITE_TESTS; do
            requests=$(requests_for "$test" "$pipeline")
            if procs_client "$test" "$pipeline"; then
                echo "$1,$2,$3,pipeline $pipeline,\"$test\",\"$(quick_client "$requests" "$test")\",processes"
                continue
            fi
            taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" \
                --threads "$CLIENT_THREADS" -c 50 -n "$requests" -r 100000 \
                -t "$test" -P "$pipeline" --csv 2>/dev/null |
                grep -v '^"test"' | grep -v '^"LPUSH (needed' |
                sed "s/^/$1,$2,$3,pipeline $pipeline,/"
        done
    done
    stop
}

# One run of the quick comparison: <requests> of <test> from one
# single-threaded redis-benchmark per client CPU with 3 connections each,
# printing the rate as the requests over the wall time read outside the
# processes. A threaded redis-benchmark ends only on a tick of about 250 ms,
# so its short runs resolve nothing, and one process is the limit on 8 or
# more server CPUs. LRANGE_100 is sent as its command after 100,000 pushes
# outside the timed span, since the RPOP runs before it may empty the list.
quick_client() {
    case $2 in
        lrange_100)
            redis-benchmark -p "$PORT" -t lpush -n 100000 -P 16 -q >/dev/null 2>&1
            what="LRANGE mylist 0 99"
            ;;
        *) what="-t $2" ;;
    esac
    begin=$(date +%s%N)
    pids=
    for cpu in $(echo "$CLIENT_CPUS" | tr ',' ' '); do
        taskset -c "$cpu" redis-benchmark -p "$PORT" -c 3 \
            -n $(($1 / CLIENT_THREADS)) -r 100000 -P 16 -q $what >/dev/null 2>&1 &
        pids="$pids $!"
    done
    wait $pids
    end=$(date +%s%N)
    awk -v requests=$(($1 / CLIENT_THREADS * CLIENT_THREADS)) -v ns=$((end - begin)) \
        'BEGIN { printf "%d\n", requests / (ns / 1e9) }'
}

# Measures <line> on <tests>: a short run sizes each test to QUICK_SECONDS,
# then QUICK_RUNS runs of every test in turn; each run goes to
# quick-runs-<line>.csv and each test's median to <file> as line,test,rate.
quick_measure() {
    start "$1"
    runs="$OUT/quick-runs-$1.csv"
    : >"$runs"
    : >"$OUT/quick-requests.txt"
    for test in $3; do
        first=$(quick_client 1600000 "$test")
        echo "$test $((first * ${QUICK_SECONDS:-3}))" >>"$OUT/quick-requests.txt"
    done
    run=1
    while [ "$run" -le "${QUICK_RUNS:-3}" ]; do
        for test in $3; do
            requests=$(awk -v t="$test" '$1 == t { print $2 }' "$OUT/quick-requests.txt")
            echo "$1,$test,$run,$(quick_client "$requests" "$test")" >>"$runs"
        done
        run=$((run + 1))
    done
    stop
    for test in $3; do
        awk -F, -v t="$test" '$2 == t { print $4 }' "$runs" | sort -n |
            awk -v line="$1" -v t="$test" '
                { rate[NR] = $1 }
                END { print line "," t "," rate[int((NR + 1) / 2)] }' >>"$2"
    done
}

# The path IMAGES gives a compare line's name.
image_path() {
    for pair in $IMAGES; do
        if [ "${pair%%=*}" = "$1" ]; then
            echo "${pair#*=}"
            return
        fi
    done
    echo "no image named $1" >&2
    exit 1
}

# The server's CPU time so far in clock ticks, user and system together.
server_ticks() {
    sed 's/^.*) //' "/proc/$server/stat" | awk '{ print $12 + $13 }'
}

# The commands compare_client sends for the linsert and lmove tests, which the
# timed runs and verify_list_forms both use. linsert_form takes the depth.
linsert_form() { echo "LINSERT ins$1:__rand_int__ AFTER pivot xxx"; }
lmove_form() { echo "LMOVE mylist mylist RIGHT LEFT"; }

# One redis-benchmark run of <test> at depth <depth> for <requests> against
# the started server, its client threads on CLIENT_CPUS; prints the rate over
# the run's wall time, the server's CPU microseconds per request, and the
# client's p50 and p99. redis-benchmark's own rate divides by a clock that
# ticks every 250 ms, a step of 5% in a five-second run, so it is not used.
# COMPARE_CLIENT=threads runs one redis-benchmark with CLIENT_THREADS
# threads instead of one process per client CPU. With cpu_record set, it copies
# /proc/stat to compare-stat-before and compare-stat-after just outside the
# timed interval, for cpu_busy.
compare_client() {
    what="-t $1"
    case $1 in
        linsert)
            # One of 100,000 short lists, each filled with copies of one pivot
            # outside the timed span, takes an element after its first pivot,
            # so that the scan is one step and a list grows by the number of
            # inserts its key draws (a few hundred); the lists are named by the
            # depth, so that no run starts from another's. A key the filling
            # missed makes its LINSERT a miss that writes nothing, the same for
            # every image.
            redis-benchmark -p "$PORT" -r 100000 -n 1000000 -P 16 -q \
                RPUSH "ins$2:__rand_int__" pivot >/dev/null 2>&1
            what=$(linsert_form "$2")
            ;;
        lmove)
            # The one list rotates onto itself, so that it keeps its length.
            redis-cli -p "$PORT" DEL mylist >/dev/null
            redis-cli -p "$PORT" RPUSH mylist a b c d e >/dev/null
            what=$(lmove_form)
            ;;
        lrange_*)
            # redis-benchmark fills the list an LRANGE test reads inside its own
            # run, which the timed span would include, and prints that fill as
            # a row of its own. Fill it here, outside the span, and send the
            # test's command.
            redis-cli -p "$PORT" DEL mylist >/dev/null
            redis-benchmark -p "$PORT" -t lpush -n 100000 -P 16 -q >/dev/null 2>&1
            case $1 in
                lrange_100) last=99 ;;
                lrange_300) last=299 ;;
                lrange_500) last=449 ;;
                *) last=599 ;;
            esac
            what="LRANGE mylist 0 $last"
            ;;
    esac
    ticks=$(server_ticks)
    [ -z "$cpu_record" ] || cat /proc/stat >"$OUT/compare-stat-before"
    begin=$(date +%s%N)
    if [ "${COMPARE_CLIENT:-procs}" = threads ]; then
        taskset -c "$CLIENT_CPUS" redis-benchmark -p "$PORT" --threads "$CLIENT_THREADS" \
            -c 50 -n "$3" -r 100000 -d 3 -P "$2" --csv $what >"$OUT/compare-client.csv" 2>"$OUT/compare-client.err"
    else
        # One process per client CPU, three connections each: one process's
        # threads stop near 6.6M requests a second on the 14900K, below what
        # four server CPUs answer. The first process's latencies are kept.
        pids=
        index=0
        for cpu in $(echo "$CLIENT_CPUS" | tr ',' ' '); do
            taskset -c "$cpu" redis-benchmark -p "$PORT" -c 3 -n $(($3 / CLIENT_THREADS)) \
                -r 100000 -d 3 -P "$2" --csv $what >"$OUT/compare-client-$index.csv" 2>"$OUT/compare-client.err" &
            pids="$pids $!"
            index=$((index + 1))
        done
        wait $pids
        cp "$OUT/compare-client-0.csv" "$OUT/compare-client.csv"
        set -- "$1" "$2" $(($3 / CLIENT_THREADS * CLIENT_THREADS))
    fi
    end=$(date +%s%N)
    [ -z "$cpu_record" ] || cat /proc/stat >"$OUT/compare-stat-after"
    ticks=$(($(server_ticks) - ticks))
    # The fields between the first and last quoted ones hold no quote.
    awk -F'","' -v n="$3" -v ns=$((end - begin)) -v ticks="$ticks" -v hz="$(getconf CLK_TCK)" '
        $1 != "\"test" {
            printf "%.0f,%.3f,%s,%s\n", n / (ns / 1e9), ticks / hz * 1e6 / n, $5, $7
        }' "$OUT/compare-client.csv"
}

# The linsert and lmove forms change a list as the tests assume: one is sent
# 100 and the other 101 times by redis-benchmark itself to a server with keys
# the timed runs never use, and the lists must show it. A form that misses its
# key, or a command that does not write, would otherwise be measured as a
# miss and pass for a fast server.
verify_list_forms() {
    start "$1"
    redis-cli -p "$PORT" RPUSH ins0:000000000000 pivot >/dev/null
    redis-benchmark -p "$PORT" -c 1 -n 100 -r 1 -q $(linsert_form 0) >/dev/null 2>&1
    length=$(redis-cli -p "$PORT" LLEN ins0:000000000000)
    redis-cli -p "$PORT" DEL mylist >/dev/null
    redis-cli -p "$PORT" RPUSH mylist a b c d e >/dev/null
    redis-benchmark -p "$PORT" -c 1 -n 101 -q $(lmove_form) >/dev/null 2>&1
    rotated=$(redis-cli -p "$PORT" LRANGE mylist 0 -1 | tr '\n' ' ')
    stop
    # 100 insertions after the one pivot make 101 elements; 101 rotations of
    # five move the last element to the front.
    if [ "$length" != 101 ] || [ "$rotated" != "e a b c d " ]; then
        fail "$1" "the linsert and lmove forms (LLEN '$length', list '$rotated')"
    fi
}

# The comparison of prebuilt images. A line whose replies fail verify stops
# the run; every sample is kept as compare.csv's line,pass,cpus,test,depth,
# requests,rps,server_cpu_us_per_request,p50_ms,p99_ms, and each test's
# request count is sized from the
# fastest image's pilot so that every image runs the same count. Where
# /proc/stat exists, each measured sample also gives compare-cpu.csv one
# line,pass,cpus,test,depth,cpu,role,busy,user,system row per server and
# client CPU: the fraction of that CPU's time not idle over the client's run,
# and its user and kernel shares. compare-limited.csv has one
# line,pass,cpus,test,depth,client_busy_mean,client_limited row per sample: the
# mean busy fraction over the client CPUs, and 1 when it is above 0.90, which
# marks the cell client-limited. Each image's server log, which holds the
# Whitefoot runtime's per-driver counts when WF_SCHED_REPORT is set, is kept
# per pass as compare-server-<line>-<cpus>-<pass>.txt.
if [ "$MODE" = compare ]; then
    total=$(nproc)
    if [ -n "${COMPARE_MAXMEMORY:-}" ]; then
        case $COMPARE_MAXMEMORY in
            *[!0-9]*|0?*)
                echo "COMPARE_MAXMEMORY must be a nonnegative integer without leading zeros, not '$COMPARE_MAXMEMORY'" >&2
                exit 1
                ;;
        esac
        echo "setting,maxmemory=$COMPARE_MAXMEMORY"
        if [ -f "$OUT/host.txt" ]; then
            echo "setting,maxmemory=$COMPARE_MAXMEMORY" >>"$OUT/host.txt"
        fi
    fi
    case ${COMPARE_AOF:-} in
        ''|true|false) ;;
        *)
            echo "COMPARE_AOF must be true, false or empty, not '$COMPARE_AOF'" >&2
            exit 1
            ;;
    esac
    echo "setting,aof=${COMPARE_AOF:-false}"
    if [ -f "$OUT/host.txt" ]; then
        echo "setting,aof=${COMPARE_AOF:-false}" >>"$OUT/host.txt"
    fi
    case ${COMPARE_REFERENCE:-} in
        ''|true|false) ;;
        *)
            echo "COMPARE_REFERENCE must be true, false or empty, not '$COMPARE_REFERENCE'" >&2
            exit 1
            ;;
    esac
    echo "setting,reference=${COMPARE_REFERENCE:-false}"
    if [ -f "$OUT/host.txt" ]; then
        echo "setting,reference=${COMPARE_REFERENCE:-false}" >>"$OUT/host.txt"
    fi
    if [ "${COMPARE_REFERENCE:-}" = true ]; then
        reference_version=$(redis-server --version)
        case " $reference_version " in
            *" v=7.2.16 "*) ;;
            *) echo "COMPARE_REFERENCE requires Redis 7.2.16: $reference_version" >&2; exit 1 ;;
        esac
    fi
    cpu_record=
    if [ -r /proc/stat ]; then
        cpu_record="$OUT/compare-cpu.csv"
        echo 'line,pass,cpus,test,depth,cpu,role,busy,user,system' >"$cpu_record"
        echo 'line,pass,cpus,test,depth,client_busy_mean,client_limited' >"$OUT/compare-limited.csv"
    else
        echo "skip,compare-cpu.csv,no /proc/stat"
    fi
    tests=${COMPARE_TESTS:-mset set get}
    depths=${COMPARE_DEPTHS:-16 1}
    names=
    for pair in $IMAGES; do
        names="$names ${pair%%=*}"
        sha256sum "${pair#*=}" | sed "s/^/image,${pair%%=*},/"
    done
    if [ "${COMPARE_REFERENCE:-}" = true ]; then
        names="$names reference"
    fi
    reversed=$(echo $names | tr ' ' '\n' | awk '{ a[NR] = $0 } END { for (i = NR; i > 0; i--) printf "%s ", a[i] }')
    echo 'line,pass,cpus,test,depth,requests,rps,server_cpu_us_per_request,p50_ms,p99_ms' >"$OUT/compare.csv"
    for n in ${COMPARE_CPUS:-1 2}; do
        placement_for_count "$n" "$((total - n < 1 ? 1 : total - n < 16 ? total - n : 16))" || continue
        for name in $names; do
            verify "$(compare_arg "$name")"
            case " $tests " in
                *" linsert "*|*" lmove "*) verify_list_forms "$(compare_arg "$name")" ;;
            esac
        done
        sizes="$OUT/compare-sizes-$n.txt"
        : >"$sizes"
        for test in $tests; do
            for depth in $depths; do
                fastest=0
                for name in $names; do
                    start "$(compare_arg "$name")"
                    rate=$(compare_client "$test" "$depth" 1000000 | cut -d, -f1)
                    stop
                    fastest=$(awk -v a="$fastest" -v b="$rate" 'BEGIN { print (b > a ? b : a) }')
                done
                echo "$test $depth $(awk -v r="$fastest" -v s="${COMPARE_SECONDS:-10}" 'BEGIN { printf "%d", r * s }')" >>"$sizes"
            done
        done
        pass=1
        while [ "$pass" -le "${COMPARE_PASSES:-6}" ]; do
            order=$names
            if [ $((pass % 2)) -eq 0 ]; then
                order=$reversed
            fi
            for name in $order; do
                start "$(compare_arg "$name")"
                while read -r test depth requests; do
                    echo "$name,$pass,$n,$test,$depth,$requests,$(compare_client "$test" "$depth" "$requests")" |
                        tee -a "$OUT/compare.csv"
                    if [ -n "$cpu_record" ]; then
                        cpu_busy "$OUT/compare-stat-before" "$OUT/compare-stat-after" \
                            "$SERVER_CPUS" "$CLIENT_CPUS" >"$OUT/compare-busy"
                        echo "$name,$pass,$n,$test,$depth,$(client_limited "$OUT/compare-busy")" >>"$OUT/compare-limited.csv"
                        awk -v key="$name,$pass,$n,$test,$depth" '{ print key "," $0 }' \
                            "$OUT/compare-busy" >>"$cpu_record"
                    fi
                done <"$sizes"
                stop
                cp "$OUT/server.log" "$OUT/compare-server-$name-$n-$pass.txt"
            done
            pass=$((pass + 1))
        done
        if [ -n "$PERF" ]; then
            # COMPARE_PROFILE lists test and depth pairs, separated by ';'.
            echo "${COMPARE_PROFILE:-mset 16}" | tr ';' '\n' | while read -r ptest pdepth; do
                test -n "$ptest" || continue
                requests=$(awk -v t="$ptest" -v d="$pdepth" '$1 == t && $2 == d { print $3 }' "$sizes")
                test -n "$requests" || { echo "skip,profile,$ptest $pdepth not measured"; continue; }
                for name in $names; do
                    start "$(compare_arg "$name")"
                    # With PERF_CALLERS set, each sample carries a DWARF-unwound
                    # stack, since firn keeps no frame pointers.
                    "$PERF" record ${PERF_CALLERS:+--call-graph dwarf,16384} -F "${PERF_FREQUENCY:-4999}" \
                        -p "$server" -o "$OUT/perf-$name-$n-$ptest.data" >/dev/null 2>&1 &
                    recorder=$!
                    sleep 1
                    compare_client "$ptest" "$pdepth" "$requests" | sed "s/^/profiled,$name,$n,$ptest,/"
                    kill -INT "$recorder"
                    wait "$recorder" || true
                    stop
                    "$PERF" report -i "$OUT/perf-$name-$n-$ptest.data" --stdio --no-children \
                        --sort dso,symbol --percent-limit 0.01 -g none >"$OUT/profile-$name-$n-$ptest.txt" 2>/dev/null
                    for symbol in $PERF_ANNOTATE; do
                        "$PERF" annotate -i "$OUT/perf-$name-$n-$ptest.data" --stdio -s "$symbol" \
                            >"$OUT/annotate-$name-$n-$ptest-$symbol.txt" 2>/dev/null || true
                    done
                    if [ -n "$PERF_CALLERS" ]; then
                        "$PERF" report -i "$OUT/perf-$name-$n-$ptest.data" --stdio --no-children \
                            --sort dso,symbol --percent-limit 0.3 -g caller,0.5,callee,function,percent \
                            >"$OUT/callers-$name-$n-$ptest.txt" 2>/dev/null
                    fi
                    rm -f "$OUT/perf-$name-$n-$ptest.data"
                    # With PERF_C2C set, a second run of the same test records
                    # perf c2c's load and store samples, which name the cache
                    # lines several cores write and read in turn.
                    if [ -n "$PERF_C2C" ]; then
                        start "$(compare_arg "$name")"
                        "$PERF" c2c record -p "$server" -o "$OUT/c2c-$name-$n-$ptest.data" >/dev/null 2>&1 &
                        recorder=$!
                        sleep 1
                        compare_client "$ptest" "$pdepth" "$requests" | sed "s/^/c2c,$name,$n,$ptest,/"
                        kill -INT "$recorder"
                        wait "$recorder" || true
                        stop
                        "$PERF" c2c report -i "$OUT/c2c-$name-$n-$ptest.data" --stdio --full-symbols \
                            >"$OUT/c2c-$name-$n-$ptest.txt" 2>&1 || true
                        rm -f "$OUT/c2c-$name-$n-$ptest.data"
                    fi
                done
            done
        fi
    done
    cat "$OUT/compare.csv"
    exit 0
fi

# Consumer workloads, isolated so the session RSS is not a mixed keyspace.
# Keep the client's exit status outside a pipeline or echo substitution: a
# RESP error, including one nested in EXEC, must fail the measurement, except
# for the exact OOM refusal counted by the eviction workload's miss SET.
if [ "$MODE" = workloads ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
    manifest="$ROOT/research/experiments/redis-bench/workload/Cargo.toml"
    target="$OUT/workload-target"
    cargo test --offline --locked --release --manifest-path "$manifest" --target-dir "$target"
    cargo build --offline --locked --release --manifest-path "$manifest" --target-dir "$target"
    client="$target/release/firn-workload"
    total=$(nproc)
    workloads=${WORKLOADS:-limiter-script limiter-tx setmany-tx session-set session-get}
    if [ -n "${WORKLOAD_IMAGES:-}" ] && [ -z "${IMAGES:-}" ]; then
        echo "WORKLOAD_IMAGES needs IMAGES, the name=path pairs of the images to measure" >&2
        exit 1
    fi
    memory_workloads=
    connection_workloads=
    # The workloads that send a publisher's or producer's commands in batches
    # of a depth: pub/sub delivery and blocking hand-off. Their files keep the
    # pub/sub names.
    delivery_workloads=
    for workload in $workloads; do
        case $workload in
            limiter-script|limiter-tx|setmany-tx|session-set|session-get) ;;
            evict-zipf|session-set-limits|session-get-limits) memory_workloads=1 ;;
            handshake|client-list) connection_workloads=1 ;;
            pubsub-channel|pubsub-pattern|blocking-handoff) delivery_workloads=1 ;;
            *) echo "unknown workload: $workload" >&2; exit 1 ;;
        esac
    done
    if [ -n "$memory_workloads$connection_workloads$delivery_workloads" ]; then
        reference_version=$(redis-server --version)
        case " $reference_version " in
            *" v=7.2.16 "*) ;;
            *)
                if [ -n "$connection_workloads" ]; then
                    echo "handshake/client-list workloads require Redis 7.2.16: $reference_version" >&2
                elif [ -n "$delivery_workloads" ]; then
                    echo "pub/sub and blocking-handoff workloads require Redis 7.2.16: $reference_version" >&2
                else
                    echo "memory workloads require Redis 7.2.16: $reference_version" >&2
                fi
                exit 1
                ;;
        esac
    fi
    # Workflow's empty input leaves the default selection intact. A nonempty
    # selection must contain known, distinct names and at least one server.
    if [ -n "${WORKLOAD_SERVERS:-}" ]; then
        seen_servers=
        set -f
        for selected in $WORKLOAD_SERVERS; do
            case $selected in
                reference|redis-latest|firn|valkey|valkey-io|dragonfly|garnet) ;;
                valkey-latest|valkey-io-latest|dragonfly-latest|garnet-latest) ;;
                *) echo "unknown WORKLOAD_SERVERS name: $selected" >&2; exit 1 ;;
            esac
            case " $seen_servers " in
                *" $selected "*) echo "duplicate WORKLOAD_SERVERS name: $selected" >&2; exit 1 ;;
            esac
            seen_servers="$seen_servers $selected"
        done
        [ -n "$seen_servers" ] || { echo "WORKLOAD_SERVERS needs at least one server name" >&2; exit 1; }
    fi
    echo 'line,pass,cpus,workload,connections,requests,seconds,rate,p50_ms,p99_ms' >"$OUT/workloads.csv"
    echo 'line,pass,cpus,workload,sessions,rss_kib' >"$OUT/workloads-memory.csv"
    echo 'line,pass,cpus,workload,connections,pipeline,published,delivered,seconds,rate,p50_ms,p99_ms' >"$OUT/pubsub.csv"
    workload_cpu=
    if [ -r /proc/stat ]; then
        workload_cpu="$OUT/workload-cpu.csv"
        echo 'line,pass,cpus,test,depth,cpu,role,busy,user,system' >"$workload_cpu"
        echo 'line,pass,cpus,workload,connections,client_busy_mean,client_work_fraction,client_limited' >"$OUT/workload-limited.csv"
        echo 'line,pass,cpus,workload,connections,pipeline,client_busy_mean,client_work_fraction,client_limited' >"$OUT/pubsub-limited.csv"
    else
        echo "skip,workload-cpu.csv,no /proc/stat"
    fi
    # Copies of /proc/stat just outside one client run, for cpu_busy: the run
    # includes the client's own connection setup and warm-up, which the
    # busy fraction therefore covers. workload_end takes <line> <connections>
    # and appends the sample's rows to workload-cpu.csv and workload-limited.csv;
    # with a third argument, the pub/sub pipeline depth, to pubsub-limited.csv
    # and workload-cpu.csv under that depth.
    workload_begin() {
        rm -f "$OUT/workload-work"
        [ -z "$workload_cpu" ] || cat /proc/stat >"$OUT/workload-stat-before"
    }
    workload_end() {
        # Every step exits on failure, so that a missing or malformed work
        # file never leaves a short row in workload-limited.csv.
        if [ -n "$workload_cpu" ]; then
            cat /proc/stat >"$OUT/workload-stat-after" || exit 1
            cpu_busy "$OUT/workload-stat-before" "$OUT/workload-stat-after" \
                "$SERVER_CPUS" "$CLIENT_CPUS" >"$OUT/workload-busy" || exit 1
            limited=$(client_limited "$OUT/workload-busy") || exit 1
            busy=${limited%%,*}
            work=$(work_limited "$OUT/workload-work") || exit 1
            if [ -n "${3:-}" ]; then
                echo "$1,$pass,$n,$workload,$2,$3,$busy,$work" >>"$OUT/pubsub-limited.csv"
                awk -v key="$1,$pass,$n,$workload,$3" '{ print key "," $0 }' "$OUT/workload-busy" >>"$workload_cpu" || exit 1
            else
                echo "$1,$pass,$n,$workload,$2,$busy,$work" >>"$OUT/workload-limited.csv"
                awk -v key="$1,$pass,$n,$workload,$2" '{ print key "," $0 }' "$OUT/workload-busy" >>"$workload_cpu" || exit 1
            fi
        fi
    }
    echo 'line,pass,cpus,workload,connections,requests,seconds,rate,p50_ms,p99_ms,keys,zipf_s,value_size,seed,warmup_seconds,sample_ms,get_count,hits,misses,refused_sets,hit_rate,evicted_keys_delta,used_memory,used_memory_peak,maxmemory,prefill_used_memory,filled_used_memory,peak_at_measurement_start,lifetime_peak_excess_bytes,lifetime_peak_excess_fraction,sampled_max_memory,sampled_excess_bytes,sampled_excess_fraction,memory_samples,max_sample_gap_ms,mem_not_counted_for_evict,writer_connections,keys_at_start,keys_at_end' >"$OUT/evict-zipf.csv"
    echo 'line,pass,cpus,workload,connections,keys,value_size,seed,warmup_seconds,sample_ms,maxmemory' >"$OUT/session-limits-settings.csv"
    # Accept only the measurement parameters shared by these opt-in workloads;
    # the harness owns port, workload, duration, seed and maxmemory. Disable glob
    # expansion and never eval workflow input. The client checks numeric values.
    memory_options() {
        set -f
        set -- ${WORKLOAD_OPTIONS:-}
        while [ "$#" -gt 0 ]; do
            case $1 in
                --keys|--value-size|--zipf-s|--warmup-seconds|--sample-ms|--window) ;;
                *) echo "unsupported WORKLOAD_OPTIONS flag: $1" >&2; exit 1 ;;
            esac
            [ "$#" -ge 2 ] || { echo "WORKLOAD_OPTIONS needs a value for $1" >&2; exit 1; }
            shift 2
        done
    }
    memory_options
    if [ -n "$connection_workloads" ]; then
        set -f
        set -- ${WORKLOAD_OPTIONS:-}
        while [ "$#" -gt 0 ]; do
            case $1 in
                --warmup-seconds|--sample-ms) ;;
                *) echo "handshake/client-list do not support WORKLOAD_OPTIONS flag: $1" >&2; exit 1 ;;
            esac
            [ "$#" -ge 2 ] || { echo "WORKLOAD_OPTIONS needs a value for $1" >&2; exit 1; }
            shift 2
        done
    fi
    if [ -n "$delivery_workloads" ]; then
        set -f
        set -- ${WORKLOAD_OPTIONS:-}
        while [ "$#" -gt 0 ]; do
            case $1 in
                --warmup-seconds|--value-size) ;;
                --window)
                    case " $workloads " in
                        *" blocking-handoff "*)
                            echo "blocking-handoff does not support WORKLOAD_OPTIONS flag: $1" >&2
                            exit 1
                            ;;
                    esac
                    ;;
                *) echo "pub/sub and blocking-handoff workloads do not support WORKLOAD_OPTIONS flag: $1" >&2; exit 1 ;;
            esac
            [ "$#" -ge 2 ] || { echo "WORKLOAD_OPTIONS needs a value for $1" >&2; exit 1; }
            shift 2
        done
    fi
    memory_run() {
        taskset -c "$CLIENT_CPUS" "$client" ${WORKLOAD_OPTIONS:-} \
            --port "$PORT" --threads "$CLIENT_THREADS" --connections "$conns" \
            --workload "$memory_workload" --seconds "${WORKLOAD_SECONDS:-10}" \
            --seed "$pass" "$@" || {
                memory_status=$?
                echo "memory workload failed: line=$line pass=$pass cpus=$n workload=$workload variant=$variant connections=$conns (exit $memory_status)" >&2
                return "$memory_status"
            }
    }
    # Only this mode changes cleanup; the workflow's registered-session cleanup
    # also handles cancellation on the shared runner.
    trap '[ -z "$server" ] || { kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; }' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    for n in ${WORKLOAD_CPUS:-1 2}; do
        placement_for_count "$n" "$((total - n < 1 ? 1 : total - n < 16 ? total - n : 16))" || continue
        pass=1
        while [ "$pass" -le "${WORKLOAD_PASSES:-3}" ]; do
            order="reference reference-aof firn-$n firn-aof-$n"
            if [ -n "${WORKLOAD_IMAGES:-}" ]; then
                order=reference
                for pair in $IMAGES; do
                    order="$order image-${pair%%=*}"
                done
            fi
            if [ -n "${WORKLOAD_SERVERS:-}" ]; then
                order=
                for selected in $WORKLOAD_SERVERS; do
                    case $selected in
                        firn)
                            if [ -n "${WORKLOAD_IMAGES:-}" ]; then
                                for pair in $IMAGES; do
                                    order="$order image-${pair%%=*}"
                                done
                            else
                                order="$order firn-$n"
                            fi
                            ;;
                        dragonfly|garnet|dragonfly-latest|garnet-latest) order="$order $selected-$n" ;;
                        *) order="$order $selected" ;;
                    esac
                done
                selected_order=
                for line in $order; do
                    if available "$line"; then
                        selected_order="$selected_order $line"
                    else
                        echo "skip,$line,no executable"
                    fi
                done
                [ -n "$selected_order" ] || {
                    echo "WORKLOAD_SERVERS has no available server to measure" >&2
                    exit 1
                }
                order=$selected_order
            fi
            if [ $((pass % 2)) -eq 0 ]; then
                reversed=
                for line in $order; do
                    reversed="$line $reversed"
                done
                order=$reversed
            fi
            for workload in $workloads; do
                workload_order=$order
                case $workload in
                    pubsub-channel|pubsub-pattern|blocking-handoff)
                        # No AOF lines: a publication is never written to the
                        # append-only file, and the hand-off is measured without
                        # it.
                        if [ -z "${WORKLOAD_SERVERS:-}${WORKLOAD_IMAGES:-}" ]; then
                            workload_order="reference firn-$n"
                            if [ $((pass % 2)) -eq 0 ]; then
                                workload_order="firn-$n reference"
                            fi
                        fi
                        for line in $workload_order; do
                            if ! available "$line"; then
                                echo "skip,$line,no executable"
                                continue
                            fi
                            start "$line"
                            unsupported=
                            for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                                for depth in ${WORKLOAD_DEPTHS:-16 1}; do
                                    [ -z "$unsupported" ] || continue
                                    workload_begin
                                    status=0
                                    result=$(taskset -c "$CLIENT_CPUS" "$client" ${WORKLOAD_OPTIONS:-} \
                                        --port "$PORT" --threads "$CLIENT_THREADS" --connections "$conns" \
                                        --workload "$workload" --seconds "${WORKLOAD_SECONDS:-10}" \
                                        --pipeline "$depth" --client-work-file "$OUT/workload-work") || status=$?
                                    if [ "$status" -eq 3 ]; then
                                        case $line in
                                            firn-*|image-*)
                                                echo "$line does not support $workload" >&2
                                                exit 1
                                                ;;
                                        esac
                                        echo "skip,$line,$workload unsupported"
                                        unsupported=1
                                        continue
                                    fi
                                    if [ "$status" -ne 0 ]; then
                                        echo "$workload failed: line=$line pass=$pass cpus=$n connections=$conns pipeline=$depth (exit $status)" >&2
                                        exit "$status"
                                    fi
                                    workload_end "$line" "$conns" "$depth"
                                    echo "$line,$pass,$n,$result" | tee -a "$OUT/pubsub.csv"
                                done
                            done
                            # Profile only after every measured run on this
                            # server, so that no measured run follows an
                            # unmeasured one; a supported firn line, pass 1.
                            case $line in firn-*|image-*) profiled=1 ;; *) profiled= ;; esac
                            if [ -n "$PERF" ] && [ -n "$profiled" ] && [ "$pass" -eq 1 ]; then
                                for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                                    for depth in ${WORKLOAD_DEPTHS:-16 1}; do
                                        name="$line-$n-$workload-$conns-$depth"
                                        "$PERF" record -F "${PERF_FREQUENCY:-4999}" -p "$server" \
                                            -o "$OUT/perf-$name.data" >/dev/null 2>&1 &
                                        recorder=$!
                                        sleep 1
                                        # The profile is evidence about a measured cell,
                                        # never part of it: a profile that cannot be
                                        # taken (no usable perf, a recorder that fails
                                        # to start) is reported and does not stop the
                                        # run, whose measurements are already written.
                                        taskset -c "$CLIENT_CPUS" "$client" ${WORKLOAD_OPTIONS:-} \
                                            --port "$PORT" --threads "$CLIENT_THREADS" --connections "$conns" \
                                            --workload "$workload" --seconds "${WORKLOAD_SECONDS:-10}" \
                                            --pipeline "$depth" >/dev/null ||
                                            echo "skip,profile,$name: the profiling client failed"
                                        kill -INT "$recorder" 2>/dev/null || true
                                        wait "$recorder" || true
                                        if "$PERF" report -i "$OUT/perf-$name.data" --stdio --no-children \
                                            --sort dso,symbol --percent-limit 0.5 -g none >"$OUT/profile-$name.txt" 2>/dev/null; then
                                            echo "== profile $name"
                                            grep -v '^#' "$OUT/profile-$name.txt" | grep -v '^$' | head -25
                                        else
                                            rm -f "$OUT/profile-$name.txt"
                                            echo "skip,profile,$name: perf report failed"
                                        fi
                                        rm -f "$OUT/perf-$name.data"
                                    done
                                done
                            fi
                            stop
                            # The log of a firn line ends with the runtime's
                            # scheduling counts when WF_SCHED_REPORT is set.
                            case $line in
                                firn-*|image-*) cp "$OUT/server.log" "$OUT/workload-server-$line-$n-$pass-$workload.txt" ;;
                            esac
                            server=
                        done
                        continue
                        ;;
                    handshake|client-list)
                        if [ -z "${WORKLOAD_SERVERS:-}${WORKLOAD_IMAGES:-}" ]; then
                            workload_order="reference firn-$n"
                            if [ $((pass % 2)) -eq 0 ]; then
                                workload_order="firn-$n reference"
                            fi
                        fi
                        for line in $workload_order; do
                            if ! available "$line"; then
                                echo "skip,$line,no executable"
                                continue
                            fi
                            start "$line"
                            for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                                workload_begin
                                result=$(taskset -c "$CLIENT_CPUS" "$client" ${WORKLOAD_OPTIONS:-} \
                                    --port "$PORT" --threads "$CLIENT_THREADS" --connections "$conns" \
                                    --workload "$workload" --seconds "${WORKLOAD_SECONDS:-10}" \
                            --client-work-file "$OUT/workload-work")
                                workload_end "$line" "$conns"
                                echo "$line,$pass,$n,$result" | tee -a "$OUT/workloads.csv"
                            done
                            stop
                            server=
                        done
                        continue
                        ;;
                esac
                case $workload in
                    evict-zipf|session-set-limits|session-get-limits)
                        # Each sample starts with empty memory and a fresh AOF,
                        # including twins and additional connection counts.
                        memory_workload=${workload%-limits}
                        variants=eviction
                        if [ "$workload" != evict-zipf ]; then
                            variants="unlimited unlimited-twin nonbinding nonbinding-twin"
                            if [ $((pass % 2)) -eq 0 ]; then
                                variants="nonbinding-twin nonbinding unlimited-twin unlimited"
                            fi
                        fi
                        for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                            for line in $workload_order; do
                                if ! available "$line"; then
                                    echo "skip,$line,no executable"
                                    continue
                                fi
                                for variant in $variants; do
                                    start "$line"
                                    if [ "$variant" = eviction ]; then
                                        result=$(memory_run)
                                        echo "$line,$pass,$n,$result" | tee -a "$OUT/evict-zipf.csv"
                                        rates=$(printf '%s\n' "$result" | cut -d, -f1-7)
                                        echo "$line,$pass,$n,$rates" | tee -a "$OUT/workloads.csv"
                                    else
                                        case $variant in
                                            unlimited*) limit=0 ;;
                                            nonbinding*) limit=68719476736 ;;
                                        esac
                                        result=$(memory_run --maxmemory "$limit")
                                        # Seven rate columns followed by the actual
                                        # parsed settings; keep both as CSV artifacts.
                                        rates=$(printf '%s\n' "$result" | cut -d, -f1-7)
                                        settings=$(printf '%s\n' "$result" | cut -d, -f8-)
                                        echo "$line-$variant,$pass,$n,$rates" | tee -a "$OUT/workloads.csv"
                                        echo "$line-$variant,$pass,$n,$memory_workload,$conns,$settings" | tee -a "$OUT/session-limits-settings.csv"
                                    fi
                                    stop
                                    server=
                                done
                            done
                        done
                        continue
                        ;;
                esac
                for line in $workload_order; do
                    if ! available "$line"; then
                        echo "skip,$line,no executable"
                        continue
                    fi
                    start "$line"
                    if [ "$workload" = session-get ]; then
                        taskset -c "$CLIENT_CPUS" "$client" --port "$PORT" --fill 1000000
                        rss=$(awk '/^VmRSS:/ { print $2; found=1 } END { if (!found) exit 1 }' "/proc/$server/status")
                        echo "$line,$pass,$n,$workload,1000000,$rss" | tee -a "$OUT/workloads-memory.csv"
                    fi
                    for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                        workload_begin
                        result=$(taskset -c "$CLIENT_CPUS" "$client" --port "$PORT" \
                            --threads "$CLIENT_THREADS" --connections "$conns" \
                            --workload "$workload" --seconds "${WORKLOAD_SECONDS:-10}" \
                            --client-work-file "$OUT/workload-work")
                        workload_end "$line" "$conns"
                        echo "$line,$pass,$n,$result" | tee -a "$OUT/workloads.csv"
                    done
                    # Profile only after every measured run on this server, so
                    # no measured run follows an unmeasured one here.
                    case $line in firn-*|image-*) profiled=1 ;; *) profiled= ;; esac
                    if [ -n "$PERF" ] && [ -n "$profiled" ] && [ "$pass" -eq 1 ]; then
                        for conns in ${WORKLOAD_CONNECTIONS:-50}; do
                            name="$line-$n-$workload-$conns"
                            "$PERF" record -F "${PERF_FREQUENCY:-4999}" -p "$server" \
                                -o "$OUT/perf-$name.data" >/dev/null 2>&1 &
                            recorder=$!
                            sleep 1
                            taskset -c "$CLIENT_CPUS" "$client" --port "$PORT" \
                                --threads "$CLIENT_THREADS" --connections "$conns" \
                                --workload "$workload" --seconds "${WORKLOAD_SECONDS:-10}" >/dev/null
                            kill -INT "$recorder"
                            wait "$recorder" || true
                            "$PERF" report -i "$OUT/perf-$name.data" --stdio --no-children \
                                --sort dso,symbol --percent-limit 0.5 -g none >"$OUT/profile-$name.txt" 2>/dev/null
                            rm -f "$OUT/perf-$name.data"
                            echo "== profile $name"
                            grep -v '^#' "$OUT/profile-$name.txt" | grep -v '^$' | head -25
                        done
                    fi
                    stop
                    server=
                done
            done
            pass=$((pass + 1))
        done
    done
    exit 0
fi

# The quick comparison, for the loop of changing firn and measuring again:
# firn on QUICK_CPUS server CPUs against the fastest of Garnet and Dragonfly,
# which led every test of the scaling run. Those two are measured once per
# CPU count and client count and kept in quick-ref-<n>-<clients>.csv, with
# its CPU lists in the adjacent .placement file, until QUICK_REFRESH is set
# or the placement changes; nothing
# is verified. Its rates are its own: its client is not the suite's.
if [ "$MODE" = quick ]; then
    n=${QUICK_CPUS:-4}
    total=$(nproc)
    all="set get incr lpush rpop sadd hset zadd lrange_100 mset"
    select_placement "$n" "${QUICK_CLIENTS:-$((total - n < 1 ? 1 : total - n < 16 ? total - n : 16))}"
    reference="$OUT/quick-ref-$n-$CLIENT_THREADS.csv"
    reference_placement="$SERVER_CPUS/$CLIENT_CPUS"
    if [ -n "$QUICK_REFRESH" ] || [ ! -s "$reference" ] ||
        [ "$(cat "$reference.placement" 2>/dev/null || true)" != "$reference_placement" ]; then
        : >"$reference.new"
        for line in "garnet-$n" "dragonfly-$n"; do
            if available "$line"; then
                quick_measure "$line" "$reference.new" "$all"
            else
                echo "skip,$line,no executable"
            fi
        done
        mv "$reference.new" "$reference"
        echo "$reference_placement" >"$reference.placement"
    fi
    line=${QUICK_LINE:-firn}-$n
    : >"$OUT/quick-$n.csv"
    quick_measure "$line" "$OUT/quick-$n.csv" "${QUICK_TESTS:-$all}"
    # The ratio every test is asked to reach, the owner's aim of 2026-10-02.
    awk -F, -v target="${QUICK_TARGET:-1.4}" -v n="$n" -v line="$line" '
        FNR == NR { if ($3 + 0 > best[$2]) { best[$2] = $3 + 0; by[$2] = $1 } next }
        FNR == 1 {
            printf "%-11s %9s %9s %-12s %6s\n", "n=" n, line, "best", "", "ratio"
        }
        {
            ratio = $3 / best[$2]
            if (ratio < target) below++
            printf "%-11s %9.0f %9.0f %-12s %6.2f %s\n", $2, $3 / 1000,
                best[$2] / 1000, by[$2], ratio, (ratio < target ? "BELOW" : "")
        }
        END { printf "%d of %d below %s\n", below, FNR, target }' \
        "$reference" "$OUT/quick-$n.csv"
    exit 0
fi

# The scaling run: on each server CPU count n in SCALE, servers and clients
# occupy distinct physical cores selected by placement, with one client
# thread per client CPU up to 16; every line is checked on n CPUs, a pilot
# sizes the runs for n, and then PASSES passes measure the lines interleaved.
if [ "$MODE" = scale ]; then
    total=$(nproc)
    SUITE_TESTS=${SCALE_TESTS:-"set get incr lpush rpop sadd hset zadd lrange_100 mset"}
    PIPELINES=${SCALE_PIPELINES:-16}
    for n in ${SCALE:-2 4 8 16}; do
        # quick_client starts one process per client CPU and counts
        # CLIENT_THREADS of them, so the two must name the same CPUs.
        placement_for_count "$n" "$((total - n < 1 ? 1 : total - n < 16 ? total - n : 16))" || continue
        lines="reference valkey-io dragonfly-$n garnet-$n firn-$n firn-base-$n"
        for line in $lines; do
            if available "$line"; then
                verify_suite "$line"
            else
                echo "skip,$line,no executable"
            fi
        done
        pilot
        pass=1
        while [ "$pass" -le "$PASSES" ]; do
            for line in $lines; do
                if available "$line"; then
                    suite_run "$line" "pass $pass" "$n server CPUs"
                fi
            done
            pass=$((pass + 1))
        done
    done
    exit 0
fi

if [ "$MODE" = suite ]; then
    two="reference valkey valkey-io dragonfly-2 garnet-2 firn-2 firn-base-2"
    one="reference valkey dragonfly-1 garnet-1 firn-1 firn-base-1"
    for line in $two; do
        if available "$line"; then
            verify_suite "$line"
        else
            echo "skip,$line,no executable"
        fi
    done
    select_placement 2 2
    pilot
    pass=1
    while [ "$pass" -le "$PASSES" ]; do
        select_placement 2 2
        for line in $two; do
            if available "$line"; then
                suite_run "$line" "pass $pass" "2 server CPUs"
            fi
        done
        select_placement 1 3
        for line in $one; do
            if available "$line"; then
                suite_run "$line" "pass $pass" "1 server CPU"
            fi
        done
        pass=$((pass + 1))
    done
    exit 0
fi

lines="reference firn-2 firn-1 $baselines reference-aof firn-aof-2"
for line in $lines; do
    verify "$line"
done
for line in reference firn-2 reference-aof firn-aof-2; do
    verify_expiry "$line"
done
for line in reference-aof firn-aof-2; do
    verify_restart "$line"
done
for line in reference firn-2; do
    verify_idle "$line"
    verify_active "$line"
done
if [ "$MODE" = verify ]; then
    exit 0
fi
round=1
while [ "$round" -le "$ROUNDS" ]; do
    for line in $lines; do
        measure "$line" "$round"
    done
    round=$((round + 1))
done
