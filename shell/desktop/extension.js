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
// 4. **The bar carries the prompt** (ADR-0035 §2): a permanent entry
//    filling the rest of the bar, which launches a program when you
//    type its name and hands anything else to the assistant. What is
//    still missing from §2 is the launcher merge — this bar shows no
//    results and neither chord lands here. See the README.
//
// Geometry lives in lib/layout.js and is unit-tested — barrier
// directions are invisible until a pointer is pushed into a real corner,
// so they are the last thing that should be written inline.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
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

import {BadgeState, badgeFor, desktopIdFromUri} from './lib/badges.js';

import {bottomRightBarriers, bottomRightOf, dockPlacement, showAppsAction} from './lib/layout.js';
import {
    ASSISTANT_BUS_NAME, ASSISTANT_METHOD, ASSISTANT_OBJECT_PATH, ASSISTANT_SIGNATURE,
    keyAction, submission,
} from './lib/prompt.js';
import {activeIconName, candidatePaths, shouldUseActive, isTransientPeek} from './lib/stateicon.js';

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

/// How wide the prompt is, as a share of the monitor, and the bounds it
/// may not leave. The sketch draws it as the widest thing in the bar —
/// wider than the app icons put together — but a 4K panel would make a
/// literal share of that comical, and a small laptop would leave no
/// room to type.
const PROMPT_SHARE = 0.26;
const PROMPT_MIN = 260;
const PROMPT_MAX = 560;

/// The prompt half of ADR-0035 §2's bar: "One bar: apps on the left,
/// the prompt filling the rest."
///
/// Emits `submitted` with the text and nothing else. It does not know
/// what happens next — whether a program starts or the assistant
/// answers is `lib/prompt.js`'s decision and the extension's call, and
/// keeping that out of the widget is what keeps the widget a text
/// field.
const DockPrompt = GObject.registerClass({
    Signals: {
        'submitted': {param_types: [GObject.TYPE_STRING]},
        // "Somebody clicked me and wants to type" / "I am done with the
        // keyboard". The dock cannot take the keyboard on its own — that
        // needs a modal grab, which belongs to the extension.
        'focus-wanted': {},
        'focus-dropped': {},
    },
}, class DockPrompt extends St.Entry {
    _init() {
        super._init({
            style_class: 'lisa-dock-prompt',
            // Both halves of what the bar is for, in the order ADR-0035
            // argues them: asking is the new thing, launching is the
            // familiar one.
            hint_text: 'Ask Lisa, or type an app name',
            can_focus: true,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        // The wireframe's right-pointing triangle, "terminating" the
        // field. A secondary icon rather than a sibling button because
        // it must sit INSIDE the rounded field, and because St.Entry
        // already makes it clickable and reachable.
        this.set_secondary_icon(new St.Icon({
            style_class: 'lisa-dock-prompt-go',
            icon_name: 'go-next-symbolic',
        }));
        this.connect('secondary-icon-clicked', () => this._submit());
        this.clutter_text.connect('key-press-event',
            (_text, event) => this._onKey(event));
        // The ONLY route into the keyboard. Not hover, and not the
        // shell's focus chain: the dock never registers with
        // `Main.ctrlAltTabManager`, so tabbing between shell surfaces
        // does not land here. ADR-0035: "A permanent text entry must
        // never steal focus."
        //
        // `captured-event`, NOT `button-press-event`. The press lands on
        // the entry's own `ClutterText`, which handles it to place the
        // caret and stops it there — so a bubbling handler on the entry
        // never runs. The capture phase walks DOWN to the target, so it
        // sees the press whatever the target does with it afterwards.
        //
        // This is not a theoretical distinction. With the bubbling
        // version the caret still moved and the field still took text
        // *in a headless shell with no windows*, because with nothing
        // focused the stage gets keys for free — and it would have been
        // dead on a real desktop the moment any window had focus.
        // `tests/dock-prompt-smoke.js` is what said so.
        this.connect('captured-event', (_actor, event) => {
            const type = event.type();
            if (type === Clutter.EventType.BUTTON_PRESS ||
                type === Clutter.EventType.TOUCH_BEGIN)
                this.emit('focus-wanted');
            return Clutter.EVENT_PROPAGATE;
        });
    }

    _onKey(event) {
        switch (keyAction(event.get_key_symbol(), {hasText: this.get_text() !== ''})) {
        case 'submit':
            this._submit();
            return Clutter.EVENT_STOP;
        case 'clear':
            this.set_text('');
            return Clutter.EVENT_STOP;
        case 'release':
            this.emit('focus-dropped');
            return Clutter.EVENT_STOP;
        default:
            return Clutter.EVENT_PROPAGATE;
        }
    }

    /// Cleared BEFORE the signal, not after: whatever handles this may
    /// open a window and take the pointer with it, and coming back to
    /// find your last question still sitting in the bar reads as a
    /// prompt that did not go anywhere.
    _submit() {
        const text = this.get_text();
        this.set_text('');
        this.emit('submitted', text);
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
        // Apps on the left, the prompt filling the rest (ADR-0035 §2).
        this.prompt = new DockPrompt();
        this.add_child(this.prompt);
    }

    /// Re-draw every remembered badge (#190).
    ///
    /// Called on `child-added` because that is the exact moment a badge
    /// is lost: the Dash throws its icon actors away and builds new ones
    /// whenever favourites change or an app starts, and a badge painted
    /// onto the old actor goes with it. The state is `BadgeState`, which
    /// holds what apps SAID rather than what was drawn.
    applyBadges(state) {
        for (const [desktopId, badge] of state.entries())
            this.setBadge(desktopId, badge);
    }

    /// Draw (or clear) a count badge on one dock item (#190).
    ///
    /// Walks the Dash's own children rather than keeping a parallel map:
    /// the Dash rebuilds its icons whenever favourites or running apps
    /// change, and a map would go stale silently — which is how a badge
    /// ends up on the wrong icon.
    setBadge(desktopId, badge) {
        const box = this.dash._box;
        if (!box)
            return;
        for (const child of box.get_children()) {
            const app = child.child?._delegate?.app ?? child._delegate?.app;
            if (app?.get_id() !== desktopId)
                continue;
            const icon = child.child ?? child;
            icon._lisaBadge?.destroy();
            icon._lisaBadge = null;
            if (badge.label === null)
                return;
            const pill = new St.Label({
                text: badge.label,
                style_class: 'lisa-dock-badge',
                x_align: Clutter.ActorAlign.END,
                y_align: Clutter.ActorAlign.START,
            });
            icon.add_child(pill);
            icon._lisaBadge = pill;
            return;
        }
    }

    /// Take every badge off, so `disable()` is a real undo.
    clearBadges() {
        for (const child of this.dash._box?.get_children() ?? []) {
            const icon = child.child ?? child;
            icon._lisaBadge?.destroy();
            icon._lisaBadge = null;
        }
    }

    /// Size the Dash to the monitor, then place the whole panel.
    reposition(monitor) {
        if (!monitor)
            return;
        const promptWidth = Math.round(Math.min(PROMPT_MAX,
            Math.max(PROMPT_MIN, monitor.width * PROMPT_SHARE)));
        this.prompt.set_width(promptWidth);
        // The Dash's budget is what is left of the bar, not the whole
        // bar: handing it 90% of the monitor and then adding the prompt
        // beside it is how a dock ends up wider than the screen it
        // floats on.
        this.dash.setMaxSize(
            Math.max(0, Math.round(monitor.width * 0.9) - promptWidth), DOCK_HEIGHT);
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
        // What each app last said about itself (#190). Kept because the
        // Dash destroys and rebuilds its icons — see BadgeState.
        this._badges = new BadgeState();
        this._connect(this._dock.dash._box, 'child-added',
            () => this._dock?.applyBadges(this._badges));
        this._connect(this._dock.prompt, 'submitted', (_p, text) => this._submit(text));
        this._connect(this._dock.prompt, 'focus-wanted', () => this._focusPrompt());
        this._connect(this._dock.prompt, 'focus-dropped', () => this._releasePrompt());
        // A fullscreen window takes the screen, and `trackFullscreen`
        // hides the dock — but a modal grab held by a hidden actor is a
        // keyboard nobody can get back.
        this._connect(this._dock, 'notify::visible', () => {
            if (!this._dock.visible)
                this._releasePrompt();
        });
        // Unity LauncherEntry (#190): the convention every toolkit
        // already emits, so a third-party app badges with no
        // Lisa-specific code. Subscribed with a null sender — any peer
        // may emit for its OWN app, and `desktopIdFromUri` is what stops
        // one badging somebody else's icon.
        this._badgeSub = Gio.DBus.session.signal_subscribe(
            null,
            'com.canonical.Unity.LauncherEntry',
            'Update',
            '/com/canonical/Unity/LauncherEntry',
            null,
            Gio.DBusSignalFlags.NONE,
            (_c, _sender, _path, _iface, _signal, params) => {
                try {
                    const [uri, props] = params.deepUnpack();
                    const id = desktopIdFromUri(uri);
                    if (!id)
                        return;
                    const plain = {};
                    for (const [k, v] of Object.entries(props ?? {}))
                        plain[k] = v?.deepUnpack ? v.deepUnpack() : v;
                    // Recorded first, drawn second. An app that emits
                    // before its icon is in the dash — Mail publishing
                    // on startup, before it is running — would otherwise
                    // have said its piece to nobody.
                    this._dock?.setBadge(id, this._badges.set(id, badgeFor(plain)));
                } catch (e) {
                    // A malformed signal is a missing badge, never a
                    // broken shell.
                    logError(e, 'lisa-desktop: bad LauncherEntry update');
                }
            });
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

        // GNOME wires its dash's show-apps button from the overview's
        // own controls, so a Dash used outside the overview has a button
        // that does nothing at all. Wire it to the same destination.
        //
        // THE PRESS IS AN EVENT, NOT A LATCH (#262).
        //
        // This used to hang off `notify::checked` and pass that latch to
        // `showAppsAction` as the intent. It is a toggle button, so the
        // signal does fire — that part was never broken, and was
        // verified on the device's own gnome-shell 50.3 by synthesising
        // a real pointer click (`tests/showapps-smoke.js`). What broke
        // is that `checked` is *shared, mutable display state*: GNOME's
        // ControlsManager writes it, our `hidden` handler writes it, a
        // ctrl+alt+tab focus callback writes it. Reading it back as
        // intent means any drift between the latch and the overview is
        // paid for with a press that does nothing — press, tooltip,
        // silence, no error, which is exactly what was reported.
        //
        // `clicked` cannot drift: it is one press, one decision, taken
        // from where the overview actually is. `checked` is now kept
        // purely for appearance and is never an input.
        const showApps = this._dock.dash.showAppsButton;
        this._connect(showApps, 'clicked', () => {
            // `visibleTarget` is the state the overview is heading for,
            // so a press during the open/close animation is judged
            // against where it will be, not where it is mid-flight.
            const visible = Main.overview.visibleTarget ?? Main.overview.visible;
            // GNOME's own button IS the app-grid flag: ControlsManager
            // writes it on every state change (overviewControls.js), so
            // it says which page an open overview is on. Reading GNOME's
            // state is not the same mistake as reading our own latch.
            const gnomeButton = Main.overview.dash.showAppsButton;
            switch (showAppsAction({
                overviewVisible: visible,
                appGridShowing: gnomeButton.checked,
            })) {
            case 'open-app-grid':
                Main.overview.showApps();
                break;
            // GNOME's dash is hidden, but its button is still the thing
            // `ControlsManager` listens to, and a hidden actor's
            // properties notify exactly the same.
            case 'show-app-grid':
                gnomeButton.checked = true;
                break;
            case 'show-windows':
                gnomeButton.checked = false;
                break;
            }
        });

        // Appearance only, both ways. Our button should look latched
        // while the grid is up and unlatched otherwise; nothing reads
        // these back, so a missed sync costs a lit pixel, never a press.
        this._connect(Main.overview, 'hidden', () => {
            this._dock.dash.showAppsButton.checked = false;
        });
        this._connect(Main.overview.dash.showAppsButton, 'notify::checked', () => {
            this._dock.dash.showAppsButton.checked =
                Main.overview.dash.showAppsButton.checked;
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
        this._installStateIcons();
    }

    // ---- the prompt (ADR-0035 §2) ---------------------------------------

    /// Give the prompt the keyboard.
    ///
    /// A modal grab, because without one an `St.Entry` in chrome only
    /// receives keystrokes while no window is focused: Mutter routes the
    /// keyboard to the focused window otherwise, and the entry would
    /// take text on an empty desktop and silently drop it everywhere
    /// else. Same mechanism the assistant overlay uses, for the same
    /// reason, in `shell/overlay-extension/extension.js`.
    ///
    /// The cost is the overlay's cost too, and worth naming: a grab eats
    /// every pointer event on the screen, so the click that takes you
    /// away from the prompt is consumed rather than delivered. One click
    /// to leave, and the dock's own icons keep working because they are
    /// inside the grabbed tree.
    _focusPrompt() {
        if (this._promptGrab)
            return;
        this._promptGrab = Main.pushModal(this._dock, {
            actionMode: Shell.ActionMode.NORMAL,
        });
        // Decided by coordinates on the grab actor, not by event target:
        // under a grab the propagation chain is truncated at the grabbed
        // actor and outside presses are retargeted INTO it, so where the
        // press really landed survives only as its screen position.
        this._outsideClickId = this._dock.connect('captured-event', (actor, event) => {
            const type = event.type();
            if (type !== Clutter.EventType.BUTTON_PRESS &&
                type !== Clutter.EventType.TOUCH_BEGIN)
                return Clutter.EVENT_PROPAGATE;
            const [x, y] = event.get_coords();
            const [ax, ay] = this._dock.get_transformed_position();
            const [aw, ah] = this._dock.get_transformed_size();
            if (x < ax || x > ax + aw || y < ay || y > ay + ah) {
                this._releasePrompt();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        this._dock.prompt.grab_key_focus();
    }

    /// Hand the keyboard back to whatever had it.
    _releasePrompt() {
        if (this._outsideClickId) {
            this._dock?.disconnect(this._outsideClickId);
            this._outsideClickId = 0;
        }
        if (!this._promptGrab)
            return;
        Main.popModal(this._promptGrab);
        this._promptGrab = null;
        // The caret has to go too, or the entry keeps a blinking cursor
        // it can no longer type into — a text field that looks like it
        // is listening and is not.
        //
        // IN AN IDLE, and this is not defensive habit: every caller
        // here is inside the entry's own key-press handler, and setting
        // the focus to null from inside event delivery does not stick.
        // Measured on the device (`tests/dock-prompt-smoke.js`) — the
        // synchronous version left the caret in the dock after both
        // Return and Escape, and the smoke run said so twice before
        // this line existed.
        this._focusDropId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._focusDropId = 0;
            // Unless somebody has clicked back into the prompt in the
            // meantime, in which case dropping focus would be taking
            // the keyboard away from a person who just asked for it.
            if (!this._promptGrab)
                global.stage.set_key_focus(null);
            return GLib.SOURCE_REMOVE;
        });
    }

    /// One submission from the bar.
    ///
    /// The keyboard goes back FIRST, whatever the text turns out to
    /// mean: an app about to open wants the focus, and so does the
    /// assistant layer about to appear. A grab still held while another
    /// surface takes the screen is the one failure here that a person
    /// cannot get out of with the mouse.
    _submit(text) {
        this._releasePrompt();
        const route = submission(text, this._appCandidates(text));
        switch (route.kind) {
        case 'launch': {
            const app = Shell.AppSystem.get_default().lookup_app(route.id);
            // Between the lookup that produced the candidate and this
            // line, an app can be uninstalled. Asking is a better
            // failure than nothing happening.
            if (app) {
                app.activate();
                return;
            }
            this._ask(route.prompt);
            return;
        }
        case 'ask':
            this._ask(route.prompt);
            return;
        default:
        }
    }

    /// Installed apps that could answer to this text.
    ///
    /// `Gio.DesktopAppInfo.search` is GNOME's own index — the same one
    /// the overview's app search uses — so an app is findable here
    /// exactly when it is findable there. It returns ranked GROUPS;
    /// only the first is
    /// consulted, because a name that is an exact match is in the first
    /// group by construction and the rest are progressively weaker
    /// guesses that `lib/prompt.js` would refuse anyway.
    _appCandidates(text) {
        const term = String(text ?? '').trim();
        if (term === '')
            return [];
        let groups = [];
        try {
            groups = Gio.DesktopAppInfo.search(term);
        } catch (e) {
            logError(e, 'lisa-desktop: app search failed');
            return [];
        }
        const system = Shell.AppSystem.get_default();
        const candidates = [];
        for (const id of groups[0] ?? []) {
            const app = system.lookup_app(id);
            if (app)
                candidates.push({id, name: app.get_name()});
        }
        return candidates;
    }

    /// Hand the text to the assistant and forget it.
    ///
    /// `dev.lisaos.Overlay1.UI.Summon` — the overlay extension's own
    /// UI surface, which opens the layer and submits. The dock is a
    /// thin frontend on the one headless backend (PLAN §5.7.1): it
    /// runs no inference, holds no query id, renders no token, and
    /// raises no confirmation dialog. That last one is ADR-0035 §4,
    /// which says in as many words that a dock owning the prompt must
    /// not also own consent.
    ///
    /// Asynchronous without exception. A synchronous D-Bus call here
    /// would block the compositor — every window on the machine stops
    /// redrawing — for as long as the assistant took to answer.
    _ask(prompt) {
        Gio.DBus.session.call(
            ASSISTANT_BUS_NAME, ASSISTANT_OBJECT_PATH, ASSISTANT_BUS_NAME,
            ASSISTANT_METHOD,
            new GLib.Variant(ASSISTANT_SIGNATURE, [prompt, {}]),
            null, Gio.DBusCallFlags.NONE, -1, null,
            (connection, res) => {
                try {
                    connection.call_finish(res);
                } catch (e) {
                    // The overlay extension is not running, or its name
                    // is not on the bus. Said out loud rather than
                    // logged: a prompt that swallows what you typed and
                    // shows nothing is the worst version of this.
                    logError(e, 'lisa-desktop: could not reach the assistant');
                    Main.notify('Lisa',
                        'The assistant is not available — the overlay extension is not running.');
                }
            });
    }

    /// State-dependent app icons (#190, lib/stateicon.js): an app that
    /// ships `<icon>-active` in hicolor gets it drawn while RUNNING —
    /// Surfer meditates on the beach until it opens, then it surfs.
    ///
    /// `create_icon_texture` is patched on the prototype because it is
    /// the one funnel every shell surface draws app icons through
    /// (dash, overview grid, alt-tab); painting only the dock would
    /// leave the same app wearing two faces at once.
    _installStateIcons() {
        this._variantCache = new Map();
        const ext = this;
        this._origCreateIcon = Shell.App.prototype.create_icon_texture;
        const orig = this._origCreateIcon;
        Shell.App.prototype.create_icon_texture = function (size) {
            const name = activeIconName(this.get_id());
            if (name && shouldUseActive(this.get_state(), ext._activeVariantExists(name)))
                return new St.Icon({gicon: new Gio.ThemedIcon({name}), icon_size: size});
            return orig.call(this, size);
        };
        // Transient peeks (lib/stateicon.js) never join the running
        // list: a quick-look panel is not an app you are running, and
        // the dash is the only consumer of get_running the user sees.
        this._origGetRunning = Shell.AppSystem.prototype.get_running;
        const origRunning = this._origGetRunning;
        Shell.AppSystem.prototype.get_running = function () {
            return origRunning.call(this).filter(a => !isTransientPeek(a.get_id()));
        };
        // Repaint the dash entry when an app with a variant changes
        // state; other surfaces (grid, alt-tab) rebuild their icons on
        // every open and need no push.
        this._connect(Shell.AppSystem.get_default(), 'app-state-changed', (_s, app) => {
            const name = activeIconName(app.get_id());
            if (!name || !this._activeVariantExists(name)) return;
            for (const item of this._dock?.dash?._box?.get_children() ?? []) {
                const child = item.child;
                if (child?.app === app) child.icon?.update?.();
            }
        });
    }

    /// Does `<name>` exist in hicolor? Answered with file checks
    /// (lib/stateicon.js lists the candidates) because St's icon lookup
    /// cannot say "missing" — it falls back to a generic instead. The
    /// answer is cached per name: it is asked on every icon paint.
    _activeVariantExists(name) {
        if (this._variantCache.has(name)) return this._variantCache.get(name);
        const dirs = [GLib.get_user_data_dir(), ...GLib.get_system_data_dirs()];
        const exists = candidatePaths(name, dirs)
            .some(p => Gio.File.new_for_path(p).query_exists(null));
        this._variantCache.set(name, exists);
        return exists;
    }

    disable() {
        if (this._origCreateIcon) {
            Shell.App.prototype.create_icon_texture = this._origCreateIcon;
            this._origCreateIcon = null;
        }
        if (this._origGetRunning) {
            Shell.AppSystem.prototype.get_running = this._origGetRunning;
            this._origGetRunning = null;
        }
        this._variantCache = null;
        this._restorePanel();
        // BEFORE the dock is destroyed and before its signals go: a
        // modal grab outlives the actor that took it, and a grab nobody
        // holds a reference to is a session with no keyboard.
        this._releasePrompt();
        // AFTER it, because releasing schedules the idle that drops the
        // caret. Cancelled the other way round, this would remove a
        // source that had not been created yet and leave the one that
        // had to run in a disabled extension.
        if (this._focusDropId) {
            GLib.source_remove(this._focusDropId);
            this._focusDropId = 0;
        }
        // A pending later holds a reference to a dock that is about to
        // be destroyed, and runs after `disable()` returns.
        if (this._repositionLater) {
            global.compositor.get_laters().remove(this._repositionLater);
            this._repositionLater = 0;
        }
        this._signals?.forEach(([obj, id]) => obj.disconnect(id));
        this._signals = null;
        this._badges?.clear();
        this._badges = null;

        if (this._badgeSub) {
            Gio.DBus.session.signal_unsubscribe(this._badgeSub);
            this._badgeSub = null;
        }
        if (this._dock) {
            this._dock.clearBadges();
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

    /// Place the dock — on the next before-redraw, never inline.
    ///
    /// Every caller is a `notify::width` / `notify::height` /
    /// `icon-size-changed` handler, and all three fire DURING the layout
    /// pass. Moving an actor from inside its own allocation is what
    /// produces
    ///
    ///     Can't update stage views actor … LisaDock … needs an
    ///     allocation
    ///
    /// which the shell logged 33 times per dash rebuild — the cosmetic
    /// warning #262 recorded and nobody attributed. Measured with the
    /// dock before this change and after, same harness, same provoked
    /// rebuilds: 33 → 0.
    ///
    /// Coalescing is the point as much as the deferral: a dash rebuild
    /// emits several of those signals in one pass and they all want the
    /// same single placement.
    _reposition() {
        if (this._repositionLater)
            return;
        this._repositionLater = global.compositor.get_laters().add(
            Meta.LaterType.BEFORE_REDRAW, () => {
                this._repositionLater = 0;
                this._dock?.reposition(Main.layoutManager.primaryMonitor);
                return GLib.SOURCE_REMOVE;
            });
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
