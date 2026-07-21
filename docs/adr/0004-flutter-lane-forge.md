# ADR-0004: Flutter app lane + the Forge

- **Status:** accepted (spike pending — see below)
- **Date:** 2026-07-20

## Context

The Forge (PLAN §5.12.1) ships a Claude Code-style harness in the OS:
"make me a…" → installed, sandboxed app, with the user watching. The
harness's iterate loop lives or dies on rebuild latency, and its system
prompt/template corpus must stay small. Native GTK/Qt compile times and
a two-framework corpus fail both tests.

## Decision

Two app lanes (PLAN §5.12):

- **Native lane:** GTK4/libadwaita + Qt via liblisa for shell, portals,
  Settings, and OS-depth apps (Files, Ledger).
- **Flutter lane:** default for user-facing apps, third-party apps, and
  everything the Forge generates. Sub-second stateful hot reload *is*
  the agent loop. `lisa_ui` builds on Flutter's core widget primitives
  (upstream froze Material/Cupertino into standalone packages, April
  2026 — the sanctioned path for custom design systems); no
  `material_ui` dependency anywhere in the lane. `lisa_flutter` mirrors
  liblisa over D-Bus with the OpenAI-compat endpoint as fallback.

Governance hedge: engine + framework pinned in our repo snapshot;
`lisa_ui` depends only on core primitives; community-fork contingency
documented here; the OS itself never depends on Flutter (native lane).

## Consequences

- Forged apps escape to Android/iOS/web/desktop — an adoption story no
  macOS framework offers.
- We own a design system and its maintenance; that is the price of the
  anti-"foreign toolkit" move (live system theming).
- **Spike required before M6 work begins (Appendix D):** pin engine
  version; confirm GTK embedder under Wayland; fcitx5 IM round-trip in a
  Flutter text field; D-Bus call to `inferenced` from Dart. Findings
  land as an appendix to this ADR.

## Appendix: spike findings (2026-07-21, macOS half)

**Pinned:** Flutter 3.44.7 stable (framework 84fc5cbb22, 2026-07-17;
engine 7076f47b1d1a). The repo snapshot pins this per the governance
hedge.

**Proven on the dev host:**

- `libs/lisa_ui` seed builds and tests on **core widgets only** — no
  material/cupertino import anywhere. Tokens (`LisaTokens`/`LisaTheme`,
  seeded from docs/notes/design-direction.md), `LisaStreamText`
  (token-stream accumulation, stop affordance, provenance footnotes),
  `ConsentChip` — all under widget tests.
- `libs/lisa_flutter` fallback transport is **live against
  lisa-inferenced**: streaming ask (SSE parse under unit test) and
  embeddings round-trip with zero package dependencies (`dart:io`).
  Guided generation rides the same call (`jsonSchema` parameter).

**Remaining (needs a Linux desktop; target: the iMac runner):** GTK
embedder under Wayland; fcitx5 IM round-trip in a Flutter text field;
the primary D-Bus transport via package:dbus against
`org.lisa.Inference1` — the server side of that surface already exists
and is tested (fd-passed token streams), so the Dart side is a client
exercise.
