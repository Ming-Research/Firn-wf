#!/usr/bin/env bash
# Run the commands of docs/benchmarking.md from nothing and check that they
# end in a comparison table. The commands are the guide's own marked blocks
# (guide-blocks.py), not a copy, so a step the guide lacks fails here.
# Usage: bash reproduce.sh OUT [THROUGH]
#   OUT      output directory, made if absent: guide.sh and guide.log, the
#            guide's build/benchmark-results as results/, summary.md
#   THROUGH  the last guide step to run (guide-blocks.py lists them); empty
#            runs every step and checks the probe cells' tables
# FIRN_REVISION names the Firn-wf commit the guide checks out; it defaults to
# this checkout's HEAD and must be on GitHub, where the guide clones from.
#
# REPRODUCE_WHERE says where the commands run:
#   container  (default) a fresh ubuntu:24.04 container with an administrator
#              user, as on a new machine; needs a docker daemon this user can
#              reach
#   native     a fresh HOME and a fresh checkout, as this user, on a host that
#              keeps its tools installed and gives no sudo (the 14900K
#              runner): the packages step is left out. When SESSION_FILE names
#              a record, the run is a session of its own, as session.py
#              describes, and what it leaves is stopped when it ends
# REPRODUCE_PLACEMENT says which CPUs the probe uses:
#   guide      (default) the guide's lists, the reference 14900K's
#   host       lists for this host from lscpu, for a host without the
#              reference's cores: one probe cell of two server CPUs on the
#              first physical core, the clients on the others
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:?usage: reproduce.sh OUT [THROUGH]}
through=${2:-}
where=${REPRODUCE_WHERE:-container}
placement=${REPRODUCE_PLACEMENT:-guide}
FIRN_REVISION=${FIRN_REVISION:-$(git -C "$root" rev-parse HEAD)}
export FIRN_REVISION
image=ubuntu:24.04
name=firn-reproduce-${GITHUB_RUN_ID:-$$}
user=bench
reproduce_table() { python3 -I "$here/reproduce-table.py" "$@"; }

case $where in
  container | native) ;;
  *) echo "reproduce: REPRODUCE_WHERE is container or native, not $where" >&2; exit 2 ;;
esac
case $placement in
  guide) ;;
  host)
    placed=$(lscpu -p=CPU,CORE,SOCKET | reproduce_table placement 2)
    read -r SERVER_2 CLIENT_CPU_LIST <<<"$placed"
    export SERVER_2 CLIENT_CPU_LIST PROBE_CELLS=2
    echo "reproduce: this host's probe cell: servers $SERVER_2, clients $CLIENT_CPU_LIST"
    ;;
  *) echo "reproduce: REPRODUCE_PLACEMENT is guide or host, not $placement" >&2; exit 2 ;;
esac
cells=${PROBE_CELLS:-2 8}

if [ "$where" = container ]; then
  command -v docker >/dev/null || { echo "reproduce: docker is not installed on this host" >&2; exit 1; }
  docker info >/dev/null 2>&1 || {
    echo "reproduce: this user cannot reach the docker daemon (groups: $(id -Gn))" >&2
    exit 1
  }
fi
mkdir -p "$out"
out=$(cd "$out" && pwd)
without=
if [ "$where" = native ]; then without=packages; fi
python3 -I "$here/guide-blocks.py" extract "$root/docs/benchmarking.md" "${through:--}" \
  ${without:+"$without"} >"$out/guide.sh"
chmod 644 "$out/guide.sh"

status=0
if [ "$where" = container ]; then
  # A container that a cancelled run left behind would load the host this run
  # measures.
  docker ps -aq --filter label=firn-reproduce | xargs -r docker rm -f >/dev/null
  cleanup() { docker rm -f "$name" >/dev/null 2>&1 || true; }
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  docker run -d --init --name "$name" --label firn-reproduce "$image" sleep infinity >/dev/null

  # The guide starts from an Ubuntu 24.04 machine and a user who can run sudo,
  # which an installation gives and the base image does not.
  docker exec --env BENCH_USER="$user" "$name" bash -ec '
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends sudo >/dev/null
    useradd --create-home --shell /bin/bash "$BENCH_USER"
    echo "$BENCH_USER ALL=(ALL) NOPASSWD:ALL" >"/etc/sudoers.d/$BENCH_USER"
  '

  docker cp "$out/guide.sh" "$name:/tmp/guide.sh"
  env_args=(--env "HOME=/home/$user" --env "FIRN_REVISION=$FIRN_REVISION")
  for variable in SERVER_2 CLIENT_CPU_LIST PROBE_CELLS; do
    [ -z "${!variable:-}" ] || env_args+=(--env "$variable=${!variable}")
  done
  docker exec --user "$user" --workdir "/home/$user" "${env_args[@]}" \
    "$name" bash -eo pipefail /tmp/guide.sh 2>&1 | tee "$out/guide.log" || status=$?

  rm -rf "$out/results"
  docker cp "$name:/home/$user/Firn-wf/build/benchmark-results" "$out/results" 2>/dev/null ||
    echo "reproduce: the guide wrote no build/benchmark-results"
else
  work=$out/work
  rm -rf "$work"
  mkdir -p "$work/home"
  # The record outlives the job's temp directory, so that the next run can stop
  # what this one left; it is read with the real home, before the fresh one.
  session_file=${SESSION_FILE:-}
  session_file=${session_file/#\~/$HOME}
  session_py=$here/session.py
  stop_recorded() { [ -z "$session_file" ] || python3 -I "$session_py" stop "$session_file"; }
  trap 'stop_recorded || true' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  # What a cancelled run left would load the host this run measures.
  stop_recorded
  cd "$work"
  if [ -z "$session_file" ]; then
    env HOME="$work/home" bash -eo pipefail "$out/guide.sh" 2>&1 | tee "$out/guide.log" || status=$?
  else
    mkdir -p "$(dirname "$session_file")"
    FIRN_REDIS_BENCH_RECORD=$session_file setsid --wait sh -c \
      'python3 -I "$1" record "$2" $$ || exit 1; shift 2; exec "$@"' \
      run "$session_py" "$session_file" env HOME="$work/home" bash -eo pipefail "$out/guide.sh" \
      2>&1 | tee "$out/guide.log" || status=$?
  fi
  stop_recorded || { [ "$status" -ne 0 ] || status=1; }
  cd "$out"

  rm -rf "$out/results"
  cp -r "$work/Firn-wf/build/benchmark-results" "$out/results" 2>/dev/null ||
    echo "reproduce: the guide wrote no build/benchmark-results"
fi

[ "$status" -eq 0 ] || { echo "reproduce: the guide's commands failed with exit $status" >&2; exit "$status"; }
if [ -n "$through" ] && [ "$through" != probe ]; then
  echo "reproduce: ran through the step $through; the table is checked after probe"
  exit 0
fi

# The probe cells must hold a measured, nonzero rate for firn, its twin and
# every baseline line; reproduce-table.py writes summary.md and checks that.
# shellcheck disable=SC2086 # the cells are separate arguments
reproduce_table table "$out" $cells
