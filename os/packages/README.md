# os/packages — PKGBUILDs & systemd units

Spec: docs/PLAN.md §6, §5.10. Milestone: M0→M1.

`lisa/` holds the split PKGBUILD (`lisa-inferenced`, `lisa-modeld`,
`lisa-cli`, `lisa-shell`) built from a git-archive tarball of HEAD, plus
`lisa-inferenced.service` — the hardened unit whose sandbox *is* the
egress guarantee: `DynamicUser`, `IPAddressDeny=any` +
`IPAddressAllow=localhost`, full filesystem/kernel lockdown.
`tests/e2e/egress-test.sh` verifies those exact directives in CI;
`tests/e2e/layer-test.sh` proves install/uninstall on vanilla Arch.

`lisa-shell` (arch=any, pure GJS) ships the M4 surfaces (PLAN §5.7):
the surface trees under `/usr/share/lisa/shell/`, the two GNOME Shell
extensions as symlinks under `/usr/share/gnome-shell/extensions/`, the
`org.lisa.Overlay1` D-Bus activation file, the Ledger and AI-settings
(`org.lisa.Settings`) desktop entries — the latter is what the native
gnome-control-center Intelligence panel opens for provider management
(ADR-0012) — and `10_lisa-shell.gschema.override` — session defaults that
enable both extensions and move GNOME's input-source switcher to
Super+Shift+Space so the assistant owns Super+Space (§5.7.1). The
Track I release image folds it in (release.yml); the fcitx5 addon
(§5.7.3 layer 2) needs its own native-build lane and is not packaged
yet.

Build a local repo with `os/repo-tools/build-packages.sh`. The hosted,
signed repo lands in M1; `lisa-modeld.service` lands with the M1 daemon
loop.
