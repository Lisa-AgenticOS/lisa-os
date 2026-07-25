# ADR-0024: ship an out-of-tree CS8409 codec module for Apple speakers

- **Status:** accepted
- **Date:** 2026-07-25
- **Issue:** #44

## Context

The reference device — an iMac18,2 — has silent internal speakers under
Linux. Field forensics on image v20260724.26 (issue #44) ruled out
everything above the driver: the boot chime plays, so the speakers and
amps are fine and firmware knows how to drive them; PipeWire and
WirePlumber are healthy with the right sink (`Built-in Audio Analog
Stereo`), the right active port (`analog-output-speaker`), unmuted, 100 %;
test tones traverse the pipeline with no errors and no sound. The codec is
a Cirrus Logic **CS8409**, subsystem **`0x106b:0x0f00`**, and kernel
7.1.4-arch1-1 binds `snd_hda_codec_cs8409` with a plausible autoconfig
(`line_outs=2 type:speaker 0x24/0x25`, `hp_outs=1 0x2c`).

The CS8409 is a bridge. It does not drive speakers; a companion codec does
— **CS42L83** on these Macs — and the driver only knows how to bring that
companion up if the machine matches its quirk table.

Verified directly against the `linux-7.1.4` tarball from kernel.org (and
`torvalds/linux` `master` at time of writing), not from memory:

- `sound/hda/codecs/cirrus/cs8409-tables.c` contains **80 `SND_PCI_QUIRK`
  entries, all of them Dell `0x1028`**. No Apple vendor ID at all.
- The string **`CS42L83` does not appear anywhere** in
  `sound/hda/codecs/cirrus/`. Mainline knows only the CS42L42 companion.

So the fixup lookup misses, the CS42L83 is never initialized, and the amps
receive nothing. `model=dolphin` (the nearest Dell dual-companion layout)
was tried live and changed nothing — as expected, since the Dell init
sequences are not the Apple ones.

**Consequence for planning: there is nothing upstream to bump to.** A
kernel upgrade cannot fix this, now or on any schedule we control. That
was the first thing checked, because "wait for mainline" would have been
much the better answer.

The community answer for 2017–2019 T1/T2 Macs is
[`davidjo/snd_hda_macbookpro`](https://github.com/davidjo/snd_hda_macbookpro)
(GPL-2.0). Verified by reading the tree at commit
`cb27cc483f4fe98be03a4f4bef466c00aa7d244b` (master, 2026-05-04):

- Its `patch_cs8409.c.diff` grafts a fall-through onto the kernel's own
  `cs8409_probe()`: when `snd_hda_pick_fixup()` leaves
  `fixup_id == HDA_FIXUP_ID_NOT_SET` — "not a Dell" — probe hands off to
  `cs8409_apple()`.
- `cs8409_apple()` gates on `codec->core.subsystem_id` and **names
  `0x106b0f00` (iMac18,2) explicitly**, alongside `0x106b0e00` (18,1),
  `0x106b1000` (18,3 / 19,1), `0x106b3300` (MBP13,1) and `0x106b3900`
  (MBP14,3). Unknown subsystems are rejected with `-ENODEV` and a
  `UNKNOWN subsystem id` log line. iMac support is real code with
  iMac-specific pin NIDs and exec-verb handlers — not an inference from
  the repo's MacBookPro name.
- Both diffs **apply cleanly (`-F0`) to `linux-7.1.4`'s `sound/hda`
  tree** — checked locally.
- Third parties run it on this exact model (davidjo issue #135 is an
  iMac18,2 owner whose complaint is that volume under 15 % is inaudible,
  which presupposes working speakers) and on Arch 7.1.x kernels
  (issue #196).

The alternative found —
[`network-garden-lab/imac18-3-cs8409-ubuntu-hwe-speaker-patch`](https://github.com/network-garden-lab/imac18-3-cs8409-ubuntu-hwe-speaker-patch)
— is a cleaner, better-documented patch but targets subsystem
`106b:1000` (iMac18,**3**, not ours), one Ubuntu HWE kernel, one test
machine, and ships **no LICENSE file**, so it is not redistributable.

## Decision

Ship `os/packages/lisa-audio-cs8409`: a pacman package that builds
upstream's replacement `snd-hda-codec-cs8409` against the image's exact
kernel, installs it to `/usr/lib/modules/<release>/updates/`, and adds a
`depmod` `override` line so it beats the in-tree module. It is folded
into the Track I release image by `release.yml` like the other Lisa
packages.

The kernel pin is hard, in five independent places (`linux-headers`
version, `kernel.release`, scriptlet agreement, `patch -F0`, built
`vermagic`), plus `depends=(linux=<exact version>)` at install time. A
kernel bump **fails the build** and must be re-pinned and re-tested on
hardware by a human.

A sixth guard fails the build if `cs8409-tables.c` ever grows a `0x106b`
row — the signal that mainline may have solved this and that this package
should be reconsidered rather than renewed by inertia.

## Consequences

- The reference iMac plausibly gets working speakers. **Unproven until
  someone listens**: CI compiles, it cannot hear. The verification
  procedure lives in the package README and its result belongs on
  issue #44.
- We now carry an out-of-tree kernel module — a real cost against
  CLAUDE.md rule 4 ("boring tech for plumbing"). It is accepted because
  the alternative is a flagship device with no audio and no upstream path
  on any timescale.
- **Every Arch kernel bump breaks the weekly release build**, by design.
  That is the trade we chose over silently shipping a module that no
  longer loads: a red pipeline is visible, silent speakers are not. Re-pin
  procedure is in the package README.
- Microphone, headphone hot-plug and multichannel profiles remain
  incomplete upstream; speakers are the scope here.
- The endgame is still upstreaming an Apple quirk, and this package does
  not advance it — the Apple path is companion-codec init sequences, not
  a table row, so it is a much larger patch than a quirk entry. Tracked
  as backlog on issue #44, not scope here.
