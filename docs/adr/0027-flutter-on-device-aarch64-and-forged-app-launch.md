# ADR-0027: the Flutter lane on-device — aarch64 SDK, and how a forged app gets launched

- **Status:** accepted
- **Date:** 2026-07-26
- Relates: ADR-0004 (Flutter lane), ADR-0014 (lisa_ui), ADR-0016 (naming),
  ADR-0019 (home partition), ADR-0020 (app channel + launcher indirection),
  ADR-0021 (aarch64 lane), ADR-0023 (slim core, /var grows), ADR-0025
  (one agent loop / Skills), issue #37

## Context

`lisa forge --flutter` scaffolds and verifies a lisa_ui app, and
`lisa forge --setup` provisions a sha256-pinned Flutter SDK into
`/var/lib/lisa/flutter`. Two gaps kept the lane from being real on a
device:

1. **aarch64 refused outright.** The ARM image now carries the whole Lisa
   stack (ADR-0021, `aarch64-image.yml`), so ARM devices are real users —
   but `forge_setup` bailed with "no official Flutter Linux SDK exists for
   aarch64".
2. **A forged app could not be launched.** `flutter analyze` clean is not
   an app; there was no build step, no install location, and no `.desktop`
   entry, so nothing reached the app grid.

### What was actually checked (2026-07-26, from the dev host)

- `releases_linux.json` carries **`dart_sdk_arch: x64` and nothing else**
  across all 724 entries. `flutter_linux_arm64_3.44.7-stable.tar.xz` and
  `releases_linux_arm64.json` are both **404**. Google publishes no arm64
  Linux SDK tarball — the original refusal was correct.
- Arch proper has **no `flutter` package** (`archlinux.org` package search:
  0 results), so Arch Linux ARM has none to rebuild either; the AUR
  `flutter` package is a source build, not a pinnable artifact.
- The *artifacts* an arm64 SDK needs **are** published by Google, under the
  engine revision this release pins
  (`69c8c61792f04cc809dfef0c910414fb9afc06cd`, read from
  `bin/internal/engine.version` at the `3.44.7` tag). All HTTP 200:
  `dart-sdk-linux-arm64.zip`, `linux-arm64/artifacts.zip`,
  `linux-arm64/font-subset.zip`, and
  `linux-arm64-{debug,profile,release}/linux-arm64-flutter-gtk.zip`.
- `bin/internal/update_dart_sdk.sh` at that tag maps any non-x86_64,
  non-riscv64 `uname -m` to `ARCH=arm64` and fetches
  `dart-sdk-linux-arm64.zip` — the SDK bootstraps itself on ARM.
- `bin/internal/update_engine_version.sh` prefers the **tracked**
  `bin/internal/engine.version` file over any git triangulation, so a
  shallow checkout of a stable tag resolves the same engine as the tarball.
- The `3.44.7` git tag resolves to commit
  `84fc5cbb223bc12f83d65b647ff8a56caf779ffd`, which is **byte-identical to
  the `hash` field of the 3.44.7 entry in Google's releases manifest**.
- A `git clone --depth 1 --branch 3.44.7` was **run**: 217 MB, `rev-parse
  HEAD` is the pinned commit, `git describe --match '*.*.*' --first-parent
  --long --tags` (what flutter_tools uses for its version) resolves
  `3.44.7-0-g84fc5cbb` from the shallow history, and `bin/flutter
  --version` bootstrapped to *the same framework and engine revisions the
  tarball install reports* (846 MB after bootstrap). Done on the macOS dev
  host — the mechanism is proven, the arm64 artifact download is not.

## Decision

### 1. aarch64 installs the same release, pinned by commit instead of sha256

`flutter_install_plan(arch)` picks the route:

| arch | route | pin |
|---|---|---|
| `x86_64` | Google's release tarball | `sha256` from `releases_linux.json` |
| `aarch64` | `git clone --depth 1 --branch 3.44.7` of `flutter/flutter` | commit `84fc5cbb…`, verified with `git rev-parse HEAD` before the checkout is moved into place |
| anything else | refuse, naming the arch | — |

A git commit id is a hash over the whole tree, so this is a pin, not a
guess — and it is *the same id* Google publishes for this release, so the
two sources cross-check each other (CLAUDE.md rule 8 satisfied by
verifying, not by excluding). The checkout is staged under
`.flutter-staging` and renamed, exactly like the tarball path; a mismatched
`rev-parse` deletes the staging area and installs nothing.

After either route, the SDK is **bootstrapped in place** (after the
rename, so the absolute paths it caches are final) with
`flutter precache --linux`: on aarch64 that is what pulls the arm64 Dart
SDK and engine artifacts listed above. Failure is a warning, not an error —
an offline device still has a usable SDK and pays the download on first
build.

The trust surface is unchanged relative to x86_64: the tarball route also
downloads unpinned engine artifacts on first `flutter build linux`. What
differs is only how the SDK *itself* is pinned.

**Rejected:** a Lisa-built arm64 SDK tarball published from CI (a pin we
could only write after the first run, and one more artifact to maintain);
a distro/AUR Flutter (no pinnable binary artifact exists); staying refused
(the artifacts exist — refusing would be inaccurate, not conservative).

### 2. `lisa forge --build` / `--run`: source → an app in the grid

- **The Linux runner comes from the SDK's own template.** The forge
  scaffold does not hand-write `linux/`; `--build` runs
  `flutter create --platforms=linux --org app.lisaos.forge --project-name
  <pkg>` into a scratch directory and copies only `linux/` in, so an
  existing `lib/`, pubspec or test can never be clobbered. Verified: the
  generated `linux/CMakeLists.txt` sets `BINARY_NAME "<pkg>"` and
  `APPLICATION_ID "app.lisaos.forge.<pkg>"`, and
  `linux/runner/my_application.cc` calls `g_set_prgname(APPLICATION_ID)`.
- **One identity, three places.** `app.lisaos.forge.<pkg>` is at once the
  GTK application id, the WM class, and the `.desktop` basename — which is
  how GNOME ties a Wayland window to its launcher entry. `app.lisaos.*` is
  the app namespace per ADR-0016.
- **The Dart package is named after the project directory** (`tip-calc` →
  `tip_calc`) instead of every forged app being `lisa_app` — it becomes the
  binary name and the app id.
- **Build products live on /var** (ADR-0023): the bundle installs to
  `/var/lib/lisa/forge/apps/<app-id>/bundle`, staged then renamed, with the
  prior build kept as `bundle.previous` (delivery rule 2 — payloads carry
  their own rollback, and the boot-rollback guarantee is not extended to
  them). `tmpfiles.d/lisa-forge.conf` creates the tree `2775 root:lisa` so
  the desktop user forges without root. Where that directory does not exist
  (dev hosts), the bundle falls back to `~/.local/share/lisa/forge/apps` —
  also outside the image, on its own partition (ADR-0019).
- **The `.desktop` entry goes to `~/.local/share/applications`.** An
  immutable root cannot receive one at runtime, and ADR-0020's
  indirection does not apply: there is no baked copy of a forged app to
  prefer `/var` over. The stable `…/<app-id>/bundle/<pkg>` path *is* the
  indirection — a rebuild swaps the bundle underneath an unchanged entry.

### 3. Skills live in `skills/<name>/SKILL.md`

ADR-0025 phase 4 makes Skills the mechanism by which Lisa learns
workflows, and names *building a lisa_ui Flutter app* as the first shipped
skill. There was no location convention, so this ADR sets one:

- Repo: `skills/<name>/SKILL.md`, `name`/`description`/`tools`
  frontmatter, parsed by `harness-core::Skill`.
- Installed: `/usr/share/lisa/skills` (lisa-cli package).
- Resolution, first definition of a name winning:
  `$LISA_SKILLS_DIR` (`:`-separated) → `$XDG_DATA_HOME/lisa/skills` →
  `/usr/share/lisa/skills`.
- Surface today: `lisa skills list` (the catalog line — the cheap part a
  prompt carries) and `lisa skills show <name>` (the body, read on use).
  The harness loop's `load_skill` tool reads the same files through the
  same order when phase 4 lands.

## Consequences

- ARM devices can install the Flutter SDK, and get it from the same
  release x86_64 gets — one version to reason about, two pin mechanisms.
- `lisa forge --setup` now needs **git** on aarch64. An immutable root
  cannot `pacman -S` it after the fact, so `git` is named in `mkosi.conf`
  (~30 MB, paid twice across the A/B slots) and is an optdepend on the
  lisa-cli package for Track L.
- Forged apps appear in the app grid, survive rebuilds, and keep one
  rollback generation — without adding a byte to the image.
- **Decided (2026-07-26): the build toolchain is a /var payload, not image
  content.** `flutter build linux` needs clang (or gcc), cmake, ninja and
  pkg-config, and the immutable image ships none of them — you cannot
  `pacman -S` them on a Track I device. Rather than bake ~250 MB into the
  image (paid twice, once per A/B slot, on every device whether or not it
  ever builds an app), `lisa forge --setup` fetches a hash-pinned
  toolchain payload into `/var/lib/lisa/toolchain`, the way ADR-0021
  pinned mkosi from the permanent Arch archive and the way the Flutter SDK
  itself already installs. This follows ADR-0023 exactly: the image
  carries the OS contract, `/var` carries what the user grows — and a
  build toolchain is unambiguously the latter. Consequences accepted:
  building needs network once, the payload lives outside boot-rollback
  (it has its own re-fetch by pin), and packages pulled from the Arch
  archive must be pinned by exact version+hash, never "latest".
- gtk3 is already in the image transitively (xdg-desktop-portal-gtk), so a
  forged app *runs* on Track I once it has been built somewhere.
