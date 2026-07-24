# ADR-0018: the /var partition UUID is pinned to a constant

- **Status:** accepted
- **Date:** 2026-07-24

## Context

Durable state lives on a separate `/var` partition (ADR era of durable-var,
`mkosi.repart/20-var.conf`). `MountPoint=/var` makes `repart --generate-fstab`
bake a `/var` entry **by PARTUUID** into each root slot's `/etc/fstab`.

Bug: mkosi assigns the `/var` partition a **fresh random PARTUUID on every
image build**, and bakes that build's PARTUUID into that slot's fstab. But the
persistent `/var` partition on a device keeps the PARTUUID from the *original*
install and is never rewritten by an update. So after a `lisa update` /
`systemd-sysupdate` stages a newer slot, that slot's fstab references a `/var`
PARTUUID that does not exist on the disk → `/var` fails to mount → `Local File
Systems` fails → the box drops to **emergency mode**. This is exactly what the
field iMac hit (it was hand-patched per slot), and the nightly `ab-sysupdate`
job has failed on it since 2026-07-24.

## Decision

**Pin the `/var` partition UUID** to a constant in `mkosi.repart/20-var.conf`
(`UUID=489dedcf-8291-4e00-bfc5-ef6b6d5f2131`). Every image build then assigns
`/var` the same PARTUUID and bakes the same value into every slot's fstab, so a
sysupdated slot's fstab always matches the one persistent `/var` partition.

Pinned only in the **image** repart (20-var.conf), not the runtime repart
(`mkosi.extra/.../repart.d/50-var.conf`): the runtime pass only grows the
existing partition and must not be given a UUID that could make it treat an
already-installed device's `/var` as a mismatch and recreate it.

## Consequences

- `lisa update` / sysupdate no longer breaks `/var` — the core self-update
  promise holds without per-slot hand-patching. Verified by the nightly
  `ab-sysupdate` job.
- All Lisa installs share one `/var` PARTUUID. Harmless here: one Lisa disk per
  machine and `root=` is explicit (no gpt-auto ambiguity). Two Lisa disks in
  one machine is out of scope.
- Existing hand-patched devices are unaffected until reinstalled; new images
  are correct from install onward.
