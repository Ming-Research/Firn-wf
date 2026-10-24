#!/usr/bin/env bash
# Run the commands of docs/benchmarking.md in a fresh Ubuntu 24.04 container
# and check that they end in a comparison table. The commands are the guide's
# own marked blocks (guide-blocks.py), not a copy, so a step the guide lacks
# fails here.
# Usage: bash reproduce.sh OUT [THROUGH]
#   OUT      output directory, made if absent: guide.sh and guide.log, the
#            guide's build/benchmark-results as results/, summary.md
#   THROUGH  the last guide step to run (guide-blocks.py lists them); empty
#            runs every step and checks the probe cells' tables
# FIRN_REVISION names the Firn-wf commit the guide checks out; it defaults to
# this checkout's HEAD and must be on GitHub, where the guide clones from.
# Needs a docker daemon this user can reach.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:?usage: reproduce.sh OUT [THROUGH]}
through=${2:-}
FIRN_REVISION=${FIRN_REVISION:-$(git -C "$root" rev-parse HEAD)}
export FIRN_REVISION
image=ubuntu:24.04
name=firn-reproduce-${GITHUB_RUN_ID:-$$}
user=bench

command -v docker >/dev/null || { echo "reproduce: docker is not installed on this host" >&2; exit 1; }
docker info >/dev/null 2>&1 || {
  echo "reproduce: this user cannot reach the docker daemon (groups: $(id -Gn))" >&2
  exit 1
}
mkdir -p "$out"
out=$(cd "$out" && pwd)
python3 -I "$here/guide-blocks.py" extract "$root/docs/benchmarking.md" ${through:+"$through"} >"$out/guide.sh"
chmod 644 "$out/guide.sh"

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
status=0
docker exec --user "$user" --workdir "/home/$user" --env "HOME=/home/$user" --env FIRN_REVISION \
  "$name" bash -eo pipefail /tmp/guide.sh 2>&1 | tee "$out/guide.log" || status=$?

rm -rf "$out/results"
docker cp "$name:/home/$user/Firn-wf/build/benchmark-results" "$out/results" 2>/dev/null ||
  echo "reproduce: the guide wrote no build/benchmark-results"
[ "$status" -eq 0 ] || { echo "reproduce: the guide's commands failed with exit $status" >&2; exit "$status"; }
if [ -n "$through" ] && [ "$through" != probe ]; then
  echo "reproduce: ran through the step $through; the table is checked after probe"
  exit 0
fi

# The probe cells must hold a measured, nonzero rate for firn, its twin and
# every baseline line; reproduce-table.py writes summary.md and checks that.
python3 -I "$here/reproduce-table.py" table "$out"
