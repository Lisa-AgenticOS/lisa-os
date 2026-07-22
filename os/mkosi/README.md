# os/mkosi — Track I image build

Spec: PLAN §3 (immutability stack), §6 (pipeline). ADR-0001.

Target state: signed UKI + `systemd-repart` partitions, A/B roots via
`systemd-sysupdate`, dm-verity base, boot-counting rollback, LUKS2+TPM2.
M0 acceptance: fresh clone → `just image` → bootable qcow2; update →
rollback demonstrated in the QEMU test.

Status: **building, booting, and rolling back in CI.** `mkosi.conf` is
a minimal bootable Arch profile (ToolsTree=default so it builds on
Ubuntu runners); `mkosi.repart/` has ESP (1G, sized for the A/B UKI
pair) + root + var — the root-b + verity partitions are the next
backlog item. Nightly CI:

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

Field hardware (first target: iMac18,2): explicit
`linux-firmware-amdgpu` / `linux-firmware-broadcom` (Radeon Pro 560
display, BCM43602 Wi-Fi), bluez for Magic input pairing, `hid_apple`
fnmode=2. Boot diagnosis: the journal is persistent, and
`lisa-boot-report.service` (also wanted by emergency/rescue) dumps the
current and previous boot's journal to `lisa-debug/` on the FAT ESP —
readable on any machine the stick is plugged into; the kernel command
line keeps unit status on the console so a hang names its unit.

Remaining for the full Track I story: dm-verity on the root slots,
swtpm in the boot test, signed sysupdate sources (M1 repo).

Requires Linux; on macOS dev hosts this directory is CI-only.
