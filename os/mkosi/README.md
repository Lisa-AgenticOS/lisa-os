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

The `ab-sysupdate` scenario is not nightly-only: it lives in
`.github/actions/ab-sysupdate` (a composite action) and `release.yml`
runs the *same* code against the release image, after every published
artifact has been extracted from it and before `gh release create`
(issue #47). It has to, because the two images are not the same build:
the release lane folds in the `lisa-*` split packages, llama.cpp,
zen-browser, the forked gnome-control-center and lisa-audio-cs8409, and
a packaging difference that broke staging (issue #45) previously reached
devices with every gate green. The action takes the image's baked
`IMAGE_VERSION` as v1 — `1` for the nightly, `YYYYMMDD.run` for a
release — and derives the staged v2 and the offered v3 from it, so
`ProtectVersion=%A` is exercised against the version the image really
carries.

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

Desktop (M4 §5.7 host): gdm + **Lisa Desktop** + a hand-picked
supporting set (each justified inline in `mkosi.conf` — no `gnome`
group). `lisa-desktop-shell` is the GNOME Shell fork (ADR-0038), built
by the `lisa-desktop` repo's own CI and pulled from the hosted `[lisa]`
index (ADR-0039 step 4; the repo is configured in
`mkosi.pkgmngr/etc/pacman.d/lisa.conf`). It installs at `/usr` with
`provides=(gnome-shell)`/`conflicts=(gnome-shell)`, so **stock GNOME
Shell is not in the image** — the fallback if it fails to start is the
previous A/B root slot, not a second desktop.

GNOME's *foundation* stays and is not up for removal (ADR-0048): mutter,
GTK4/libadwaita, `gnome-session`, `gnome-settings-daemon`,
`gsettings-desktop-schemas`, the portals.

Lisa Desktop is also the **default session**, which needs its own file
because GDM has no default-session setting — its fallback is the
hardcoded string `"gnome"` (`gdm-session.c`,
`get_fallback_session_name`). The only lever is the user's saved session
in accountsservice, so a factory record ships at
`mkosi.extra/usr/share/factory/var/lib/AccountsService/users/lisa` and
`tmpfiles.d/lisa-default-session.conf` copies it onto the persistent
`/var` on first boot (`/var` is a separate partition, so a file baked at
the real path would be shadowed — the same trap `lisa-home-factory.conf`
exists for). It is a default, not a lock: GDM rewrites that file with
whatever the user last picked at the greeter.

`gnome-session` still ships its own `gnome.desktop`, so "GNOME" remains
in the greeter's list. Selecting it runs `/usr/bin/gnome-shell`, which
is Lisa Desktop — the same shell, without Lisa's session drop-in.

The release build folds in `lisa-shell` (os/packages/lisa), which
installs and default-enables the assistant overlay + semantic launcher
extensions and the Ledger app, and moves GNOME's input-source switcher
to Ctrl+Super+Space so the launcher can own Super+Space (§5.7.2) and
the assistant overlay Super+Shift+Space (§5.7.1). Both
halves reach the forked shell only because it installs at `/usr`: the
extensions are found through `XDG_DATA_DIRS`
(`js/misc/fileUtils.js`, `collectFromDatadirs` — not the shell's
compiled-in datadir), and `10_lisa-shell.gschema.override` is compiled
into the same `/usr/share/glib-2.0/schemas` the shell reads. The
release job asserts all of that against the mounted image.
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

## The desktop is pinned (#273), and its ABI is checked (#277)

**What.** `os/mkosi/desktop.lock` names exactly one file — filename,
sha256 and source URL — for `lisa-desktop-shell`, the GNOME Shell fork
built by the separate `lisa-desktop` repo (ADR-0038, ADR-0039).
`os/mkosi/check-desktop.sh` then asserts two things about the image that
was actually built: that the installed shell is that version, and that
the shell and `mutter` come from the same GNOME major series.

**Why.** Every other remote input was already pinned — the ports by
sha256 (`os/packages/ports.lock`, ADR-0051), the models by hash, the
container bases by digest, mkosi by archive version (#271), the in-tree
packages by construction. The shell was not: `Packages=lisa-desktop-shell`
(`mkosi.conf.d/x86_64.conf`) resolved against the **rolling** `current`
tag of the `[lisa]` index, so a publish from `lisa-desktop` between two
image builds changed what the image shipped with nothing in the commit
to record it — two builds of the same SHA could contain different
desktops. And `mutter` still arrives unpinned from Arch, while GNOME
Shell links `libmutter-<N>.so` and loads Mutter's typelib: 50.3 against
50.4 works (the soname is stable inside a series, verified on the
reference iMac), 50.3 against 51.0 dies at a device's login screen.

**How it works**, three enforcement points, because a pin nobody checks
is a comment:

1. `release.yml` downloads the pinned file into `PackageDirectories=`
   and refuses a sha256 mismatch. mkosi writes
   `Include = /etc/mkosi-local.conf` ahead of the `[lisa]` repo and
   pacman takes the first repo that has the package, so the pinned file
   wins over the index. It goes into `/build/pkgs` and **not**
   `/build/repo-out`, so the publish step never pushes the pinned copy
   back onto the device channel.
2. `mkosi.finalize` runs `check-desktop.sh` against `$BUILDROOT` in
   **every** lane — nightly, release, aarch64, local `just image`. It
   reads `/usr/lib/lisa/packages.manifest` (written by
   `mkosi.postinst.chroot` from `pacman -Q`, because `/var/lib/pacman`
   does not survive onto the shipped root), never a variable somebody
   could forget to update. In the nightly — which has no local package
   directory and installs from `[lisa]` — the pin is a version
   assertion, so that lane goes red the day the index publishes a shell
   this repo has not taken.
3. `release.yml`'s desktop step runs the same script again against the
   mounted artifact, so the fact is proven to have survived into the
   image that is about to be published.

**How to extend / bump.** Take the version from a `lisa-desktop`
**release tag** (not the rolling index — the tag is what makes the pin
immutable):

```
gh release view <tag> --repo Lisa-AgenticOS/lisa-desktop \
  --json assets --jq '.assets[] | "\(.name)  \(.digest)"'
```

Copy the filename, the sha256 (drop the `sha256:` prefix) and the asset
URL into `desktop.lock` — one commit, reviewable, recorded in git. The
lock lives here rather than beside `os/packages/ports.lock` because the
image build reads it: mkosi bind-mounts only its own config directory
into the script sandbox as `$SRCDIR`, and the release lane copies
`os/mkosi/` alone to `/build/mkosi`.

**Limits.**

- The `[lisa]` index is still configured for the build
  (`mkosi.pkgmngr/etc/pacman.d/lisa.conf`), because `nightly.yml` builds
  without a local package directory and has nothing else to install the
  shell from. So #273's option 1 is only half done: the *release* image
  no longer takes the index's word, the *nightly* still does and is
  merely asserted against the lock. Dropping the index from the build
  entirely (and with it the keyring-seeding class of #270) needs the
  nightly to fetch the pinned file too.
- `mutter` itself is still unpinned. This gate makes the drift loud, it
  does not prevent it; pinning the Arch snapshot is a separate, larger
  change.
- The aarch64 lane ships stock `gnome-shell` (ADR-0021), so it has no
  shell pin to check. The ABI half of the gate still runs there.
- If an image carries no `packages.manifest` at all, `mkosi.finalize`
  prints that the gates did not run and continues — `nightly.yml` and
  `release.yml` independently fail on a missing manifest.

## Boot splash

`quiet splash` + `console=ttyS0` (`mkosi.conf` `KernelCommandLine=`) hand
the real display to **Plymouth** so boot shows the Lisa logo on brand
violet — not scrolling kernel/unit text — between the Mac's Apple logo
and GDM. All console/kernel/systemd text goes to the serial line, leaving
tty0 (the framebuffer) clean for Plymouth.

The splash is Arch's **stock `bgrt` theme**, unmodified, with one file
swapped: the watermark it draws from the `spinner` theme directory. We
ship no theme of our own — an earlier custom `lisa.plymouth` was deleted
because replacing one PNG is the whole requirement, and a fork of a theme
is a thing to maintain.

    usr/share/plymouth/themes/spinner/watermark.png   128x37, white Lisa
    usr/share/plymouth/themes/spinner/.lisa-branded   marker, issue #45

**Both copies must move together.** The watermark lives in *two* trees:
`initrd-overlay/` and `mkosi.extra/`. `plymouthd` resolves its theme
inside the **initrd**, so editing only the rooted copy changes nothing
you can see — which is exactly how a "fixed" splash once shipped without
ever having rendered.

Rendered from `branding/lisa-wordmark-white.svg` (24x7 viewBox):

    rsvg-convert -w 128 -h 37 -f png -o watermark.png lisa-wordmark-white.svg

Halved from 256x75 on 2026-07-29 — at full size it read as huge on the
reference iMac's panel.

**Initrd (ADR-0017, mechanism fixed by ADR-0028).** The mkosi image builds
its own systemd initrd (*mkosi-initrd*, not dracut). It carries **Plymouth +
the `lisa` theme**, so the violet Lisa splash comes up **during the initrd
phase — right after the Apple logo**, not only at `sysinit.target` in the
rooted system. Because the theme ships alongside, this is never the theme-less
non-Lisa flash — the reason it used to be kept out.

> ADR-0017 said this happened through a `mkosi.initrd/` overlay directory.
> **There is no such convention in mkosi 26** (the version CI installs), and
> the directory this repo carried was read by nothing at all — neither its
> `Packages=` nor its `mkosi.extra/` tree. The splash was never configured in
> the initrd and Plymouth was never *in* it. See issue #50 / ADR-0028; below
> is how it actually works now.

The default initrd is an internal mkosi sub-image configured only from
mkosi's own bundled `mkosi-initrd` resources plus the `Initrd*=` settings
the parent may push down. So:

- **Packages** come from **`InitrdProfiles=plymouth`** in `mkosi.conf` —
  mkosi's own switch for a graphical initrd.
- **Files** come from **`initrd-overlay/`**, which `mkosi.finalize` packs
  into a cpio and drops in `$ARTIFACTDIR/io.mkosi.initrd/`; mkosi joins
  everything there onto the initrd set (`mkosi.1`, `finalize_initrds()`).
  That tree carries the Lisa watermark, the
  `sysinit.target.wants/plymouth-start.service` symlink, the ADR-0022
  rescue root resolver + its unit, and the issue #16 boot-disk udev rule.
- `Initrds=` is deliberately **not** used: it *replaces* the default initrd
  rather than adding to it (`want_default_initrd()`), which on this path is
  a brick.
- The nightly asserts the whole payload inside the built UKI ("Initrd must
  carry the Lisa initrd overlay"), because a mechanism that stops working
  must not keep looking like one that works.

Earlier this was a
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
