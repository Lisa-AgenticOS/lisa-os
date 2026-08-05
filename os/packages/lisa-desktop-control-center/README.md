# gnome-control-center-lisa — GNOME Settings with a native Lisa panel

Spec: PLAN §5.3 (Settings panel), §5.11, §8. Decision: **ADR-0012**.

Our own build of `gnome-control-center`, pinned to the image's GNOME
version (**50.3**), that adds a native **Intelligence** panel to the
Settings sidebar. gnome-control-center has no plugin API — panels are
compiled in — so a sidebar entry requires building the package ourselves.
This is a *thin, maintained patch*, not a source fork: everything is
stock upstream except the panel dir and two anchored edits.

## What the panel does (v1)

- **Local models** — reads `lisa models catalog --json` (§8 hardware-aware
  fit) and renders each model with a plain-words badge (*installed* /
  *runs on this machine* / *tight fit* / *too big — use a provider*) and a
  one-click **Get** (`lisa models get`) for pinned models that fit. Local
  inference never leaves the machine — nothing here is egress-marked.
- **Providers & privacy** — opens the `app.lisaos.Settings` app for the full
  provider / key / Sign-in-with-Claude / offload-consent flow. v2 moves
  these native into the panel (same `dev.lisaos.Remote1` D-Bus calls).

The panel talks only to existing backends (`dev.lisaos.Remote1`, the `lisa`
CLI); no backend logic is duplicated.

## The OS updates row in Settings → System

GNOME 50 already reserves a **Software Updates** group on the System page
(`software_updates_group` in `panels/system/cc-system-panel.blp`), but
keeps it hidden unless `gnome-software` or `gpk-update-viewer` is
installed, and its row spawns one of those two binaries. Lisa ships
neither — OS updates are whole-image A/B swaps through
`systemd-sysupdate`. So on Lisa that page answered neither *what am I
running* nor *is there anything newer*.

We **claim that group** rather than adding a competing one:

- `show_software_updates_group()` returns `TRUE` — sysupdate is not
  optional on a Track I system, so there is no configuration where this
  page should stay silent about the OS version.
- GNOME's gnome-software row is stripped from the `.blp`, keeping the
  group and its id so the template-child binding still resolves.
- `lisa_system_updates_attach()` fills it with the Lisa row: **Lisa OS**,
  the running `IMAGE_VERSION` as subtitle (shown before anything is
  clicked), and a **Check for Updates** button.

The button runs `lisa update --check`, which downloads nothing and
changes nothing. It reports three separate facts — `running:`, `staged:`,
`available:` — and the row renders the one that most changes what to do
next: a staged update outranks an available one, because the download
already happened and only a restart is missing.

**Why a subprocess and not D-Bus.** Everything here could talk to
`org.freedesktop.sysupdate1` directly. That would also mean a second
implementation of "which of running, staged and available is which" — in
C, in a GNOME panel, untested — beside the Rust one. Getting that
distinction wrong is precisely issue #144, so it lives in one place with
unit tests and this row asks it.

**Checking never installs.** Staging is privileged, slow and writes
partitions; it stays behind the explicit Update control in the
Intelligence panel. A button labelled *check* must not.

## Files

Panel sources sit beside the PKGBUILD (makepkg resolves local sources by
basename in `$startdir` — no subdirs) and `prepare()` copies them into
`panels/lisa/`:

- `PKGBUILD` — derived from Arch's `gnome-control-center` 50.3-1; the only
  delta is the four panel sources + the `prepare()` injection.
- `cc-lisa-panel.{h,c}` — the panel (`CcPanel : AdwNavigationPage`, built
  programmatically — no `.ui`/gresource) → `panels/lisa/`.
- `cc-lisa-panel-meson.build` — per-panel build (mirrors upstream, minus
  blueprint/gresource, plus `json-glib` + `gio-unix-2.0`) →
  `panels/lisa/meson.build`.
- `gnome-lisa-panel.desktop.in` — id `lisa`, category
  `X-GNOME-SystemSettings` (System group), search keywords.

- `cc-system-updates-lisa.{h,c}` — the OS updates row for Settings →
  System → `panels/system/`. It lives here, as a readable file, rather
  than as a here-doc inside the PKGBUILD: the Lisa delta should be
  reviewable as code.

`prepare()` copies those into `panels/lisa/` and applies three anchored
edits (guarded — a GNOME bump that moves an anchor fails the build):
`panels/meson.build` gains `'lisa'`; `shell/cc-panel-loader.c` gains the
`extern` decl and a `PANEL_TYPE("lisa", …)` row. (The exact edits are
verified against upstream 50.3.)

The System-page delta adds four more anchors, all guarded the same way —
the group in the `.blp`, the `show_software_updates_group` signature, the
`gtk_widget_set_visible(…)` call site, and the `sources` list in
`panels/system/meson.build`.

**These were verified by execution, not by reading.** The awk/sed were run
under GNU tooling against the real 50.3 sources, and then each anchor was
mutated (group renamed, function signature changed) to confirm the guard
goes red rather than silently producing a panel with no row.
`cc-system-updates-lisa.c` compiles clean under `-Wall -Wextra` against
real libadwaita headers, and that check was itself controlled by breaking
an `adw_action_row_set_subtitle` call and confirming the failure.

## Build & verify (Linux / CI / container — not macOS)

```sh
# In an Arch environment (CI runner, or `podman run --rm -it archlinux`):
cd os/packages/gnome-control-center-lisa
makepkg -s                      # builds gnome-control-center + -keybindings
# smoke: the panel registered and the desktop file is present
pacman -Qlp gnome-control-center-*.pkg.tar.zst | grep gnome-lisa-panel.desktop
```

`makepkg`'s `check()` runs upstream's test suite under xvfb. A green build
is the gate before this package reaches an image (ADR-0012).

## Wire into the image (next step, not yet enabled)

The image still pulls **stock** `gnome-control-center`. To switch:

1. CI job builds this package and publishes it to the Lisa pacman repo
   (`os/repo-tools`), layered **above** `[extra]` so it wins the stock
   package by repo priority (same name+version).
2. No change needed in `os/mkosi/mkosi.conf` (`gnome-control-center` stays
   in `Packages`) once the Lisa repo is in the image's `pacman.conf`
   ahead of `[extra]`.
3. Track L (`os/layer`) installs it the same way.

Until that CI job exists, this package is built and verified standalone;
it does not yet ship in a release. See ADR-0012 → Consequences.

## Maintenance

Re-pin on a GNOME bump: set `pkgver`, refresh `b2sums`/tarball as Arch
does, rebuild. If a guard in `prepare()` trips, the `wellbeing` anchor
moved upstream — point the three edits at a current neighbour panel and
rebuild. The panel C tracks the `CcPanel`/libadwaita API; a green
`makepkg` is the check.
