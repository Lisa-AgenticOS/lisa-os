# ADR-0021: aarch64 image lane on an Arch Linux ARM base

- **Status:** accepted, partially executed — the aarch64 image builds and
  boots in CI on an ALARM base with the same package set as x86_64. No
  aarch64 image has been published, and ARM has speech in but not out (no
  onnxruntime on Arch Linux ARM).
- **Date:** 2026-07-25

## Context

Track I (the immutable mkosi image, ADR-0001) builds an Arch Linux image —
and Arch Linux proper is x86_64-only. The aarch64 world already matters to
this project twice over: the dev host is an Apple Silicon Mac (QEMU/UTM
guests there are arm64), and the Lisa userland is proven on aarch64 — the
full Track L e2e passed natively in an Arch Linux ARM container
(`docker.io/menci/archlinuxarm`) on the dev machine. What is missing is the
*image*: an aarch64 Track I build that boots.

The pieces, verified live from an ALARM container on 2026-07-25 unless
marked otherwise:

- **Arch Linux ARM** (archlinuxarm.org) is the aarch64 counterpart: same
  pacman world, package repos at `mirror.archlinuxarm.org` (reachable,
  redirects to geo mirrors), distinct package set — the kernel is
  `linux-aarch64` (core, 7.1.4-1 at time of writing); there is **no
  `linux` package**. Every other package in `mkosi.conf`'s list exists on
  ALARM under the same name (checked one by one: the full GNOME set,
  firmware subpackages, `dart`, all of it).
- **mkosi ≥ 25 natively supports ALARM**: with `Distribution=arch` and an
  arm architecture, `mkosi/distributions/arch.py repositories()` defaults
  the mirror to `http://mirror.archlinuxarm.org` with ALARM's
  `$arch/$repo` layout and appends the `alarm` repo after core/extra
  (read from the mkosi 25.3/26 source). No `Mirror=` override needed.
- **ALARM's own `mkosi` package is useless here**: ALARM extra ships
  mkosi **14-3**, a pre-v15-rewrite version that cannot parse this
  config (`ToolsTree=`, `KernelModulesInitrdInclude=` are v15+). mkosi
  is **not on PyPI** (404, checked). A pip install from the upstream git
  tag (`v25.3` and `v26` both tried) parses the config but **fails at
  build time** — its sandboxed pacman dies on
  `failed to resolve path '/var/cache/pacman/pkg'`. What works is the
  **Arch-proper package**: `mkosi` is `arch=(any)`, so the exact build
  the x86_64 nightly resolves (`mkosi 26-5`, checked) installs on
  aarch64 via `pacman -U` from the permanent Arch archive
  (`https://archive.archlinux.org/packages/m/mkosi/mkosi-26-5-any.pkg.tar.zst`,
  HTTP 200, checked) — and builds.
- **Drop-in matching**: `[Match] Architecture=arm64` in
  `mkosi.conf.d/*.conf` is the mkosi per-arch conditioning mechanism —
  the `Architecture` setting registers `config_make_enum_matcher`, and
  the enum uses systemd-style names (`arm64`, `x86-64`), not uname
  names (verified from source *and* by running `mkosi summary` for both
  architectures against our config in the ALARM container: arm64
  resolves `linux-aarch64`, x86-64 resolves `linux`).
- **Initrd module list**: `KernelModulesInitrdInclude=` patterns are
  OR'd regexes `search`ed against module paths; a pattern that matches
  nothing (x86-only `uhci_hcd` on arm64, say) is silently skipped, never
  an error (mkosi `kmod.py filter_kernel_modules()`). The shared list
  needs no per-arch split.

## Decision

1. **Base the aarch64 image on Arch Linux ARM**, via mkosi's built-in
   ALARM support — the same `Distribution=arch` config, no mirror
   override.
2. **Per-arch conditioning via `mkosi.conf.d/` `[Match] Architecture=`**:
   - `mkosi.conf.d/aarch64.conf` — adds `linux-aarch64` and appends
     `console=ttyAMA0` (PL011, what `-machine virt` guests have) to the
     kernel command line.
   - `mkosi.conf.d/x86_64.conf` — carries `linux`, moved out of the
     shared `mkosi.conf` because `Packages=` is append-only across
     drop-ins (a drop-in cannot subtract a package). The **resolved
     x86_64 package set is unchanged** — verified with
     `mkosi --architecture=x86-64 summary` before/after.
3. **Native arm64 CI runners**: `.github/workflows/aarch64-image.yml`
   (workflow_dispatch + weekly cron) on `runs-on: ubuntu-24.04-arm` —
   GitHub's free arm64 runners for public repos — building inside a
   `docker.io/menci/archlinuxarm` container, mirroring the x86_64
   nightly's arch-container pattern. No qemu-user/binfmt cross-build:
   native is simpler and fast.
4. **QEMU `-machine virt` is the first boot target** (covers UTM on the
   Mac, which is QEMU underneath). The CI boot-check direct-kernel-boots
   to `poweroff.target` and greps "Welcome to", exactly like the x86_64
   nightly `image` job. UEFI aarch64 (AAVMF/`QEMU_EFI.fd`, from Ubuntu's
   `qemu-efi-aarch64` package — file paths verified in an Ubuntu 24.04
   arm64 container) is how the A/B rollback/sysupdate jobs will run when
   they are ported; systemd-boot + UKIs work the same way on aarch64
   UEFI.
5. **Asahi bare-metal is explicitly deferred.** Booting Apple Silicon
   hardware needs the m1n1 → U-Boot chain and the Asahi kernel/firmware
   work — a different base and boot path entirely, out of scope for this
   lane. This lane targets VMs (QEMU/UTM guests on Apple Silicon and
   arm64 servers).

### Package deltas (aarch64 vs x86_64)

Amended 2026-07-25 (issue #28) — the three "backlog/excluded" rows are
resolved and **there is now no package delta at all** beyond the kernel:

| Package | x86_64 | aarch64 | Note |
|---|---|---|---|
| kernel | `linux` | `linux-aarch64` | ALARM has no `linux` |
| everything else in `mkosi.conf` | ✓ | ✓ | verified present on ALARM |
| `lisa-*` split packages | ✓ | ✓ | already `arch=(x86_64 aarch64)`; `lisa-shell` is `arch=(any)` (pure GJS). Proven by the Track L container e2e |
| `llama.cpp` | ✓ | ✓ | `arch+=(aarch64)`, **no cmake delta**. With `GGML_NATIVE=OFF` and neither `GGML_CPU_ARM_ARCH` nor `GGML_CPU_ALL_VARIANTS` set, ggml adds no `-march`/`-mcpu` on ARM at all (read from `ggml/src/ggml-cpu/CMakeLists.txt` at the pinned tag `b10093`) — armv8-a baseline, the same portability trade the x86_64 build makes. Cost: no dotprod/i8mm-accelerated quantized matmul. See the follow-up below |
| `gnome-control-center-lisa` | ✓ | ✓ | `arch+=(aarch64)`, no other delta. ALARM builds stock `gnome-control-center` **50.3-1** for aarch64 — the exact version this fork pins — so upstream compiles there and `pkgrel=2` outranks it exactly as on Arch proper. Every `depends`/`makedepends` entry (incl. `devtools` for `arch-meson`, `blueprint-compiler`, `glib2-devel`) resolves on ALARM, checked one by one against `mirror.archlinuxarm.org`'s core/extra DBs |
| `zen-browser` | ✓ | ✓ | **This ADR's original "no aarch64 artifact" line was wrong** — an assumption, not a check. Upstream's 1.21.8b release publishes `zen.linux-aarch64.tar.xz` beside the x86_64 one. Downloaded and hashed on 2026-07-25: same top-level `zen/` layout, `zen/zen` is an aarch64 ELF, the icon the `.desktop` points at is present; the x86_64 asset re-hashed byte-identical to the digest already pinned, which is what makes the arm64 digest trustworthy. Per-arch `source_*`/`sha256sums_*` now; a pulled or tampered asset fails the build rather than shipping unverified (rule 8 satisfied by verifying, not by excluding) |

`release.yml` still **publishes** x86_64 only. The arm64 packages are
built and proven in `aarch64-image.yml` instead, which now folds the same
package set into the arm64 image (`PackageDirectories=` +
`Packages=` drop-in, generated in CI, mirroring release.yml's
`50-release.conf`) and asserts the binaries are present and aarch64 in
the built image before the boot check. Giving `release.yml` an
architecture matrix would additionally mean arm64 sysupdate transfers,
per-arch release assets and per-arch installed-version detection — none
of which is needed to answer "does the Lisa stack build and install on
ARM", and all of which is separate work.

Follow-up, not blocking: a faster ARM llama.cpp. Either
`-DGGML_CPU_ARM_ARCH=armv8.2-a+dotprod` (narrows the baseline — drops
armv8.0 parts such as the Cortex-A72 the CI boot check emulates) or
runtime dispatch (`GGML_CPU_ALL_VARIANTS=ON` + `GGML_BACKEND_DL=ON` and
shared libs, which changes what the package ships). Neither should be
taken silently.

## What the container e2e already proved

The full Track L e2e (build → install → daemons up → `lisa ask`
round-trip) passed natively in the `menci/archlinuxarm` container on the
Apple Silicon dev host. That derisks the userland: the Rust workspace,
the pacman packaging, and the daemons all work on aarch64/ALARM. This
lane only has to prove the *image plumbing* (mkosi build, initrd, boot).

## Verified live vs. needs-first-CI-run

Verified live (ALARM container on the arm64 dev host, 2026-07-25):

- ALARM mirror reachability; `linux-aarch64` exists; `linux` does not;
  every other `mkosi.conf` package exists on ALARM.
- mkosi 26-5 (the Arch `any` package) installs and runs on ALARM;
  `mkosi summary` parses this config; the `[Match]` drop-ins select the
  right kernel per arch; mkosi's ALARM mirror/repo defaults (from
  source *and* exercised — the build syncs and installs from
  mirror.archlinuxarm.org with no `Mirror=` set).
- mkosi's **default tools tree cannot be built from ALARM repos** (its
  package list hardcodes `dnf5` and `edk2-ovmf`; "target not found") —
  hence `ToolsTree=` is reset in the aarch64 drop-in and the CI
  container installs the build tools itself.
- pacman ≥ 7 needs `DisableSandbox` inside containers (same as the
  x86_64 nightly).
- Ubuntu 24.04 arm64 package names for the boot check:
  `qemu-system-arm` (ships `qemu-system-aarch64`), `qemu-efi-aarch64`
  (ships `/usr/share/qemu-efi-aarch64/QEMU_EFI.fd` and
  `/usr/share/AAVMF/AAVMF_CODE.fd`).
- A full local `mkosi build` inside the ALARM container ran to **final
  image assembly**: ALARM repo sync, the entire package install
  (GNOME set included), mkosi-initrd build, and systemd-repart
  partitioning/formatting all pass. It stopped at the last step — "A
  bootable image was requested but no kernel was found" — because
  ALARM's `linux-aarch64` installs the kernel as `/boot/Image`
  (mkinitcpio packaging), not `/usr/lib/modules/<kver>/vmlinuz` where
  mkosi looks. `mkosi.postinst.chroot` now copies it into place
  (no-op on x86_64); that fix is the one unverified link in the local
  chain — no rebuild was run after it.
- Local quirk, not a CI concern: outputs cannot land on a
  virtiofs-mounted directory (the xattr-preserving final copy fails);
  CI's bind mount is ext4.

Verified for the package lane (issue #28, from the macOS dev host,
2026-07-25) — evidence, not inference:

- `zen.linux-aarch64.tar.xz` exists for the pinned 1.21.8b tag;
  downloaded, sha256 recorded in the PKGBUILD, `zen/zen` confirmed
  `ELF 64-bit … ARM aarch64`, `zen/` top-level layout and the
  `default128.png` icon path both confirmed. The x86_64 asset re-hashed
  to the digest already in the PKGBUILD — the method is sound.
- Every `depends`/`makedepends` of the forked gnome-control-center, plus
  the whole package-build toolchain the workflow installs
  (`base-devel rust git glib2 cmake devtools`, and `sudo`), exists in
  ALARM's aarch64 core/extra DBs; ALARM carries stock
  `gnome-control-center 50.3-1`, matching this fork's pin.
- llama.cpp needs no ARM cmake flags: read from the pinned tag's own
  `ggml/src/ggml-cpu/CMakeLists.txt` (ARM branch), `GGML_NATIVE=OFF`
  with neither `GGML_CPU_ARM_ARCH` nor `GGML_CPU_ALL_VARIANTS` appends
  no arch flag and there is no fatal path.

Needs the first CI run to confirm:

- The `ubuntu-24.04-arm` runner label (documented as GA for public
  repos; not exercisable from here).
- KVM availability on arm64 runners — the boot check keeps the
  `-accel kvm -accel tcg` fallback either way.
- The `/boot/Image` → modules-dir postinst fix (added after the local
  build pinpointed the failure; no rebuild was run after it).
- The end-to-end boot check (never run locally — no KVM in the podman
  machine VM, and the local build stopped one step short of a UKI).
- **That the three source packages actually compile on aarch64.** Their
  dependencies and build flags are verified; the compiles themselves are
  not — no aarch64 Arch container build was run for this change. The
  workflow is the proof: it fails if any package fails, and the
  `Verify the Lisa stack is inside the arm64 image` step fails if a
  binary is missing or not aarch64.
- **Runner budget.** The job now compiles the Rust workspace, llama.cpp
  and gnome-control-center before the image build; timeout raised
  90 → 180 min, and it inherits the x86_64 release lane's ~14 GB disk
  squeeze (same `rm -rf */src */pkg` reclaim). Either could bite on the
  first run.
- Loop-mounting the btrfs root slot on the `ubuntu-24.04-arm` runner
  (the verify step) — expected to work, not exercised.

## Consequences

- Two kernel lineages: Arch's `linux` and ALARM's `linux-aarch64`
  (different versioning cadence — ALARM core had 7.1.4 while Arch
  tracked its own). Kernel-sensitive fixes must be checked on both.
- ALARM is a distinct upstream with its own outage/lag profile; the
  aarch64 lane is weekly, not nightly, until it earns its keep.
- The pinned-snapshot mirror plan (`os/repo-tools`) must eventually grow
  an ALARM twin — the current pin design is Arch-proper-only.
- mkosi comes from a pinned upstream git tag on aarch64 (v26, matching
  Arch proper's package) instead of a distro package — one more pin to
  bump, in `.github/workflows/aarch64-image.yml`.
