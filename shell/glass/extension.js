// Lisa Glass — the frosted half of the glass, done where it is possible.
//
// An app can make its window TRANSPARENT (plain alpha compositing, which
// works today) but it cannot BLUR what is behind that window: the app
// never sees the desktop under its own surface. That is
// GNOME/mutter#3023, open, and its own status says the way forward is to
// finalize a Wayland protocol first.
//
// The compositor has no such problem — it owns the wallpaper and every
// window actor. We fork the Shell (ADR-0038), so this is ours to do.
//
// Technique, the same one Blur My Shell uses: clone the background,
// blur the clone, and place it UNDER the target window clipped to that
// window's frame. The window's transparent regions then show blurred
// wallpaper instead of sharp wallpaper.
//
// API checked against /usr/lib/gnome-shell/Shell-18.typelib rather than
// copied from a tutorial: the property is `radius`
// (shell_blur_effect_set_radius), NOT `sigma` — which is what the
// obvious search result says, and would have been silently ignored.

import Shell from 'gi://Shell';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/// Lisa's own surfaces only. Blurring behind every window on the system
/// is a different product decision and a much larger performance bill.
const GLASSY = ['notes', 'surfer', 'mail', 'lisaos'];

const RADIUS = 24;
const BRIGHTNESS = 1.0;

export default class LisaGlass {
    enable() {
        this._backers = new Map();
        this._ids = [];
        const wm = global.window_manager;
        for (const sig of ['map', 'destroy', 'size-changed']) {
            this._ids.push([wm, wm.connect(sig, (_wm, actor) => {
                if (sig === 'destroy')
                    this._drop(actor);
                else if (sig === 'map')
                    this._add(actor);
                else
                    this._fit(actor);
            })]);
        }
        for (const actor of global.get_window_actors())
            this._add(actor);
        // Stacking is not a fact you can assert once (#318): the backer
        // is inserted below its window at map, but every raise, lower,
        // new map and workspace switch reorders window_group, and a
        // full-stage clone that drifts ABOVE a window paints blurred
        // wallpaper over that window's content — seen on the device as
        // Mail rendering with its sidebar and headers "missing". So the
        // pin is re-applied on every restack the display reports.
        this._ids.push([global.display, global.display.connect('restacked', () => {
            for (const [actor, entry] of this._backers ?? []) {
                const parent = actor.get_parent();
                if (parent && entry.bg.get_parent() === parent)
                    parent.set_child_below_sibling(entry.bg, actor);
            }
        })]);
        log('lisa-glass: enabled');
    }

    /// The stale-clip failure (#314): a full-stage clone kept invisible
    /// only by its clip is a visible patch ANYWHERE on screen the moment
    /// the clip stops tracking the window. So the clip follows every way
    /// a frame rect can change — including a drag, which the first
    /// version did not watch — and the backer is hidden whenever the
    /// window actor is (minimise, workspace switch), so even a bug in
    /// the geometry path fails invisible rather than painting wallpaper
    /// over someone's work.
    _track(actor, bg) {
        const win = actor.meta_window;
        const ids = [
            [win, win.connect('position-changed', () => this._fit(actor))],
            [win, win.connect('size-changed', () => this._fit(actor))],
            [actor, actor.connect('notify::visible', () => {
                bg.visible = actor.visible;
            })],
        ];
        bg.visible = actor.visible;
        return ids;
    }

    disable() {
        for (const [obj, id] of this._ids ?? [])
            obj.disconnect(id);
        this._ids = [];
        for (const actor of [...(this._backers?.keys() ?? [])])
            this._drop(actor);
        this._backers = null;
    }

    _wanted(actor) {
        const win = actor?.meta_window;
        if (!win || win.window_type !== Meta.WindowType.NORMAL)
            return false;
        const cls = `${win.get_gtk_application_id() ?? ''} ${win.get_wm_class() ?? ''}`.toLowerCase();
        return GLASSY.some((k) => cls.includes(k));
    }

    _add(actor) {
        if (!this._backers || this._backers.has(actor) || !this._wanted(actor))
            return;
        try {
            // The wallpaper as the Shell draws it. Cloning the group
            // rather than one monitor's actor keeps this correct on a
            // multi-monitor desk without tracking which monitor a
            // window is on.
            const source = Main.layoutManager._backgroundGroup;
            if (!source)
                return;
            const bg = new Clutter.Clone({source, reactive: false});
            bg.add_effect(new Shell.BlurEffect({
                radius: RADIUS,
                brightness: BRIGHTNESS,
                mode: Shell.BlurMode.ACTOR,
            }));
            const parent = actor.get_parent();
            if (!parent) {
                bg.destroy();
                return;
            }
            parent.insert_child_below(bg, actor);
            this._backers.set(actor, {bg, ids: this._track(actor, bg)});
            this._fit(actor);
        } catch (e) {
            logError(e, 'lisa-glass: could not back this window');
        }
    }

    _fit(actor) {
        const bg = this._backers?.get(actor)?.bg;
        const r = actor?.meta_window?.get_frame_rect?.();
        if (!bg || !r)
            return;
        // The clone covers the whole stage so the wallpaper lines up
        // 1:1; the CLIP is what confines the frost to this window.
        bg.set_position(0, 0);
        bg.set_size(global.stage.width, global.stage.height);
        bg.set_clip(r.x, r.y, r.width, r.height);
    }

    _drop(actor) {
        const entry = this._backers?.get(actor);
        if (!entry)
            return;
        for (const [obj, id] of entry.ids)
            obj.disconnect(id);
        entry.bg.get_parent()?.remove_child(entry.bg);
        entry.bg.destroy();
        this._backers.delete(actor);
    }
}
