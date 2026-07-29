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
import Gio from 'gi://Gio';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Layout from 'resource:///org/gnome/shell/ui/layout.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as SystemActions from 'resource:///org/gnome/shell/misc/systemActions.js';
import {Dash} from 'resource:///org/gnome/shell/ui/dash.js';

import {bottomRightBarriers, bottomRightOf, dockPlacement, showAppsAction} from './lib/layout.js';

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

        // "About Lisa OS", like the Apple menu's "About This Mac": it
        // opens the page that answers the question rather than answering
        // it in the menu.
        //
        // This used to print `PRETTY_NAME` + `IMAGE_VERSION` inline and
        // do nothing when clicked. Two reasons it moved: a version is a
        // fact people want to *act* on — check for an update, copy it
        // into a bug report — and a menu is the wrong place for either;
        // and Settings → System now carries the version alongside a
        // Check for Updates button, so the menu was the second, dumber
        // copy of a live surface.
        this._action('About Lisa OS', () => openSystemSettings());
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

/// Open Settings on the System page — where the OS version and the
/// Check for Updates button live (the Lisa row in our
/// gnome-control-center build).
///
/// Spawned as `gnome-control-center system` rather than activated
/// through `Shell.AppSystem`: a .desktop activation opens Settings on
/// whatever page it last showed, and the whole point of this entry is
/// the page. GNOME's own panels navigate by the same argument.
///
/// A failure here is logged, not thrown: a panel menu that raises into
/// the Shell's main loop takes more with it than the click that caused
/// it.
function openSystemSettings() {
    try {
        Gio.Subprocess.new(['gnome-control-center', 'system'], Gio.SubprocessFlags.NONE);
    } catch (e) {
        logError(e, 'lisa-desktop: could not open Settings on the System page');
    }
}

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

        // ONE dock, everywhere — GNOME's own goes away.
        //
        // The first attempt did the opposite: ours hid inside the
        // overview so GNOME's could take over. That produced two docks a
        // few pixels apart, because `LayoutManager` owns the `visible`
        // property of chrome registered with `trackFullscreen` and
        // rewrites it on every relayout as
        // `!(window_group.visible && monitor.inFullscreen)`. In the
        // overview `window_group.visible` is false, so that expression
        // is `true` and the dock was forcibly re-shown. Hiding it harder
        // was never going to work.
        //
        // Hiding GNOME's dash instead removes the conflict rather than
        // fighting it, and it is what the design asked for anyway: on
        // macOS the Dock does not vanish when you open Mission Control.
        this._dash = Main.overview.dash;
        this._dash.hide();
        // The overview's controls re-show it on state changes, so once
        // is not enough.
        this._connect(Main.overview, 'showing', () => this._dash.hide());
        this._connect(Main.overview, 'hidden', () => {
            // The button latches when clicked; nothing unlatches it when
            // the overview closes by other means (Escape, Super, a
            // click), leaving it stuck lit and dead to the next press.
            this._dock.dash.showAppsButton.checked = false;
        });

        // GNOME wires its dash's show-apps button from the overview's
        // own controls, so a Dash used outside the overview has a button
        // that does nothing at all. Wire it to the same destination.
        //
        // `Main.overview.showApps()` alone was right only from the
        // desktop: it is `show(APP_GRID)`, and `show()` returns early
        // when the overview is already up. Pressed from inside the
        // overview — which is exactly where a person goes looking for
        // "all apps" — the button latched and nothing happened.
        //
        // See `showAppsAction` for the reasoning; the two branches are
        // unit-tested there, which is the only place they can be.
        const showApps = this._dock.dash.showAppsButton;
        this._connect(showApps, 'notify::checked', () => {
            const visible = Main.overview.visibleTarget ?? Main.overview.visible;
            switch (showAppsAction({overviewVisible: visible, checked: showApps.checked})) {
            case 'open-app-grid':
                Main.overview.showApps();
                break;
            case 'mirror':
                // GNOME's dash is hidden, but its button is still the
                // thing `ControlsManager` listens to, and a hidden
                // actor's properties notify exactly the same.
                Main.overview.dash.showAppsButton.checked = showApps.checked;
                break;
            }
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
        this._dash?.show();
        this._dash = null;
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
