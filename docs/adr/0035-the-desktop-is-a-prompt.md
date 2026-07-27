# ADR-0035: The desktop is a prompt — a floating dock-prompt, no top bar

- Status: **proposed** (design only — no code yet)
- Date: 2026-07-27
- Source: hand wireframe, Flakerim, 2026-07-27
- Relates: ADR-0013 (intent routing), ADR-0020 (app channel — how this
  ships without an image), ADR-0030 (the guardrail boundary), ADR-0033
  (identity from the transport), PLAN §5.7 (Shell surfaces)
- Supersedes nothing; changes the *primary* surface established in
  PLAN §5.7.1

## Context

Lisa's shell today is stock GNOME 50 plus additions: a full-width top
bar with Activities and quick settings, the Dash inside the overview,
and three Lisa surfaces reached by keyboard — the assistant overlay
(Super+Shift+Space), the launcher (Super+Space), and the persistent
Assistant window.

That arrangement makes intelligence a *mode you enter*. You press a
chord, a layer appears, you type, it goes away. Every AI-native claim in
`VISION.md` is behind a keyboard shortcut that a new user does not know
exists, and the desktop they do see is indistinguishable from GNOME.

The wireframe proposes the opposite arrangement.

## What the sketch shows

Read literally, in the order it reads on the page:

1. **`LISA`** — plain wordmark, top-left corner. No panel behind it; it
   sits directly on the wallpaper.
2. **A pill and a dot, centred at the very top.** Read as the display's
   camera cutout — the target hardware (iMac) has a centred camera — and
   therefore as an instruction: chrome stays clear of the top centre.
3. **Status floats, top-right:** Wi-Fi, Bluetooth, `13:30`, and one
   circled glyph (system menu / avatar). No bar, no background, no
   full-width strip — four items sitting on the wallpaper.
4. **An empty desktop.** No icons, no widgets, nothing between the top
   edge and the bottom bar.
5. **One floating rounded bar, bottom-centre:** four round app icons, a
   fifth distinct glyph, then a wide rounded text field terminating in a
   right-pointing triangle.

The whole of the shell's persistent chrome is the corner wordmark, four
floating indicators, and that one bar.

## Decision

### 1. The prompt is chrome, not a mode

The text field in the bottom bar is a permanent, always-visible part of
the desktop. It is where you talk to Lisa, and it needs no chord, no
discovery and no prior knowledge — the way a browser's address bar needs
none.

Super+Shift+Space stays as the keyboard path to the same entry, and the
transient overlay stays for the case it is actually good at: acting on
*this window* or *this selection* without moving the pointer. What
changes is which one is primary. Today the chord is the only door; after
this, it is the shortcut to a door that is always visible.

### 2. The dock and the prompt are one object

Not a dock with a search box bolted on. One bar: pinned apps on the left,
the prompt filling the rest. This is the load-bearing claim of the
sketch, and it is a claim about what a desktop is for — launching a
program and asking for an outcome are the same gesture, so they get the
same surface.

Concretely: typing a program name launches it, typing a sentence routes
through intent (ADR-0013) to a tool call or the model. That is the
launcher's existing job (`shell/launcher`, §5.7.2) and the overlay's
existing job (§5.7.1) reaching the user through one control instead of
two chords.

### 3. The top bar dissolves into floating indicators

No full-width panel. Status becomes four items floating at the top-right
over the wallpaper, and Activities becomes the `LISA` wordmark at the
top-left.

The top centre stays empty, permanently, because on this hardware there
is a camera there.

### 4. What must NOT follow from this

The bar is a **human input surface**, and the guardrail rules apply to it
unchanged (ADR-0030): the prompt field belongs to the person, so nothing
in it is a guardrail and nothing about it may be reachable by the model.

This has one sharp consequence worth stating now rather than
rediscovering later. Issue #135 was closed by making the desktop consent
surface — and not the requesting peer — the peer that may approve a
destructive call, with the surface identified by the broker's owner of
`dev.lisaos.Overlay1`. That fix has a residual gap already recorded: the
overlay backend both *hosts the model* and *raises the confirmation
dialog*, so for calls it originates itself, requester and approver are
one process.

**A dock that owns the prompt must not also own the confirmation
dialog.** If it does, this redesign makes that gap permanent and puts it
in the centre of the screen. The consent dialog belongs to a separate
peer — the pattern `xdg-desktop-portal` already uses, and the reason it
uses it.

## Consequences

### Cheap

- The dock, the prompt and the wordmark are new surfaces we own outright.
  They are GJS, so they ship through the app channel (ADR-0020) —
  `lisa apps update` in seconds, no image rebuild, which is what makes
  iterating on a design like this on real hardware affordable.
- Hiding the top panel's background and moving Activities is extension
  work, not a fork.

### Not cheap, and worth naming

- **Quick settings are GNOME's, and they live in the panel.** Wi-Fi,
  Bluetooth, volume, the power menu and their popovers are panel
  children. Re-hosting them means either reparenting existing indicators
  (keeps GNOME's popovers, working network and Bluetooth UI, and
  accessibility — and inherits their layout constraints) or rebuilding
  them (total control, and we own every regression in a subsystem we did
  not write). Reparent first. Rebuilding four indicators is how a shell
  project loses a year.
- **A permanent text entry must never steal focus.** It has to be
  typable on click and invisible to keyboard navigation until asked for,
  or it breaks every keyboard workflow on the machine.
- **A permanent bottom bar takes vertical space** on a laptop-class
  display. Auto-hide or reveal-on-approach is a real question and this
  ADR does not settle it.
- **Fullscreen and games** must reclaim the whole screen. Floating chrome
  that survives a fullscreen window is a bug, not a feature.
- The launcher (§5.7.2) and this prompt overlap enough that keeping both
  needs a reason. Most likely the launcher becomes the bar's expanded
  state rather than a separate surface.

## What this ADR does not decide

The sketch is one page and does not answer these; they are recorded here
so the next revision has a list rather than a re-derivation:

1. **The fifth glyph.** Distinct from the four round app icons — the Lisa
   spark, a separator, or the overview button. Assumed to be the Lisa
   entry (the button that expands the bar into the Assistant window),
   because that is the one thing the bar needs that the wireframe does
   not otherwise place.
2. **Are the four round icons pinned apps or running apps?** Read as
   pinned; a running-window list would need overflow behaviour the sketch
   does not show.
3. **The circled glyph, top-right** — system menu, avatar, or the Lisa
   status. Assumed system menu, since the wordmark already carries Lisa's
   identity.
4. **The pill+dot at top centre.** Read as the camera cutout. It could
   instead be a compact notification pill, which would be a substantive
   addition rather than a hardware accommodation. If it is a notch, it is
   a constraint; if it is a pill, it is a fifth surface.
5. **Auto-hide behaviour**, multi-monitor placement, and what the bar does
   while a response is streaming.

## Status of the work

Nothing is implemented. Before any of it is, the ADR needs the five
answers above and a decision on where the consent dialog lives (§4), and
those are design questions, not code.
