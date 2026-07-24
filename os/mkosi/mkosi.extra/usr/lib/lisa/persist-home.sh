#!/usr/bin/env bash
# Back /home with the durable `var` partition so GNOME settings (mouse
# speed, power/suspend toggles), the wallpaper, and the SSH key survive A/B
# updates. /home is otherwise part of the per-slot root, so every
# update+reboot lands on a fresh /home and resets everything (field report
# 2026-07-24). Seeding once from the slot's baked /home also captures the
# settings the user already has, and makes the per-slot SSH-key re-seed
# unnecessary going forward.
#
# Boot-safe, mirroring lisa-model-store.sh: every step tolerates failure and
# on any problem /home stays the per-slot directory — fully usable, just not
# persistent. The unit runs before gdm and user sessions so the bind is in
# place before anyone logs in.
set -uo pipefail

PERSIST=/var/home
MARKER="$PERSIST/.lisa-home-seeded"

# /var must be the durable partition (its own mount). If it is not, there is
# nothing to gain — the root fs is per-slot too — and masking /home would be
# pure downside.
mountpoint -q /var || exit 0

# Idempotent within a boot: already bound (or /home is its own partition).
mountpoint -q /home && exit 0

# Seed exactly once, preserving ownership/perms/timestamps/ACLs. The marker
# is belt-and-suspenders with the mountpoint check above: never copy the
# (empty, freshly-booted) per-slot /home over real persistent data.
if [ ! -f "$MARKER" ]; then
  mkdir -p "$PERSIST" || exit 0
  cp -a /home/. "$PERSIST/" 2>/dev/null || exit 0
  : > "$MARKER" 2>/dev/null || true
fi

# Bind the persistent store over /home. On failure the per-slot /home stays.
mount --bind "$PERSIST" /home 2>/dev/null || exit 0
exit 0
