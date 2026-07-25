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
# Never the default: SORT_KEY alone was NOT enough — a nightly proved
# systemd-boot happily booted the rescue entry, and with no console= on
# its cmdline the CI boots went silent and timed out. So the file is
# named `zz-lisa-rescue.efi` (outside the `lisa*` glob) and loader.conf
# pins `default lisa*.efi`, which matches both the release naming
# (lisa_<ver>.efi) and the image-baked one (lisa-<kver>.efi) and cannot
# match the rescue entry.
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
    --cmdline='root=/dev/lisa/newest-good rw console=tty0 console=ttyS0 systemd.gpt_auto=no' \
    --output="$WORK/lisa-rescue.efi"

mcopy -o -i "$ESP" "$WORK/lisa-rescue.efi" ::/EFI/Linux/zz-lisa-rescue.efi
echo "make-rescue-uki: installed ::/EFI/Linux/zz-lisa-rescue.efi"

# Pin the default so entry sorting can never promote the rescue entry.
if mcopy -i "$ESP" ::/loader/loader.conf "$WORK/loader.conf" 2>/dev/null; then
    grep -v '^default' "$WORK/loader.conf" > "$WORK/loader.new" || true
else
    : > "$WORK/loader.new"
fi
echo 'default lisa*.efi' >> "$WORK/loader.new"
mcopy -o -i "$ESP" "$WORK/loader.new" ::/loader/loader.conf
echo "make-rescue-uki: pinned loader default to lisa*.efi"
mdir -b -i "$ESP" ::/EFI/Linux/
