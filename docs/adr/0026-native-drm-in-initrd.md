# ADR-0026: The native GPU driver + its firmware ride the initrd

- **Status:** accepted
- **Date:** 2026-07-26
- **Amends:** [ADR-0017](0017-plymouth-in-initrd.md) (the "amdgpu is
  intentionally NOT forced into the initrd" clause of its Decision)
- **Claims:**
  - `symbol:amdgpu@os/mkosi/mkosi.conf` — the native DRM drivers in the initrd
  - `path:os/mkosi/initrd-overlay/usr/lib/modules-load.d/lisa-drm.conf` — loaded early, ahead of the splash

## Context

ADR-0017 moved Plymouth and the `lisa` theme into the mkosi-initrd and
added **`simpledrm`** to the parent `KernelInitrdModules=`, on the theory
that the EFI GOP framebuffer needs no firmware and therefore gives
Plymouth a surface with zero size cost. `amdgpu` was left out precisely
*because* it needs firmware blobs.

That did not fix the field device. The reference iMac18,2 (Radeon Pro 560
— Polaris11/Baffin, `amdgpu`) still shows a **black screen from the Apple
logo until GDM, every boot**. The desktop wallpaper and branding render
correctly once GNOME starts, so the defect is confined to the early-boot
window.

Two facts found while investigating, both verified against the artifacts
rather than assumed:

1. **`simpledrm` was never in the initrd, and could not be.** In Arch's
   kernel (7.1.4-arch1-1) `simpledrm`, `drm` and `drm_kms_helper` are
   **built in** — `modules.builtin` lists
   `kernel/drivers/gpu/drm/sysfb/simpledrm.ko`. `KernelInitrdModules=`
   matches `.ko` files under `/usr/lib/modules/<kver>`, so the
   `simpledrm` entry matches nothing. It is not a *missing* driver (the
   builtin binds the GOP framebuffer anyway), but it does mean ADR-0017's
   stated mechanism never actually shipped anything, and whatever the
   Mac's firmware framebuffer does between the Apple logo and the root
   switch, it does not put Lisa's splash on the panel.
2. **Ubuntu's answer to the same problem is the native driver.** Its
   initramfs carries the real KMS driver *and* the firmware that driver
   needs, so the panel is driven at native resolution from the initrd
   onwards. That is the behaviour asked for here.

## Decision

Carry the **native KMS driver and its firmware in the initrd**:

- `KernelInitrdModules=` gains **`amdgpu`** (the field hardware's driver)
  and **`virtio_gpu`** (the same role in QEMU, where the display is
  `-device virtio-vga`). `simpledrm` stays as an inert placeholder should
  a future kernel modularize it.
- **No `FirmwareFiles=`.** mkosi already pulls the module *and firmware*
  dependencies of everything matched by `KernelInitrdModules=`
  (`mkosi/kmod.py`: `resolve_module_dependencies` parses `modinfo`
  `depends=`/`firmware=`; `mkosi.1`: "Firmware dependencies of kernel
  modules installed in the image are automatically included"), so
  `amdgpu`'s 694 declared `amdgpu/*.bin` blobs and its twelve
  `depends=` modules ride along with no extra configuration.
- The temptation to trim that to the ~360 KB of `amdgpu/polaris11_*`
  this GPU actually uses is **explicitly rejected**: `FirmwareFiles=`
  maps to mkosi's `firmware_include`, which is consumed **both** by
  `build_kernel_modules_initrd()` *and* by `run_depmod()` →
  `process_kernel_modules()`, which prunes `/usr/lib/firmware` **in the
  image root**. `process_kernel_modules()` returns early only when all
  four module/firmware filter lists are empty, so setting `FirmwareFiles=`
  at all switches on rootfs pruning, which deletes every firmware file
  that no module declares — including `brcm/*.hcd`, since `btbcm` and
  `btusb` declare **zero** `MODULE_FIRMWARE` (verified against the
  shipped `.ko`s). That would silently kill Bluetooth on the very device
  this ADR exists to fix. 36 MB of ESP is cheaper than that class of bug.

## Consequences

- **Initrd/UKI growth: ~36 MB** (`amdgpu.ko.zst` 5.5 MB + ~0.3 MB of
  helper modules + ≤28–36 MB of already-zstd-compressed `amdgpu/*.bin`;
  the range is hardlink dedup in `linux-firmware-amdgpu`, whose whole
  directory is 28 MB on disk / 35.6 MB summed). The ESP is 1 GiB sized
  for the A/B UKI pair plus the rescue UKI (three UKIs); +36 MB each is
  ~108 MB of a budget with hundreds of MB spare. **No threat to the ESP.**
- The VM path is unchanged in kind: `virtio_gpu` is additive, `simpledrm`
  stays, and the CI boot-checks direct-kernel-boot with `-nographic`
  where no DRM device binds and Plymouth no-ops.
- `amdgpu` now initializes during the initrd phase, adding its (short)
  init to the pre-root window; if its firmware fails to load, Plymouth
  degrades to blank/text exactly as before — the splash never blocks boot.
- **Verification:** the nightly asserts `amdgpu.ko`, its modular DRM
  helpers, and the `polaris11_*` firmware are inside the built UKI's
  initrd — the same style of assertion as the storage/HID stack check
  that exists because of issue #46. Splash *appearance* remains only
  truly verifiable on the graphical hardware.
- Track L (dracut, `os/layer`) is untouched: dracut's host-only mode
  already pulls the bound GPU driver and its firmware into the initramfs.
