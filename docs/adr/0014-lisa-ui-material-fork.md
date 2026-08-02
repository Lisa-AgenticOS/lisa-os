# ADR-0014: lisa_ui becomes the kit Lisa apps import — Material-backed now, vendored fork later

- **Status:** accepted, implemented (corrected 2026-08-02). The ADR's
  own premise — "today's lisa_ui is a 229-line, widgets-only seed" — is
  no longer true: it is ~1100 lines, `lisa forge --flutter` scaffolds
  against it, and the tree installs to /usr/share/lisa/lisa_ui on device.
- **Date:** 2026-07-24

## Context

The directive (project owner, 2026-07-24): the coding harness
(`lisa forge`) must be able to generate **real Flutter apps** using
`lisa_ui`. Today's `lisa_ui` is a 229-line, widgets-only seed
(`LisaTokens`, `LisaTheme`, `LisaStreamText`, `ConsentChip`) under
ADR-0004's rule of *no `material_ui`/`cupertino_ui` dependency anywhere
in the lane*. That seed proves tokens and the AI-native widgets, but it
is too thin to build real apps: no buttons, no scaffolding, no
navigation, no inputs.

Meanwhile Flutter is **removing Material/Cupertino from the framework
core into standalone packages**. A first-party kit that *is* the API
apps import — with the backend swappable underneath — is the
forward-aligned move: when the decoupling lands, a design system that
already owns its import surface becomes a fork of the package, not a
rewrite of every app.

## Decision

`lisa_ui` becomes **the API Lisa apps import** — one line:

```dart
import 'package:lisa_ui/lisa_ui.dart';
```

This **supersedes ADR-0004's no-Material rule for `lisa_ui`**.
ADR-0004 itself stays on the record: its Flutter-lane decision and its
spike findings are untouched history, and its core-widgets-only rule is
what the fork path below replaces.

**Phase 1 (this change) — wrap, don't vendor.** `lisa_ui` depends on
`package:flutter/material.dart` and re-exports a curated slice of it
(app structure, navigation, buttons, inputs, lists, dialogs, feedback,
theming primitives) alongside the Lisa widgets Material doesn't have
(`LisaStreamText`, `ConsentChip`, unchanged). Theming is ours from the
first pixel: `lisaTheme` derives light + dark schemes from the violet
seed `Color(0xFF6D45C9)` via `ColorScheme.fromSeed`, Material 3, Rubik
as the default font family — deliberately *not* bundled and no
google_fonts dependency, so the family resolves against the
OS-installed font and falls back to the platform default sans.
`LisaApp` (a `MaterialApp` pre-wired to `lisaTheme`), `LisaScaffold`,
and `LisaCard` give generated apps sane defaults, and `LisaTokens` /
`LisaTheme` keep working for the existing token consumers and tests.
The forge-harness system prompt now instructs generated apps to import
`package:lisa_ui/lisa_ui.dart` and build on `LisaApp` / `LisaScaffold`.

**Phase 2 (later) — the real fork.** When Flutter's Material/Cupertino
decoupling lands stable, vendor the Material package source into
`libs/lisa_ui` (a true fork), re-theme it at the source level, and drop
the `package:flutter/material.dart` re-export. Because apps only ever
imported `lisa_ui`, the swap is **no app-facing API change** — that is
the entire point of owning the import surface in phase 1.

## Consequences

- The Forge can generate real apps today: the full Material vocabulary
  is available through the one import the harness emits.
- We inherit no fork-maintenance burden in phase 1; in phase 2 we
  inherit Material's source and its upstream-merge cost — budgeted then,
  not now.
- Apps compiled against phase 1 observe only `lisa_ui` symbols; the
  phase-2 backend swap is invisible to them by construction.
- Rubik rendering depends on the OS font being installed; where absent,
  text falls back to the platform default sans (acceptable — the OS
  ships Rubik).
- `LisaTokens`/`LisaTheme` remain the token source; `lisaTheme` maps
  them into component shapes (card/dialog/input radius). The Appendix E
  live theme file still replaces `LisaTokens.fallback` later, unchanged
  by this ADR.

## Alternatives considered

- **Vendor Material's source today.** A real fork immediately, but we
  would carry merge/maintenance cost against a framework whose
  decoupling hasn't settled — paying the fork price before the upstream
  layout stabilizes, for zero app-facing benefit. Rejected for phase 1;
  it *is* phase 2.
- **Stay widgets-only and grow bespoke widgets.** Keeps ADR-0004's rule
  pure, but re-implementing buttons, menus, dialogs, and navigation
  badly and slowly fails the Forge directive — generated apps need a
  real vocabulary now.
- **Let apps depend on Material directly, no re-export.** Smallest
  change, but then the API surface is Material's, not ours: the phase-2
  fork would rewrite every generated app instead of one library.
  Rejected.
