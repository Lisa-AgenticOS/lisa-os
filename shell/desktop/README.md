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

The prompt half of ADR-0035's bar is **not** here yet. This extension
owns the dock and the corner; the entry field is the next slice.

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

Inside the overview GNOME shows its *own* dash, so ours hides for the
duration rather than fighting it for z-order:

```js
Main.overview.connect('showing', () => this._dock.hide());
Main.overview.connect('hidden',  () => this._dock.show());
```

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

## How to extend it

- **The prompt field** (ADR-0035 §2) belongs in the same panel as the
  Dash, to the right of it, and should hand queries to
  `dev.lisaos.Overlay1` — the launcher becomes this bar's expanded
  state, not a second window.
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
- **Requires a session restart to load.** GNOME Shell only scans the
  extension directories at startup, and on Wayland there is no in-place
  restart, so a newly installed copy needs a logout/login. That is a
  property of GNOME, not of this extension, but it is why iterating on
  it costs more than iterating on the GJS apps.
