# ADR-0062 — one summon surface: the typed ask lives in Spotlight

- **Status:** accepted
- **Date:** 2026-08-07
- **Owner decision:** seated on v20260807.85, screenshot in hand —
  the overlay popup, the Assistant window, the dock's Ask bar and the
  overview search all on screen at once: *"I think Overlay should be
  merged with Spotlight."*
- **Supersedes in part:** ADR-0015 (the assistant app) — its split of
  "quick overlay for one-shots, window for conversations" retires on
  the typed side; the overlay popup is no longer a typing surface.
- **Claims:**
  - `path:shell/overlay-extension/extension.js` — the summon rerouting lives here
  - `path:shell/launcher/extension.js` — the Ask-Lisa row this converges with (#210)

## Context

Four prompt surfaces had accreted: the overlay's dark popup
(keyboard-summoned), the Assistant window, the dock bar's Ask-Lisa
lane, and the overview search those last two live in. The launcher had
already stopped routing through the popup (#210: a one-shot answer box
with nowhere to type a reply hands off to the conversation instead) —
so the popup's typed path was a second, poorer route to the same
place, and pressing the summon key while using the bar produced two
surfaces at once.

The overlay ARCHITECTURE was never the problem: one headless backend,
many thin frontends, deliberately. This merges frontends.

## Decision

Two mouths, each with one job:

- **Spotlight = ask now.** `Super+Shift+Space` opens the overview with
  search focused — the same landing the dock bar gives. Typing reaches
  the Ask-Lisa row; activating it opens the conversation.
- **The Assistant = the conversation.** Every typed ask, from any
  route, lands there (#210's contract, now universal).

The popup does not die; it narrows to the two jobs nothing else can
do yet:

- **Consent.** `ConfirmationNeeded` still renders in the popup — an
  agent confirmation must never be silently unroutable.
- **Voice.** `Summon` with `listen: true` (double-tap-Shift) keeps the
  popup's mic OSD and streaming box; Spotlight has neither. Folding
  voice into Spotlight is future work, not this decision.

`Overlay1.UI.Summon` keeps its name and callers: a non-empty prompt
hands off to the Assistant, an empty one opens Spotlight, `listen`
keeps the voice flow. The backend (`dev.lisaos.Overlay1`) is untouched.

## Consequences

- The IME's double-tap-Shift and any external `Summon` caller keep
  working with no interface change.
- The popup's chips (`[this window]`, `[selection]`, `[my stuff]`)
  now only appear on voice summons; their typed-path future belongs to
  the Assistant's composer (the `my_stuff` affordance is already
  planned there for the ambient producer).
- ADR-0015's overlay-as-quick-surface text stays as history; its
  keybinding table is superseded by this behaviour.

## Limits

- Voice still needs the popup; "one surface" is true for typing only
  until Spotlight grows a listening affordance.
- The overview search entry is GNOME's; prefilling it with a summoned
  prompt is not implemented — an empty summon lands you typing, which
  is the intent.
