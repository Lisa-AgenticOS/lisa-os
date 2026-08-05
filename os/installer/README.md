# os/installer — getting Lisa onto a disk

Spec: `docs/PLAN.md` §6. Decision: **ADR-0055** (the live USB is the one
image on removable media), building on ADR-0052/ADR-0053. Milestone: the
guided OOBE is M7.

## What it does

There are two halves, and only one of them exists.

**The proto-installer — exists.** `lisa install` writes the release image
onto a whole disk. It is how a Lisa stick is made, and how a machine gets
Lisa from a stick. Because a Lisa stick *is* a Lisa system (ADR-0055 —
the image is the same bytes either way), installing is a byte copy, not a
partitioning run: the image carries its own GPT.

The verb's job is therefore almost entirely **deciding which disk may be
erased**. That decision lives in `cli/lisa/src/install_plan.rs` — pure
functions over an injected block-device topology and an injected view of
the running system — and it is the only part of the install path that has
tests, because it is the only part whose bug class is *somebody else's
data is gone*.

**The guided OOBE — not started.** Language, disk (TPM-LUKS default),
user, then Intelligence setup: honest hardware tier, model lineup with
sizes and licences, every context source OFF by default. This directory
holds no code for it. Read PLAN §6 before writing any.

## How it works

The smallest real use is the picker. It writes nothing:

```
$ lisa install --list
>> running from removable media (/dev/sda) — a live session; installing to another disk is what this verb is for

  [   refused ] /dev/sda — 59.6 GiB removable, Cruzer Blade
                /dev/sda is the disk this system is running from
                - / is mounted from /dev/sda2
                - the boot loader was loaded from /dev/sda1 (EFI LoaderDevicePartUUID)
                Boot the Lisa USB stick and install to the internal disk from there.
  [installable] /dev/nvme0n1 — 476.9 GiB internal, SAMSUNG MZVLB512HBJQ
                erases /dev/nvme0n1p1 (0.3 GiB, EFI system partition)
                erases /dev/nvme0n1p2 (0.0 GiB, Microsoft reserved partition)
                erases /dev/nvme0n1p3 (476.2 GiB, Basic data partition)
```

Then `lisa install /dev/nvme0n1`, which reprints that list for the chosen
disk and demands the literal word `ERASE` before it opens the device.

`install_plan::plan()` refuses in this order, most-actionable first:

| refusal | what it caught |
|---|---|
| `NoSuchDevice` | a typo; the picker lists what is real |
| `IsAPartition` | `lisa install /dev/nvme0n1p3` — a GPT image written into somebody's third partition. Names the disk that was meant |
| `BootDiskUnknown` | neither `/` nor the EFI loader variable resolved to a disk. **Fails closed**: every disk is refused, because any of them could be the one we are standing on |
| `IsTheBootDisk` | the stick you are running from |
| `ReadOnly` | a write-protect switch |
| `TooSmall` | below `MIN_TARGET_BYTES` (23 GiB, the sum of `os/mkosi/mkosi.repart/`). A partial write destroys what was there and boots nothing |
| `InUse` | a filesystem from the target is mounted — the user opened it in Files first |

Two independent signals identify the booted disk, because each alone has
a blind spot: `/` in `/proc/mounts` (absent when the running root is not
a block device — initrd, rescue, overlay) and systemd-stub's
`LoaderDevicePartUUID` EFI variable (absent on direct-kernel and non-EFI
boots). Either one is sufficient; neither is required.

What this replaced was a single line — `mounts.lines().any(|l| … 
d.starts_with(target))` — which could not see the disk it booted from
unless something from it happened to be mounted, could not tell a
partition from a disk, could not tell 16 GB from enough, and treated
`starts_with` as "is a partition of" (which eMMC's `mmcblk0boot0`
disproves in both directions). The module header carries the full
autopsy.

## How to extend

- **A new refusal** is a `Refusal` variant, a `Display` arm that says
  what to do instead, and a test. Then **break it and watch the test go
  red** — the module exists so that is possible without a disk. Every
  check in it has been mutated once already; a check that has never been
  seen to fail is a check nobody knows works.
- **A new fact about the machine** goes in `SystemFacts` and is read in
  `read_facts()`. Keep `read_topology`/`read_facts` decision-free: they
  are the only functions here that cannot be tested off Linux, so
  anything they decide is untested by construction.
- **The picker's text** is `render_targets()`, a pure `String`, asserted
  on whole. Print it; do not grow a second copy of it in `main.rs`.
- **The guided OOBE** should call `install_plan` for disk selection
  rather than restate it. A rule that exists twice is the defect this
  repo keeps re-learning.

## Limits

- **Nothing here has been run against a real disk by the author of this
  file.** Development happens on macOS, where `lisa install --list`
  refuses (`lsblk` is Linux-only) and block-device targets are refused by
  `install_cmd` itself. The unit tests, the `--from <file>` write path
  and the refusal wiring are what have been executed; the lsblk call, the
  `/proc/mounts` read, the efivar read and every erase have not.
- **The write is not verified and not resumable.** A failure mid-copy
  leaves a destroyed disk and reports the error. Nothing reads the disk
  back and compares it to the image.
- **`--yes` skips the typed confirmation but not the refusals.** It is
  for CI. There is no flag that skips a refusal, deliberately.
- **eMMC boot areas** (`/dev/mmcblk0boot0`) are sibling block devices,
  not children, so nothing relates them to `/dev/mmcblk0`. They are
  refused today only because they are 4 MiB and the size floor catches
  them. That is a coincidence.
- **`MIN_TARGET_BYTES` is a second copy of the partition arithmetic** in
  `os/mkosi/mkosi.repart/`. Its unit test asserts the sum, but no lint
  links the two files; `check-repart-slots.py` is the place that should
  (ADR-0055 consequences).
- **The live-session mount scoping this verb assumes is only mitigated,
  not proven.** ADR-0055 "What is not built" is the list:
  `lisa-boot-disk-generator` fails open, the initrd-side `root=` is
  scoped only by udev link priority, and **no CI gate has ever booted two
  Lisa disks at once** — which is exactly the machine where the GPT
  labels are ambiguous (issue #16's open remainder,
  `docs/STATUS.md`).
- **No dual-boot, no resize, no partition picker.** The image owns the
  whole disk. That is ADR-0001's A/B layout, not an omission — but it
  does mean "install alongside Windows" is not a thing `lisa install`
  can be asked for, and the refusal text says so by naming every
  partition it is about to take.
- **The guided OOBE (M7) does not exist**, and neither does LUKS/TPM
  enrolment. `lisa install` produces the same provisional `lisa`/`lisa`
  autologin account the image ships with.
