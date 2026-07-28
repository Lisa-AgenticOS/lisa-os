# lisa-audio-cs8409 — Apple CS8409/CS42L83 speaker override

Spec: docs/PLAN.md §3, §6 (image content). Decision: ADR-0024. Issue: #44.
Milestone: M0+ (hardware enablement for the reference device).

Internal speakers on Apple Macs whose HD-audio bridge is the Cirrus Logic
**CS8409** with a **CS42L83** companion codec — the reference iMac18,2
(`0x106b:0x0f00`), and by the same code path iMac18,1 / 18,3 / 19,1 and
MacBookPro13,1 / 14,x.

## Why

Mainline's CS8409 driver picks its init sequence from a quirk table keyed
on the codec subsystem ID. In `linux-7.1.4`,
`sound/hda/codecs/cirrus/cs8409-tables.c` holds **80 `SND_PCI_QUIRK`
rows, every single one Dell (`0x1028`)** — and the string `CS42L83`
appears **nowhere** in that directory. (Both checked by hand against the
kernel.org tarball, not from memory; the same is true of Linus' `master`
at time of writing.)

So on Apple hardware `snd_hda_pick_fixup()` misses, the CS42L83 that
actually drives the amps is never brought up, and the result is the exact
field symptom in issue #44: PipeWire healthy, correct sink, correct port,
unmuted, 100%, zero errors — and zero sound. The boot chime still works
because *firmware* initializes the amp before Linux takes over.

There is nothing upstream to bump to. A newer kernel does not fix this.

## What it ships

One file that matters:

```
/usr/lib/modules/<release>/updates/snd-hda-codec-cs8409.ko.zst
/usr/lib/depmod.d/50-lisa-audio-cs8409.conf     # override, so ours wins
```

built from [`davidjo/snd_hda_macbookpro`](https://github.com/davidjo/snd_hda_macbookpro)
(GPL-2.0), pinned to a commit, grafted onto that same kernel's own
`cs8409.c`. Upstream's hook: when `snd_hda_pick_fixup()` leaves
`fixup_id == HDA_FIXUP_ID_NOT_SET` — i.e. "not a Dell" — probe falls
through to `cs8409_apple()`, which dispatches on
`codec->core.subsystem_id` and has explicit
`0x106b0e00 / 0x0f00 / 0x1000` (iMac18,1 / 18,2 / 18,3+19,1) branches
with iMac-specific pin NIDs, exec-verb handlers and CS42L83 + TDM amp
bring-up.

The in-tree module stays on disk; the `updates/` copy plus the depmod
`override` line decides which one `modprobe` loads.

## Honest scope — read before promising anyone audio

- **iMac18,2 is a supported machine in upstream's code**, not an
  extrapolation from MacBookPro: `0x106b0f00` is named in the probe gate
  and in the iMac-specific paths. Third parties run it on iMac18,2 today
  (davidjo issue #135 is an iMac18,2 user complaining that volume below
  15 % is inaudible — i.e. their speakers work), and on Arch kernel 7.1.x
  (issue #196).
- **Upstream's README only advertises MacBookPro**, and iMac reports are
  more mixed than MacBookPro ones. Amp coverage is MAX98706, SSM3515 and
  TAS5764L; the 2017 iMacs use the TAS576x family.
- **Not verified by us on hardware, and not verifiable by CI** — nothing
  in this pipeline has ears. CI proves it *compiles for the right
  kernel*. Only the steps below prove it *makes sound*.
- Known upstream limitations even when it works: internal/headset
  microphone is incomplete, headphone hot-plug is fiddly, and the raw
  `hw:0,0` device has no volume control (it is **very** loud).
- x86_64 only. Intel Macs; the aarch64 lane (ADR-0021) has no such
  hardware.

Rejected alternative:
[`network-garden-lab/imac18-3-cs8409-ubuntu-hwe-speaker-patch`](https://github.com/network-garden-lab/imac18-3-cs8409-ubuntu-hwe-speaker-patch)
— a focused, well-documented iMac18,**3** patch, but it targets subsystem
`106b:1000` (not our `106b:0f00`), a single Ubuntu HWE kernel, one test
machine, and it **carries no LICENSE file**, so we cannot redistribute
it. Kept on file as a cross-check if the davidjo path disappoints.

## Verifying on hardware

Neither the author of this package nor CI can hear anything. Someone has
to sit in front of the iMac. Do these in order and stop at the first
failure — each step localises the fault.

**1. The right module is loaded.** Before anything else:

```sh
modinfo -n snd_hda_codec_cs8409
```

Must print a path under **`/updates/`**. If it prints
`.../kernel/sound/hda/codecs/cirrus/...`, the override lost — run
`sudo depmod 7.1.5-arch1-2`, reboot, and check again. Cross-check the
build matches the running kernel:

```sh
uname -r                                        # 7.1.5-arch1-2
modinfo -F vermagic snd_hda_codec_cs8409        # same release
```

**2. The Apple path actually ran.** The in-tree driver is silent about
Apple; upstream's is chatty:

```sh
sudo dmesg | grep -i -e cs8409 -e cs42l83 -e cs42 -e 'Primary cs8409'
```

Expect `Primary patch_cs8409 NOT FOUND trying APPLE` followed by CS42L83
/ TDM / amp bring-up lines. If instead you see
`UNKNOWN subsystem id 0x...`, the module loaded but does not recognise
this machine — record that subsystem ID, it is the whole bug report.
Confirm the codec is the expected one:

```sh
cat /proc/asound/card0/codec#0 | head -20      # Cirrus Logic CS8409
grep -r . /sys/class/sound/hwC0D0/subsystem_id # 0x106b0f00 on iMac18,2
```

**3. Make a noise.** Straight at ALSA first, bypassing PipeWire, so a
session-level problem cannot be mistaken for a driver problem:

```sh
speaker-test -D default -c 2 -t sine -f 440 -l 1
```

Then through the normal stack:

```sh
wpctl status                       # sink present, not suspended
wpctl get-volume @DEFAULT_AUDIO_SINK@
pw-play /usr/share/sounds/freedesktop/stereo/bell.oga
```

Keep the profile at `output:analog-stereo+input:analog-stereo`. Upstream
warns that 4.0 / multichannel profiles may produce noise or desync the
desktop volume control.

**4. Volume floor.** Known upstream quirk on iMac18,2: below roughly 15 %
there may be no audible output at all. Test at 50 % before concluding it
is broken.

**5. It survives.** Reboot and repeat step 3 — and after a
`lisa update` to a **same-kernel** release, repeat step 1. A release
whose kernel moved will not have installed this package at all (see
below), which is the loud failure working as designed.

Report back on issue #44 with the output of steps 1 and 2 either way.
A "no sound" report without the `dmesg` lines is not actionable.

## Re-pinning (when the kernel moves)

This package is welded to one kernel release, because a codec module
built for a different one does not degrade — it simply never loads, and
the machine goes back to silent speakers with no error printed anywhere.
Every guard here exists to convert that silence into a build failure.

Five guards, any of which stops the build:

1. `prepare()` compares the pinned `_archkernver` against `pacman -Q
   linux-headers` in the build root.
2. `prepare()` requires `/usr/lib/modules/<_modrel>/build` and checks its
   `include/config/kernel.release`, and that the scriptlet's copy of
   `_modrel` agrees.
3. The two upstream diffs are applied with `-F0` — a hunk that only
   nearly applies is kernel drift, not a rounding error.
4. `build()` reads the built module's `vermagic` back and requires the
   pinned release.
5. `build()` greps the built `.ko` for the Apple fall-through printk, so
   a module that compiled with the Apple branch preprocessed away cannot
   masquerade as a fix.

Plus one at install time: `depends=(linux=<exact version>)`, so pacman
and mkosi refuse to put this package in an image with a different kernel.

And one deliberate obsolescence guard: `prepare()` **fails** if the
kernel's `cs8409-tables.c` has grown any `0x106b` (Apple) row. That would
mean mainline may finally have fixed issue #44, and shipping an
out-of-tree override past that point should be a decision, not an
oversight.

To re-pin:

1. `_archkernver` / `_modrel` ← the new Arch kernel
   (`https://archlinux.org/packages/core/x86_64/linux/json/`).
2. `_kernver` ← the matching mainline version, and take its **published**
   sha256 from `https://cdn.kernel.org/pub/linux/kernel/vN.x/sha256sums.asc`.
   Never compute and paste your own digest — that authenticates nothing.
3. `_modrel` in `lisa-audio-cs8409.install` — same value.
4. `_commit` ← re-pin `davidjo/snd_hda_macbookpro` if the diffs no longer
   apply; bump `pkgver`'s date to that commit's date.
5. Rebuild, then **redo the hardware verification above**. A green build
   means "compiled", not "audible".

Upstream tracks kernel support in `install.cirrus.driver.sh`
(`current_major` / `current_minor`); anything past it prints "Kernel
version later than implemented version". As of the pinned commit that
ceiling is 6.17, yet 7.1.x builds and works in the field (davidjo issue
#196) — treat the ceiling as a hint, not a verdict.

## Building it by hand

```sh
cd os/packages/lisa-audio-cs8409
makepkg -d --nocheck --noconfirm     # linux-headers must already match
```

Arch container only. In CI it is built and folded into the image by
`.github/workflows/release.yml`.

## The 7.1.5 header rebase (issue #44)

Upstream's `patch_cs8409.h.diff` was written against a `cs8409.h` that
predates two lines the kernel later added **inside the diff's context**,
not in the regions it edits:

```c
#include "../side-codecs/hda_component.h"
unsigned int speaker_muted:1;
```

Two of its four hunks therefore miss, and the package was dropped from
the release for a week because of it. Nothing actually conflicts — the
diff only inserts Apple blocks elsewhere — so `prepare()` lifts those
two lines out, applies the diff with **no fuzz**, and puts them back
after the lines they followed.

**Why not just raise the fuzz.** `-F2` makes this apply too, and would
go on applying through drift that genuinely matters, producing a module
that loads and misbehaves rather than one that fails to build. Naming
the two known lines keeps every *other* movement a red build. Each step
is guarded: the two lines must be present before stripping, the patch
must apply at `-F0`, and each must come back exactly once — zero means
an anchor moved, two means the strip missed.

When mainline grows a `0x106b` row in `cs8409-tables.c`, Guard 3 fails
the build on purpose and this whole package should be reconsidered
rather than re-pinned.
