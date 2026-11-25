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
#   run.sh soft-limit MINUTES INTERVAL
#
# MINUTES bounds a mode from its start, builds included: no pass or run
# starts that would end after it. INTERVAL is the seconds between two starts
# of the soft-limit case.
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

# Runs the soft-limit case, with soft-limit-stall.patch applied, again and
# again, each run a process of its own with its own server, a new one every
# INTERVAL seconds, and summarises them. The first run also prints a sample of
# a send that waits for its reader.
soft_limit() {
  budget=$(( $1 * 60 ))
  interval=$2
  begun=$(date +%s)
  make --no-print-directory -C "$root" firn-test > "$out/build.log" 2>&1 || { tail -n 40 "$out/build.log"; exit 1; }
  git -C "$root" apply "$here/soft-limit-stall.patch" || exit 1
  cargo test --manifest-path "$root/tests/Cargo.toml" --locked --no-run > "$out/cargo.log" 2>&1 || { tail -n 60 "$out/cargo.log"; exit 1; }
  binary=$(ls -t "$root"/tests/target/debug/deps/network-* | grep -v '\.d$' | head -n 1)
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

case "${1:-}" in
  environment) environment ;;
  mechanism) mechanism ;;
  suite) suite "$2" ;;
  soft-limit) soft_limit "$2" "$3" ;;
  *)
    echo "usage: run.sh environment | mechanism | suite MINUTES | soft-limit MINUTES INTERVAL" >&2
    exit 2
    ;;
esac
