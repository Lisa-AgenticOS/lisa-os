// Lisa desktop shell — always-visible dock + bottom-right hot corner
// (ADR-0035, PLAN §5.7).
//
// Two changes to the stock GNOME desktop, both from the wireframe:
//
// 1. **The dock is always visible**, macOS-style, instead of appearing
//    only inside the overview. GNOME's own `Dash` is reused rather than
//    reimplemented — it already does pinned-plus-running merged, running
//    dots, click-to-focus, drag reordering and app context menus, which
//    is exactly the behaviour ADR-0035 §2 asks for. Reimplementing it
//    would mean owning every one of those regressions.
//
// 2. **The hot corner moves to the bottom-right.** The top-left corner
//    is where the `LISA` wordmark goes, and a hot corner underneath a
//    thing you click is a trap.
//
// The prompt half of ADR-0035's bar is NOT here yet: this extension owns
// the dock and the corner, and the entry field is the next slice.
//
// Geometry lives in lib/layout.js and is unit-tested — barrier
// directions are invisible until a pointer is pushed into a real corner,
// so they are the last thing that should be written inline.

import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Layout from 'resource:///org/gnome/shell/ui/layout.js';
import {Dash} from 'resource:///org/gnome/shell/ui/dash.js';

import {bottomRightBarriers, bottomRightOf, dockPlacement} from './lib/layout.js';

/// Gap between the dock and the bottom edge of the screen. The dock
/// floats; it does not sit in the corner.
const DOCK_MARGIN = 8;

/// Height budget handed to the Dash, which sizes its icons to fit.
const DOCK_HEIGHT = 80;

/// A hot corner at the BOTTOM-RIGHT of a monitor.
///
/// Everything except the barrier placement is GNOME's: the pressure
/// threshold, the overview toggle, the fullscreen guard and the ripple
/// animation all come from the base class, so this corner behaves like
/// the one it replaces rather than like an approximation of it.
const BottomRightCorner = GObject.registerClass(
class BottomRightCorner extends Layout.HotCorner {
    setBarrierSize(size) {
        // Same teardown as the base class. Barriers are kernel/compositor
        // objects, not actors, so they are not freed by destroying the
        // corner — leaking them leaves a live trigger at the old corner
        // with nothing attached to it.
        if (this._verticalBarrier) {
            this._pressureBarrier.removeBarrier(this._verticalBarrier);
            this._verticalBarrier.destroy();
            this._verticalBarrier = null;
        }
        if (this._horizontalBarrier) {
            this._pressureBarrier.removeBarrier(this._horizontalBarrier);
            this._horizontalBarrier.destroy();
            this._horizontalBarrier = null;
        }
        if (size <= 0)
            return;

        const spec = bottomRightBarriers({x: this._x, y: this._y}, size);
        this._verticalBarrier = new Meta.Barrier({
            backend: global.backend,
            x1: spec.vertical.x1, x2: spec.vertical.x2,
            y1: spec.vertical.y1, y2: spec.vertical.y2,
            directions: Meta.BarrierDirection[spec.vertical.direction],
        });
        this._horizontalBarrier = new Meta.Barrier({
            backend: global.backend,
            x1: spec.horizontal.x1, x2: spec.horizontal.x2,
            y1: spec.horizontal.y1, y2: spec.horizontal.y2,
            directions: Meta.BarrierDirection[spec.horizontal.direction],
        });
        this._pressureBarrier.addBarrier(this._verticalBarrier);
        this._pressureBarrier.addBarrier(this._horizontalBarrier);
    }
});

/// The always-visible dock: GNOME's Dash in a floating rounded panel.
const LisaDock = GObject.registerClass(
class LisaDock extends St.BoxLayout {
    _init() {
        super._init({
            style_class: 'lisa-dock',
            reactive: true,
            track_hover: true,
        });
        this.dash = new Dash();
        // The Dash hides itself when it believes it is off-duty; ours is
        // never off-duty.
        this.dash.show();
        this.add_child(this.dash);
    }

    /// Size the Dash to the monitor, then place the whole panel.
    reposition(monitor) {
        if (!monitor)
            return;
        this.dash.setMaxSize(Math.round(monitor.width * 0.9), DOCK_HEIGHT);
        // Ask for the natural size only AFTER setMaxSize, or the first
        // frame is laid out against the previous monitor's budget.
        const [, width] = this.get_preferred_width(-1);
        const [, height] = this.get_preferred_height(width);
        const {x, y} = dockPlacement(monitor, {width, height}, DOCK_MARGIN);
        this.set_position(x, y);
    }
});

export default class LisaDesktopExtension extends Extension {
    enable() {
        this._signals = [];

        this._dock = new LisaDock();
        // trackFullscreen: a fullscreen window gets the whole screen.
        // Floating chrome that survives fullscreen is a bug (ADR-0035).
        // affectsStruts stays false: the dock floats over maximized
        // windows rather than shrinking the work area — see the README.
        Main.layoutManager.addChrome(this._dock, {
            trackFullscreen: true,
            affectsStruts: false,
        });
        this._reposition();

        // In the overview GNOME shows its OWN dash. Two docks on screen
        // at once is worse than either, so ours stands down for the
        // duration rather than fighting it for z-order.
        this._connect(Main.overview, 'showing', () => this._dock.hide());
        this._connect(Main.overview, 'hidden', () => this._dock.show());
        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._reposition();
            this._installHotCorners();
        });
        // The Dash populates itself asynchronously — at `enable()` time
        // it has no icons yet, so its natural size is zero and a
        // placement computed now puts a zero-width dock in the corner,
        // invisible, forever. Follow the panel's own size instead of
        // guessing when it has settled.
        this._connect(this._dock, 'notify::width', () => this._reposition());
        this._connect(this._dock, 'notify::height', () => this._reposition());
        this._connect(this._dock.dash, 'icon-size-changed', () => this._reposition());

        this._installHotCorners();
    }

    disable() {
        this._signals?.forEach(([obj, id]) => obj.disconnect(id));
        this._signals = null;

        if (this._dock) {
            Main.layoutManager.removeChrome(this._dock);
            this._dock.destroy();
            this._dock = null;
        }
        this._restoreHotCorners();
    }

    _connect(object, signal, callback) {
        this._signals.push([object, object.connect(signal, callback)]);
    }

    _reposition() {
        this._dock?.reposition(Main.layoutManager.primaryMonitor);
    }

    // ---- hot corners -------------------------------------------------

    /// Replace GNOME's corner-building with ours.
    ///
    /// Overriding `_updateHotCorners` rather than repositioning the
    /// corners it builds is deliberate: GNOME rebuilds them on monitor
    /// changes, on panel resize and when the hot-corner setting flips,
    /// and every one of those rebuilds would put the corner back at the
    /// top-left.
    _installHotCorners() {
        const layoutManager = Main.layoutManager;
        this._originalUpdateHotCorners ??= layoutManager._updateHotCorners;
        layoutManager._updateHotCorners = () => this._updateHotCorners();
        layoutManager._updateHotCorners();
    }

    _restoreHotCorners() {
        if (!this._originalUpdateHotCorners)
            return;
        Main.layoutManager._updateHotCorners = this._originalUpdateHotCorners;
        this._originalUpdateHotCorners = null;
        Main.layoutManager._updateHotCorners();
    }

    _updateHotCorners() {
        const layoutManager = Main.layoutManager;
        layoutManager.hotCorners.forEach(corner => corner?.destroy());
        layoutManager.hotCorners = [];

        // The user's own setting still governs. A guardrail belongs
        // between the model and the machine, never between a person and
        // their own desktop — if they turned hot corners off, they are
        // off (ADR-0030).
        if (!layoutManager._interfaceSettings.get_boolean('enable-hot-corners')) {
            layoutManager.emit('hot-corners-changed');
            return;
        }

        // One corner, on the primary monitor. GNOME builds one per
        // monitor because the top-left of a secondary monitor can be
        // free; the bottom-right of the primary is unambiguous, and a
        // second trigger the user cannot see is worse than none. The
        // array still carries one slot per monitor, because that is the
        // shape the rest of the Shell indexes into.
        const {primaryIndex, monitors} = layoutManager;
        for (let i = 0; i < monitors.length; i++) {
            if (i !== primaryIndex) {
                layoutManager.hotCorners.push(null);
                continue;
            }
            const {x, y} = bottomRightOf(monitors[i]);
            const corner = new BottomRightCorner(layoutManager, monitors[i], x, y);
            corner.setBarrierSize(layoutManager.panelBox.height);
            layoutManager.hotCorners.push(corner);
        }
        layoutManager.emit('hot-corners-changed');
    }
}
