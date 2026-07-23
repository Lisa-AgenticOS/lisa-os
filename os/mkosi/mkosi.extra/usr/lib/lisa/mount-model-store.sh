#!/usr/bin/env bash
# Claim the disk's `var` partition as the Lisa model store, and grow it to
# fill the disk.
#
# Why this exists: systemd only auto-mounts a GPT `var` partition at /var
# when the partition UUID equals the machine-id. On a `lisa install` the
# machine-id is generated fresh on first boot and never matches the image's
# baked partition UUID, so /var stays on the ~10G root and the (often huge)
# `var` partition sits unused — models capped at a few GB on a 447G disk.
# This claims that partition for the model store instead. It also fixes the
# companion bug where systemd-repart grows the *partition* but not the btrfs
# *filesystem* inside it (GrowFileSystem=yes leaves the fs at its baked 2G).
#
# Boot-safe: every step tolerates failure and the unit never blocks boot —
# worst case the store stays on the root fs (smaller, but functional). This
# automates the procedure verified by hand on the field iMac (2026-07-23).
set -uo pipefail
STORE=/var/lib/lisa-models

# Idempotent: already mounted (ran before, or /var itself is the partition).
mountpoint -q "$STORE" && exit 0

root_src=$(findmnt -no SOURCE / 2>/dev/null) || exit 0
root_disk=$(lsblk -no PKNAME "$root_src" 2>/dev/null | head -1) || exit 0
[ -n "${root_disk:-}" ] || exit 0

# The `var` partition on the SAME disk as root — never a plugged-in USB
# stick that happens to also carry a `var`-labelled partition.
var_part=$(lsblk -rno NAME,PARTLABEL "/dev/$root_disk" 2>/dev/null \
             | awk '$2=="var"{print $1; exit}')
[ -n "${var_part:-}" ] || exit 0
var_dev="/dev/$var_part"

# If systemd already mounted it at /var (machine-id happened to match),
# leave it be — /var/lib/lisa-models is already on the big partition.
findmnt -no TARGET "$var_dev" 2>/dev/null | grep -qx /var && exit 0

mkdir -p "$STORE"
mount -t btrfs -o rw,relatime,nofail "$var_dev" "$STORE" 2>/dev/null || exit 0
# Grow the btrfs to fill the (repart-extended) partition.
btrfs filesystem resize max "$STORE" 2>/dev/null || true

# Restore the model-store layout on the freshly-mounted fs: group `lisa`,
# setgid, world-readable — modeld writes and inferenced reads it under
# DynamicUser (PLAN §5.1/§5.2).
chgrp lisa "$STORE" 2>/dev/null || true
chmod 2775 "$STORE" 2>/dev/null || true
for d in blobs refs; do
  mkdir -p "$STORE/$d"
  chgrp lisa "$STORE/$d" 2>/dev/null || true
  chmod 2775 "$STORE/$d" 2>/dev/null || true
done
exit 0
