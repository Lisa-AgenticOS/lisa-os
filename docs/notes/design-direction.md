# Design direction — input for the M4 shell ADR

- **Date:** 2026-07-21 · owner preference, recorded ahead of M4
- Feeds §13's open question: GNOME patch-set vs. extension-only — and
  the Phase 3+ shell decision (PLAN §3).

## The preference

The project owner likes **elementary OS**: restrained typography, quiet
color, humane defaults, curated first-party apps with one visual voice.

## What that means technically

Pantheon is not a GNOME reskin — Gala is elementary's own window
manager on **Mutter's compositor library**, with their own panel/dock
and the Granite widget kit. That is the proven mid-path between
patching GNOME Shell and building a compositor from scratch, and it is
the documented escalation path if Lisa's identity outgrows the
patch-set approach.

Adopting Pantheon wholesale is the wrong trade for Lisa today: its
Wayland maturity lags, and Lisa's trust boundary is portal-shaped —
GNOME's portal maturity is why the plan picked it (§3).

## Direction

1. **Tokens first:** the Appendix E theme file is the single source of
   design language — shell CSS, GTK/libadwaita, Qt, and `lisa_ui`
   (GJS/GTK4 since ADR-0047) all read it. One voice across every surface.
2. **Identity where freedom is total:** first-party apps and `lisa_ui`
   carry the elementary-inspired look first — our widgets, our rules.
3. **Shell at M4:** extensions + a minimal patch-set on the pinned
   GNOME base; decide patch-set depth with real experience.
4. **Escalation path:** own shell on Mutter (the Pantheon pattern),
   only if the identity demands it — never a from-scratch compositor
   before the substrate is proven.
