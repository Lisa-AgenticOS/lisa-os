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
- A shipped `var.mount` unit
  (`mkosi.extra/usr/lib/systemd/system/var.mount`, enabled via
  `local-fs.target.wants/`) mounts `What=/dev/disk/by-partlabel/var` at `/var`,
  ordered into `local-fs.target`.

This supersedes the initial attempt (pinning the partition UUID): a pinned
PARTUUID would still not match an *already-installed* device (its `/var` keeps
the original UUID), whereas the label matches every existing and future install
with **no disk surgery**.

## Consequences

- `lisa update` / sysupdate no longer breaks `/var`; the self-update path holds
  without per-slot hand-patching. Verified by the nightly `ab-sysupdate` job.
- The **existing field iMac needs nothing** — its `/var` is already labeled
  `var`, so a release carrying this change mounts it correctly on first update.
- gpt-auto stays inert (explicit `root=`), so the explicit `var.mount` is what
  drives the mount. Cross-disk label ambiguity (two Lisa disks in one machine)
  is out of scope.
