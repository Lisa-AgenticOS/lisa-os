# ADR-0060 — the app bundle, `lisa.framework` and `lisa.sdk`

- **Status:** accepted, not implemented — this record fixes the shape
  before the code exists, because the alternative is seven more
  surfaces hand-rolling the same proxies while the shape stays
  folklore.
- **Date:** 2026-08-06
- **Owner direction:** "work on lisa.framework, on lisa ui … this is
  how we make all apps, bake it in ui library" (2026-08-06).
- **Supersedes in part:** ADR-0050's payload layout (the flat
  `<app>/` + `lisa_ui/` tree stays as the *interim*, not the end
  state); the location amendment in ADR-0056 gains its exit criterion.
- **Claims:**
  - `path:apps/notes/icon.svg` — the first bundle-shaped fact: the icon lives inside the app, and the `.desktop` points at it by absolute path
  - `path:apps/lisa.sdk/ui/window.js` — the sdk's UI half growing by extraction
  - `absent:libs/lisa-framework` — the runtime bundle does not exist yet
  - `absent:libs/lisa-sdk` — the GJS binding does not exist yet

## Context

Every Lisa app today is scattered across the filesystem: its GJS source
in the apps payload, its icon (all but Notes') in hicolor, its
`.desktop` in `/usr/share/applications`, its manifest in
`/usr/share/lisa/manifests`, its daemon in `/usr/bin`, its D-Bus proxies
hand-rolled inline. What an app *is* has no place; it is a convention
smeared over six directories, and conventions are things you can only
half-follow.

The D-Bus half is worse than scattered — it is **repeated**. The seven
system interfaces are hand-declared roughly sixty times across the GJS
surfaces (counted 2026-08-06: Overlay1 ×23, Context1 ×10, Harness1 ×8,
Agent1 ×8, Inference1 ×6, Voice1 ×6, Remote1 ×6). Sixty copies of an
interface declaration is sixty places a signature change becomes a
runtime error nobody's build caught. #218 — one dispatcher bug fixed
three times because it existed in three copies — is the recorded cost
of exactly this pattern at smaller scale.

### What macOS actually does (measured, not assumed)

The bundle research was run on a real Mac on 2026-08-06, because rule 8
forbids building on folklore:

- **The bundle is the truth; the index is a cache.** LaunchServices is
  a rebuildable per-user database seeded by scanning directories. Throw
  it away and the system re-learns from the bundles.
- **`Versions/Current` is not how anything resolves.** Of 1983 recorded
  framework load paths inspected, zero route through `Current`. The
  indirection only matters if the recorded path uses it; almost nothing
  does (Chrome is the one real user found).
- **Apps do not link shared frameworks off disk.** 339 of 341 system
  framework binaries are not on disk at all — they live in the dyld
  shared cache. "Apps symlink to the shared framework" is not what
  macOS ships.
- **Signing seals names, not versions.** The version directory name and
  `Current`'s target are not sealed; symlinks are sealed by target
  string. Of 9109 sealed symlinks inspected, zero point outside their
  own bundle.

The lesson is not "copy the mechanism"; the mechanisms mostly do not
survive contact with measurement. The lesson is the *invariant*: **one
directory is authoritative for what an app is, and everything else is
derived and rebuildable.**

### What GJS makes easy that Mach-O makes hard

Interpreted source has no install-name problem. Verified on the
reference device:

- a bare absolute import specifier **fails**; `file://` works;
- a bundle-relative import **through a symlink** works, and yields ONE
  module instance across bundles — so a shared library symlinked into
  each bundle is genuinely shared state, not N copies;
- an absolute-path `Icon=` in a `.desktop` resolves to a `Gio.FileIcon`
  — `apps/notes/icon.svg` ships this way today.

## Decision

Three named things, with a hard rule about what they are *not*:

**1. The app bundle.** An app is ONE directory:

    Notes.app/
      app.lisaos.Notes.desktop     ← Exec + absolute-path Icon into the bundle
      app.lisaos.notes.json        ← the manifest the Agent Bus enforces
      icon.svg
      lisa-notes-app.js            ← the window (GJS, interpreted)
      lib/                         ← the app's own modules
      lisa.sdk → (shared)          ← symlink; resolves to one shared instance

  Everything outside the bundle — the applications directory entry, the
  manifest agentd reads, any launcher index — is **derived from the
  bundle by a `lisa` verb**, never authored separately. The macOS
  invariant, kept; the macOS machinery, not copied. Compiled daemons
  (Notes' Rust storage daemon) stay packaged system-side: a bundle is
  the *interpreted* app, per ADR-0020/0047 — that is what makes an app
  update a file copy.

**2. `lisa.framework`** — the runtime a *device* carries: the D-Bus
  interface definitions (generated, one copy), the peer-identity and
  provenance plumbing, the token sheets. System-scope, ships with the
  OS, lives on `/var`-or-image per ADR-0034. Rust core remains
  `libs/liblisa`.

**3. `lisa.sdk`** — the GJS binding *apps* import: today's
  `apps/lisa_ui` (mcp edge, client, window shapes, style) plus
  generated proxies for the seven interfaces, so the sixty hand-rolled
  declarations become zero.

**The rule that outranks the names:** framework and sdk are **bindings
over the same D-Bus interfaces — never two implementations of
policy.** Guard decisions, tier enforcement, provenance: daemon-side
(rule 6a), reachable from either binding, implemented once.

## Consequences

- ADR-0056's "where it actually lives" question gets its answer: the
  sdk is symlinked into each bundle, so the relative-import constraint
  that pinned `lisa_ui` beside the apps dissolves — the exit criterion
  the amendment asked for.
- The payload staging in `build-apps-payload.sh` becomes the *bundle
  builder*, and `cli/lisa/tests/apps_payload.rs` becomes the gate that
  every derived artifact (desktop entry, manifest copy) matches its
  bundle.
- Sequencing (capability before storefront, ADR-0055 discipline): the
  proxies are generated **first** (they delete the sixty copies), the
  bundle layout second, the index verb last. No store, no discovery UI,
  until the manifest is enforced end-to-end.

## Limits

- Nothing here is built. This ADR exists so the next surface is not
  built the old way while the shape lives in one person's head.
- The signing story (sealing a bundle the way macOS seals by name) is
  deliberately out of scope until the `[lisa]` index signing work
  (#270) settles — one signing design at a time.
