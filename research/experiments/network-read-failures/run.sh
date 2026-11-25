#!/bin/sh
# The instruments of the investigation
# research/investigations/firn/network-read-failures.md, which README.md
# beside this file describes. Each mode is one experiment. They run on a
# GitHub-hosted ubuntu-24.04 runner, the machine check.yml's network job runs
# on, with passwordless sudo for the kernel's tracing, after the setup of that
# job: the pinned compiler and its LLVM, Rust, strace and the reference Redis.
#
#   run.sh environment
#   run.sh mechanism
#   run.sh suite MINUTES
#   run.sh soft-limit MINUTES INTERVAL [RECEIVE_MAX]
#   run.sh stall-loop MINUTES PROCESSES WAITS [RECEIVE_MAX]
#   run.sh tcp-reopen MINUTES INTERVAL_MS WAITS [RECEIVE_MAX]
#   run.sh tcp-reopen-vm MINUTES KERNEL INTERVAL_MS WAITS [RECEIVE_MAX]
#   run.sh firn-reopen MINUTES INTERVAL_MS WAITS
#   run.sh firn-reopen-vm MINUTES KERNEL INTERVAL_MS WAITS
#
# MINUTES bounds a mode from its start, builds included: no pass or run
# starts that would end after it. INTERVAL is the seconds between two starts
# of the soft-limit case. PROCESSES is how many loops run side by side, each
# with a server of its own, and WAITS the milliseconds a round lets the
# server's send wait, one round after another, separated by commas; for
# tcp-reopen, INTERVAL_MS and WAITS are tcp_window_reopen.rs's. RECEIVE_MAX,
# where given, is the most bytes the kernel may grow a TCP socket's receive
# buffer to (the third number of net.ipv4.tcp_rmem) for the mode: 6291456 is
# Linux's older default, with which a 14 MiB frame does not fit a reader's
# buffer, and 33554432 the newer one, with which it does. KERNEL is `host`,
# for the runner's own kernel, or the name of an Ubuntu kernel image package,
# such as linux-image-6.17.0-1022-azure, the kernel of the runners on which
# the soft-limit case stalled: tcp-reopen-vm runs tcp_window_reopen.rs as the
# only program of a virtual machine that boots that kernel. firn-reopen and
# firn-reopen-vm are tcp-reopen and tcp-reopen-vm with a firn as each pair's
# sender, as in the case, in place of a thread's write.
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
build=$root/build
out=$build/network-read-failures
tracing=/sys/kernel/tracing
mkdir -p "$out"

# Records every SIGCHLD as it is generated (with whether it was discarded,
# queued or already pending) and as a thread dequeues it, and every new task,
# from which trace_summary.py tells the process of each thread.
trace_start() {
  sudo sh -c "
    echo 0 > $tracing/tracing_on
    echo > $tracing/trace
    echo 16384 > $tracing/buffer_size_kb
    echo 32768 > $tracing/saved_cmdlines_size
    echo 1 > $tracing/options/record-tgid
    echo 'sig == 17' > $tracing/events/signal/signal_generate/filter
    echo 'sig == 17' > $tracing/events/signal/signal_deliver/filter
    echo 1 > $tracing/events/signal/signal_generate/enable
    echo 1 > $tracing/events/signal/signal_deliver/enable
    echo 1 > $tracing/events/task/task_newtask/enable
    echo 1 > $tracing/tracing_on
  " || echo "trace: not available"
}

# Writes what was traced since the last call to the file named, says how many
# events each CPU's buffer lost, and starts again.
trace_take() {
  sudo sh -c "echo 0 > $tracing/tracing_on; cat $tracing/trace" > "$1"
  echo "trace: events lost per CPU buffer: $(sudo sh -c "grep -h overrun $tracing/per_cpu/cpu*/stats" | tr '\n' ' ')"
  sudo sh -c "echo > $tracing/trace; echo 1 > $tracing/tracing_on"
}

# Sets the most a TCP receive buffer may grow to, when a mode is given one.
receive_max() {
  [ -n "${1:-}" ] || return 0
  sudo sysctl -w net.ipv4.tcp_rmem="4096 131072 $1" || exit 1
}

environment() {
  uname -r
  ldd --version | head -n 1
  rustc --version
  echo "processors: $(nproc)"
  ss -V
  sysctl net.ipv4.tcp_rmem net.ipv4.tcp_wmem net.ipv4.tcp_mem net.core.rmem_max net.core.wmem_max kernel.yama.ptrace_scope
  free -m | head -n 2
}

mechanism() {
  rustc --edition 2024 -O -o "$out/sigchld_eintr" "$here/sigchld_eintr.rs" || exit 1
  trace_start
  for configuration in \
    "reads nowait open" "reads wait open" "reads fork-nowait open" \
    "waits nowait open" \
    "starts-threads nowait open" "starts-threads wait open" "starts-threads fork-nowait open" \
    "reads nowait blocked" "starts-threads nowait blocked" "starts-threads fork-nowait blocked"; do
    set -- $configuration
    "$out/sigchld_eintr" run "$1" "$2" "$3" 3 > "$out/mechanism.line" 2>&1
    status=$?
    cat "$out/mechanism.line"
    [ $status -eq 0 ] || echo "mechanism: $configuration exited with $status"
    sed -n 's/^run pid=\([0-9]*\) .*/\1/p' "$out/mechanism.line" > "$out/mechanism.pids"
    trace_take "$out/mechanism.trace"
    python3 "$here/trace_summary.py" --pids "$out/mechanism.pids" --label "$1/$2/$3" --examples 2 "$out/mechanism.trace"
  done
  "$out/sigchld_eintr" child-mask open
  "$out/sigchld_eintr" child-mask blocked
}

# Runs the gate's network target, `make test`, pass after pass, and says for
# each pass how it ended, what its failing cases said, and what became of the
# SIGCHLD of the test processes.
suite() {
  budget=$(( $1 * 60 ))
  begun=$(date +%s)
  # The builds are not part of a pass.
  make --no-print-directory -C "$root" firn-test list-ceiling > "$out/build.log" 2>&1 || { tail -n 40 "$out/build.log"; exit 1; }
  cargo test --manifest-path "$root/tests/Cargo.toml" --locked --no-run > "$out/cargo.log" 2>&1 || { tail -n 60 "$out/cargo.log"; exit 1; }
  echo "suite: built in $(( $(date +%s) - begun )) s"
  # The test processes, by the name of their first thread.
  ( while :; do pgrep '^network-[0-9a-f]' >> "$out/pids.sampled"; sleep 0.2; done ) &
  sampler=$!
  trace_start
  pass=0
  failed=0
  last=0
  while :; do
    now=$(date +%s)
    if [ $pass -gt 0 ] && [ $(( now - begun + last )) -gt $budget ]; then break; fi
    pass=$((pass + 1))
    : > "$out/pids.sampled"
    make --no-print-directory -C "$root" test > "$out/pass-$pass.log" 2>&1
    status=$?
    last=$(( $(date +%s) - now ))
    sort -u "$out/pids.sampled" > "$out/pids-$pass"
    trace_take "$out/trace-$pass"
    echo "pass $pass: make test exited with $status after $last s"
    grep -a '^test result' "$out/pass-$pass.log" | sed "s/^/pass $pass: /"
    if [ $status -ne 0 ]; then
      failed=$((failed + 1))
      echo "pass $pass: FAILED; what its failing cases said:"
      grep -a -A 3 'panicked at' "$out/pass-$pass.log" | cut -c 1-600 | head -n 80
      grep -a -A 14 'BEGIN TEST CHILD DIAGNOSTICS' "$out/pass-$pass.log" | cut -c 1-600 | head -n 80
      awk '/^failures:$/ { keep = 1; text = "" } keep { text = text $0 "\n" } /^test result/ { if (keep) printf "%s", text; keep = 0 }' "$out/pass-$pass.log" | tail -n 30
      tail -n 15 "$out/pass-$pass.log" | cut -c 1-600
    fi
    python3 "$here/trace_summary.py" --pids "$out/pids-$pass" --label "pass $pass" "$out/trace-$pass"
  done
  kill $sampler
  echo "suite: passes=$pass failed=$failed"
}

# Builds the network cases with soft-limit-stall.patch applied and names the
# test program in $binary. The path is the one cargo reports: where it puts a
# test program differs between cargo releases.
build_patched() {
  make --no-print-directory -C "$root" firn-test > "$out/build.log" 2>&1 || { tail -n 40 "$out/build.log"; exit 1; }
  git -C "$root" apply "$here/soft-limit-stall.patch" || exit 1
  cargo test --manifest-path "$root/tests/Cargo.toml" --locked --no-run --message-format=json \
    > "$out/cargo.json" 2> "$out/cargo.log" || { tail -n 60 "$out/cargo.log"; exit 1; }
  binary=$(python3 -c '
import json, sys
for line in open(sys.argv[1], encoding="utf-8"):
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") == "compiler-artifact" and message.get("executable") and message["target"]["name"] == "network":
        print(message["executable"])
' "$out/cargo.json" | tail -n 1)
  [ -x "$binary" ] || { echo "no test program among cargo's artifacts"; tail -n 20 "$out/cargo.log"; exit 1; }
}

# Runs the soft-limit case, with soft-limit-stall.patch applied, again and
# again, each run a process of its own with its own server, a new one every
# INTERVAL seconds, and summarises them. The first run also prints a sample of
# a send that waits for its reader.
soft_limit() {
  budget=$(( $1 * 60 ))
  interval=$2
  begun=$(date +%s)
  receive_max "${3:-}"
  build_patched
  name=pubsub::soft_output_limit_interval_restarts_after_the_backlog_drains
  echo "soft-limit: built $binary in $(( $(date +%s) - begun )) s; one run starts every $interval s"
  number=0
  # A run takes about 75 s: none starts later than 100 s before the bound.
  while [ $(( $(date +%s) - begun )) -lt $(( budget - 100 )) ]; do
    number=$((number + 1))
    (
      if [ $number -eq 1 ]; then export FIRN_STALL_SAMPLE=1; fi
      FIRN=$build/firn-test "$binary" --exact "$name" --nocapture > "$out/soft-$number.log" 2>&1
      echo $? > "$out/soft-$number.status"
    ) &
    if [ $(( number % 50 )) -eq 0 ]; then echo "soft-limit: $number started; load $(cut -d ' ' -f 1-4 /proc/loadavg); $(grep '^TCP:' /proc/net/sockstat)"; fi
    sleep "$interval"
  done
  wait
  echo "soft-limit: $number runs ended after $(( $(date +%s) - begun )) s"
  python3 "$here/soft_summary.py" "$out"
}

# Runs the patch's stall_loop case in PROCESSES processes side by side until
# the bound, and summarises the rounds by how long the send waited and by
# whether the server was asked a question before the read.
stall_loop() {
  budget=$(( $1 * 60 ))
  processes=$2
  waits=$3
  begun=$(date +%s)
  receive_max "${4:-}"
  build_patched
  seconds=$(( budget - ($(date +%s) - begun) - 30 ))
  echo "stall-loop: built $binary in $(( $(date +%s) - begun )) s; $processes loops of $seconds s with waits of $waits ms"
  number=0
  while [ $number -lt "$processes" ]; do
    number=$((number + 1))
    (
      FIRN=$build/firn-test FIRN_STALL_SECONDS=$seconds FIRN_STALL_WAITS_MS=$waits \
        "$binary" --exact pubsub::stall_loop --nocapture > "$out/loop-$number.log" 2>&1
      echo $? > "$out/loop-$number.status"
    ) &
  done
  wait
  echo "stall-loop: ended after $(( $(date +%s) - begun )) s"
  python3 "$here/soft_summary.py" --loop "$out"
}

# Builds the firn the pairs of firn-reopen start, and names it in $server.
build_server() {
  make --no-print-directory -C "$root" firn-test > "$out/build.log" 2>&1 || { tail -n 40 "$out/build.log"; exit 1; }
  server=$build/firn-test
}

# Runs tcp_window_reopen.rs until the bound: with no server of ours in it, or
# for firn-reopen with a firn as each pair's sender.
tcp_reopen() {
  budget=$(( $1 * 60 ))
  begun=$(date +%s)
  receive_max "${4:-}"
  server=
  if [ "$sender" = firn ]; then build_server; fi
  rustc --edition 2024 -O -o "$out/tcp_window_reopen" "$here/tcp_window_reopen.rs" || exit 1
  "$out/tcp_window_reopen" $(( budget - ($(date +%s) - begun) - 20 )) "$2" "$3" $server > "$out/tcp-reopen.log" 2>&1
  status=$?
  reopen_report "$out/tcp-reopen.log"
  echo "tcp-reopen: the program exited with $status"
}

# Says what a run of tcp_window_reopen.rs printed, shortest first so that the
# notices keep it: the machine's lines and the summary, then one line for
# each frame that waited (its last second without a byte), then the kernel's
# account of the sockets for the first waits. Every pair's lines stay in the
# file.
reopen_report() {
  grep -a -E '^(vm:|tcp-window-reopen)|Linux version' "$1" | cut -c 1-600
  echo "reopen: FRAME lines: $(grep -a -c '^FRAME' "$1"); STALL lines: $(grep -a -c '^STALL' "$1")"
  grep -a -A 2 -E '^thread |panicked|Kernel panic' "$1" | cut -c 1-500 | head -n 24
  grep -a '^STALL' "$1" | awk '{ key = $2 " " $3; if (!(key in last)) keys[++count] = key; last[key] = $0 }
    END { for (i = 1; i <= count; i++) print "WAITED " substr(last[keys[i]], 7) }' | head -n 150
  grep -a -E '^(SAMPLE|DUMP)' "$1" | cut -c 1-700 | head -n 16
}

# Runs tcp_window_reopen.rs under another kernel: as the init of a QEMU
# machine whose whole file system is an initramfs that holds the program,
# built for musl so that it needs no library, and a static busybox to mount
# /proc and bring the loopback interface up. The machine has 4 processors, as
# the runner has, and its console is the mode's output.
tcp_reopen_vm() {
  budget=$(( $1 * 60 ))
  kernel=$2
  interval=$3
  waits=$4
  receive=${5:-}
  begun=$(date +%s)
  vm=$out/vm
  sudo rm -rf "$vm"
  mkdir -p "$vm/root/bin" "$vm/root/proc" "$vm/root/sys" "$vm/root/dev" "$vm/root/tmp" "$vm/package"
  server=
  if [ "$sender" = firn ]; then
    # The server and the libraries it is linked with, at the paths it finds
    # them by.
    build_server
    cp "$server" "$vm/root/firn-test" || exit 1
    for library in $(ldd "$server" | grep -o '/[^ ]*'); do
      mkdir -p "$vm/root$(dirname "$library")" && cp -L "$library" "$vm/root$library" || exit 1
    done
    ldd "$server" | cut -c 1-200
    server=/firn-test
  fi
  { sudo apt-get update && sudo apt-get install -y --no-install-recommends qemu-system-x86 busybox-static cpio; } \
    > "$out/vm-apt.log" 2>&1 || { tail -n 30 "$out/vm-apt.log"; exit 1; }
  rustup target add x86_64-unknown-linux-musl > "$out/vm-rustup.log" 2>&1 || { tail -n 30 "$out/vm-rustup.log"; exit 1; }
  rustc --edition 2024 -O --target x86_64-unknown-linux-musl -o "$vm/root/tcp_window_reopen" "$here/tcp_window_reopen.rs" || exit 1
  file "$vm/root/tcp_window_reopen"
  if [ "$kernel" = host ]; then
    sudo cat "/boot/vmlinuz-$(uname -r)" > "$vm/vmlinuz" || exit 1
  else
    # apt checks the package against the archive's signed index.
    ( cd "$vm/package" && apt-get download "$kernel" > "$out/vm-kernel.log" 2>&1 ) || { tail -n 30 "$out/vm-kernel.log"; exit 1; }
    dpkg-deb -x "$vm"/package/*.deb "$vm/package/files" || exit 1
    cp "$vm"/package/files/boot/vmlinuz-* "$vm/vmlinuz" || exit 1
    sha256sum "$vm"/package/*.deb
  fi
  cp "$(command -v busybox)" "$vm/root/bin/busybox" || exit 1
  file "$vm/root/bin/busybox"
  seconds=$(( budget - ($(date +%s) - begun) - 120 ))
  cat > "$vm/root/init" << INIT
#!/bin/busybox sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sys /sys
/bin/busybox ip link set lo up || /bin/busybox ifconfig lo 127.0.0.1 up
[ -z "$receive" ] || echo "4096 131072 $receive" > /proc/sys/net/ipv4/tcp_rmem
echo "vm: kernel \$(/bin/busybox uname -r), processors \$(/bin/busybox grep -c ^processor /proc/cpuinfo), \$(/bin/busybox grep MemTotal /proc/meminfo)"
echo "vm: tcp_rmem \$(/bin/busybox cat /proc/sys/net/ipv4/tcp_rmem) tcp_wmem \$(/bin/busybox cat /proc/sys/net/ipv4/tcp_wmem) tcp_mem \$(/bin/busybox cat /proc/sys/net/ipv4/tcp_mem)"
/tcp_window_reopen $seconds $interval $waits $server
echo "vm: the program exited with \$?"
/bin/busybox poweroff -f
INIT
  chmod +x "$vm/root/init"
  sudo mknod "$vm/root/dev/console" c 5 1 || exit 1
  sudo mknod -m 666 "$vm/root/dev/null" c 1 3 || exit 1
  sudo mknod -m 444 "$vm/root/dev/urandom" c 1 9 || exit 1
  ( cd "$vm/root" && sudo find . | sudo cpio -o -H newc 2> /dev/null | gzip > "$vm/initramfs.gz" ) || exit 1
  ls -l "$vm/vmlinuz" "$vm/initramfs.gz"
  if [ -e /dev/kvm ]; then accelerator="-accel kvm -accel tcg"; else accelerator="-accel tcg"; fi
  echo "tcp-reopen-vm: starting the machine with $accelerator for $seconds s of pairs, $(( $(date +%s) - begun )) s after the mode began"
  sudo timeout $(( seconds + 100 )) qemu-system-x86_64 $accelerator -cpu max -smp 4 -m 10240 -display none -no-reboot \
    -kernel "$vm/vmlinuz" -initrd "$vm/initramfs.gz" -append "console=ttyS0 panic=-1 loglevel=4" \
    -serial stdio < /dev/null > "$out/vm.log" 2>&1
  echo "tcp-reopen-vm: qemu exited with $? after $(( $(date +%s) - begun )) s; the console has $(wc -l < "$out/vm.log") lines"
  tr -d '\r' < "$out/vm.log" > "$out/vm.txt"
  reopen_report "$out/vm.txt"
  if ! grep -a -q '^tcp-window-reopen: pairs' "$out/vm.txt"; then
    echo "tcp-reopen-vm: the program did not reach its summary; the console's last lines:"
    tail -n 40 "$out/vm.txt" | cut -c 1-300
  fi
}

# In a job of GitHub Actions, repeats what a mode printed as notices of the
# job, which GitHub's GraphQL interface gives when the job's log cannot be
# read. A step keeps ten notices: the environment takes one and another mode
# up to nine, of what it printed first and last when that is too long.
notices() {
  [ -n "${GITHUB_ACTIONS:-}" ] || return 0
  python3 - "$1" "$2" "$(git -C "$root" rev-parse --short HEAD)" << 'PYTHON'
import sys

mode, path, commit = sys.argv[1:4]
most = 1 if mode == "environment" else 9
size = 3800
with open(path, encoding="utf-8", errors="replace") as printed:
    lines = [line.rstrip("\n") for line in printed if not line.startswith("trace: events lost")]
chunks = [[]]
for line in lines:
    line = line[:size]
    if sum(len(kept) + 1 for kept in chunks[-1]) + len(line) + 1 > size:
        chunks.append([])
    chunks[-1].append(line)
if len(chunks) > most:
    left = len(chunks) - most
    chunks = chunks[: most - 2] + [chunks[most - 2] + [f"(the notices leave out {left} parts here; the job's log has them)"]] + chunks[-1:] if most > 2 else chunks[:most]
for index, chunk in enumerate(chunks, start=1):
    text = "\n".join(chunk).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::notice title=network-read-failures {mode} {commit} {index} of {len(chunks)}::{text}")
PYTHON
}

mode=${1:-}
case "$mode" in
  environment | mechanism | suite | soft-limit | stall-loop | tcp-reopen | tcp-reopen-vm | firn-reopen | firn-reopen-vm) ;;
  *)
    echo "usage: run.sh environment | mechanism | suite MINUTES | soft-limit MINUTES INTERVAL [RECEIVE_MAX] | stall-loop MINUTES PROCESSES WAITS [RECEIVE_MAX] | tcp-reopen MINUTES INTERVAL_MS WAITS [RECEIVE_MAX] | tcp-reopen-vm MINUTES KERNEL INTERVAL_MS WAITS [RECEIVE_MAX] | firn-reopen MINUTES INTERVAL_MS WAITS | firn-reopen-vm MINUTES KERNEL INTERVAL_MS WAITS" >&2
    exit 2
    ;;
esac
# The mode runs in a subshell, so that it can stop itself, and its status
# comes through a file, since a pipe gives only the status of its last command.
rm -f "$out/$mode.status"
(
  case "$mode" in
    environment) environment ;;
    mechanism) mechanism ;;
    suite) suite "$2" ;;
    soft-limit) soft_limit "$2" "$3" "${4:-}" ;;
    stall-loop) stall_loop "$2" "$3" "$4" "${5:-}" ;;
    tcp-reopen) sender=thread; tcp_reopen "$2" "$3" "$4" "${5:-}" ;;
    tcp-reopen-vm) sender=thread; tcp_reopen_vm "$2" "$3" "$4" "$5" "${6:-}" ;;
    firn-reopen) sender=firn; tcp_reopen "$2" "$3" "$4" "" ;;
    firn-reopen-vm) sender=firn; tcp_reopen_vm "$2" "$3" "$4" "$5" "" ;;
  esac
  echo $? > "$out/$mode.status"
) 2>&1 | tee "$out/$mode.out"
notices "$mode" "$out/$mode.out"
[ -f "$out/$mode.status" ] || exit 1
exit "$(cat "$out/$mode.status")"
