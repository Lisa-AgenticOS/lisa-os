#!/usr/bin/env bash
# make-rescue-uki.sh <image.raw> — add the "Lisa Rescue" entry to an
# image's ESP (ADR-0022 phase 2, issue #23).
#
# The rescue entry is the same kernel and initrd as the versioned UKI,
# rebuilt with one difference: `root=/dev/lisa/newest-good`, which the
# initrd resolver (usr/lib/lisa/newest-good-root.sh) points at the newest
# root slot that actually mounts. That is the hand-edit of the kernel
# command line the field iMac needed three times on 2026-07-25, automated
# — it survives a slot erased under its own entry, a label pointing at
# corrupt bytes, and an update that relabeled partitions mid-flight.
#
# It also drops `quiet splash` and keeps the console on tty0, so a rescue
# boot is readable instead of a black screen.
#
# Sorting: the embedded os-release carries SORT_KEY=zz-lisa-rescue so
# systemd-boot ranks it below every versioned entry — present in the
# menu, never the default.
#
# Requires: mtools, fdisk, jq, binutils, systemd-ukify (CI installs them).
set -euo pipefail

RAW="${1:?usage: make-rescue-uki.sh <image.raw>}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

OFF=$(( $(sfdisk -J "$RAW" | jq -r '.partitiontable.partitions[0].start') * 512 ))
ESP="${RAW}@@${OFF}"

NAME=$(mdir -b -i "$ESP" ::/EFI/Linux/ | grep -o 'lisa[-_][^/]*\.efi' | head -1)
if [ -z "$NAME" ]; then
    echo "make-rescue-uki: no UKI found in the ESP — nothing to base a rescue entry on" >&2
    exit 1
fi
echo "make-rescue-uki: basing the rescue entry on $NAME"

mcopy -i "$ESP" "::/EFI/Linux/$NAME" "$WORK/base.efi"
objcopy \
    --dump-section .linux="$WORK/vmlinuz" \
    --dump-section .initrd="$WORK/initrd.img" \
    --dump-section .osrel="$WORK/osrel" \
    "$WORK/base.efi" "$WORK/discard.efi"

# Rank below every versioned entry, and label it in the menu.
sed -e '/^SORT_KEY=/d' -e '/^PRETTY_NAME=/d' "$WORK/osrel" > "$WORK/osrel.rescue"
{
    echo 'SORT_KEY=zz-lisa-rescue'
    echo 'PRETTY_NAME="Lisa OS (Rescue)"'
} >> "$WORK/osrel.rescue"

UKIFY=$(command -v ukify || echo /usr/lib/systemd/ukify)
"$UKIFY" build \
    --linux="$WORK/vmlinuz" \
    --initrd="$WORK/initrd.img" \
    --os-release="@$WORK/osrel.rescue" \
    --cmdline='root=/dev/lisa/newest-good rw systemd.gpt_auto=no' \
    --output="$WORK/lisa-rescue.efi"

mcopy -o -i "$ESP" "$WORK/lisa-rescue.efi" ::/EFI/Linux/lisa-rescue.efi
echo "make-rescue-uki: installed ::/EFI/Linux/lisa-rescue.efi"
mdir -b -i "$ESP" ::/EFI/Linux/
