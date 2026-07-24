# ADR-0012 — A native "Intelligence" panel in a forked gnome-control-center

Status: accepted (2026-07-23)
Relates to: PLAN §5.3 (Settings panel), §5.11 (providers), §8 (local
model fit); ADR-0008 (remoted); supersedes the standalone-only settings
plan for the AI surface.

## Context

The AI settings (local models, providers, offload consent, and — later —
default model / voice) should live **in the GNOME Settings sidebar**,
next to Wi-Fi / Network / Displays, not only in a separate window. That
is a deliberate product decision: intelligence is a *system* service in
Lisa, so it belongs in the system Settings.

The hard constraint: **gnome-control-center has no plugin API.** Its
panels are compiled into the binary and registered in a static table
(`shell/cc-panel-loader.c`); there is no runtime drop-in directory. The
only way to add a sidebar entry is to build our own gnome-control-center
with the panel included.

We already have the backends a panel needs: the `dev.lisaos.Remote1` D-Bus
broker (providers/keys/consent) and `lisa models catalog --json` (§8
hardware-aware local-model fit). The standalone GJS `app.lisaos.Settings`
app (shell/settings) already drives them and is unit-tested.

## Decision

Ship **our own `gnome-control-center` package** (`os/packages/
gnome-control-center-lisa`) that is stock upstream at a pinned version
(50.3, matching the image) **plus a minimal, additive delta**:

- a new panel `panels/lisa/` (id `lisa`, title *Intelligence*), and
- two anchored edits applied at build time (`prepare()`), not a fork of
  the whole tree:
  - `shell/cc-panel-loader.c`: one `extern` decl + one `PANEL_TYPE("lisa",
    cc_lisa_panel_get_type, NULL)` row;
  - `panels/meson.build`: add `'lisa'` to the `panels` list.

"Fork" here means *own the package with a small maintained patch*, not
diverge the source. The delta is a handful of lines; re-pinning to a new
GNOME release is: bump `pkgver`, re-run the build, fix the two anchors if
upstream moved them. Everything else is upstream, unmodified.

### Panel scope, staged

- **v1 (this ADR):** native **Local models** section — reads
  `lisa models catalog --json` via `GSubprocess`, renders each model with
  its §8 fit badge and a one-click **Get** for pinned models that fit
  (spawns `lisa models get`). Local inference never leaves the machine,
  so nothing here is egress-marked. Plus a **Providers & privacy** group
  that summarizes offload state and opens the existing `app.lisaos.Settings`
  app for the full provider/key/OAuth flow (reuse, don't reimplement the
  amber-egress UI in C yet).
- **v2:** providers/keys/consent move native into the panel (same
  `dev.lisaos.Remote1` calls the GJS app makes), and a **Lisa** group
  (default model, voice/wake-word) once those settings are
  daemon-readable. The GJS app stays as the reference/fallback.

The panel is written **programmatically** (no `.ui`/blueprint/gresource):
`CcPanel` derives `AdwNavigationPage`, so the content is an
`AdwToolbarView` + `AdwPreferencesPage` set with
`adw_navigation_page_set_child`. This keeps the delta to plain C with no
template-binding surface, which matters because the panel only compiles
in a full g-c-c build (CI/Linux), never on the macOS dev host.

Sidebar placement: `Categories=…X-GNOME-SystemSettings…` puts it in the
**System** group (intelligence-as-a-system-service). A dedicated
top-level category would need patching `CcPanelCategory` + the sidebar
list — deferred.

## Alternatives rejected

- **Standalone GJS app only** — already built, but not in the sidebar;
  the user's requirement is the sidebar entry.
- **Rebuild Settings in `lisa_ui` (Flutter)** — our own design system,
  testable on macOS, but it's an empty scaffold today and would be a
  separate app, not the GNOME sidebar. Revisit if Lisa ever ships its own
  Settings shell wholesale.
- **Runtime/external panel** — no such mechanism exists in g-c-c.

## Consequences

- We build and ship `gnome-control-center` from `os/packages`, replacing
  the stock Arch package in both tracks (mkosi `Packages` / Track L
  layer). The image gains a build step (Linux/CI only).
- Maintenance is bounded to the two anchored edits + the panel dir; the
  panel reuses existing daemons, so no backend duplication.
- Verification is Linux-only: the package builds in CI (and can be built
  in a container from the dev host via `makepkg`); it cannot be compiled
  on macOS. The panel is written against the fetched 50.3 API to be
  correct-by-construction, and gated on a green package build before it
  reaches an image.
