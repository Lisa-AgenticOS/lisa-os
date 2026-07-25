# ADR-0018: /var is mounted by partition LABEL, not by UUID

- **Status:** accepted
- **Date:** 2026-07-24

## Context

Durable state lives on a separate `/var` partition (`mkosi.repart/20-var.conf`).
It was mounted via `MountPoint=/var`, which makes `repart --generate-fstab`
bake a `/var` fstab entry keyed on an identifier that is **assigned fresh on
every image build** — the btrfs filesystem UUID (from `Format=`) and/or the
partition PARTUUID. The persistent `/var` partition, however, keeps the
identifiers from the **original** install and is never rewritten by an update.

Bug: after `lisa update` / `systemd-sysupdate` stages a newer root slot, that
slot's baked fstab references a `/var` identifier the disk's `/var` never had →
`/var` fails to mount → `Local File Systems` fails → the box drops to
**emergency mode** (no networking, no SSH). This is exactly what the field iMac
hit — it had to be hand-patched every update — and it is why the nightly
`ab-sysupdate` job began failing on 2026-07-24. Confirmed on the live device:
its `/var` fstab was hand-patched to `UUID=<btrfs-fs-uuid>`, and every fresh
slot baked a different value.

## Decision

Mount `/var` by its **partition LABEL** (`var`), which is identical on every
build and on the installed disk:

- `20-var.conf` drops `MountPoint=/var` (no per-build fstab identifier) and adds
  `Label=var`.
- A shipped `/etc/fstab` (mkosi.extra) carries
  `PARTLABEL=var /var btrfs …` — **an fstab entry, not a unit file, on
  purpose**: nightly forensics (2026-07-25) showed that with `/var` absent
  from fstab, **systemd-gpt-auto generates its own var.mount keyed on a
  machine-id-derived partition UUID** that exists on no real disk — a 90 s
  device timeout and emergency mode on the updated slot even after the
  fstab/byte-copy fixes (the mysterious constant `…4b47d0…` device wait).
  gpt-auto was initially suspected of racing the fstab-generator and is now
  **fully disabled** (`systemd.gpt_auto=no`) as defense-in-depth — every mount
  is explicit: `root=` for the root, fstab PARTLABEL entries for `/var`,
  `/home`, and `/efi` (the ESP mount sysupdate needs for staging UKIs;
  mountpoint created in postinst).

  **Final forensics (2026-07-25), recorded honestly:** the persistent phantom
  UUID wait was neither gpt-auto nor this build's fstab. The nightly's
  ab-sysupdate VM was **non-hermetic**: the image bakes the production update
  channel (`/usr/lib/sysupdate.d` → releases/latest), sysupdate merges it
  with the test's local transfer configs and picks the highest version — so
  slot B was silently updated to the *published* v25 release (whose fstab
  carries the original per-build /var UUID, i.e. the very bug this ADR
  fixes), not to the build under test. The constant mystery UUID was v25's
  baked /var filesystem UUID. The test now masks the baked channel
  (same-name `/etc/sysupdate.d` symlinks to /dev/null), making ab-sysupdate
  actually test the current build.

This supersedes two earlier attempts: pinning the partition UUID (would not
match an already-installed device), and a `var.mount` unit in /usr (loses to
the gpt-auto generator on some boots — generator output ranks above /usr/lib
in the unit load path). The label matches every existing and future install
with **no disk surgery**.

## Consequences

- `lisa update` / sysupdate no longer breaks `/var`; the self-update path holds
  without per-slot hand-patching. Verified by the nightly `ab-sysupdate` job.
- The **existing field iMac needs nothing** — its `/var` is already labeled
  `var`, so a release carrying this change mounts it correctly on first update.
- gpt-auto stays inert (explicit `root=`), so the explicit `var.mount` is what
  drives the mount.
- ~~Cross-disk label ambiguity (two Lisa disks in one machine) is out of
  scope.~~ **Revised (issue #16):** `lisa install`'s byte-copy makes the
  two-disk case the *normal* post-install state (installer USB still
  inserted), not an exotic one. Three layers now address it:
  `lisa-boot-disk-generator` pins `var`/`home`/`efi` to partitions on the
  disk backing `/` via drop-ins over the fstab units; `lisa install` runs
  `btrfstune -m` on every copied btrfs so fsids never collide; and the
  fstab `PARTLABEL=` sources remain as the fallback for the one-disk case.
