# shell/desktop — the always-visible dock and the bottom-right corner

Spec: `docs/adr/0035-the-desktop-is-a-prompt.md`, `docs/PLAN.md` §5.7.
GNOME Shell extension, uuid `lisa-desktop@lisa-os.org`.

## What it does

Two changes to the stock GNOME desktop, both from the ADR-0035
wireframe:

1. **The dock is always visible**, macOS-style, instead of appearing
   only inside the overview.
2. **The hot corner moves to the bottom-right.** The top-left is where
   the `LISA` wordmark goes, and a hot corner underneath something you
   click is a trap.
3. **The top bar is reordered** to the sketch: the `LISA` wordmark at the
   left, the workspace switcher moved into the centre the clock vacates,
   and the clock moved right to sit with the quick settings.
4. **The dock carries a prompt** (ADR-0035 §2): a permanent text field
   filling the rest of the bar. Type a program name and it launches;
   type anything else and it goes to the assistant.
5. **Dock icons carry state** (#190): an app publishes a count over the
   Unity LauncherEntry convention and the dock draws a badge.
6. **The wordmark opens a menu** — **About Lisa OS**, the Lisa apps, and
   the session actions, including the **Log Out** GNOME hides on a
   single-user autologin machine (#139).

   *About Lisa OS* spawns `gnome-control-center system` rather than
   activating the Settings `.desktop`, because a plain activation reopens
   Settings on whatever page it last showed and the page is the entire
   point of the entry. The menu used to print `PRETTY_NAME` +
   `IMAGE_VERSION` inline and do nothing when clicked; Settings → System
   now carries the version next to a Check for Updates button, so the
   menu had become a second, dumber copy of a live surface.

What is **not** built of ADR-0035 §2: the launcher merge. §2 says
`shell/launcher` stops being a separate window and becomes this bar's
expanded state, "growing upward into results as you type", with
Super+Space and Super+Shift+Space both landing here. None of that is
here. The bar takes one line of text and routes it; it shows no
results, ranks nothing, and neither chord is bound to it. Super+Space
still opens the launcher (#255) and Super+Shift+Space still opens the
overlay.

## How it works

### The dock is GNOME's own Dash, moved

`extension.js` instantiates `Dash` from
`resource:///org/gnome/shell/ui/dash.js` and puts it in a floating
rounded panel added to `Main.layoutManager.addChrome()`.

Reusing the Dash rather than reimplementing it is the whole design.
Pinned-plus-running merged, running dots, click-to-focus, drag
reordering and app context menus are exactly what ADR-0035 §2 asks for,
and they already work. A hand-rolled dock would mean owning every one of
those regressions in a widget we did not write.

**There is one dock, and it is ours.** GNOME's own dash is hidden
(`Main.overview.dash.hide()`, re-shown on `disable()`), and ours stays
visible in the overview too — on macOS the Dock does not vanish when you
open Mission Control.

The first attempt did the opposite: ours hid inside the overview so
GNOME's could take over. That produced **two docks a few pixels apart**,
because `LayoutManager` owns the `visible` property of chrome registered
with `trackFullscreen` and rewrites it on every relayout:

```js
actor.visible = !(global.window_group.visible && monitor && monitor.inFullscreen)
```

In the overview `global.window_group.visible` is false, so that
expression is `true` and the dock was forcibly re-shown. Calling
`hide()` harder was never going to work; hiding GNOME's dash removes the
conflict instead of fighting it.

**The dock carries no styling of its own.** The Dash brings GNOME's
`.dash-background` with it, so it looks exactly like the dash it
replaces. An earlier version drew a second rounded panel around it,
which read as two docks nested inside each other and would have drifted
out of step with the theme the moment GNOME restyled the dash.

**Placement is deferred to before-redraw.** `_reposition()` is called
from `notify::width`, `notify::height` and `icon-size-changed`, all of
which fire *during* the layout pass, and moving an actor from inside its
own allocation made the shell log `Can't update stage views actor …
LisaDock … needs an allocation` — the cosmetic warning #262 recorded
and nobody attributed. A `BEFORE_REDRAW` later also coalesces the
several signals one dash rebuild emits into the single placement they
all want. Measured on gnome-shell 50.3, same harness, same provoked
rebuilds: **33 warnings before, 0 after**, and 33 on the pre-change dock
too — it was never the prompt's doing.

**The show-apps button has to be wired by hand, and the press is an
event — not a latch (#262).** GNOME connects its dash's button from the
overview's own controls, so a `Dash` used outside the overview has a
button that does nothing when clicked. Ours hangs off `clicked` and
decides from where the overview already is; `showAppsAction` in
`lib/layout.js` holds the decision.

The first version hung off `notify::checked` and passed that latch in as
the intent, which was wrong twice over:

- **`checked` is shared display state.** GNOME's `ControlsManager`
  writes it, the ctrl+alt+tab focus callbacks write it, our own `hidden`
  handler writes it. Coupling it to an action meant *anyone* writing the
  property performed one — setting `checked = true` by hand opened the
  overview, which is not what an assignment to a display property should
  do.
- **Drift cost a press.** Latched over a closed overview, the next click
  was spent unlatching it: press, tooltip, silence, no error. Reachable,
  because `Overview.hide()` has three paths that return *before*
  emitting `hidden` (the ctrl+click guard, `_animateNotVisible`'s
  `!this._visible` return, and `_syncGrab`'s `is_grabbed()` bail) while
  that `hidden` handler was the only thing that unlatched the button.

`checked` is now synced *from* the overview for appearance only, and
nothing reads it back. A missed sync costs a lit pixel, never a press.

### The prompt is an input surface and nothing else

The entry sits to the right of the Dash in the same panel — one bar,
which is ADR-0035 §2's actual claim ("Not a dock with a search box
bolted on"). `lib/prompt.js` decides what a submission means, with no
GNOME imports, and `tests/prompt.test.js` covers it.

**Where a submission goes.** An exact match on an installed app's name
or its desktop-id stem launches that app; everything else is handed to
`dev.lisaos.Overlay1.UI.Summon`, which opens the assistant overlay with
the query already submitted. Prefixes deliberately do **not** launch:
until the bar grows a result list there is nothing on screen to
disambiguate against, and opening a window the person never named is a
worse failure than a sentence reaching the assistant, which they can
read and ignore.

**The dock never sees the answer.** It sends a string over D-Bus and
forgets — no `dev.lisaos.Overlay1` client, no query id, no token
stream, and no confirmation dialog. That last one is ADR-0035 §4, which
says a dock owning the prompt must not also own consent; a surface that
never receives a reply cannot grow one by accident. It is the third
thin frontend on one headless backend (PLAN §5.7.1), not a second
implementation of it.

**Focus.** ADR-0035: *"A permanent text entry must never steal focus."*
The only route in is a press that lands in the field. The dock is not
registered with `Main.ctrlAltTabManager`, so the shell's focus chain
never reaches it, and nothing focuses on hover. Escape is two-stage —
it clears text first, and only an already-empty field hands the
keyboard back.

**A modal grab, and why.** An `St.Entry` in chrome receives keystrokes
only while no window has focus; Mutter routes the keyboard to the
focused window otherwise. So a press takes `Main.pushModal` on the dock
(the same mechanism, for the same reason, as the assistant overlay) and
submit/Escape/click-outside release it. Two consequences worth knowing:
a click outside the dock is *consumed* to release the grab rather than
delivered to what is under it, and the caret is dropped in an idle,
because setting the key focus from inside the entry's own key handler
does not stick.

The last two paragraphs are not analysis — they are what
`tests/dock-prompt-smoke.js` measured. Before it ran, the press handler
was on `button-press-event`, which the entry's own `ClutterText`
consumes for caret placement, so **no grab was ever taken**. In a
headless shell with no windows the field still took text (nothing else
had focus) and every visible symptom was absent; on a real desktop it
would have been dead the moment any window was focused. The fix is
`captured-event`, and the smoke asserts `stage.get_grab_actor()` now.

### Dock badges: apps publish, we render (#190)

`com.canonical.Unity.LauncherEntry.Update` — a convention we did not
invent, which every toolkit and Electron app already emits, so a
third-party app badges with **no Lisa-specific code** and our dock is
just another consumer. `lib/badges.js` parses it as hostile input: a
count is believed only if it is a real non-negative integer, an
`app_uri` only if it is a plausible desktop id (so a peer cannot badge
somebody else's icon), and `count-visible: false` is the only way to
say "clear it" — a badge that cannot be dismissed is impossible by
construction.

`BadgeState` remembers what each app last said, bounded at 64 apps
because the emitter list is not ours to bound. That store exists for
one reason: **the Dash destroys its icon actors**. It reuses an item
for an app that stays in the list, but unpin/repin — or a
non-favourite whose last window closes and reopens — destroys the
actor and builds a new one, taking the badge with it. `child-added` on
the dash box re-applies. Actor references go stale; what an app said
about itself does not.

Mail is the first emitter (`apps/mail/lib/launcher.js`).

### The hot corner is GNOME's own HotCorner, mirrored

`BottomRightCorner extends Layout.HotCorner` and overrides one method,
`setBarrierSize`. Everything else — the pressure threshold, the overview
toggle, the fullscreen guard, the ripples — is inherited, so the corner
behaves like the one it replaces instead of like an approximation.

`_updateHotCorners` is overridden rather than the built corners being
repositioned, because GNOME rebuilds them on monitor changes, on panel
resize, and whenever the hot-corner setting flips — each rebuild would
put the corner back at the top-left.

**The user's setting still governs.** If they turned hot corners off in
Settings, ours is off too: a guardrail belongs between the model and the
machine, never between a person and their own desktop (ADR-0030).

### The top bar is reordered through GNOME's own role lists

GNOME builds each panel box from `Main.sessionMode.panel.{left,center,
right}`, so `_reorderPanel()` rewrites those lists and calls
`Main.panel._updatePanel()`. `_addToPanelBox` already reparents a
container out of its old box, so `activities` migrating from left to
centre needs nothing special, and `disable()` restores the *original*
object rather than a guess at the defaults.

The `LISA` wordmark is a `PanelMenu.Button` with no menu — a popup there
would compete with the overview it opens — added at position 0 of the
left box. It handles `vfunc_event` rather than `button-press-event` so
it answers keyboard and touch too; it replaces Activities, which did.

**A session-mode change re-syncs those lists.** Locking the screen,
unlocking, or switching user rebuilds the panel from the mode
definition and silently undoes the reorder, so the extension re-applies
on `sessionMode`'s `updated` signal. Without that the panel is correct
only until the first screen lock — which is exactly the kind of bug that
gets reported as "it randomly resets".

### The geometry is a pure module

`lib/layout.js` holds the barrier placement and the dock placement, with
no GNOME imports, and `tests/layout.test.js` covers them under
`just shell-test`.

That split is deliberate. Barrier directions are invisible until a
pointer is pushed into a real corner on real hardware, so they are the
last thing that should be written inline. GNOME's top-left corner runs
its barriers *down* and *right* with `POSITIVE_X`/`POSITIVE_Y`; the
bottom-right mirrors both axes, so the barriers run *up* and *left* and
both directions flip to `NEGATIVE_*`. The tests pin that mirroring, and
that a dock is clamped on-screen and centred on the monitor it belongs
to rather than on the primary.

### The connection is tested too, not just the decision

`tests/showapps-smoke.js` + `tests/run-showapps-smoke.sh` start a real
throwaway `gnome-shell --headless`, load this extension, synthesise a
real pointer press on the real Show Apps button and assert the overview
lands on the app grid.

```
shell/desktop/tests/run-showapps-smoke.sh
```

It needs Linux with gnome-shell ≥ 50, gjs and `dbus-run-session`; on a
macOS dev host it prints `SKIP` and exits 0. It is **not** wired into
`just shell-test`, which must stay runnable on any dev host — run it by
hand on a Linux box, or see the limits below.

This exists because #262 was invisible to every unit test here by
construction: `showAppsAction` was pure, tested and correct, while the
wiring feeding it was untested and wrong — the same shape as #241, #244
and #255. A test that a function returns the right string says nothing
about whether anything calls it. This one presses the button.

Verified against gnome-shell 50.3 on the reference hardware: red on the
pre-fix wiring (`a press is not swallowed by a stale latch: expected 2,
got 1`), green after.

### The prompt and the badges are pressed, not reasoned about

`tests/dock-prompt-smoke.js` + `tests/run-dock-prompt-smoke.sh` start a
real throwaway `gnome-shell --headless`, load this extension, and drive
it with a virtual pointer and a virtual keyboard:

```
shell/desktop/tests/run-dock-prompt-smoke.sh
```

It clicks the prompt, checks the dock took the keyboard grab, types,
presses Return and asserts the call arrived at a stub owning the real
`dev.lisaos.Overlay1.UI` name on the real bus; Escape clears then
releases; a program name launches a program that leaves a file behind;
a badge is drawn from a real signal, survives an icon being destroyed
and recreated, and is cleared by `count: 0`. Any line the extension
logs fails the run, because a `logError` inside a signal handler never
reaches the transcript.

Same isolation as the show-apps smoke — read that file's header before
touching either. Verified on gnome-shell 50.3 on the reference iMac:
17 assertions green, and red on the pre-fix wiring described above.

### State-dependent app icons

An app that ships an icon named `<icon>-active` in hicolor gets it
drawn wherever the shell paints its icon **while the app is running**
(#190, the state half). Surfer opts in: a meditating robot on flat
water when closed, riding the wave while open. There is no per-app code
here — the variant's existence in the icon theme is the opt-in.

Mechanics: `Shell.App.create_icon_texture` is patched on the prototype
(the one funnel the dash, the overview grid and alt-tab all draw
through; painting only the dock would leave one app wearing two faces),
the swap decision and the candidate paths live pure in
`lib/stateicon.js` (tested: STARTING deliberately keeps the idle icon,
so a launch that dies never leaves a lying "active" icon), existence is
answered by file checks because St's icon lookup cannot say "missing" —
it falls back to a generic — and `app-state-changed` repaints the dash
entry. `disable()` restores the original method.

### Transient peeks stay out of the dock

A quick-look panel is not an app you are running: Preview's Space
instance runs under its own NoDisplay id (`app.lisaos.PreviewPeek`),
and `Shell.AppSystem.get_running` is patched to drop ids
`lib/stateicon.js` lists as transient — so the dock never grows a
Preview icon for a peek, exactly as macOS keeps the Quick Look panel
out of its Dock. Alt-tab still shows the window (with Preview's icon,
resolved through the NoDisplay .desktop), because a visible window you
cannot reach is worse than a crowded switcher. `disable()` restores
the original method.

## How to extend it

- **The results list** is the rest of ADR-0035 §2: the bar grows upward
  as you type, `shell/launcher`'s ranking moves in, and both chords land
  here. That change is where prefix matching becomes safe, because there
  would finally be something on screen to disambiguate against.
- **A new badge field** (progress arcs, `urgent`) is parsed and carried
  by `lib/badges.js` already; what is missing is the drawing.
- **Anything visual** goes in `stylesheet.css`; the Dash's own
  background is zeroed there so the outer panel does not double-frame.
- **Anything with arithmetic in it** goes in `lib/layout.js` with a
  test. Nothing in `extension.js` should be doing sums.

## Limits and open questions

- **`affectsStruts` is false**, so a maximized window runs underneath
  the dock rather than stopping above it. macOS reserves the space; we
  do not yet. Turning it on for a *centred* floating panel needs
  thought — GNOME computes struts from the actor's bounding box against
  a screen edge, and this panel does not touch one.
- **One corner, on the primary monitor only.** GNOME builds one per
  monitor because a secondary monitor's top-left can be unoccupied; a
  second bottom-right trigger the user cannot see is worse than none.
- **Ripples are inherited** and anchored at the corner point. They were
  designed for the top-left; whether they read correctly mirrored is a
  question for hardware, not for review.
- **No settings yet** — no auto-hide, no icon-size control, no choice of
  corner. ADR-0035 does not settle auto-hide either.
- **The prompt has no history, no completion and no results.** Up-arrow
  does nothing; there is no way to see what an exact match would launch
  before pressing Return.
- **A click outside the dock is eaten while the prompt is focused.** The
  modal grab has no way to forward it, which is the trade the assistant
  overlay already makes. One click to leave the field, and the dock's own
  icons keep working because they are inside the grab.
- **Progress and `urgent` are parsed and not drawn.** An app emitting
  either gets no arc and no highlight today.
- **Neither smoke check is in CI.** `run-showapps-smoke.sh` and
  `run-dock-prompt-smoke.sh` run on demand on a Linux host and skip
  elsewhere; nothing in the `justfile` or the workflows calls them, so
  they cannot fail a PR. Both need a Linux CI job with a render node,
  which is a change outside this directory.
- **Requires a session restart to load.** GNOME Shell only scans the
  extension directories at startup, and on Wayland there is no in-place
  restart, so a newly installed copy needs a logout/login. That is a
  property of GNOME, not of this extension, but it is why iterating on
  it costs more than iterating on the GJS apps.
