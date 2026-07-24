# ADR-0019: a dedicated /home partition on fresh installs, weight-split with var

- **Status:** accepted
- **Date:** 2026-07-24

## Context

/home persistence shipped as a bind of `/var/home` over `/home`
(lisa-home-persist.service): the durable var partition was the only persistent
storage, so home rode on it. The project owner asked the obvious question —
*why not a real `/home` like Ubuntu?* The honest answer: the image's disk
budget. A 32 GB stick must hold 1G ESP + 2×10G root slots + a var that wants
everything left (models). A fixed home partition doesn't fit; and an
already-installed disk (the field iMac) has var grown to fill the disk, leaving
zero room to add one.

## Decision

A real home partition — created **at first boot on fresh disks** by
systemd-repart, not baked into the image:

- `mkosi.extra/usr/lib/repart.d/60-home.conf`: Type=home, btrfs, `Label=home`,
  Weight=100. `50-var.conf` gets Weight=300 — free disk space splits var 3 :
  home 1 (models dominate storage needs).
- `home.mount` (by **partlabel**, per ADR-0018 — never per-build UUIDs),
  `ConditionPathExists=/dev/disk/by-partlabel/home` so a disk without the
  partition neither waits for a device nor fails a unit; ordered after
  systemd-repart (a first boot sees the partition it just created) and before
  lisa-home-persist.
- First-login state: an empty home partition would hide the baked
  `/home/lisa` (the g-i-s suppression marker), so postinst mirrors it to
  `/usr/share/factory/home/lisa` and `tmpfiles.d/lisa-home-factory.conf`
  copies it in exactly once (`C` = only when missing).
- **Legacy disks keep working untouched**: no home partition → the condition
  is false → lisa-home-persist's `/var/home` bind continues exactly as today
  (it already no-ops when /home is a mountpoint, so the two mechanisms
  compose in either world).

## Consequences

- Fresh installs get an update-proof, first-class `/home`; no bind, no seed
  marker, Ubuntu-familiar.
- Existing installs migrate only via reinstall; their `/var/home` path stays
  supported indefinitely (it is the same code that also serves as the
  fallback).
- The image layout itself is unchanged — nothing new to fit into the 32 GB
  budget; CI's image/boot checks cover the unchanged path, and the fresh-disk
  path is exercised by `lisa install` on hardware.
