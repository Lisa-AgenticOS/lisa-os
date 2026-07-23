// Lisa assistant overlay — GNOME Shell frontend (PLAN §5.7.1).
//
// Thin by design: all state and token streams live in the headless
// backend (org.lisa.Overlay1, backend/lisa-overlayd.js); this
// extension renders it. Summon with Super+Shift+Space (Super+Space is
// the Spotlight-style search, §5.7.2) — or programmatically
// via org.lisa.Overlay1.UI (the §5.7.2 launcher's "Ask Lisa" lane
// hands queries over here): a translucent layer over the current
// workspace with the three per-invocation context chips —
// [this window], [selection], [my stuff] — a prompt entry, and the
// streamed answer. Escape cancels/dismisses.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {OVERLAY_IFACE_XML, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH,
    OVERLAY_UI_IFACE_XML, OVERLAY_UI_BUS_NAME, OVERLAY_UI_OBJECT_PATH}
    from './lib/iface.js';

const OverlayProxy = Gio.DBusProxy.makeProxyWrapper(OVERLAY_IFACE_XML);

const CHIPS = [
    // [id, label, hint] — window/selection ship in later layers
    // (§5.7.4 / §5.7.3 layer 3); the backend reports them unavailable.
    ['window', 'this window', 'screen context arrives in M6'],
    ['selection', 'selection', 'selection context arrives with layer 3'],
    ['my_stuff', 'my stuff', 'search your indexed files (ledgered)'],
];

class OverlayWidget extends St.BoxLayout {
    static {
        GObject.registerClass(this);
    }

    constructor(proxy) {
        super({
            style_class: 'lisa-overlay',
            vertical: true,
            reactive: true,
        });
        this._proxy = proxy;
        this._activeQuery = 0;
        this._chipState = {my_stuff: true, window: false, selection: false};

        this._chipRow = new St.BoxLayout({style_class: 'lisa-overlay-chips'});
        this._chips = {};
        for (const [id, label] of CHIPS) {
            const chip = new St.Button({
                style_class: 'lisa-chip',
                label: `[${label}]`,
                toggle_mode: true,
                checked: this._chipState[id],
            });
            chip.connect('notify::checked', () => {
                this._chipState[id] = chip.checked;
            });
            this._chips[id] = chip;
            this._chipRow.add_child(chip);
        }
        this.add_child(this._chipRow);

        this._entry = new St.Entry({
            style_class: 'lisa-overlay-entry',
            hint_text: 'Ask Lisa…',
            can_focus: true,
        });
        this._entry.clutter_text.connect('activate', () => this._ask());
        this.add_child(this._entry);

        this._scroll = new St.ScrollView({style_class: 'lisa-overlay-scroll'});
        this._response = new St.Label({style_class: 'lisa-overlay-response'});
        this._response.clutter_text.line_wrap = true;
        const box = new St.BoxLayout({vertical: true});
        box.add_child(this._response);
        this._scroll.add_child(box);
        this.add_child(this._scroll);

        this._footer = new St.Label({style_class: 'lisa-overlay-footer', text: ''});
        this.add_child(this._footer);

        this._signalIds = [
            this._proxy.connectSignal('Started', (p, s, args) => this._onStarted(...args)),
            this._proxy.connectSignal('Token', (p, s, args) => this._onToken(...args)),
            this._proxy.connectSignal('Finished', (p, s, args) => this._onFinished(...args)),
        ];
    }

    focusEntry() {
        this._entry.grab_key_focus();
    }

    // --- summon support (org.lisa.Overlay1.UI) ---------------------------

    setPrompt(text) {
        this._entry.set_text(text);
        this._entry.grab_key_focus();
        this._entry.clutter_text.set_cursor_position(-1);
    }

    applyChips(state) {
        for (const [id, checked] of Object.entries(state)) {
            if (!(id in this._chipState))
                continue;
            this._chipState[id] = checked;
            this._chips[id]?.set_checked(checked);
        }
    }

    submit() {
        this._ask();
    }

    cancelActive() {
        if (this._activeQuery === 0)
            return false;
        this._proxy.CancelRemote(this._activeQuery, () => {});
        return true;
    }

    _ask() {
        const prompt = this._entry.get_text().trim();
        if (prompt === '')
            return;
        this._response.text = '';
        this._footer.text = '…';
        const options = {};
        for (const id of Object.keys(this._chipState))
            options[id] = GLib.Variant.new_boolean(this._chipState[id]);
        this._proxy.AskRemote(prompt, options, ([queryId], error) => {
            if (error) {
                this._footer.text = `backend unavailable: ${error.message}`;
                return;
            }
            this._activeQuery = Number(queryId);
        });
    }

    _onStarted(queryId, metaJson) {
        if (Number(queryId) !== this._activeQuery)
            return;
        let meta = {sources: [], unavailable: []};
        try {
            meta = JSON.parse(metaJson);
        } catch {}
        const parts = [];
        if (meta.sources.length > 0)
            parts.push(`context: ${meta.sources.map(s => `${s.provenance} ${s.source}`).join(', ')}`);
        if (meta.unavailable.length > 0)
            parts.push(`unavailable: ${meta.unavailable.join(', ')}`);
        parts.push('ledgered');
        this._footer.text = parts.join(' · ');
    }

    _onToken(queryId, text) {
        if (Number(queryId) !== this._activeQuery)
            return;
        this._response.text += text;
    }

    _onFinished(queryId, status, detail) {
        if (Number(queryId) !== this._activeQuery)
            return;
        this._activeQuery = 0;
        if (status === 'error')
            this._footer.text = `error: ${detail}`;
        else if (status === 'cancelled')
            this._footer.text = 'cancelled';
    }

    destroy() {
        for (const id of this._signalIds)
            this._proxy.disconnectSignal(id);
        this._signalIds = [];
        super.destroy();
    }
}

export default class LisaOverlayExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._proxy = null;
        this._overlay = null;
        this._grab = null;

        Main.wm.addKeybinding(
            'toggle-overlay',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._toggle());

        // UI-control surface (org.lisa.Overlay1.UI): other shell
        // surfaces — the §5.7.2 launcher's "Ask Lisa" lane — summon
        // this overlay with a prompt. Owned by the frontend because
        // the headless backend has no UI; the wlr client can own the
        // same name.
        this._uiImpl = Gio.DBusExportedObject.wrapJSObject(OVERLAY_UI_IFACE_XML, {
            Summon: (prompt, options) => this._summon(prompt, options),
            Hide: () => this._hide(),
            GetVisible: () => this._overlay !== null,
        });
        this._uiImpl.export(Gio.DBus.session, OVERLAY_UI_OBJECT_PATH);
        this._uiOwnerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, OVERLAY_UI_BUS_NAME,
            Gio.BusNameOwnerFlags.NONE, null, null);
    }

    disable() {
        Main.wm.removeKeybinding('toggle-overlay');
        if (this._uiOwnerId) {
            Gio.bus_unown_name(this._uiOwnerId);
            this._uiOwnerId = 0;
        }
        if (this._uiImpl) {
            this._uiImpl.unexport();
            this._uiImpl = null;
        }
        this._hide();
        this._proxy = null;
        this._settings = null;
    }

    _ensureProxy() {
        if (!this._proxy) {
            // D-Bus activation starts lisa-overlayd on first use.
            this._proxy = new OverlayProxy(
                Gio.DBus.session, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH);
        }
        return this._proxy;
    }

    _toggle() {
        if (this._overlay)
            this._hide();
        else
            this._show();
    }

    // org.lisa.Overlay1.UI.Summon: show the layer (if hidden), preset
    // any chip toggles the caller passed, and submit a non-empty
    // prompt straight away — a live stream is replaced, matching the
    // launcher's "new query wins" behavior. Empty prompt = plain show.
    _summon(prompt, options) {
        if (!this._overlay)
            this._show();
        const chips = {};
        for (const key of ['my_stuff', 'window', 'selection']) {
            const v = options?.[key];
            if (v === undefined)
                continue;
            chips[key] = v instanceof GLib.Variant ? v.recursiveUnpack() : Boolean(v);
        }
        this._overlay.applyChips(chips);
        this._overlay.setPrompt(prompt);
        if (prompt.trim() !== '') {
            this._overlay.cancelActive();
            this._overlay.submit();
        }
    }

    _show() {
        const monitor = Main.layoutManager.primaryMonitor;
        this._overlay = new OverlayWidget(this._ensureProxy());
        this._overlay.set_width(Math.min(680, monitor.width - 80));
        Main.layoutManager.addTopChrome(this._overlay);
        this._overlay.set_position(
            monitor.x + Math.floor((monitor.width - this._overlay.width) / 2),
            monitor.y + Math.floor(monitor.height / 6));

        this._grab = Main.pushModal(this._overlay, {
            actionMode: Shell.ActionMode.NORMAL,
        });
        this._keyPressId = this._overlay.connect('key-press-event', (actor, event) => {
            if (event.get_key_symbol() === Clutter.KEY_Escape) {
                // First Escape cancels a live stream, second dismisses.
                if (!this._overlay.cancelActive())
                    this._hide();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        this._overlay.focusEntry();
    }

    _hide() {
        if (!this._overlay)
            return;
        if (this._keyPressId) {
            this._overlay.disconnect(this._keyPressId);
            this._keyPressId = 0;
        }
        if (this._grab) {
            Main.popModal(this._grab);
            this._grab = null;
        }
        this._overlay.destroy();
        this._overlay = null;
    }
}
