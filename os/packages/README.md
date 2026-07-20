# os/packages — PKGBUILDs & systemd units

Spec: docs/PLAN.md §6, §5.10. Milestone: M0→M1.

`lisa/` holds the split PKGBUILD (`lisa-inferenced`, `lisa-modeld`,
`lisa-cli`) built from a git-archive tarball of HEAD, plus
`lisa-inferenced.service` — the hardened unit whose sandbox *is* the
egress guarantee: `DynamicUser`, `IPAddressDeny=any` +
`IPAddressAllow=localhost`, full filesystem/kernel lockdown.
`tests/e2e/egress-test.sh` verifies those exact directives in CI;
`tests/e2e/layer-test.sh` proves install/uninstall on vanilla Arch.

Build a local repo with `os/repo-tools/build-packages.sh`. The hosted,
signed repo lands in M1; `lisa-modeld.service` lands with the M1 daemon
loop.
