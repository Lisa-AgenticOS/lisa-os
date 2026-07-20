# os/mkosi — Track I image build

Spec: PLAN §3 (immutability stack), §6 (pipeline). ADR-0001.

Target state: signed UKI + `systemd-repart` partitions, A/B roots via
`systemd-sysupdate`, dm-verity base, boot-counting rollback, LUKS2+TPM2.
M0 acceptance: fresh clone → `just image` → bootable qcow2; update →
rollback demonstrated in the QEMU test.

Status: **building in CI.** `mkosi.conf` is a minimal bootable Arch
profile (ToolsTree=default so it builds on Ubuntu runners);
`mkosi.repart/` sketches the partition set (single root for now — the
A/B pair + verity partitions are the next backlog item). The nightly
workflow validates, builds, and boot-checks the image in QEMU
(direct-kernel boot to `poweroff.target`); swtpm + the sysupdate
rollback demo complete the M0 gate.

Requires Linux; on macOS dev hosts this directory is CI-only.
