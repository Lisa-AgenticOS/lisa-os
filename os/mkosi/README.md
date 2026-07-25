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
- `ab-sysupdate` job: **the update direction demonstrated, in the
  issue #20 three-version shape** — the disk starts FULL (v1 booted =
  oldest in slot A, v2 staged in slot B) with v3 (root partition image
  + UKI, with SHA256SUMS manifest) served over HTTP.
  `systemd-sysupdate` must install v3 OVER v2's slot and never touch
  the booted v1 partition: sysupdate has no built-in guard for the
  partition backing `/` — vacuuming for `InstancesMax=` evicts the
  OLDEST version, which on the field iMac was the running root. The
  shipped transfer definitions therefore carry `ProtectVersion=%A`
  (sysupdate.d(5), since systemd v251; `%A` = running os-release
  `IMAGE_VERSION`), and the job asserts v1's PARTLABEL, fs UUID,
  version marker, and a baked 1 MiB canary all survive, then reboots
  into v3. The PLAN §10 "A/B update + rollback demonstrated" line is
  closed.

**The pull stack is declared, not inherited** (issue #45). Downloads
are not done by `systemd-sysupdate` itself: for every `Type=url-file`
source it forks `/usr/lib/systemd/systemd-pull`, which `dlopen()`s
`libcurl.so.4` at runtime and reports *any* dlopen failure as a bare
`EOPNOTSUPP` — the field iMac's `Failed to allocate puller: Operation
not supported` (systemd `src/import/pull.c` → `curl_glue_new` →
`dlopen_curl`). Arch ships libcurl only as an **optional** dependency
of systemd ("curl: … machinectl pull-tar and pull-raw"), so before
this the image's ability to update itself rode on `networkmanager`
happening to depend on `curl`. `curl` and `ca-certificates` are now
named in `Packages=`, and the nightly `image` job asserts
`systemd-pull`, `systemd-sysupdate`, `libcurl.so.4`, a CA bundle and
`ProtectVersion=` in every shipped transfer on the built image.

**Unfinished transfer targets are never booted.** `transfer_acquire_
instance()` relabels *and* retypes the target partition **before** the
first byte is downloaded — new PARTLABEL, plus a derived "partial" GPT
type that is only promoted to the real root type once the install
commits. So an interrupted staging run leaves a slot advertising the
new version over stale or half-written bytes (issue #45: after a
killed rerun, *both* slots stopped switch-rooting). The rescue path's
`newest-good-root.sh` therefore skips any candidate whose GPT type is
not `SD_GPT_ROOT_X86_64`/`SD_GPT_ROOT_ARM64`, and `lisa update` stages
inside a transient `systemd-run` unit so a dropped SSH session cannot
SIGHUP a partition write half way through.

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
only at `sysinit.target` in the rooted system. Because the theme ships
alongside, this is never the theme-less
non-Lisa flash — the reason it used to be kept out. Earlier this was a
rooted-system-only splash with a black window between the Apple logo and
`sysinit`; on the field iMac that window was long enough to read as "powered
off" (the reason for this change). `etc/dracut.conf.d/50-lisa-plymouth.conf`
still pulls Plymouth + the `lisa` theme into any **dracut**-built initrd
(installed-system regeneration, Track L `os/layer`). `plymouth-quit*.service` /
`plymouth-read-write.service` are held enabled in `00-lisa.preset` so the
handoff to GDM is not disabled by a stock `disable *` preset. A missing
or failed splash never blocks boot — Plymouth degrades to blank/text.

**The surface Plymouth paints on (ADR-0025).** A splash in the initrd is
worth nothing without a DRM device in the initrd, and ADR-0017's
`simpledrm` entry shipped none: `simpledrm`, `drm` and `drm_kms_helper`
are **built into** Arch's kernel (`modules.builtin`), so a
`KernelInitrdModules=` glob — which only ever matches `.ko` files —
matched nothing. The iMac18,2 stayed black from the Apple logo to GDM.
So `KernelInitrdModules=` now carries the **native** driver the way
Ubuntu's initramfs does: **`amdgpu`** (with the `amdgpu/*.bin` firmware
and the twelve `depends=` DRM helpers mkosi resolves automatically) and
**`virtio_gpu`** for QEMU's `-device virtio-vga`. Cost: ~36 MB per UKI,
against a 1 GiB ESP holding three of them. `FirmwareFiles=` is
deliberately left unset — it would also switch on mkosi's image-wide
firmware pruning and delete every blob no module declares, Bluetooth's
`brcm/*.hcd` among them (see ADR-0025). The nightly asserts `amdgpu.ko`
+ `polaris11_*` firmware inside the built UKI.

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

## aarch64 lane (ADR-0021)

The same profile builds for arm64 on an **Arch Linux ARM** base:
`mkosi.conf.d/` carries the per-arch split (`[Match] Architecture=` —
`aarch64.conf` picks `linux-aarch64`, appends `console=ttyAMA0`, and
resets `ToolsTree=` because mkosi's default tools tree demands packages
ALARM doesn't have; `x86_64.conf` carries `linux`, moved there because
`Packages=` is append-only across drop-ins). mkosi ≥ 25 targets ALARM's
mirrors natively when the architecture is arm. The resolved x86_64
package set is unchanged by the split.

`.github/workflows/aarch64-image.yml` (weekly + dispatch, native arm64
runner) builds inside the `menci/archlinuxarm` container and boot-checks
with `qemu-system-aarch64 -machine virt` to `poweroff.target`. First
target is QEMU/UTM guests; Asahi bare-metal is explicitly deferred, and
the A/B jobs and the release lane (zen-browser is an x86_64 binary
repackage; llama.cpp needs its PKGBUILD arch extended) are follow-ups —
see the ADR for the verified-vs-first-CI-run ledger.

Remaining for the full Track I story: dm-verity on the root slots,
swtpm in the boot test, signed sysupdate sources (M1 repo).

Requires Linux; on macOS dev hosts this directory is CI-only. The
aarch64 build has, however, been exercised in a local ALARM container on
the Apple Silicon dev host up to final image assembly (repos, full
package install, initrd, partitioning all pass); it stopped at "no
kernel found" because ALARM's kernel lands at /boot/Image — the postinst
now copies it to the modules dir mkosi scans, a fix the first CI run
still has to confirm. Local quirk: the output dir must be
container-local (virtiofs mounts reject the xattr-preserving final
copy).
