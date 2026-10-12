#!/usr/bin/env bash
# Builds and drives the block device that tests/aof_sync_fault.rs makes the
# append-only file's sync fail on. Run as root (the case calls it through
# `sudo -n`); the `aof-sync-fault` job of .github/workflows/check.yml runs
# `setup` before the case.
#
# usage: aof_sync_fault_device.sh setup MOUNT OWNER
#        aof_sync_fault_device.sh fail | heal | remount
#
#   setup    makes a 64 MiB loop file, puts device-mapper device
#            firn-sync-fault over it as a linear map, makes an ext4
#            filesystem without a journal on it, mounts it at MOUNT with
#            errors=continue, gives MOUNT to OWNER (uid:gid), and checks the
#            fault it is for: under `fail` a write(2) still succeeds while
#            fsync(2) fails with EIO, again when nothing new was written, and
#            fsync succeeds once `heal` restores the map
#   fail     swaps the live map for dm-flakey with error_writes and an up
#            interval of zero: every write and flush request to the device
#            fails, reads still succeed
#   heal     swaps the linear map back
#   remount  unmounts and mounts the filesystem again, which drops its page
#            cache, so what is read afterwards is what reached the device
#
# Why this filesystem: buffered write(2) only fills the page cache, so the
# device's failure first shows at fsync, as Redis's background fsync and
# firn's sync see it. ext4 without a journal reports the failed data
# writeback and the failed cache flush through fsync and, with
# errors=continue, keeps running; with a journal the failed journal commit
# aborts the journal and the filesystem turns read-only, so it could not
# recover when the device does. The suspend uses --nolockfs, because
# freezing the filesystem first would write its dirty pages before the swap
# and so hide a server that never synced.
set -euo pipefail

name=firn-sync-fault
device=/dev/mapper/$name

usage() {
    sed -n '/^# usage:/,/^#   remount/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The sector count and the underlying device (major:minor) the live map uses,
# read back from device-mapper, so the swaps need no state of their own.
read_map() {
    local table
    table=$(dmsetup table "$name")
    sectors=$(awk '{print $2}' <<<"$table")
    under=$(awk '{print $4}' <<<"$table")
}

swap() {
    dmsetup suspend --nolockfs "$name"
    dmsetup load "$name" --table "$1"
    dmsetup resume "$name"
    dmsetup table "$name"
}

fail() {
    read_map
    swap "0 $sectors flakey $under 0 0 1 1 error_writes"
}

heal() {
    read_map
    swap "0 $sectors linear $under 0"
}

mount_point() {
    findmnt -n -o TARGET --source "$device"
}

remount() {
    local target
    target=$(mount_point)
    umount "$target"
    mount -o errors=continue "$device" "$target"
}

# The fault this device exists for, checked before any server runs on it:
# the write reaches the page cache, the sync fails, the device's next sync
# with nothing new also fails, and the healed device syncs.
probe() {
    python3 -I -u - "$1/probe" "$0" <<'EOF'
import errno, os, subprocess, sys

path, script = sys.argv[1], sys.argv[2]

def sync_fails(fd, what):
    try:
        os.fsync(fd)
    except OSError as error:
        if error.errno != errno.EIO:
            sys.exit(f"{what}: fsync failed with {error!r}, not EIO")
        print(f"probe: {what}: fsync failed with EIO")
        return
    sys.exit(f"{what}: fsync succeeded on the failing device")

fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
os.write(fd, b"before\n")
os.fsync(fd)
subprocess.run([script, "fail"], check=True)
written = os.write(fd, b"during\n")
if written != 7:
    sys.exit(f"write on the failing device wrote {written} of 7 bytes")
print("probe: write(2) on the failing device succeeded")
sync_fails(fd, "new bytes")
# Later syncs have no new data to write and must still fail while the device
# refuses what they send, which keeps a server that retries its sync
# refusing writes until the device heals.
for attempt in range(1, 4):
    sync_fails(fd, f"repeat {attempt}, nothing new written")
subprocess.run([script, "heal"], check=True)
os.fsync(fd)
print("probe: fsync on the healed device succeeded")
os.close(fd)
os.unlink(path)
EOF
}

setup() {
    local target=$1 owner=$2 image loop
    # Hosted runners' kernels may keep dm-flakey in the extra modules package.
    if ! modprobe dm_flakey 2>/dev/null; then
        apt-get install -y --no-install-recommends "linux-modules-extra-$(uname -r)"
        modprobe dm_flakey
    fi
    image=$(mktemp /var/tmp/firn-sync-fault.XXXXXX)
    truncate -s 64M "$image"
    loop=$(losetup --find --show "$image")
    dmsetup create "$name" --table "0 $(blockdev --getsz "$loop") linear $loop 0"
    mkfs.ext4 -q -F -O ^has_journal "$device"
    mkdir -p "$target"
    mount -o errors=continue "$device" "$target"
    chown "$owner" "$target"
    findmnt "$target"
    # A volatile write cache makes fsync send a flush request even when no
    # page is dirty, which is why a repeated sync keeps failing.
    echo "write cache: $(cat "/sys/block/$(basename "$(readlink -f "$device")")/queue/write_cache")"
    probe "$target"
}

[ $# -ge 1 ] || usage
action=$1
shift
case $action in
    setup) [ $# -eq 2 ] || usage; setup "$1" "$2" ;;
    fail) [ $# -eq 0 ] || usage; fail ;;
    heal) [ $# -eq 0 ] || usage; heal ;;
    remount) [ $# -eq 0 ] || usage; remount ;;
    *) usage ;;
esac
