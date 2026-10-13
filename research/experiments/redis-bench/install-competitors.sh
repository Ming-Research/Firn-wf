#!/usr/bin/env bash
# Install the pinned baseline competitors without sudo.
# Usage: bash install-competitors.sh [PREFIX] (default: ~/.local).
# Releases and receipts live in PREFIX/opt; PREFIX/bin contains symlinks.
# Archive checks, staging and receipts match the redis-bench install CI job.
set -euo pipefail

prefix=${1:-$HOME/.local}

export VALKEY_VERSION='9.1.2'
export VALKEY_URL=https://github.com/valkey-io/valkey/archive/refs/tags/9.1.2.tar.gz
export VALKEY_SHA256=19c23908e7d57e8d91ef85b41f5646307582f10f4f0fb999bbf89ed24ec9c983
export DRAGONFLY_VERSION='2.0.2'
export DRAGONFLY_URL=https://github.com/dragonflydb/dragonfly/releases/download/v2.0.2/dragonfly-x86_64.tar.gz
export DRAGONFLY_SHA256=eb9dfe654c69ab190cb3aa329ed3599ebe19c5e9a3a437bb7bc38ef52f665c88
export GARNET_VERSION='2.2.1'
export GARNET_URL=https://github.com/microsoft/garnet/releases/download/v2.2.1/linux-x64-based.tar.xz
export GARNET_SHA256=605d91a6ba4c3f4f416b74e676e945b983cf73fdd28e8cad569aa84e699d3903

for tool in curl sha256sum tar python3 make cc timeout; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done
python3 - <<'PY'
import os
import re
for name in ('VALKEY', 'DRAGONFLY', 'GARNET'):
    value = os.environ[name + '_SHA256']
    if not re.fullmatch(r'[0-9a-f]{64}', value):
        raise SystemExit(f'{name}: literal archive SHA-256 pin is unavailable or invalid; installation refused')
    print(f'{name}: version={os.environ[name + "_VERSION"]}, archive_sha256={value}')
PY

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/firn-competitors.XXXXXX")
stages=()
cleanup() { rm -rf "$work" "${stages[@]}"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
download() {
  local name=$1 url=$2 want=$3 target=$4 got
  curl --fail --location --retry 3 --output "$target" "$url"
  got=$(sha256sum "$target")
  got=${got%% *}
  [ "$got" = "$want" ] || {
    echo "$name: archive digest mismatch: expected $want, got $got" >&2
    return 1
  }
  echo "$name: verified $url, SHA-256 $got"
}
# Validate every archive before creating or replacing release paths.
download valkey "$VALKEY_URL" "$VALKEY_SHA256" "$work/valkey-9.1.2.tar.gz"
download dragonfly "$DRAGONFLY_URL" "$DRAGONFLY_SHA256" "$work/dragonfly-x86_64.tar.gz"
download garnet "$GARNET_URL" "$GARNET_SHA256" "$work/linux-x64-based.tar.xz"
mkdir -p "$prefix/opt" "$prefix/bin"
opt=$(cd "$prefix/opt" && pwd -P)
bin_dir=$(cd "$prefix/bin" && pwd -P)

# Same-filesystem sibling staging preserves the old release until
# preparation finishes; restore it immediately if the rename fails.
replace_release() {
  local stage=$1 dest=$2 backup="$2.previous-$$"
  [ ! -e "$backup" ] || { echo "Backup path exists: $backup" >&2; return 1; }
  if [ -e "$dest" ]; then mv "$dest" "$backup"; fi
  if ! mv "$stage" "$dest"; then
    if [ -e "$backup" ]; then mv "$backup" "$dest"; fi
    return 1
  fi
  rm -rf "$backup"
}
pick_file() {
  python3 - "$1" "$2" "$3" <<'PY'
import os
from pathlib import Path
import sys
root = Path(sys.argv[1]).resolve()
names = sys.argv[2].split(',')
executable = sys.argv[3] == 'executable'
matches = sorted(set(p.resolve() for p in root.rglob('*')
                 if p.name in names and p.is_file()
                 and (not executable or os.access(p, os.X_OK))))
if not matches:
    raise SystemExit(2)
if len(matches) != 1 or not matches[0].is_relative_to(root):
    raise SystemExit(f'Expected one contained {names} payload; found {matches}')
print(matches[0].relative_to(root))
PY
}
receipt() {
  python3 - "$@" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
dest, version, url, archive_sha, executable, kind, *extras = sys.argv[1:]
def bound(path):
    target = Path(path).resolve(strict=True)
    digest = hashlib.sha256()
    with target.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return str(target), digest.hexdigest()
exe_path, exe_sha = bound(executable)
record = dict(version=version, archive_url=url, archive_sha256=archive_sha,
              executable_path=exe_path, executable_sha256=exe_sha,
              executable_kind=kind)
if kind == 'dotnet_apphost_wrapper':
    if len(extras) != 2:
        raise SystemExit('Apphost and runtime are required for wrapper receipts')
    record['payload_path'], record['payload_sha256'] = bound(extras[0])
    record['runtime_path'], record['runtime_sha256'] = bound(extras[1])
elif kind != 'native' or extras:
    raise SystemExit('Invalid executable receipt kind')
output = Path(dest) / 'firn-install-receipt'
temporary = output.with_name(output.name + '.tmp')
temporary.write_text(json.dumps(record, indent=2) + '\n')
temporary.replace(output)
print(output.read_text(), end='')
PY
}

# Valkey is built from its verified formal-release source archive.
mkdir "$work/valkey-source"
tar -xzf "$work/valkey-9.1.2.tar.gz" --strip-components=1 -C "$work/valkey-source"
valkey_stage=$(mktemp -d "$opt/.valkey-$VALKEY_VERSION.XXXXXX")
stages+=("$valkey_stage")
make -C "$work/valkey-source" -j2
make -C "$work/valkey-source" PREFIX="$valkey_stage" install
# Each check exits on its own: set -e ignores a failure before && .
[ -x "$valkey_stage/bin/valkey-server" ] || { echo "valkey-server missing after build" >&2; exit 1; }
[ -x "$valkey_stage/bin/valkey-cli" ] || { echo "valkey-cli missing after build" >&2; exit 1; }
valkey_dest="$opt/valkey-$VALKEY_VERSION"
replace_release "$valkey_stage" "$valkey_dest"
receipt "$valkey_dest" "$VALKEY_VERSION" "$VALKEY_URL" "$VALKEY_SHA256" \
  "$valkey_dest/bin/valkey-server" native
ln -sfn "$valkey_dest/bin/valkey-server" "$bin_dir/valkey-server"
ln -sfn "$valkey_dest/bin/valkey-cli" "$bin_dir/valkey-cli"

dragonfly_stage=$(mktemp -d "$opt/.dragonfly-$DRAGONFLY_VERSION.XXXXXX")
stages+=("$dragonfly_stage")
tar -xzf "$work/dragonfly-x86_64.tar.gz" -C "$dragonfly_stage"
dragonfly_relative=$(pick_file "$dragonfly_stage" dragonfly,dragonfly-x86_64 executable)
dragonfly_dest="$opt/dragonfly-$DRAGONFLY_VERSION"
replace_release "$dragonfly_stage" "$dragonfly_dest"
receipt "$dragonfly_dest" "$DRAGONFLY_VERSION" "$DRAGONFLY_URL" "$DRAGONFLY_SHA256" \
  "$dragonfly_dest/$dragonfly_relative" native
ln -sfn "$dragonfly_dest/$dragonfly_relative" "$bin_dir/dragonfly"

garnet_stage=$(mktemp -d "$opt/.garnet-$GARNET_VERSION.XXXXXX")
stages+=("$garnet_stage")
tar -xJf "$work/linux-x64-based.tar.xz" -C "$garnet_stage"
garnet_dest="$opt/garnet-$GARNET_VERSION"
# v2.2.1 linux-x64-based is framework-dependent PublishSingleFile,
# containing net8.0 and net10.0. Pick net8.0 explicitly; its runtime
# configuration is embedded and need not exist as a separate file.
# Source: main/GarnetServer/Properties/PublishProfiles/linux-x64-based.pubxml
# and .azure/pipelines/createbinaries.ps1 at the pinned release.
garnet_payload_relative=$(python3 - "$garnet_stage" <<'PY'
import os
from pathlib import Path
import sys
root = Path(sys.argv[1]).resolve()
matches = sorted(set(path.resolve() for path in root.rglob('GarnetServer')
                 if path.is_file() and 'net8.0' in path.relative_to(root).parts
                 and os.access(path, os.X_OK)))
if len(matches) != 1 or not matches[0].is_relative_to(root):
    raise SystemExit(f'Expected one contained net8.0/GarnetServer apphost; found {matches}')
print(matches[0].relative_to(root))
PY
)
# The pinned csproj/profile have no RuntimeFrameworkVersion override.
target_version=8.0.0
channel=8.0
runtime_compatible() {
  timeout --kill-after=2s 10s "$1" --list-runtimes >"$work/runtimes" || return 1
  python3 - "$work/runtimes" "$target_version" <<'PY'
import re
import sys
target = tuple(map(int, sys.argv[2].split('.')))
for line in open(sys.argv[1]):
    match = re.match(r'Microsoft.NETCore.App ([0-9]+\.[0-9]+\.[0-9]+) ', line)
    if match:
        version = tuple(map(int, match[1].split('.')))
        if version[:2] == target[:2] and version >= target:
            raise SystemExit(0)
raise SystemExit(1)
PY
}
runtime=
if [ -x "$opt/dotnet/dotnet" ] && runtime_compatible "$opt/dotnet/dotnet"; then
  runtime="$opt/dotnet/dotnet"
elif command -v dotnet >/dev/null && runtime_compatible "$(command -v dotnet)"; then
  runtime=$(command -v dotnet)
else
  # Install the framework-dependent apphost's runtime without
  # sudo. The receipt binds actual runtime bytes; this acquisition
  # does not establish an archive provenance pin for that runtime.
  runtime_stage=$(mktemp -d "$opt/.dotnet-$channel.XXXXXX")
  stages+=("$runtime_stage")
  curl --fail --location --retry 3 https://dot.net/v1/dotnet-install.sh -o "$work/dotnet-install.sh"
  bash "$work/dotnet-install.sh" --runtime dotnet --channel "$channel" --install-dir "$runtime_stage"
  [ -x "$runtime_stage/dotnet" ] || { echo "dotnet missing after install" >&2; exit 1; }
  runtime_compatible "$runtime_stage/dotnet" || { echo "installed dotnet runtime incompatible" >&2; exit 1; }
  replace_release "$runtime_stage" "$opt/dotnet"
  runtime="$opt/dotnet/dotnet"
fi
# Match the exact wrapper syntax that compare provenance verifies.
# The selected apphost path is relative to the resolved staging root,
# so concatenating the canonical final root preserves its path.
python3 - "$garnet_stage/garnet-server" "$garnet_dest/$garnet_payload_relative" "$runtime" <<'PY'
from pathlib import Path
import shlex
import sys
output, payload, runtime = map(Path, sys.argv[1:])
payload = str(payload.absolute())
runtime = str(runtime.resolve(strict=True))
output.write_text('#!/bin/sh\nexport DOTNET_ROOT=' + shlex.quote(str(Path(runtime).parent))
                  + '\nexec ' + shlex.quote(payload) + ' "$@"\n')
output.chmod(0o755)
PY
garnet_relative=garnet-server
replace_release "$garnet_stage" "$garnet_dest"
receipt "$garnet_dest" "$GARNET_VERSION" "$GARNET_URL" "$GARNET_SHA256" \
  "$garnet_dest/garnet-server" dotnet_apphost_wrapper "$garnet_dest/$garnet_payload_relative" "$runtime"
ln -sfn "$garnet_dest/$garnet_relative" "$bin_dir/garnet-server"
# Garnet --version may start a server: do not invoke it here. The
# instrument's probe records bounded CLI output. Print receipts.
for release in "$valkey_dest" "$dragonfly_dest" "$garnet_dest"; do
  cat "$release/firn-install-receipt"
done
