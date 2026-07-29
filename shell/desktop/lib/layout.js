// Pure geometry for the Lisa desktop shell (ADR-0035).
//
// Both functions here are the parts most likely to be wrong and hardest
// to see wrong: barrier directions are invisible until you push a mouse
// into a corner on real hardware, and an off-by-one in dock placement
// looks like "the design is a bit off" rather than like a bug. So they
// are plain functions over plain numbers, tested without GNOME.
//
// No imports on purpose — `just shell-test` runs this under gjs, node
// and macOS jsc alike.

/// Where the two pointer barriers for a BOTTOM-RIGHT hot corner go.
///
/// GNOME's `HotCorner.setBarrierSize` builds a top-left corner: a
/// vertical barrier running DOWN from the corner with `POSITIVE_X`, and
/// a horizontal barrier running RIGHT from it with `POSITIVE_Y`. (Its
/// RTL branch mirrors the X axis only, and flips the vertical barrier
/// to `NEGATIVE_X` — which is what tells you the sign follows the axis
/// you mirrored.)
///
/// Bottom-right is that corner mirrored in BOTH axes: the barriers run
/// UP and LEFT from the corner, and both directions flip.
///
/// Returns direction names rather than `Meta.BarrierDirection` values so
/// this stays testable off-device; the caller maps them.
export function bottomRightBarriers(corner, size) {
    const {x, y} = corner;
    return {
        vertical: {
            x1: x, x2: x,
            y1: y - size, y2: y,
            direction: 'NEGATIVE_X',
        },
        horizontal: {
            x1: x - size, x2: x,
            y1: y, y2: y,
            direction: 'NEGATIVE_Y',
        },
    };
}

/// The bottom-right corner point of a monitor.
///
/// A monitor's `x`/`y` are its top-left in the global coordinate space,
/// so the corner is the far edge — not `width`/`height`, which is the
/// mistake that puts the hot corner on the wrong monitor in a multi-head
/// layout.
export function bottomRightOf(monitor) {
    return {x: monitor.x + monitor.width, y: monitor.y + monitor.height};
}

/// Top-left position for a dock of `size` centred along the bottom edge
/// of `monitor`, sitting `margin` pixels clear of it.
///
/// Clamped so a dock wider than the monitor starts at the monitor's left
/// edge instead of at a negative offset — which would put its first
/// icons off-screen, or onto the neighbouring monitor.
export function dockPlacement(monitor, size, margin) {
    const x = monitor.x + Math.max(0, Math.round((monitor.width - size.width) / 2));
    const y = monitor.y + Math.max(0, monitor.height - size.height - margin);
    return {x, y};
}

/// What clicking the dock's "Show Applications" button should do.
///
/// The dock is a `Dash` built outside the overview, so its show-apps
/// button is wired by nobody — GNOME connects its own dash's button
/// inside `ControlsManager`. The extension therefore drives it, and the
/// obvious wiring (`Main.overview.showApps()`) is right exactly half the
/// time:
///
/// ```js
/// showApps() { this.show(ControlsState.APP_GRID); }
/// show(state) { …; if (this._shown) return; … }   // gnome-shell 50
/// ```
///
/// From the desktop that opens the overview on the app grid. From
/// *inside* the overview `_shown` is already true, so it returns
/// immediately and the button does nothing at all — which is what a
/// person sees as "show all apps is broken", because the overview is
/// where you look for it.
///
/// The supported way to move between pages of an open overview is
/// GNOME's own dash button: `ControlsManager` listens to its
/// `notify::checked` and eases the state adjustment between
/// WINDOW_PICKER and APP_GRID. Our button mirrors onto it.
///
/// Returns an action name so the decision is testable without GNOME:
///   'open-app-grid'  — overview closed: open it straight to the grid.
///   'mirror'         — overview open: copy `checked` onto GNOME's
///                      button and let ControlsManager animate.
///   'none'           — nothing to do.
export function showAppsAction({overviewVisible, checked}) {
    if (!overviewVisible)
        return checked ? 'open-app-grid' : 'none';
    // Both directions matter: unchecking is how you get back to the
    // window picker without closing the overview.
    return 'mirror';
}
