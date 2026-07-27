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
// 3. **The top bar is reordered** to the sketch: the `LISA` wordmark at
//    the left, the workspace switcher moved to the centre, and the clock
//    moved out of the centre to sit with the quick settings on the
//    right.
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
import Shell from 'gi://Shell';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Layout from 'resource:///org/gnome/shell/ui/layout.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as SystemActions from 'resource:///org/gnome/shell/misc/systemActions.js';
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

/// The `LISA` wordmark, top-left — and the menu behind it.
///
/// The Apple menu's job, in Lisa's terms: what this machine is, the
/// things only Lisa has, and the session actions. It is deliberately
/// short. A menu that lists everything is a menu nobody reads.
///
/// It also carries **Log Out**, which GNOME hides on a single-user
/// machine with autologin (issue #139) — so on the reference hardware
/// this is the only way to end a session short of a power cycle.
const LisaWordmark = GObject.registerClass(
class LisaWordmark extends PanelMenu.Button {
    _init() {
        super._init(0.0, 'Lisa');
        // The real wordmark, not the letters L-I-S-A set in the UI font.
        //
        // A `St.Icon` would be wrong here: it renders a gicon into a
        // SQUARE `icon_size`, and this mark is 24x7, so it would come out
        // letterboxed and tiny. A plain widget with the SVG as its
        // background honours the aspect ratio we give it.
        this.add_child(new St.Widget({
            style_class: 'lisa-wordmark',
            y_align: Clutter.ActorAlign.CENTER,
            accessible_name: 'Lisa',
        }));
        this._buildMenu();
    }

    _buildMenu() {
        const actions = SystemActions.getDefault();

        // What this machine is. Informational, like "About This Mac" —
        // it reports, it does not act, so it is insensitive rather than
        // a dead click.
        const about = new PopupMenu.PopupMenuItem(osRelease(), {reactive: false});
        about.add_style_class_name('lisa-menu-about');
        this.menu.addMenuItem(about);
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // The things only Lisa has, then the system's own settings.
        //
        // NOT `app.lisaos.Settings.desktop`: that is the standalone Lisa
        // settings app, which was merged into GNOME Settings as the
        // Intelligence panel (ADR-0012). Its .desktop is still installed
        // and still launches, so pointing here at the old app would open
        // a second, stale settings window beside the real one.
        this._app('Assistant', 'app.lisaos.Assistant.desktop');
        this._app('Ledger', 'app.lisaos.LedgerApp.desktop');
        this._app('Intelligence', 'gnome-lisa-panel.desktop');
        this._app('Settings', 'org.gnome.Settings.desktop');
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // The wordmark used to toggle the overview on click. The hot
        // corner and the Super key still do, but taking the click away
        // without leaving a route here would be a removal, not a move.
        this._action('Activities Overview', () => Main.overview.toggle());
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // Session actions, in the order macOS puts them.
        this._action('Lock Screen', () => actions.activateLockScreen());
        this._action('Log Out…', () => actions.activateLogout());
        this._action('Restart…', () => actions.activateRestart());
        this._action('Power Off…', () => actions.activatePowerOff());
    }

    /// A menu entry that launches an installed app, shown only when the
    /// app is actually installed — an entry that does nothing when
    /// clicked is worse than an absent one.
    _app(label, desktopId) {
        const app = Shell.AppSystem.get_default().lookup_app(desktopId);
        if (!app)
            return;
        this._action(label, () => app.activate());
    }

    _action(label, fn) {
        const item = new PopupMenu.PopupMenuItem(label);
        item.connect('activate', () => fn());
        this.menu.addMenuItem(item);
    }
});

/// What this machine calls itself, straight from `/etc/os-release`.
///
/// Read, never guessed: a hard-coded product string is wrong the moment
/// a build ships, and a wrong version in the About line is worse than no
/// About line.
///
/// `PRETTY_NAME` alone is just "Lisa OS" — the number that matters is
/// `IMAGE_VERSION`, because that is the one `lisa update` moves and the
/// one an issue report needs.
function osRelease() {
    const field = (text, key) => {
        const match = new RegExp(`^${key}="?([^"\\n]+)"?`, 'm').exec(text);
        return match ? match[1] : null;
    };
    try {
        const [ok, bytes] = GLib.file_get_contents('/etc/os-release');
        if (ok) {
            const text = new TextDecoder().decode(bytes);
            const name = field(text, 'PRETTY_NAME') ?? field(text, 'NAME') ?? 'Lisa OS';
            const version = field(text, 'IMAGE_VERSION');
            return version ? `${name} ${version}` : name;
        }
    } catch {
        // A missing or unreadable os-release is not worth an exception
        // in a panel menu.
    }
    return 'Lisa OS';
}

/// The always-visible dock: GNOME's Dash in a floating rounded panel.
///
/// Two actors, not one, and the split is load-bearing.
///
/// `LayoutManager` OWNS the `visible` property of any chrome registered
/// with `trackFullscreen`, and rewrites it on every relayout:
///
///     actor.visible = !(global.window_group.visible &&
///                       monitor && monitor.inFullscreen)
///
/// In the overview `global.window_group.visible` is false, so that
/// expression is `true` — entering the overview forcibly *re-showed*
/// this dock on top of GNOME's own dash, which is the two-docks bug.
///
/// So the OUTER actor is unstyled and belongs to LayoutManager, which
/// keeps GNOME's fullscreen handling for free; the INNER panel carries
/// the styling and is ours to hide. LayoutManager never touches
/// children.
const LisaDock = GObject.registerClass(
class LisaDock extends St.Widget {
    _init() {
        super._init({layout_manager: new Clutter.BinLayout()});
        this.panel = new St.BoxLayout({
            style_class: 'lisa-dock',
            reactive: true,
            track_hover: true,
        });
        this.dash = new Dash();
        this.dash.show();
        this.panel.add_child(this.dash);
        this.add_child(this.panel);
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
        // duration rather than fighting it for z-order. Hiding the inner
        // panel, not the tracked outer actor — see LisaDock.
        this._connect(Main.overview, 'showing', () => this._dock.panel.hide());
        this._connect(Main.overview, 'hidden', () => {
            this._dock.panel.show();
            // The button latches when clicked; nothing unlatches it when
            // the overview closes by other means (Escape, Super, a
            // click), leaving it stuck lit and dead to the next press.
            this._dock.dash.showAppsButton.checked = false;
        });

        // GNOME wires its dash's show-apps button from the overview's
        // own controls, so a Dash used outside the overview has a button
        // that does nothing at all. Wire it to the same destination.
        const showApps = this._dock.dash.showAppsButton;
        this._connect(showApps, 'notify::checked', () => {
            if (showApps.checked)
                Main.overview.showApps();
        });
        this._connect(Main.layoutManager, 'monitors-changed', () => {
            this._reposition();
            this._installHotCorners();
        });
        // Keep the panel centred whenever the Dash changes size — a
        // favourite pinned or unpinned, an app opening or closing, the
        // icon size settling. Without this the dock keeps its original
        // placement and drifts off-centre as it grows.
        this._connect(this._dock, 'notify::width', () => this._reposition());
        this._connect(this._dock, 'notify::height', () => this._reposition());
        this._connect(this._dock.dash, 'icon-size-changed', () => this._reposition());

        this._installHotCorners();
        this._reorderPanel();
    }

    disable() {
        this._restorePanel();
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

    // ---- the top bar ---------------------------------------------------

    /// Reorder the panel to the sketch.
    ///
    /// GNOME builds each box from role lists on `Main.sessionMode.panel`,
    /// so the reorder is a change to those lists plus a rebuild — not a
    /// reparenting of actors behind the Shell's back. `_addToPanelBox`
    /// already moves a container out of its old box, so `activities`
    /// migrating from left to centre needs nothing special.
    _reorderPanel() {
        const sessionMode = Main.sessionMode;
        // Keep the ORIGINAL object, not a copy: restoring it is what
        // makes `disable()` a real undo rather than an approximation of
        // whatever the defaults happened to be.
        this._originalPanel ??= sessionMode.panel;
        const {left, center, right} = this._originalPanel;

        sessionMode.panel = {
            // The wordmark is added separately — it is ours, and only
            // roles GNOME knows about belong in these lists.
            left: left.filter(role => role !== 'activities'),
            // The workspace switcher takes the centre the clock leaves.
            center: center.filter(role => role !== 'dateMenu')
                .concat(left.includes('activities') ? ['activities'] : []),
            // The clock joins the quick settings, after them: the sketch
            // reads wifi, bluetooth, then the time.
            right: right.concat(center.includes('dateMenu') ? ['dateMenu'] : []),
        };
        Main.panel._updatePanel();

        if (!this._wordmark) {
            this._wordmark = new LisaWordmark();
            Main.panel.addToStatusArea('lisa-wordmark', this._wordmark, 0, 'left');
        }

        // A session-mode change (lock, unlock, switch user) re-syncs
        // these lists from the mode definition and would silently undo
        // the reorder. Without this the panel is correct until the first
        // time the screen locks.
        this._sessionSignal ??= sessionMode.connect('updated', () => this._reorderPanel());
    }

    _restorePanel() {
        if (this._sessionSignal) {
            Main.sessionMode.disconnect(this._sessionSignal);
            this._sessionSignal = null;
        }
        if (this._wordmark) {
            this._wordmark.destroy();
            this._wordmark = null;
        }
        if (this._originalPanel) {
            Main.sessionMode.panel = this._originalPanel;
            this._originalPanel = null;
            Main.panel._updatePanel();
        }
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
