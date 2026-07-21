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

Remaining for the full Track I story: root-partition A/B via
`systemd-sysupdate` (+ verity), swtpm in the boot test.

Requires Linux; on macOS dev hosts this directory is CI-only.
