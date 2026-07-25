#!/bin/sh
# lisa-newest-good-root — resolve /dev/lisa/newest-good to the newest root
# slot that ACTUALLY MOUNTS (ADR-0022 phase 2, issue #23).
#
# The Lisa Rescue boot entry carries `root=/dev/lisa/newest-good` instead
# of a baked PARTLABEL, so it survives every failure that made the field
# iMac need a keyboard on 2026-07-25: a slot erased under its own entry, a
# label pointing at corrupt bytes, an update that relabeled partitions
# mid-flight. Runs in the initrd, before initrd-root-device.target.
#
# Selection: every root_<ver> partition, newest version first, each
# probed by a read-only mount that must expose /usr/lib/os-release. The
# first that passes wins. Nothing is written; a failed probe just moves
# on. If nothing passes we exit silently and the boot fails the way it
# would have anyway — this can only ever improve the outcome.

set -u

link=/dev/lisa/newest-good
probe=/run/lisa-rootprobe

# Only for the rescue entry; normal entries carry their own root=.
grep -q 'root=/dev/lisa/newest-good' /proc/cmdline || exit 0
[ -e "$link" ] && exit 0

command -v udevadm >/dev/null 2>&1 && udevadm settle --timeout=30 || sleep 3
mkdir -p /dev/lisa "$probe"

# Restrict to the disk the boot loader ran from when systemd-stub told us
# (issue #16): with an installer USB inserted, several disks carry
# root_<ver> labels and the wrong one would win.
loader_disk=""
efivar=/sys/firmware/efi/efivars/LoaderDevicePartUUID-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f
if [ -r "$efivar" ]; then
    loader=$(tail -c +5 "$efivar" 2>/dev/null | tr -d '\000' |
        tr '[:upper:]' '[:lower:]' | tr -d ' \r\n')
    if [ -n "$loader" ]; then
        for d in /sys/class/block/*/; do
            name=$(basename "$d")
            [ -e "/sys/class/block/$name/partition" ] && continue
            if sfdisk -J "/dev/$name" 2>/dev/null | tr '[:upper:]' '[:lower:]' |
                grep -q "\"uuid\": *\"$loader\""; then
                loader_disk="$name"
                break
            fi
        done
    fi
fi

# Candidate partitions, newest version first (sort -Vr on the version
# suffix). Kept POSIX-plain: this runs under the initrd's shell.
list_candidates() {
    for p in /dev/disk/by-partlabel/root_*; do
        [ -e "$p" ] || continue
        dev=$(readlink -f "$p")
        devname=${dev##*/}
        if [ -n "$loader_disk" ]; then
            stripped=${devname#"$loader_disk"}
            [ "$stripped" = "$devname" ] && continue
        fi
        printf '%s\t%s\n' "${p##*/root_}" "$dev"
    done | sort -Vr | cut -f2
}
candidates=$(list_candidates)

for dev in $candidates; do
    [ -b "$dev" ] || continue
    if mount -o ro "$dev" "$probe" 2>/dev/null; then
        if [ -f "$probe/usr/lib/os-release" ]; then
            umount "$probe" 2>/dev/null
            ln -sf "$dev" "$link"
            echo "lisa-rescue: booting newest mountable root: $dev"
            exit 0
        fi
        umount "$probe" 2>/dev/null
    fi
    echo "lisa-rescue: $dev did not probe as a Lisa root — trying older"
done

echo "lisa-rescue: no mountable root slot found"
exit 0
