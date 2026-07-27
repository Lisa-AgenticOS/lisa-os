# ADR-0035: The desktop is a prompt — a floating dock-prompt, no top bar

- Status: **proposed** (design only — no code yet)
- Date: 2026-07-27
- Source: hand wireframe, Flakerim, 2026-07-27, with three readings
  corrected by the author the same day (top centre, launcher, dock)
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
2. **A pill and a dot, centred at the very top** — GNOME's workspace
   switcher: the elongated pill is the current workspace, the dot the
   other one. It keeps its place, centred, exactly where GNOME 50 puts
   it.
3. **Status floats, top-right:** Wi-Fi, Bluetooth, `13:30`, and one
   circled glyph (system menu / avatar). No bar, no background, no
   full-width strip — four items sitting on the wallpaper.
4. **An empty desktop.** No icons, no widgets, nothing between the top
   edge and the bottom bar.
5. **One floating rounded bar, bottom-centre:** four round app icons, a
   fifth distinct glyph, then a wide rounded text field terminating in a
   right-pointing triangle.

The whole of the shell's persistent chrome is the corner wordmark, the
workspace switcher, four status indicators, and that one bar — and only
the last of those is new.

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

Not a dock with a search box bolted on. One bar: apps on the left, the
prompt filling the rest. This is the load-bearing claim of the sketch,
and it is a claim about what a desktop is for — launching a program and
asking for an outcome are the same gesture, so they get the same
surface.

The app half is **pinned plus running, merged**: the pinned set always
shown in its chosen order, a window from something unpinned appended,
and a dot marking what is live. The most familiar arrangement, and the
most behaviour to specify — overflow, grouping per application, and what
a click does on an app that is both pinned and already running all need
answers before this is built.

Concretely: typing a program name launches it, typing a sentence routes
through intent (ADR-0013) to a tool call or the model. **The launcher and
the prompt are one surface** — `shell/launcher` (§5.7.2) stops being a
separate centred window and becomes this bar's expanded state, growing
upward into results as you type. Super+Space and Super+Shift+Space both
land here.

That is one control where there are currently two chords and three
windows, and it is the reason the merge is worth the churn: a person
should not have to know in advance whether what they are about to type
is a program name or a sentence.

### 3. The top bar loses its background, not its layout

This is the cheap half, and it is worth being precise about why.

The sketch keeps GNOME's panel *structure* — three groups, left, centre,
right — and changes two things: what is in them, and the strip behind
them.

The contents move. Today the workspace switcher sits at the left edge
and the clock in the centre; the sketch puts the `LISA` wordmark at the
left, the workspace switcher in the centre the clock vacates, and the
clock on the right beside the quick settings, so that corner reads
wifi, bluetooth, time.

GNOME builds each box from role lists on `Main.sessionMode.panel`, so
that reorder is a change to those lists plus a rebuild — not a
reparenting of actors behind the Shell's back. One wrinkle worth
recording: a session-mode change (lock, unlock, switch user) re-syncs
those lists from the mode definition, so a reorder that is not
re-applied on `updated` is correct only until the first time the screen
locks.

The *bar* — the full-width opaque strip — is what the sketch removes,
leaving the three groups floating directly on the wallpaper.

So this is a restyle of the existing panel, not a re-hosting of its
contents. Quick settings stay GNOME's: Wi-Fi, Bluetooth, volume, the
power menu and all their popovers keep working, keep their
accessibility, and keep being somebody else's maintenance burden. The
only Lisa-side change at the left edge is the `LISA` wordmark taking
Activities' place.

An earlier draft of this ADR assumed the indicators would have to be
rebuilt outside the panel and priced that as the expensive part of the
redesign. Reading the workspace switcher correctly removes that cost
entirely. Rebuilding four indicators is how a shell project loses a
year, and this design never asked us to.

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

- **Merging the launcher into the bar is the largest single change.**
  `shell/launcher` is a working surface with its own tests; folding it in
  means the bar inherits result ranking, keyboard navigation and every
  search provider, and the separate window goes away rather than
  lingering as a second path to the same thing.
- **A permanent text entry must never steal focus.** It has to be
  typable on click and invisible to keyboard navigation until asked for,
  or it breaks every keyboard workflow on the machine.
- **A permanent bottom bar takes vertical space** on a laptop-class
  display. Auto-hide or reveal-on-approach is a real question and this
  ADR does not settle it.
- **Fullscreen and games** must reclaim the whole screen. Floating chrome
  that survives a fullscreen window is a bug, not a feature.

## What this ADR does not decide

The sketch is one page and does not answer these; they are recorded here
so the next revision has a list rather than a re-derivation:

1. **The fifth glyph** in the bar, distinct from the round app icons —
   the Lisa spark, a separator, or the overview button. Assumed to be
   the Lisa entry (the button that expands the bar into the Assistant
   window), because that is the one thing the bar needs that the
   wireframe does not otherwise place.
2. **The circled glyph, top-right** — system menu, avatar, or Lisa's
   status. Assumed GNOME's system menu, since the wordmark already
   carries Lisa's identity at the other end.
3. **Dock behaviour once pinned and running are merged:** overflow when
   many windows are open, whether windows group per application, and
   what a click does on an app that is both pinned and already running.
4. **Auto-hide**, multi-monitor placement, and what the bar shows while a
   response is streaming.
5. **Where the confirmation dialog lives** (§4) — the one question that
   is a security decision rather than a visual one.

## Status of the work

Nothing is implemented. Before any of it is, the ADR needs the answers
above — and the last of them, where the consent dialog lives, is the only
one that is not a matter of taste.
