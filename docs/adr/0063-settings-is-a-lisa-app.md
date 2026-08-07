# ADR-0063 — Settings is a Lisa app; the g-c-c fork retires

- **Status:** accepted, not implemented
- **Date:** 2026-08-07
- **Owner decision:** "we should convert settings to Lisa UI" — made
  seated, while asking for a glass toggle and being told where it
  would have to live (a C panel in the fork).
- **Supersedes in part:** ADR-0012 v2 (the Intelligence panel inside a
  forked gnome-control-center) — the destination changes; the panel's
  content and its OAuth flows carry over.
- **Claims:**
  - `path:shell/settings` — the seed: the pre-panel standalone Settings app, kept as reference + tests when ADR-0012 v2 merged it into the fork
  - `path:apps/lisa.sdk/ui/window.js` — the chrome it converts onto

## Context

The Settings fork is the one place Lisa still patches GNOME's own app
— the exact thing rule 11 forbids everywhere else — and it keeps
collecting the bill: the 50.3→50.4 rebase consumed a day (#284, nine
anchor guards, a b2sum chase, four failed ports dispatches), the
`replaces=` packaging race shipped broken once, and every GNOME
release re-opens the ledger. Meanwhile the features Lisa actually
needs in Settings keep growing: providers and OAuth, Policies (#253),
the glass toggle (#168), knobs the fork makes expensive and a Lisa
app makes trivial.

The trigger was small and telling: a one-switch appearance toggle
would have cost a C panel in a forked codebase rebased against
upstream forever — or one GJS row in an app we own.

## Decision

1. **Lisa Settings is a first-party GJS app on `lisa.sdk`** — glass
   sidebar, slate ground, the shared chrome, by default like every
   other Lisa surface. Its scope is everything *Lisa-specific*:
   Intelligence (models, providers, sign-in), Policies, Appearance
   (including the glass toggle), agent and device knobs.
2. **Stock gnome-control-center ships unpatched** for what GNOME does
   well — network, displays, accessibility, hardware. Rule 11's honest
   interim, now applied to Settings itself.
3. **The fork retires** once the GJS app covers the Intelligence
   panel's function: `lisa-desktop-control-center` and
   `-keybindings` leave ports.lock and the image; the rebase ledger
   closes. The Google OAuth work (#276) redirects to the new surface.

## Consequences

- `shell/settings` revives as the seed rather than starting empty; it
  predates the panel and carries tests.
- Two settings entry points exist during the interim (Lisa Settings
  and stock GNOME Settings) — acceptable because their scopes do not
  overlap; the fork's Intelligence panel is the thing that must not
  ship alongside its replacement.
- The keybindings fork question is folded in: if its only reason was
  riding the g-c-c fork, it retires with it; if not, it gets its own
  justification.

## Limits

- Nothing is built yet; this records the decision so the glass toggle
  and every future knob land in the right place the first time.
- Sequencing: after the current queue (#169), unless the owner pulls
  it forward.
