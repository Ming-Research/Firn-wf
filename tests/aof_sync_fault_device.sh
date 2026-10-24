#!/usr/bin/env bash
# Builds and drives the block device that tests/aof_sync_fault.rs makes the
# append-only file's sync fail on. Run as root (the cases call it through
# `sudo -n`); the `aof-sync-fault` job of .github/workflows/check.yml runs
# `setup` before the cases and `teardown` after them, whatever they did.
#
# usage: aof_sync_fault_device.sh setup MOUNT OWNER
#        aof_sync_fault_device.sh fail | heal | remount | reset | fill | unfill
#        aof_sync_fault_device.sh check-writeback MOUNT
#        aof_sync_fault_device.sh bind SOURCE TARGET | unbind TARGET
#        aof_sync_fault_device.sh teardown MOUNT
#
#   setup    makes a 64 MiB loop file, puts device-mapper device
#            firn-sync-fault over it as a linear map, makes an ext4
#            filesystem without a journal on it, mounts it at MOUNT with
#            errors=continue, gives MOUNT to OWNER (uid:gid), and checks the
#            fault it is for: under `fail` a write(2) still succeeds while
#            fsync(2) fails with EIO, again when nothing new was written, and
#            fsync succeeds once `heal` restores the map. It also saves
#            the kernel's periodic-writeback settings and turns that
#            writeback off (see "Why no periodic writeback" below)
#   fail     swaps the live map for dm-flakey with error_writes and an up
#            interval of zero: every write and flush request to the device
#            fails, reads still succeed
#   heal     swaps the linear map back
#   remount  unmounts the filesystem, checks the unmounted device with
#            e2fsck -fn and mounts it again, which drops the page cache, so
#            what is read afterwards is what reached the device; it fails
#            with "harness fault" when e2fsck finds the metadata damaged
#   reset    heals the device and makes the filesystem anew (unmount, mkfs,
#            mount, give the mount to the OWNER `setup` was given), so that
#            a case whose metadata was lost cannot break the next; each case
#            calls it before it starts
#   fill     allocates the remaining blocks to a disposable file until the
#            kernel returns ENOSPC; the rewrite cases stop the rewrite worker
#            first, then observe ENOSPC on its temporary file after resuming
#   unfill   removes that file, leaving the server's files alone
#   check-writeback MOUNT
#            the self-check of the writeback setting: creates a file on
#            MOUNT, fails the device for 40 s with no fsync, heals, remounts
#            and requires the file and undamaged metadata
#   bind SOURCE TARGET
#            covers the healthy old-increment pathname with SOURCE on the
#            faulty device before startup; the manifest stays healthy
#   unbind TARGET
#            removes a registered binding after its server has stopped
#   teardown unmounts MOUNT, removes the device-mapper device, detaches the
#            loop device, deletes the image and restores the writeback
#            settings; each step is skipped when
#            its part is already gone, so it is safe after a failed or
#            partial `setup`. `setup` runs it itself when one of its steps
#            fails, leaving nothing behind
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
#
# Why no periodic writeback: the kernel's flusher thread wakes every 5 s
# (vm.dirty_writeback_centisecs) and writes what has been dirty for 30 s
# (vm.dirty_expire_centisecs). Without a journal that includes the
# filesystem's own metadata, so a flush that falls inside a failing window
# fails too and the kernel drops those dirty buffers ("lost async page
# write"); after `remount` ext4 then reads stale metadata and the case dies
# with ENOSPC or "checksum invalid" for a reason that is not the server's.
# `setup` therefore turns the periodic flusher off (writeback 0, expire very
# large): during a failing window only the server's own fsync and the
# harness's explicit umount may write to the device, so a server that never
# fsyncs is still caught. No `sync` is added, for the reason given above.
# `check-writeback` shows the setting works, and `remount` checks the
# metadata with e2fsck -fn rather than the kernel log: the log cannot tell a
# lost write of a data block, which the cases cause on purpose, from a lost
# write of metadata, while e2fsck looks only at what the filesystem needs to
# be consistent (it ignores the "mounting fs with errors" flag the setup's
# probe leaves, because of -f, and finds a failed data write harmless).
set -euo pipefail

name=firn-sync-fault
device=/dev/mapper/$name
# What `setup` made or changed, one per line, written as each exists so that
# `teardown` finds them even when the map is gone: the loop device, the
# image, the saved vm.dirty_writeback_centisecs, the saved
# vm.dirty_expire_centisecs and the owner of the mount.
state=/var/tmp/$name.state
# File bindings the isolated close-sync case owns, for teardown after a
# killed test process as well as its normal, ordered cleanup.
bindings=/var/tmp/$name.bindings
writeback=/proc/sys/vm/dirty_writeback_centisecs
expire=/proc/sys/vm/dirty_expire_centisecs

usage() {
    sed -n '/^# usage:/,/^#   teardown/p' "$0" | sed 's/^# \{0,1\}//' >&2
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

# A failed load leaves the device suspended, and every I/O to it then hangs,
# so it is resumed with the table it had before the failure is reported.
swap() {
    dmsetup suspend --nolockfs "$name"
    if ! dmsetup load "$name" --table "$1"; then
        dmsetup resume "$name"
        echo "dmsetup load failed; resumed $name with its previous table" >&2
        return 1
    fi
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
    findmnt -n -o TARGET --source "$device" --first-only
}

# Only a regular file below the live fault mount may cover an existing
# regular file on another filesystem. Directories, links and newline paths
# are refused; the registry is root-owned and names only these test mounts.
bind_file() {
    local source target root
    source=$(realpath -e -- "$1")
    target=$(realpath -e -- "$2")
    root=$(mount_point)
    [[ "$1" = "$source" && "$2" = "$target" && "$source" = "$root/"* ]] || return 1
    [[ "$source" != *$'\n'* && "$target" != *$'\n'* ]] || return 1
    [ -f "$source" ] && [ -f "$target" ] || return 1
    [ "$(stat -c %d "$source")" = "$(stat -c %d "$root")" ] || return 1
    [ "$(stat -c %d "$target")" != "$(stat -c %d "$root")" ] || return 1
    ! findmnt -rn --mountpoint "$target" >/dev/null || return 1
    # Register before mounting so teardown also finds a partially failed bind.
    printf '%s\n' "$target" >>"$bindings"
    mount --bind "$source" "$target"
}

unbind_file() {
    local target=$1
    [ -f "$bindings" ] && grep -Fxq -- "$target" "$bindings" || return 1
    if findmnt -rn --mountpoint "$target" >/dev/null; then
        umount -- "$target"
    fi
    # Keep an entry until its unmount succeeded: a failed cleanup must leave
    # enough information for the workflow's teardown.
    python3 -I - "$bindings" "$target" <<'EOF'
import pathlib, sys
registry, target = pathlib.Path(sys.argv[1]), sys.argv[2]
registry.write_text(''.join(line + '\n' for line in registry.read_text().splitlines() if line != target))
EOF
}

unbind_all() {
    local target
    [ -f "$bindings" ] || return 0
    while IFS= read -r target; do
        [ -n "$target" ] || continue
        if findmnt -rn --mountpoint "$target" >/dev/null; then
            if ! umount -- "$target"; then
                [ "${1:-}" = lazy ] && umount -l -- "$target" || return 1
            fi
        fi
    done <"$bindings"
    rm -f "$bindings"
}

# Run on the unmounted device. -n changes nothing; any exit status but 0
# means the metadata on the device is damaged.
check_metadata() {
    if ! e2fsck -fn "$device"; then
        echo "harness fault: filesystem metadata lost on the faulty device" >&2
        return 1
    fi
}

remount() {
    local target
    target=$(mount_point)
    umount "$target"
    check_metadata
    mount -o errors=continue "$device" "$target"
}

# A new filesystem on the healed device, for a case to start from.
reset() {
    local target loop image wb exp owner
    { read -r loop && read -r image && read -r wb && read -r exp && read -r owner; } <"$state"
    target=$(mount_point)
    heal >/dev/null
    unbind_all
    umount "$target"
    mkfs.ext4 -q -F -O ^has_journal "$device"
    mount -o errors=continue "$device" "$target"
    chown "$owner" "$target"
}

# Real allocation, not a sparse file or RLIMIT_FSIZE (which returns EFBIG).
# Keep the allocation durable so a later remount tests the server's files,
# without a filesystem-wide write failure losing unrelated metadata.
fill() {
    python3 -I - "$(mount_point)/rewrite-fault-filler" <<'EOF'
import errno, os, sys

fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
offset = 0
try:
    for chunk in (1024 * 1024, 4096):
        while True:
            try:
                os.posix_fallocate(fd, offset, chunk)
                offset += chunk
            except OSError as error:
                if error.errno != errno.ENOSPC:
                    raise
                break
    os.fsync(fd)
    print(f"rewrite-fault fill: allocated {offset} bytes; final allocation failed with ENOSPC")
finally:
    os.close(fd)
EOF
}

unfill() {
    rm -f "$(mount_point)/rewrite-fault-filler"
}

# Shows that no periodic writeback reaches the failing device: a file made
# on MOUNT and never synced is there after the device failed for longer than
# the kernel's 30 s dirty expiry plus its 5 s wakeup, and the metadata is
# whole. With the flusher on, it writes the new file's metadata during the
# window, the write fails and the buffers are dropped, so the file is gone or
# e2fsck reports the damage.
check_writeback() {
    local target=$1
    echo "check-writeback: dirty_writeback_centisecs=$(cat "$writeback") dirty_expire_centisecs=$(cat "$expire")"
    mkdir "$target/writeback-check"
    echo data >"$target/writeback-check/file"
    trap 'heal >/dev/null 2>&1 || true' EXIT
    fail
    sleep 40
    heal
    trap - EXIT
    remount
    if [ ! -f "$target/writeback-check/file" ]; then
        echo "harness fault: a file made before the device failed is gone: periodic writeback reached the failing device" >&2
        return 1
    fi
    rm -r "$target/writeback-check"
    echo "check-writeback: no periodic writeback reached the failing device"
}

# The fault this device exists for, checked before any server runs on it:
# the write reaches the page cache, the sync fails, the device's next sync
# with nothing new also fails, and the healed device syncs.
probe() {
    python3 -I -u - "$1/probe" "$0" <<'EOF'
import errno, os, subprocess, sys

path, script = sys.argv[1], sys.argv[2]

def sync_fails(fd, what, sync=os.fsync):
    try:
        sync(fd)
    except OSError as error:
        if error.errno != errno.EIO:
            sys.exit(f"{what}: sync failed with {error!r}, not EIO")
        print(f"probe: {what}: sync failed with EIO")
        return
    sys.exit(f"{what}: sync succeeded on the failing device")

fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
os.write(fd, b"before\n")
os.fsync(fd)
subprocess.run([script, "fail"], check=True)
# The close-sync case writes nothing after the fault. Both servers use
# fdatasync on Linux; verify both it and fsync on this already-synced inode.
# Check this independently of failed dirty-page writeback below.
sync_fails(fd, "clean file fsync")
sync_fails(fd, "clean file fdatasync", os.fdatasync)
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
os.fdatasync(fd)
print("probe: fsync on the healed device succeeded")
os.close(fd)
os.unlink(path)
EOF
}

# Undoes `setup` as far as it got. Every step tolerates a part that is
# already gone, so the function cannot fail, and the order is the reverse of
# the making: the mount, the map, the loop device, the image, the writeback
# settings.
teardown() {
    local target=$1 loop='' image='' wb='' exp='' owner=''
    if [ -f "$state" ]; then
        { read -r loop && read -r image && read -r wb && read -r exp && read -r owner; } <"$state" || true
    fi
    if dmsetup info "$name" >/dev/null 2>&1; then
        heal >/dev/null 2>&1 || true
    fi
    # Release file bindings before their source filesystem; otherwise a
    # killed case keeps the device busy after its main mount disappears.
    unbind_all lazy || true
    if findmnt -n "$target" >/dev/null 2>&1; then
        umount "$target" 2>/dev/null || umount -l "$target" || true
    fi
    if dmsetup info "$name" >/dev/null 2>&1; then
        dmsetup remove --retry "$name" 2>/dev/null || dmsetup remove --force "$name" || true
    fi
    if [ -n "$loop" ] && losetup "$loop" >/dev/null 2>&1; then
        losetup -d "$loop" || true
    fi
    if [ -n "$image" ]; then
        rm -f "$image"
    fi
    if [ -n "$wb" ]; then
        echo "$wb" >"$writeback" || true
    fi
    if [ -n "$exp" ]; then
        echo "$exp" >"$expire" || true
    fi
    rm -f "$state"
    rmdir "$target" 2>/dev/null || true
    return 0
}

setup() {
    local target=$1 owner=$2 image loop wb exp
    if [ -e "$state" ] || dmsetup info "$name" >/dev/null 2>&1; then
        echo "$name is already set up; run teardown first" >&2
        return 1
    fi
    # A step that fails removes what the earlier ones made.
    trap "teardown '$target'" EXIT
    # Hosted runners' kernels may keep dm-flakey in the extra modules package.
    if ! modprobe dm_flakey 2>/dev/null; then
        apt-get install -y --no-install-recommends "linux-modules-extra-$(uname -r)"
        modprobe dm_flakey
    fi
    image=$(mktemp /var/tmp/firn-sync-fault.XXXXXX)
    wb=$(cat "$writeback")
    exp=$(cat "$expire")
    # The settings are saved before they change, and the state file already
    # names them, so a failed step restores them. 100000000 centiseconds
    # (about 11 days) is far below the overflow of the kernel's conversion
    # to milliseconds.
    printf '\n%s\n%s\n%s\n%s\n' "$image" "$wb" "$exp" "$owner" >"$state"
    echo 0 >"$writeback"
    echo 100000000 >"$expire"
    truncate -s 64M "$image"
    loop=$(losetup --find --show "$image")
    printf '%s\n%s\n%s\n%s\n%s\n' "$loop" "$image" "$wb" "$exp" "$owner" >"$state"
    dmsetup create "$name" --table "0 $(blockdev --getsz "$loop") linear $loop 0"
    mkfs.ext4 -q -F -O ^has_journal "$device"
    mkdir -p "$target"
    mount -o errors=continue "$device" "$target"
    chown "$owner" "$target"
    findmnt "$target"
    # A volatile write cache makes fsync send a flush request even when no
    # page is dirty, which is why a repeated sync keeps failing.
    echo "writeback: $(cat "$writeback") expire: $(cat "$expire")"
    echo "write cache: $(cat "/sys/block/$(basename "$(readlink -f "$device")")/queue/write_cache")"
    probe "$target"
    trap - EXIT
}

[ $# -ge 1 ] || usage
action=$1
shift
case $action in
    setup) [ $# -eq 2 ] || usage; setup "$1" "$2" ;;
    fail) [ $# -eq 0 ] || usage; fail ;;
    heal) [ $# -eq 0 ] || usage; heal ;;
    remount) [ $# -eq 0 ] || usage; remount ;;
    reset) [ $# -eq 0 ] || usage; reset ;;
    bind) [ $# -eq 2 ] || usage; bind_file "$1" "$2" ;;
    unbind) [ $# -eq 1 ] || usage; unbind_file "$1" ;;
    fill) [ $# -eq 0 ] || usage; fill ;;
    unfill) [ $# -eq 0 ] || usage; unfill ;;
    check-writeback) [ $# -eq 1 ] || usage; check_writeback "$1" ;;
    teardown) [ $# -eq 1 ] || usage; teardown "$1" ;;
    *) usage ;;
esac
