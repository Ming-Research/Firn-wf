# Sourced by redis-bench.sh, including its placement-selftest path. Arguments
# are emitted one per line (empty Redis --save argument included), never eval'd.
# Persistence/thread options follow perf-baseline/README.md rules 1-3.
competitor_args() (
    line=$1 n=$2 aof=$3 port=$4 directory=$5 idle=$6 affinity=$7
    case $n in ''|*[!0-9]*|0) echo 'invalid competitor CPU count' >&2; exit 1 ;; esac
    case $aof in true|false) ;; *) echo 'invalid competitor AOF setting' >&2; exit 1 ;; esac
    case $line in
        reference|reference-io|valkey|valkey-io)
            case $line in *-io) [ "$n" -ge 2 ] || { echo 'I/O line requires at least two CPUs' >&2; exit 1; } ;; esac
            set -- --port "$port" --save '' --timeout "$idle" --daemonize no
            if [ "$aof" = true ]; then
                set -- "$@" --appendonly yes --appendfsync everysec \
                    --auto-aof-rewrite-percentage 0 --dir "$directory"
            else
                set -- "$@" --appendonly no
            fi
            case $line in *-io) set -- "$@" --io-threads "$n" ;; esac
            [ "$line" != reference-io ] || set -- "$@" --io-threads-do-reads yes
            ;;
        dragonfly)
            [ "$aof" = false ] || { echo 'Dragonfly has no AOF; refusing unequal durability' >&2; exit 1; }
            set -- "--port=$port" "--proactor_threads=$n" --dbfilename= --logtostderr
            case $affinity in
                on) ;;
                off) set -- "$@" --proactor_affinity_mode=off ;;
                *) echo 'invalid Dragonfly affinity mode' >&2; exit 1 ;;
            esac
            ;;
        garnet)
            set -- --port "$port" --bind 127.0.0.1 --lua
            if [ "$aof" = true ]; then
                # WaitForCommit stays false (the pinned default); no reply waits
                # for the commit. This does not establish fsync equivalence.
                # Pinned Garnet's AOF base is CheckpointDir. LogDir is only for
                # tiered storage and throws when tiering is disabled.
                set -- "$@" --aof --aof-commit-freq 1000 --checkpointdir "$directory"
            fi
            ;;
        *) echo "unknown competitor: $line" >&2; exit 1 ;;
    esac
    printf '%s\n' "$@"
)

# The helpers use Python already required by the instrument. Fake proc trees
# exercise missing inputs and escaped CPU masks without starting any server.
competitor_observation() {
    python3 - "$@" <<'PY'
import csv
from pathlib import Path
import re
import sys

def cpus(text):
    result = set()
    for part in text.split(','):
        if not re.fullmatch(r'[0-9]+(?:-[0-9]+)?', part):
            raise ValueError(f'invalid CPU list: {text!r}')
        ends = [int(x) for x in part.split('-')]
        if ends[0] > ends[-1]:
            raise ValueError(f'reversed CPU range: {part}')
        result.update(range(ends[0], ends[-1] + 1))
    return result

def descendants(proc, pid):
    found = {pid}
    parents = {}
    for path in proc.glob('[0-9]*/status'):
        try:
            parents[int(path.parent.name)] = int(field(path.read_text(), 'PPid'))
        except FileNotFoundError:
            continue  # An unrelated process ended during enumeration.
    while True:
        children = {child for child, parent in parents.items() if parent in found}
        if children <= found:
            return sorted(found)
        found |= children

def field(text, key):
    for line in text.splitlines():
        if line.startswith(key + ':'):
            return line.split(':', 1)[1].strip()
    raise ValueError(f'missing {key}')

def affinity(proc, pid, mask):
    allowed = cpus(mask)
    checked = 0
    for process in descendants(proc, pid):
        tasks = list((proc / str(process) / 'task').glob('[0-9]*/status'))
        if not tasks:
            raise ValueError(f'no threads for pid {process}')
        for path in tasks:
            try:
                value = field(path.read_text(), 'Cpus_allowed_list')
            except FileNotFoundError:
                continue  # A short-lived thread ended between list and read.
            checked += 1
            if not cpus(value) <= allowed:
                # Only an escaped mask gets status 2 and permits fallback.
                print(f'affinity escaped: {path}: {value}, server list {mask}', file=sys.stderr)
                return 2
    if not checked:
        raise ValueError('no live threads to check')
    print(f'threads_checked={checked},server_cpus={mask}')
    return 0

def memory(proc, pid, info):
    rss = hwm = 0
    processes = descendants(proc, pid)
    for process in processes:
        text = (proc / str(process) / 'status').read_text()
        for key in ('VmRSS', 'VmHWM'):
            value = field(text, key)
            if not re.fullmatch(r'[0-9]+\s+kB', value):
                raise ValueError(f'invalid {key} for pid {process}: {value}')
            number = int(value.split()[0]) * 1024
            if key == 'VmRSS':
                rss += number
            else:
                hwm += number
    values = {}
    for line in info.read_text().splitlines():
        key, sep, value = line.partition(':')
        if sep and re.fullmatch(r'[0-9]+', value.strip()):
            values[key] = int(value)
    # A missing used_memory remains empty, never a fabricated zero. RSS must
    # cover the process tree: INFO is usable only for a single process.
    source = 'proc_status_sum'
    if len(processes) == 1 and 'used_memory_rss' in values:
        rss = values['used_memory_rss']
        source = 'info_memory'
    return [values.get('used_memory', ''), rss, source, hwm]

def aof_bytes(directory):
    # This directory is fresh and belongs only to this AOF server; no server
    # console log is placed in it. Size growth demonstrates file writes, not
    # fsync. Record allocated blocks too, for sparse/preallocated segments;
    # committed() provides logical growth when physical size stays constant.
    sizes = blocks = 0
    for path in (directory / 'AOF').glob('aof.log*'):
        if path.is_file():
            stat = path.stat()
            sizes += stat.st_size
            blocks += stat.st_blocks * 512
    return sizes, blocks

def committed(info):
    # The pinned default is one physical AOF sublog. Its INFO address is an
    # integer; N/A means disabled and vectors imply a different configuration.
    values = [line.split(':', 1)[1].strip() for line in info.read_text().splitlines()
              if line.startswith('CommittedUntilAddress:')]
    if len(values) != 1 or not re.fullmatch(r'[0-9]+', values[0]):
        raise ValueError('Garnet needs one numeric CommittedUntilAddress in INFO persistence')
    return int(values[0])

def selftest(directory):
    proc = directory / 'proc'
    for pid, parent, rss, hwm, mask in [(100, 1, 10, 15, '2,4'), (101, 100, 20, 30, '4')]:
        task = proc / str(pid) / 'task' / str(pid)
        task.mkdir(parents=True)
        (task / 'status').write_text(f'Cpus_allowed_list:\t{mask}\n')
        (proc / str(pid) / 'status').write_text(f'PPid:\t{parent}\nVmRSS:\t{rss} kB\nVmHWM:\t{hwm} kB\n')
    assert cpus('0-3,8') == {0, 1, 2, 3, 8}
    for bad in ('', '3-1', 'a', '1,,2'):
        try:
            cpus(bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f'accepted invalid CPU list {bad!r}')
    assert affinity(proc, 100, '2,4') == 0
    (proc / '101/task/101/status').write_text('Cpus_allowed_list:\t4-5\n')
    assert affinity(proc, 100, '2,4') == 2
    (proc / '101/task/101/status').unlink()
    try:
        affinity(proc, 100, '2,4')
    except ValueError:
        pass
    else:
        raise AssertionError('accepted missing thread status')
    info = directory / 'info'
    info.write_text('used_memory:99\nused_memory_rss:999\n')
    assert memory(proc, 100, info) == [99, 30 * 1024, 'proc_status_sum', 45 * 1024]
    (proc / '101/status').write_text('PPid:\t1\nVmRSS:\t20 kB\nVmHWM:\t30 kB\n')
    assert memory(proc, 100, info) == [99, 999, 'info_memory', 15 * 1024]
    info.write_text('ERR unknown command\n')
    assert memory(proc, 100, info) == ['', 10 * 1024, 'proc_status_sum', 15 * 1024]
    (proc / '100/status').write_text('PPid:\t1\n')
    try:
        memory(proc, 100, info)
    except ValueError:
        pass
    else:
        raise AssertionError('accepted missing process memory')
    log = directory / 'log'
    log.mkdir()
    assert aof_bytes(log) == (0, 0)
    (log / 'AOF').mkdir()
    (log / 'unrelated-checkpoint').write_bytes(b'not an AOF')
    assert aof_bytes(log) == (0, 0)
    (log / 'AOF/aof.log.0').write_bytes(b'witness')
    assert aof_bytes(log)[0] == 7
    info.write_text('# Persistence_DB_0\nCommittedUntilAddress:64\r\n')
    assert committed(info) == 64
    for invalid in ('N/A', '1,2', '-1', 'junk'):
        info.write_text(f'CommittedUntilAddress:{invalid}\n')
        try:
            committed(info)
        except ValueError:
            pass
        else:
            raise AssertionError(f'accepted invalid committed offset {invalid}')
    for text in ('', 'CommittedUntilAddress:1\nCommittedUntilAddress:2\n'):
        info.write_text(text)
        try:
            committed(info)
        except ValueError:
            pass
        else:
            raise AssertionError('accepted absent or duplicate committed offset')

try:
    command, *args = sys.argv[1:]
    if command == 'affinity':
        raise SystemExit(affinity(Path(args[0]), int(args[1]), args[2]))
    elif command == 'memory':
        csv.writer(sys.stdout, lineterminator='\n').writerow(memory(Path(args[0]), int(args[1]), Path(args[2])))
    elif command == 'aof-bytes':
        print(*aof_bytes(Path(args[0])))
    elif command == 'committed':
        print(committed(Path(args[0])))
    elif command == 'selftest':
        selftest(Path(args[0]))
    else:
        raise ValueError(f'unknown observation: {command}')
except (OSError, ValueError, AssertionError) as error:
    print(f'competitor observation failed: {error}', file=sys.stderr)
    raise SystemExit(1)
PY
}

compare_affinity_check() {
    case $1 in
        dragonfly|dragonfly-*)
            competitor_observation affinity /proc "$server" "$SERVER_CPUS" \
                >"$OUT/affinity-$PORT.txt" || return $?
            echo "affinity,$1,$(cat "$OUT/affinity-$PORT.txt"),mode=${DRAGONFLY_AFFINITY:-on}"
            ;;
    esac
}

compare_garnet_aof_check() {
    garnet_aof_line=$1
    before=$(competitor_observation aof-bytes "$AOF_DIR") || return 1
    set -- $before
    before_size=$1 before_blocks=$2
    redis-cli -p "$PORT" INFO persistence >"$OUT/garnet-aof-before-$PORT.txt" || return 1
    before_committed=$(competitor_observation committed "$OUT/garnet-aof-before-$PORT.txt") || return 1
    # A write/delete witness checks that the log advances after startup. It
    # leaves no key in the measured dataset and runs outside measured time.
    reply=$(printf '%s' firn-aof-witness | redis-cli -p "$PORT" -x SET __firn_aof_probe__) || return 1
    [ "$reply" = OK ] || { echo "$garnet_aof_line: AOF witness SET replied '$reply'" >&2; return 1; }
    reply=$(redis-cli -p "$PORT" DEL __firn_aof_probe__) || return 1
    [ "$reply" = 1 ] || { echo "$garnet_aof_line: AOF witness DEL replied '$reply'" >&2; return 1; }
    aof_tries=0
    while [ "$aof_tries" -lt 100 ]; do
        after=$(competitor_observation aof-bytes "$AOF_DIR") || return 1
        set -- $after
        redis-cli -p "$PORT" INFO persistence >"$OUT/garnet-aof-after-$PORT.txt" || return 1
        after_committed=$(competitor_observation committed "$OUT/garnet-aof-after-$PORT.txt") || return 1
        # Logical committed growth remains observable for fully preallocated
        # files. A nonempty actual AOF log rules out a metadata-only check.
        if [ "$after_committed" -gt "$before_committed" ] && [ "$1" -gt 0 ]; then
            echo "aof,$garnet_aof_line,committed_before=$before_committed,committed_after=$after_committed,log_size_before=$before_size,log_size_after=$1,allocated_before=$before_blocks,allocated_after=$2,durability=not_equal_unverified_fsync"
            return 0
        fi
        aof_tries=$((aof_tries + 1))
        sleep 0.05
    done
    echo "$garnet_aof_line: no committed AOF growth with a nonempty AOF/aof.log* file after 100 polls (50 ms sleep between observations)" >&2
    return 1
}

compare_memory() {
    # INFO errors/unsupported fields use the process tree; proc failure fails
    # the run so an absent memory observation cannot look like a cheap server.
    redis-cli -p "$PORT" INFO memory >"$OUT/memory-info-$PORT.txt" 2>/dev/null || :
    memory_row=$(competitor_observation memory /proc "$server" "$OUT/memory-info-$PORT.txt") || return 1
    if [ ! -f "$OUT/compare-memory.csv" ]; then
        echo 'line,pass,cpus,used_memory_bytes,rss_bytes,rss_source,vmhwm_bytes' >"$OUT/compare-memory.csv"
    fi
    echo "$1,$2,$3,$memory_row" >>"$OUT/compare-memory.csv"
}

competitors_selftest() (
    fixture=$(mktemp -d)
    trap 'rm -rf "$fixture"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    # Fixed argv witnesses catch an omitted empty argument, wrong IO count,
    # deprecated Valkey option, accidental AOF or missing Lua/AOF options.
    competitor_args reference-io 2 true 1234 '/tmp/aof path' 0 on >"$fixture/args"
    printf '%s\n' --port 1234 --save '' --timeout 0 --daemonize no \
        --appendonly yes --appendfsync everysec --auto-aof-rewrite-percentage 0 \
        --dir '/tmp/aof path' --io-threads 2 --io-threads-do-reads yes >"$fixture/expected"
    cmp "$fixture/expected" "$fixture/args"
    competitor_args valkey-io 4 false 1234 '' 0 on >"$fixture/args"
    printf '%s\n' --port 1234 --save '' --timeout 0 --daemonize no \
        --appendonly no --io-threads 4 >"$fixture/expected"
    cmp "$fixture/expected" "$fixture/args"
    competitor_args dragonfly 8 false 1234 '' 0 off >"$fixture/args"
    printf '%s\n' --port=1234 --proactor_threads=8 --dbfilename= \
        --logtostderr --proactor_affinity_mode=off >"$fixture/expected"
    cmp "$fixture/expected" "$fixture/args"
    competitor_args garnet 2 true 1234 '/tmp/aof path' 0 on >"$fixture/args"
    printf '%s\n' --port 1234 --bind 127.0.0.1 --lua --aof \
        --aof-commit-freq 1000 --checkpointdir '/tmp/aof path' >"$fixture/expected"
    cmp "$fixture/expected" "$fixture/args"
    competitor_args garnet 2 false 1234 '' 0 on >"$fixture/args"
    printf '%s\n' --port 1234 --bind 127.0.0.1 --lua >"$fixture/expected"
    cmp "$fixture/expected" "$fixture/args"
    for invalid in 'dragonfly 2 true' 'reference-io 1 false' 'valkey-io 1 false' \
        'missing 2 false' 'reference 0 false' 'reference 2 bad'; do
        set -- $invalid
        if competitor_args "$1" "$2" "$3" 1234 '' 0 on >/dev/null 2>&1; then
            echo "competitors-selftest accepted $invalid" >&2
            exit 1
        fi
    done
    competitor_observation selftest "$fixture"
    echo 'competitors-selftest: passed'
)
