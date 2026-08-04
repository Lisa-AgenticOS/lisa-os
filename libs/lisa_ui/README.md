# lisa_ui — parked: the Flutter widget kit, kept but not the lane

Spec: docs/PLAN.md §5.12. Milestone: M6. Governance: **ADR-0047** (GJS +
GTK4/Adwaita is the one toolkit), ADR-0014 (this kit, phase 1),
ADR-0004 (history).

**Status: parked** (ADR-0047 §2). Unshipped, unproven on hardware, not
the default. This directory is four `.dart` files that no user-facing
surface has ever imported, and #37 — shipping the Flutter SDK and this
kit on-device — is closed **won't-do** under ADR-0047 §3. Since
2026-08-04 the OS package installs no copy of it.

To build a Lisa app, do not use this. Every shipped app is GJS +
GTK4/Adwaita: see `docs/ANATOMY-OF-AN-APP.md`,
`skills/build-lisa-app/SKILL.md`, and `lisa dev check` (ADR-0050), which
is the authority on what a valid app is. `lisa forge` targets GJS.

Nothing here is deleted, and this file says why rather than describing a
lane that works — the history is intact and reversing the decision costs
four files. ADR-0047's "What would reverse this" states the evidence
that would justify it: a shipped app GTK4 genuinely cannot serve, a GJS
wall we cannot design around, or a third-party story that demands Dart.
None is true today.

## What it is, for whoever picks it up

One import gives a Flutter app the whole vocabulary:

```dart
import 'package:lisa_ui/lisa_ui.dart';
```

- **Material-backed (ADR-0014 phase 1):** a curated re-export of
  `package:flutter/material.dart` — app structure, navigation, buttons,
  inputs, lists, dialogs, feedback, theming.
- **Lisa theming:** `LisaApp` (a `MaterialApp` pre-wired to `lisaTheme`)
  derives light + dark schemes from the violet seed `Color(0xFF6D45C9)`,
  Material 3, Rubik as the default font. Rubik is not bundled and there
  is no google_fonts dependency — the family resolves against the
  OS-installed font and falls back to the platform default sans when
  absent.
- **Lisa widgets:** `LisaScaffold`, `LisaCard`, plus `LisaStreamText`
  and `ConsentChip`.
- **Tokens:** `LisaTokens`/`LisaTheme`, generated from
  `branding/tokens.json` (ADR-0038).

The widget tests here still run under a Flutter SDK. They are not in
`just test`, because the toolchain is not in the image and not in CI.

## The name is spoken for

ADR-0047 §6 gives **`libs/lisa_ui` to the shared GJS/GTK4 library** —
the same name, pointed at the toolkit we actually use, whose first job
is the Agent Bus edge that currently exists in triplicate (`mcp-protocol.js`
in mail, surfer and preview; issue #218 had to be found once and fixed
three times). That library is **not written yet**. When it lands, the
Dart files here move under `libs/lisa_flutter`, which is where the
parked lane lives.

## Limits

- Unproven on hardware: no Lisa device has ever run a Flutter app.
- No on-device runtime: #37 is closed won't-do, so there is no SDK on a
  reference machine unless someone runs `lisa forge --setup` themselves.
- No CI: nothing here is built or tested by `just ci`.
