# ADR-0017: Plymouth + the lisa theme move into the mkosi-initrd

- **Status:** accepted
- **Date:** 2026-07-24

## Context

The Track I image builds its own systemd initrd (*mkosi-initrd*, not dracut).
Plymouth was deliberately kept OUT of that initrd (ADR-0012 era, documented in
`os/mkosi/README.md`): injecting a theme-less Plymouth would flash a non-Lisa
splash. Instead the splash started in the rooted system at `sysinit.target`
(a `sysinit.target.wants/plymouth-start.service` symlink), accepting a "brief
black window" between the Apple logo and `sysinit.target`.

On the field iMac (Radeon Baffin/amdgpu, ~2-minute boot) that window is **not**
brief — the screen is black for most of boot and reads as "powered off / no
logo" (project owner, 2026-07-24). Confirmed on-device: `lsinitrd | grep -c
plymouth` = 0; boot runs on `simpledrm` (EFI framebuffer); Plymouth starts only
at `sysinit`.

## Decision

Carry Plymouth **and the lisa theme** in the mkosi-initrd, via mkosi's
`mkosi.initrd/` overlay (`os/mkosi/mkosi.initrd/`):

- `mkosi.initrd/mkosi.conf` adds the `plymouth` package to the initrd.
- `mkosi.initrd/mkosi.extra/` ships the `lisa` theme, `plymouthd.conf`
  (`Theme=lisa`), the `default.plymouth` symlink, and a
  `sysinit.target.wants/plymouth-start.service` symlink so the splash comes up
  during the initrd phase — right after the Apple logo.
- The parent `mkosi.conf` adds **`simpledrm`** to `KernelModulesInitrdInclude`:
  on EFI it binds the firmware GOP framebuffer immediately, with **no firmware
  blob**, so Plymouth has a surface to render on inside the initrd. `amdgpu`
  (which needs firmware) is intentionally NOT forced into the initrd; it takes
  over in the rooted system, a possible brief mode-switch but no black gap.

Because the theme ships with it, this is never the theme-less flash the earlier
design avoided — it addresses that concern head-on rather than working around
it.

## Consequences

- The Lisa splash appears from the earliest boot stage; the black window is
  gone. The rooted-system `plymouth-start` wiring stays (harmless; the daemon
  is already up) and `plymouth-quit*`/`read-write` still hand off to GDM.
- The initrd grows by the plymouth binary + the small theme (a few PNGs).
- **Verification:** the CI boot-check direct-kernel-boots the built kernel+initrd
  to `poweroff.target`, so a broken initrd fails CI. Splash *appearance* is
  only truly verifiable on the graphical hardware (the iMac) — this ADR lands
  the mechanism; the on-device confirmation is a follow-up when the device is
  reachable.
- The dracut conf (`50-lisa-plymouth.conf`) is unchanged and still governs
  installed-system regeneration + the Track L pacman layer.
