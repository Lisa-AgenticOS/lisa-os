# ADR-0055 — The live USB is the one image on removable media; liveness is where it booted from, not a lineage

- **Status:** accepted, partially executed — the medium and the boot are
  what ship today and are CI-gated; the *guarantee* below (a live
  session touches only the disk it booted from) is enforced in the
  installer as of this ADR and only mitigated, never verified, in the
  mount path. §"What is not built" is the honest list.
- **Date:** 2026-08-05
- **Claims:**
  - `symbol:fn is_live_session@cli/lisa/src/install_plan.rs` — liveness is read off the booted medium, not a lineage
  - `path:os/mkosi/mkosi.extra/usr/lib/systemd/system-generators/lisa-boot-disk-generator` — the topology resolver §"What is not built" says fails open
  - `path:os/mkosi/mkosi.extra/usr/lib/udev/rules.d/59-lisa-boot-disk.rules` — the link_priority scoping the same section calls insufficient

## Context

The ask was the ordinary one: *write an image to a stick, boot it, try
the real desktop without touching my disk, then install it like Ubuntu.*

The first surprise on reading the source is how much of that already
exists. Every release publishes `lisa-usb-<ver>.raw.zst` (~2.0 GB
compressed, ~19 GiB written); `release.yml`'s own notes say **"pre-alpha,
runs-from-USB — not an installer"**; the nightly and release lanes both
boot the shipped artifact in QEMU before anything is published. Writing
it to a 32 GB stick and booting it gives a real Lisa Desktop session, and
`lisa install /dev/<internal>` copies that same image onto an internal
disk from inside it.

So the question was never "how do we build a live lane". It is: **what
kind of thing is the live medium, given that ADR-0052 says install mode
is an image lineage and ADR-0053 says don't mint a lineage until it has
features that justify its own download page?**

Three answers were available and two of them are wrong.

## Decision

**Liveness is a property of the medium a Lisa disk was written to, not a
second image, not a second lineage, and not a mode flag.** The stick and
the internal disk carry byte-identical images. "Trying Lisa" is running
Lisa; "installing" is copying the disk you are running from onto another
disk.

This is ADR-0053's step-1 reasoning applied to a second axis. ADR-0053
declined to mint the `lisa-server` lineage ADR-0052 specified, because a
second lineage doubles the image build *and* the A/B test matrix and buys
nothing until the product has features of its own. A live ISO buys even
less: it has no features of its own at all, and it would cost more —

- a second root filesystem format (squashfs) beside btrfs,
- a second initrd path (overlay assembly) beside `root=PARTLABEL=`,
- a second boot target and a second set of what-persists rules,
- and a second thing to boot-gate in CI on every release.

ADR-0052's mechanics are what make this conclusive rather than merely
cheaper. Its load-bearing clause is **"the update channel is part of the
mode"**: two lineages exist precisely when two `sysupdate` transfer names
exist. A live medium has *no* update channel — it is the release artifact
itself, the thing the channel serves. There is nothing for a second
lineage to be.

### What "live" therefore means here, exactly

Not the Ubuntu casper meaning. Naming the difference is the point of
writing this down:

| | Ubuntu live ISO | A Lisa stick |
|---|---|---|
| root | squashfs + tmpfs overlay | the real btrfs root, slot A |
| survives reboot | no (unless "persistence" is configured) | yes, on the stick |
| what you install | a different set of bits, unpacked | **the same bytes, copied** |
| medium size | ~5 GB | 32 GB+ (the A/B layout is 19 GiB) |
| installer | a separate app that partitions | `lisa install`, a byte copy |

The row that earns the decision is the third. What a person tries is
what they get, with no "the installed system behaves differently"
class of bug — a class the immutable A/B design exists to eliminate and
which a squashfs live lane would reintroduce at the front door.

### The safety property, stated once

Persistence-off is the wrong frame; the stick is *deliberately*
persistent, so you can try Lisa across a reboot, and that is why the
download asks for 32 GB. The property that actually matters is:

> **A live session may read and write only the disk it booted from,
> until the person deliberately says otherwise — and the only verb that
> says otherwise is `lisa install`, which must prove the disk it is
> about to erase is not the one it is running from.**

Both halves have teeth here, and they are separately enforced:

1. **Mount scoping.** The image mounts `/var`, `/home` and `/efi` by GPT
   partition label (ADR-0018, `mkosi.extra/etc/fstab`). On a machine that
   *already has Lisa installed*, both disks carry partitions labelled
   `var`, `home` and `esp`, and the label is ambiguous by construction —
   `20-var.conf` says so in as many words ("Cross-disk label ambiguity is
   out of scope: one Lisa disk per machine"). A live USB breaks that
   assumption on purpose. Issue #16's three mitigations
   (`lisa-boot-disk-generator`, `59-lisa-boot-disk.rules` +
   `lisa-loader-disk`, `btrfstune -m` after install) are what stand
   between a live session and someone's installed `/var`.
2. **Install targeting.** `lisa install` decides which disk gets erased.
   That decision now lives in `cli/lisa/src/install_plan.rs` as pure
   functions over an injected `lsblk` topology and an injected view of
   `/proc/mounts` + the EFI `LoaderDevicePartUUID`, with a test per
   refusal — because a wrong-disk guard that can only be exercised by
   attaching a disk is a guard nobody has ever seen fail.

## What this is not

- **Not a rejection of a small ISO forever.** It is a rejection of one
  *now*, on ADR-0053's grounds. The day Lisa has a reason a live medium
  must differ from an installed one — a rescue environment with tools the
  installed image does not carry, an installer that repartitions rather
  than overwrites — that reason mints the lineage, and ADR-0052's
  mechanics describe how.
- **Not a claim that the stick is safe on a machine with Lisa already
  installed.** See below. That is the one configuration where the label
  ambiguity is real, and nobody has booted it.
- **Not a guided installer.** `lisa install` erases a whole disk. It does
  not shrink a Windows partition, it does not dual-boot, and it does not
  ask about keyboard layouts. The guided OOBE is M7 (`os/installer/`,
  PLAN §6) and this ADR does not bring it forward.

## What is not built

Named here rather than in a user guide, because a page telling somebody
how to boot a stick nobody has booted is this repo's most-repeated
defect:

- **`lisa-boot-disk-generator` fails open.** It exits silently when the
  topology does not resolve ("the fstab defaults then apply"), and the
  fstab default is the ambiguous `PARTLABEL=`. On the one machine where
  ambiguity is real — a live stick beside an installed Lisa — failing
  open means the live session's Ledger, models and journal can land in
  the installed system's `/var`. Failing closed there is a behaviour
  change to a boot-critical unit and needs a QEMU two-disk test to land
  with it, which is Linux work.
- **The initrd-side `root=` remains scoped only by udev link priority.**
  `docs/STATUS.md` has carried "Open remainder: initrd-side `root=`
  scoping" since issue #16 landed. `root=PARTLABEL=root_1` on a
  two-Lisa-disk machine is resolved by `59-lisa-boot-disk.rules` raising
  `link_priority` on the booted disk's partitions, which depends on
  `LoaderDevicePartUUID` being readable — absent on non-EFI and
  direct-kernel boots, where the helper marks *every* disk and stock
  behaviour returns.
- **No two-disk boot test exists in CI.** Every gate boots one disk. The
  scenario this ADR is about — stick plus installed internal disk — has
  never been executed anywhere, in CI or on hardware.
- **The installer's write is not resumable or verified.** A failed
  `io::copy` half way through leaves a destroyed disk and says so; it
  does not check the written bytes against the image afterwards.
- **eMMC boot areas are outside the topology.** `/dev/mmcblk0boot0` is a
  sibling block device of `/dev/mmcblk0`, not a child, so the
  boot-disk check cannot relate them. In practice they are 4 MiB and the
  size floor refuses them; that is a coincidence, not a guarantee.

## Consequences

- The download page's instructions become *accurate* rather than
  aspirational for the first two steps and, with `lisa install --list`,
  for the third. They were already published; the gap was that step 3
  had one weak guard behind it.
- The 32 GB floor is now a decision with a reason attached, not an
  artifact of the A/B layout that keeps embarrassing the release notes.
  A smaller medium requires a different root format, which is a lineage,
  which is ADR-0052.
- `MIN_TARGET_BYTES` in `install_plan.rs` is a second copy of the
  partition arithmetic in `os/mkosi/mkosi.repart/`. It is asserted
  against the sum in that module's tests, but nothing links the two
  files — a slot-size change that forgets it will fail a unit test with
  a confusing message rather than a lint with a clear one. The honest
  fix is to extend `check-repart-slots.py`; it is not done.
