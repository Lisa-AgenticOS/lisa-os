# ADR-0028: Files reach the default initrd through `io.mkosi.initrd`, not `mkosi.initrd/`

- **Status:** accepted
- **Date:** 2026-07-26
- **Fixes:** issue #50
- **Amends:** [ADR-0017](0017-plymouth-in-initrd.md) (its stated
  mechanism — a `mkosi.initrd/` overlay directory — does not exist),
  [ADR-0022](0022-rescue-boot-path.md) (phase 2's resolver shipped
  nowhere for the same reason)

## Context

`os/mkosi/mkosi.initrd/` held a `mkosi.conf` (`Packages=plymouth
util-linux`) and a `mkosi.extra/` tree (`plymouthd.conf`, the `lisa`
Plymouth theme, the rescue root resolver + its unit, the boot-disk udev
rule). It was written on the belief — stated in ADR-0017 and repeated in
`os/mkosi/README.md` — that mkosi treats a `mkosi.initrd/` directory
beside the main config as an overlay for the default initrd.

**No version of mkosi in this pipeline does.** Verified against the
exact version the nightly installs — `pacman -Syu --noconfirm mkosi` in
`archlinux:latest` resolves to **extra/mkosi 26-5** — by reading that
version's source and by running its own `mkosi --directory os/mkosi
summary`:

- `mkosi/config.py:finalize_default_initrd()` builds the default initrd
  as an internal sub-image whose configuration is `chdir`'d into
  `mkosi/resources/mkosi-initrd/` and parsed from **there and nowhere
  else**, seeded only by the settings the main config may push down
  (`Initrd*=`, plus `Keymap`/`Timezone`/`Hostname`/… marked
  `initrd_inherit`).
- Accordingly `mkosi summary` reports, for the `default-initrd` image:
  `Extra Trees: …/resources/mkosi-initrd/mkosi.extra` — ours absent —
  and a `Packages:` list with no `plymouth` in it.

So both halves were inert, not just the file tree: the splash was never
configured in the initrd, the `lisa` theme was never there, **and
Plymouth itself was never installed there either**. ADR-0022 phase 2's
resolver was likewise absent, which is why its CI boot waited 90 s for a
device nothing created and logged nothing (issue #23).

This is the fourth line in this repo that parsed cleanly and did nothing
(see #46 for `KernelModulesInitrdInclude=`, and `simpledrm`, which
matched no `.ko` because Arch builds it in). The pattern is the point:
mkosi silently ignores what it does not recognise, so *belief* about a
mechanism is worth nothing without an assertion on the artifact.

## Decision

**Two mechanisms, both documented in `mkosi.1` for version 26, and one
CI assertion that proves they are still working.**

1. **Packages in the default initrd — `InitrdProfiles=plymouth`** in
   `os/mkosi/mkosi.conf`. This is mkosi's purpose-built switch (scope
   `initrd`, i.e. explicitly pushed down into the initrd sub-image); the
   profile is `Packages=plymouth`. `util-linux` needs no line: mkosi's
   own Arch drop-in already installs it in the initrd, which is where
   `sfdisk` for the `lisa-loader-disk` helper comes from.

2. **Files in the default initrd — `$ARTIFACTDIR/io.mkosi.initrd/`.**
   `os/mkosi/mkosi.finalize` packs `os/mkosi/initrd-overlay/` into an
   uncompressed `newc` cpio and drops it there. `mkosi.1`: "All files in
   this directory are used as initrds and joined in lexicographical
   order"; `mkosi/__init__.py:finalize_initrds()` is
   `config.initrds + sorted(artifacts/"io.mkosi.initrd"/*)`, called from
   `install_kernel()` — which `build_image()` runs immediately *after*
   `run_finalize_scripts()`, with `$ARTIFACTDIR` bind-mounted read-write.
   The old `mkosi.initrd/mkosi.extra/` tree moved to
   `os/mkosi/initrd-overlay/` unchanged; `mkosi.initrd/mkosi.conf` is
   gone, its substance folded into `mkosi.conf` next to
   `InitrdProfiles=`.

3. **`Initrds=` is rejected.** It is the other documented route and it is
   a trap here: `mkosi/config.py:want_default_initrd()` returns `False`
   as soon as `Initrds=` is non-empty, so naming a cpio there *replaces*
   the systemd initrd rather than adding to it. On a path where a mistake
   costs every boot, additive-only is the only acceptable shape.

4. **The nightly asserts the payload inside the built UKI** ("Initrd must
   carry the Lisa initrd overlay"), in the same shape as the storage/HID
   (#46) and display (ADR-0025) checks: dump `.initrd` from the UKI in
   the ESP, decompress every zstd frame, and fail on a missing marker.
   It checks cpio member *names* and distinctive strings from the file
   *bodies*, plus `plymouthd` itself, so a regression in either mechanism
   is a red build rather than a silent hole.

## Consequences

- **Plymouth genuinely enters the initrd for the first time.** ADR-0017's
  intent is only now in effect. The splash comes up during the initrd
  phase, themed — theme, `plymouthd.conf` and the
  `sysinit.target.wants/plymouth-start.service` symlink all ride in the
  overlay, so this is never the theme-less flash ADR-0017 set out to
  avoid. A failed or missing splash still never blocks boot.
- **The overlay wins ties.** It is joined after mkosi's initrd and before
  the kernel-modules initrd; the kernel's unpacker lets later archives
  overwrite earlier ones, which is the direction we want for config files.
- **Uncompressed cpio.** The unpacker decompresses each archive in the
  concatenation independently (how the early-microcode cpio has always
  worked), so a plain cpio between two zstd ones is fine, and the initrd
  grows by only the payload's size.
- **`bash`, not `find`.** The overlay's file list comes from a `globstar`
  expansion because `findutils` is not in mkosi's default tools tree,
  while `bash` and `cpio` are. `--owner=0:0` is passed explicitly: CI
  checks the repo out as the runner user and cpio run as root would
  otherwise carry that uid into the initrd.
- **ADR-0022 phase 2's resolver is now present in the initrd**; its boot
  *entry* remains uninstalled for the unrelated reason in issue #23
  (systemd-boot kept selecting the rescue entry as default). Presence is
  what this ADR delivers.
- **Unverified until the nightly runs on Linux:** the numbers this ADR
  quotes come from mkosi 26's source, its own `summary` on this repo's
  config, and an end-to-end build proving the `io.mkosi.initrd` route on
  a non-Arch distribution. The Arch image itself builds only in CI.
