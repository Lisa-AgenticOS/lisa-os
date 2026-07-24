# os/mkosi — Track I image build

Spec: PLAN §3 (immutability stack), §6 (pipeline). ADR-0001.

Target state: signed UKI + `systemd-repart` partitions, A/B roots via
`systemd-sysupdate`, dm-verity base, boot-counting rollback, LUKS2+TPM2.
M0 acceptance: fresh clone → `just image` → bootable qcow2; update →
rollback demonstrated in the QEMU test.

Status: **building, booting, and rolling back in CI.** `mkosi.conf` is
a bootable Arch profile (ToolsTree=default so it builds on Ubuntu
runners) that boots into a **GNOME desktop session** (PLAN §3 desktop
strategy: GNOME base, patched not forked); `mkosi.repart/` has ESP
(1G, sized for the A/B UKI pair) + two 8G root slots + var — 19 GiB
total, so USB media must be 32 GB+; the smallest field target disk
(28,000,002,048 bytes ≈ 26 GiB) holds it with room for /var to grow.
Verity partitions are the next backlog item. Nightly CI:

- `image` job: validates, builds, and boot-checks the image in QEMU
  (direct-kernel boot to `poweroff.target`); uploads `lisa.raw`.
- `ab-rollback` job: **automatic rollback demonstrated** — a broken
  higher-version UKI with `+2` systemd-boot try counters is preferred,
  fails twice (reboots), exhausts its counters (renamed `+0-2` in the
  ESP), and the good entry boots to a clean poweroff. Real UEFI via
  OVMF, so systemd-boot itself is exercised.
- `ab-sysupdate` job: **the update direction demonstrated** — v1 boots,
  `systemd-sysupdate` pulls a v2 (root partition image + UKI, with
  SHA256SUMS manifest) over HTTP, installs it into the `_empty` slot
  (relabeled `root_2`), reboots, and v2 boots from slot B to a clean
  poweroff. The PLAN §10 "A/B update + rollback demonstrated" line is
  closed.

Desktop (M4 §5.7 host): gdm + gnome-shell + a hand-picked supporting
set (each justified inline in `mkosi.conf` — no `gnome` group). The
release build folds in `lisa-shell` (os/packages/lisa), which installs
and default-enables the assistant overlay + semantic launcher
extensions and the Ledger app, and moves GNOME's input-source switcher
to Super+Shift+Space so the assistant owns Super+Space (§5.7.1).
Networking on desktop images is NetworkManager over the iwd backend
(the GNOME shell network indicator only speaks NM; iwd stays the
supplicant) — the field test proved a CLI-only Wi-Fi story is a dead
end. Non-NM images keep the networkd DHCP profile path.

**PROVISIONAL field-test login** (on the record, replace with the M7
first-boot OOBE, PLAN §6): user `lisa`, password `lisa`, in `wheel`
with password sudo (`mkosi.extra/etc/sudoers.d/10-wheel`), GDM
autologin (`mkosi.extra/etc/gdm/custom.conf`). The home directory
lives on the root slot (no /home partition yet), so an A/B update does
not carry it over — acceptable for field-test sticks, not for real
installs.

**No first-boot prompts.** Timezone/locale/keymap are baked in
`mkosi.conf` (`Timezone=Europe/Tirane`, `Locale=en_US.UTF-8`,
`Keymap=us`) so `systemd-firstboot` has nothing to ask — without them,
first boot stops at an interactive "select timezone" question on the
console before gdm. `en_US.UTF-8` is generated in the postinst
(`locale-gen`; Arch falls back to C otherwise), and the autologin user
gets `gnome-initial-setup-done` so GNOME's welcome wizard is skipped
too. These are field-device defaults, changeable in GNOME Settings ›
Date & Time / Region (firstboot runs once — an already-provisioned
device won't re-prompt).

Field hardware (first target: iMac18,2): explicit
`linux-firmware-amdgpu` / `linux-firmware-broadcom` (Radeon Pro 560
display, BCM43602 Wi-Fi), bluez for Magic input pairing, `hid_apple`
fnmode=2. Boot diagnosis: the journal is persistent, and
`lisa-boot-report.service` (also wanted by emergency/rescue) dumps the
current and previous boot's journal to `lisa-debug/` on the FAT ESP —
readable on any machine the stick is plugged into. The kernel command
line now routes all console output to `console=ttyS0` (serial) so the
framebuffer is free for the boot splash; a hang is diagnosed from the
ESP journal dump rather than the on-screen unit status it used to show.

## Boot splash

`quiet splash` + `console=ttyS0` (`mkosi.conf` `KernelCommandLine=`) hand
the real display to **Plymouth** so boot shows the Lisa logo on brand
violet — not scrolling kernel/unit text — between the Mac's Apple logo
and GDM. All console/kernel/systemd text goes to the serial line, leaving
tty0 (the framebuffer) clean for Plymouth.

The theme lives in `mkosi.extra/usr/share/plymouth/themes/lisa/`
(`lisa.plymouth`, `ModuleName=two-step` — the same module Arch's stock
`spinner` theme uses): a solid `#6D45C9` background, the white `Lisa`
wordmark (`watermark.png`, recolored from `branding/lisa-wordmark.svg`),
and a subtle comet spinner (`throbber-*.png`). `lisa` is the default via
`etc/plymouth/plymouthd.conf` (`Theme=lisa`) **and** the
`themes/default.plymouth` symlink — no `plymouth-set-default-theme` run,
deterministic in an immutable image.

**Initrd (ADR-0017).** The mkosi image builds its own systemd initrd
(*mkosi-initrd*, not dracut). It now carries **Plymouth + the `lisa` theme**
via the `mkosi.initrd/` overlay (`mkosi.initrd/mkosi.conf` adds `plymouth`;
`mkosi.initrd/mkosi.extra/` ships the theme, `plymouthd.conf`, and a
`sysinit.target.wants/plymouth-start.service` symlink), so the violet Lisa
splash comes up **during the initrd phase — right after the Apple logo**, not
only at `sysinit.target` in the rooted system. `simpledrm`
(`KernelModulesInitrdInclude`) gives Plymouth the EFI framebuffer with no
firmware blob; `amdgpu` (firmware-dependent) takes over later in the rooted
system. Because the theme ships alongside, this is never the theme-less
non-Lisa flash — the reason it used to be kept out. Earlier this was a
rooted-system-only splash with a black window between the Apple logo and
`sysinit`; on the field iMac that window was long enough to read as "powered
off" (the reason for this change). `etc/dracut.conf.d/50-lisa-plymouth.conf`
still pulls Plymouth + the `lisa` theme into any **dracut**-built initrd
(installed-system regeneration, Track L `os/layer`). `plymouth-quit*.service` /
`plymouth-read-write.service` are held enabled in `00-lisa.preset` so the
handoff to GDM is not disabled by a stock `disable *` preset. A missing
or failed splash never blocks boot — Plymouth degrades to blank/text.

**CI is unaffected.** Both boot-checks direct-kernel-boot with their own
`-append` (`nightly.yml`, `release.yml`) and never read this
`KernelCommandLine=`; they keep `console=ttyS0` and still grep "Welcome
to" on the serial log. Under `-nographic` Plymouth finds no DRM device
and no-ops without touching the serial output.

**Follow-up (needs a graphical boot to verify).** systemd-boot may show
its menu with text before the splash; if the menu ever appears on the
real display, set the loader `timeout` to 0 so the Apple logo hands
straight to the splash. There is no on-disk `loader.conf` to edit here
yet (mkosi assembles the ESP), so this is left as a verify-in-CI item.

Remaining for the full Track I story: dm-verity on the root slots,
swtpm in the boot test, signed sysupdate sources (M1 repo).

Requires Linux; on macOS dev hosts this directory is CI-only.
