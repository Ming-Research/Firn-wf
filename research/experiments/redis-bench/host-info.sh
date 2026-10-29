# Read-only host inventory shared by the workflow and standalone instrument.
# Sourcing starts nothing. Keep raw class membership/capacity next to each
# CPU's policies: governors, preferences and frequency limits can differ even
# within one class.
# Placement records supply the selected CPUs' normalized P/E/- class labels.
host_read_file() {
    host_value=$(cat "$1" 2>/dev/null) || host_value=
    printf '%s\n' "${host_value:-unknown}"
}

# Optional host facts must never stop a measurement. The subshell also keeps
# the caller's errexit setting and variables intact. A missing tool, unreadable
# sysfs node, or empty result is recorded as unknown, without changing policy.
record_host_system() (
    set +e
    host_sysfs=${SYSFS_ROOT:-/sys}
    printf '%s\n' 'host-system-begin'
    host_value=$(uname -srvmo 2>/dev/null) || host_value=
    printf 'kernel: %s\n' "${host_value:-unknown}"
    printf '%s\n' 'distro-release-begin'
    host_read_file /etc/os-release
    printf '%s\n' 'distro-release-end' 'cpu-summary-begin'
    host_value=$(LC_ALL=C lscpu 2>/dev/null) || host_value=
    printf '%s\n' "${host_value:-unknown}"
    printf '%s\n' 'cpu-summary-end' 'cpu-topology-begin'
    host_value=$(LC_ALL=C lscpu -e=CPU,SOCKET,CORE,ONLINE 2>/dev/null) || host_value=
    printf '%s\n' "${host_value:-unknown}"
    printf '%s\n' 'cpu-topology-end' 'microcode-begin'
    # Preserve all reported revisions, including hosts with unlike sockets.
    host_value=$(grep '^microcode[[:space:]]*:' /proc/cpuinfo 2>/dev/null) || host_value=
    printf '%s\n' "${host_value:-unknown}"
    printf '%s\n' 'microcode-end'
    host_value=$(grep '^MemTotal:' /proc/meminfo 2>/dev/null) || host_value=
    printf 'total-memory: %s\n' "${host_value:-unknown}"
    printf 'performance-class-cpus: %s\n' "$(host_read_file "$host_sysfs/devices/cpu_core/cpus")"
    printf 'efficiency-class-cpus: %s\n' "$(host_read_file "$host_sysfs/devices/cpu_atom/cpus")"
    host_found_cpu=false
    for host_cpu_path in "$host_sysfs"/devices/system/cpu/cpu[0-9]*; do
        [ -d "$host_cpu_path" ] || continue
        host_found_cpu=true
        printf 'cpu-policy,cpu=%s,core_type=%s,cpu_capacity=%s,scaling_governor=%s,energy_performance_preference=%s,scaling_min_freq_khz=%s,scaling_max_freq_khz=%s,cpuinfo_max_freq_khz=%s\n' \
            "${host_cpu_path##*/cpu}" \
            "$(host_read_file "$host_cpu_path/topology/core_type")" \
            "$(host_read_file "$host_cpu_path/cpu_capacity")" \
            "$(host_read_file "$host_cpu_path/cpufreq/scaling_governor")" \
            "$(host_read_file "$host_cpu_path/cpufreq/energy_performance_preference")" \
            "$(host_read_file "$host_cpu_path/cpufreq/scaling_min_freq")" \
            "$(host_read_file "$host_cpu_path/cpufreq/scaling_max_freq")" \
            "$(host_read_file "$host_cpu_path/cpufreq/cpuinfo_max_freq")"
    done
    if [ "$host_found_cpu" = false ]; then
        printf '%s\n' 'cpu-policy,cpu=unknown,core_type=unknown,cpu_capacity=unknown,scaling_governor=unknown,energy_performance_preference=unknown,scaling_min_freq_khz=unknown,scaling_max_freq_khz=unknown,cpuinfo_max_freq_khz=unknown'
    fi
    printf 'intel_pstate-no_turbo: %s\n' "$(host_read_file "$host_sysfs/devices/system/cpu/intel_pstate/no_turbo")"
    printf 'intel_pstate-status: %s\n' "$(host_read_file "$host_sysfs/devices/system/cpu/intel_pstate/status")"
    printf 'cpufreq-boost: %s\n' "$(host_read_file "$host_sysfs/devices/system/cpu/cpufreq/boost")"
    printf 'smt-active: %s\n' "$(host_read_file "$host_sysfs/devices/system/cpu/smt/active")"
    printf 'smt-control: %s\n' "$(host_read_file "$host_sysfs/devices/system/cpu/smt/control")"
    printf 'transparent-hugepages-enabled: %s\n' "$(host_read_file "$host_sysfs/kernel/mm/transparent_hugepage/enabled")"
    printf 'transparent-hugepages-defrag: %s\n' "$(host_read_file "$host_sysfs/kernel/mm/transparent_hugepage/defrag")"
    printf 'vm.overcommit_memory: %s\n' "$(host_read_file /proc/sys/vm/overcommit_memory)"
    printf 'vm.swappiness: %s\n' "$(host_read_file /proc/sys/vm/swappiness)"
    # host.txt is a public CI artifact: record only performance-relevant
    # kernel parameters, never the raw command line (root=PARTUUID, secrets).
    # An unreadable command line is unknown, not none.
    if host_cmdline=$(cat "${PROC_CMDLINE:-/proc/cmdline}" 2>/dev/null); then
        host_params=
        for host_word in $host_cmdline; do
            case ${host_word%%=*} in
                isolcpus|nohz_full|rcu_nocbs|irqaffinity|mitigations|nosmt|intel_pstate|amd_pstate|idle|processor.max_cstate|intel_idle.max_cstate|transparent_hugepage|hugepages|default_hugepagesz|threadirqs|preempt|nmi_watchdog|skew_tick)
                    host_params="${host_params:+$host_params }$host_word" ;;
            esac
        done
    else
        host_params=unknown
    fi
    printf 'kernel-command-line-allowlisted: %s\n' "${host_params:-none}"
    # is-active prints inactive/failed even with a nonzero status; retain those
    # observations. No systemd or unreadable service state means unknown.
    host_value=$(timeout 3s systemctl --no-pager is-active irqbalance.service 2>/dev/null)
    printf 'irqbalance-active: %s\n' "${host_value:-unknown}"
    printf 'perf_event_paranoid: %s\n' "$(host_read_file /proc/sys/kernel/perf_event_paranoid)"
    printf 'loopback-mtu: %s\n' "$(host_read_file "$host_sysfs/class/net/lo/mtu")"
    printf '%s\n' 'host-system-end'
    return 0
)
