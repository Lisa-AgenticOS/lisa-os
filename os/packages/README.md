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
`dev.lisaos.Overlay1` D-Bus activation file, the Ledger and AI-settings
(`app.lisaos.Settings`) desktop entries — the latter is what the native
gnome-control-center Intelligence panel opens for provider management
(ADR-0012) — and `10_lisa-shell.gschema.override` — session defaults that
enable both extensions and move GNOME's input-source switcher to
Super+Shift+Space so the assistant owns Super+Space (§5.7.1). The
Track I release image folds it in (release.yml); the fcitx5 addon
(§5.7.3 layer 2) needs its own native-build lane and is not packaged
yet.

`lisa-audio-cs8409/` is hardware enablement, not Lisa code: an
out-of-tree `snd-hda-codec-cs8409` for Apple Macs whose CS8409 bridge
needs a CS42L83 companion mainline has never heard of — the reference
iMac18,2's silent speakers (issue #44, ADR-0024). Unlike every other
package here it is welded to one kernel release and **fails the release
build when Arch's kernel moves**, deliberately: a codec module built for
another kernel does not load at all, so drift has to be loud. Re-pinning
and the on-hardware verification procedure are in its README — CI can
prove it compiles, only a human can prove it makes sound.

`whisper.cpp/` and `piper/` are the two halves of voice (PLAN §5.7.5,
ADR-0011): the ASR engine behind `lisa transcribe` and the TTS engine
behind a spoken reply. Neither is in Arch, so before these existed the
voice code had nothing to run — the loop was written and unrunnable.
Both are built from source with `GGML_NATIVE`-style portability flags and
**neither is built `--nocheck`**: their `check()` functions execute the
binary they just produced. That is not ceremony. whisper.cpp's first
version linked its libraries through a `RUNPATH` pointing into
`/home/builder/...`; it passed a run test on the builder and would have
failed on every device with "cannot open shared object file". Both
PKGBUILDs now set an install RPATH, ship their own soname symlinks
rather than relying on `ldconfig` to synthesise unowned ones, and assert
in `package()` that the libraries are actually in the package.

`piper` is **not** the `piper` in Arch's repos — that name belongs to a
gaming-mouse configuration GUI. This is `OHF-Voice/piper1-gpl`, and it
is built as `libpiper` (C++ shared library + CLI) rather than the
upstream Python package, so the image needs no Python or onnxruntime
wheel. It takes onnxruntime from Arch, but **vendors espeak-ng at a
pinned commit**: piper calls `espeak_TextToPhonemesWithTerminator()`,
which exists in no espeak-ng release (verified absent at tag 1.52.0, the
version Arch ships). Upstream's CMake fetches both mid-build with no hash
check; the patch here moves that pin into `source=()` where makepkg
fetches it and a reader can see it. **piper is excluded from the ARM
image**: Arch Linux ARM has no onnxruntime, so aarch64 gets speech in
and no speech out — stated in `aarch64-image.yml` rather than faked.

Build a local repo with `os/repo-tools/build-packages.sh`. The hosted,
signed repo lands in M1; `lisa-modeld.service` lands with the M1 daemon
loop.

**Architectures.** Every PKGBUILD here is `arch=(x86_64 aarch64)` (bar
`lisa-shell`, `arch=(any)`) — there is no Lisa package that ships on
x86_64 only (ADR-0021, issue #28). x86_64 is built and published by
`release.yml`; aarch64 is built and folded into the ARM image by
`aarch64-image.yml`, on an Arch Linux ARM base. Per-arch notes live in
each PKGBUILD's header: llama.cpp needs no ARM cmake flags but takes an
armv8-a baseline, and `zen-browser` pins a separate verified digest per
architecture. Anything that genuinely cannot ship on an architecture is
excluded there explicitly — never faked (CLAUDE.md rule 8).

**Payloads that leave the image.** `zen-browser` is a split build
(ADR-0023 phase 1, issue #51): `zen-browser-launcher` is image contract
and stays; `zen-browser` is 363 MiB of `/opt/zen` that moves to the
ADR-0020 apps channel. `os/repo-tools/build-zen-payload.sh` packs the
channel artifact from the *same* pinned digest this PKGBUILD uses, per
architecture, so image and channel can never ship different browsers —
re-pinning a Zen version is still a one-file change here.
