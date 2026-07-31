// Lisa assistant overlay — GNOME Shell frontend (PLAN §5.7.1).
//
// Thin by design: all state and token streams live in the headless
// backend (dev.lisaos.Overlay1, backend/lisa-overlayd.js); this
// extension renders it. Summon with Super+Shift+Space (Super+Space is
// the Spotlight-style search, §5.7.2; Super+C opens the persistent
// Lisa Assistant chat window, ADR-0015) — or programmatically
// via dev.lisaos.Overlay1.UI (the §5.7.2 launcher's "Ask Lisa" lane
// hands queries over here): a translucent layer over the current
// workspace with the three per-invocation context chips —
// [this window], [selection], [my stuff] — a prompt entry, and the
// streamed answer. Escape cancels/dismisses.
//
// Agent Bus lane (M5, ADR-0013): actionable prompts execute via
// dev.lisaos.Agent1 in the backend; when a call parks for consent the
// backend raises ConfirmationNeeded and this layer renders it — a
// chip-weight box for confirm-chip, a heavier modal-weight box for
// confirm-modal (escalated/untrusted chains and destructive tiers call
// out their warnings) — and answers via Respond(). Confirmations
// parked by other clients (e.g. `lisa do`) surface in the same queue.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {consentView} from './lib/agent.js';
import {OVERLAY_IFACE_XML, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH,
    OVERLAY_UI_IFACE_XML, OVERLAY_UI_BUS_NAME, OVERLAY_UI_OBJECT_PATH,
    VOICE_IFACE_XML, VOICE_BUS_NAME, VOICE_OBJECT_PATH}
    from './lib/iface.js';

const OverlayProxy = Gio.DBusProxy.makeProxyWrapper(OVERLAY_IFACE_XML);
const VoiceProxy = Gio.DBusProxy.makeProxyWrapper(VOICE_IFACE_XML);

// How long the hands-free lane listens before stopping itself. Long
// enough for a sentence, short enough that a forgotten session is not an
// open microphone. Push-to-talk does not use this — it ends when the key
// does — and the Voice1 service keeps a much longer stuck-key backstop
// underneath, which is a safety net rather than an interaction.
const HANDS_FREE_SECONDS = 12;

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

        // Consent surface (Agent Bus lane): one pending confirmation at
        // a time, chip weight or modal weight per the spec's
        // confirmation level; further requests queue behind it.
        this._consents = []; // [{queryId, view}]
        this._consentBox = new St.BoxLayout({
            style_class: 'lisa-consent',
            vertical: true,
            visible: false,
        });
        this._consentTitle = new St.Label({style_class: 'lisa-consent-title'});
        this._consentBox.add_child(this._consentTitle);
        this._consentDescription = new St.Label({style_class: 'lisa-consent-description'});
        this._consentDescription.clutter_text.line_wrap = true;
        this._consentBox.add_child(this._consentDescription);
        this._consentArgs = new St.Label({style_class: 'lisa-consent-args'});
        this._consentArgs.clutter_text.line_wrap = true;
        this._consentBox.add_child(this._consentArgs);
        this._consentWarnings = new St.Label({style_class: 'lisa-consent-warnings'});
        this._consentBox.add_child(this._consentWarnings);
        const buttons = new St.BoxLayout({style_class: 'lisa-consent-buttons'});
        this._allowButton = new St.Button({
            style_class: 'lisa-consent-allow', label: 'Allow', can_focus: true,
        });
        this._allowButton.connect('clicked', () => this._answerConsent(true));
        buttons.add_child(this._allowButton);
        this._denyButton = new St.Button({
            style_class: 'lisa-consent-deny', label: 'Deny', can_focus: true,
        });
        this._denyButton.connect('clicked', () => this._answerConsent(false));
        buttons.add_child(this._denyButton);
        this._consentBox.add_child(buttons);
        this.add_child(this._consentBox);

        this._footer = new St.Label({style_class: 'lisa-overlay-footer', text: ''});
        this.add_child(this._footer);

        this._signalIds = [
            this._proxy.connectSignal('Started', (p, s, args) => this._onStarted(...args)),
            this._proxy.connectSignal('Token', (p, s, args) => this._onToken(...args)),
            this._proxy.connectSignal('ConfirmationNeeded',
                (p, s, args) => this._onConfirmationNeeded(...args)),
            this._proxy.connectSignal('Finished', (p, s, args) => this._onFinished(...args)),
        ];
    }

    focusEntry() {
        this._entry.grab_key_focus();
    }

    // --- summon support (dev.lisaos.Overlay1.UI) ---------------------------

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
        // A new query replaces the live one ("new query wins", matching
        // the launcher); its parked consent goes with it. Consents for
        // other clients' calls (external query ids) stay up.
        if (this._activeQuery !== 0)
            this._dropConsents(id => id === this._activeQuery);
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
        if (meta.mode === 'agent' && meta.tool)
            parts.push(`agent: ${meta.tool}`);
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

    // ConfirmationNeeded carries Agent1's spec_json (tool, args, tiers,
    // escalation, chain) for a parked call — ours or another client's.
    _onConfirmationNeeded(queryId, specJson) {
        this._consents.push({queryId: Number(queryId), view: consentView(specJson)});
        if (this._consents.length === 1)
            this._renderConsent();
    }

    _renderConsent() {
        const front = this._consents[0];
        if (!front) {
            this._consentBox.hide();
            return;
        }
        const {view} = front;
        this._consentBox.style_class =
            `lisa-consent ${view.modal ? 'lisa-consent-modal' : 'lisa-consent-chip'}`;
        this._consentTitle.text = view.title;
        this._consentDescription.text = view.description;
        this._consentDescription.visible = view.description !== '';
        this._consentArgs.text = view.argsText;
        this._consentArgs.visible = view.argsText !== '';
        this._consentWarnings.text = view.warnings.join(' · ');
        this._consentWarnings.visible = view.warnings.length > 0;
        this._allowButton.set_reactive(true);
        this._denyButton.set_reactive(true);
        this._consentBox.show();
    }

    _answerConsent(approve) {
        const front = this._consents[0];
        if (!front)
            return;
        // One click only; the box closes on Finished for this query.
        this._allowButton.set_reactive(false);
        this._denyButton.set_reactive(false);
        this._proxy.RespondRemote(front.queryId, approve, (res, error) => {
            if (error) {
                this._dropConsents(id => id === front.queryId);
                this._footer.text = `confirm failed: ${error.message}`;
            }
        });
    }

    // Finished closes a consent (any lane) and reports external
    // outcomes — the active query's own outcome is reported by the
    // main branch below.
    _dropConsents(match, outcome = null) {
        const frontId = this._consents[0]?.queryId;
        const dropped = this._consents.filter(c => match(c.queryId));
        if (dropped.length === 0)
            return;
        this._consents = this._consents.filter(c => !match(c.queryId));
        if (frontId !== this._consents[0]?.queryId)
            this._renderConsent();
        if (outcome && dropped.some(c => c.queryId !== this._activeQuery)) {
            const [status, detail] = outcome;
            if (status === 'executed')
                this._footer.text = 'done · ledgered';
            else if (status === 'denied')
                this._footer.text = 'denied';
            else if (status === 'failed' || status === 'error')
                this._footer.text = `${status}: ${detail}`;
        }
    }

    _onFinished(queryId, status, detail) {
        const id = Number(queryId);
        this._dropConsents(c => c === id, [status, detail]);
        if (id !== this._activeQuery)
            return;
        this._activeQuery = 0;
        if (status === 'error')
            this._footer.text = `error: ${detail}`;
        else if (status === 'cancelled')
            this._footer.text = 'cancelled';
        else if (status === 'executed')
            this._footer.text = detail === '' ? 'done · ledgered' : `done · ledger #${detail}`;
        else if (status === 'failed')
            this._footer.text = `action failed: ${detail}`;
        else if (status === 'denied')
            this._footer.text = `denied: ${detail}`;
    }

    destroy() {
        for (const id of this._signalIds)
            this._proxy.disconnectSignal(id);
        this._signalIds = [];
        this._consents = [];
        super.destroy();
    }
}

export default class LisaOverlayExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._proxy = null;
        this._overlay = null;
        this._grab = null;
        // Push-to-talk state (§5.7.5). 0 means "not recording", which
        // is also what a session id can never be.
        this._voiceProxy = null;
        this._voiceSignals = null;
        this._talkSession = 0;
        this._talkWatch = 0;
        this._talkKey = 0;
        this._handsFreeTimer = 0;
        this._listenOsd = null;

        Main.wm.addKeybinding(
            'toggle-overlay',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._toggle());

        // A dedicated hotkey for the persistent chat window (ADR-0015):
        // open it, or focus it if already running (it's single-instance).
        Main.wm.addKeybinding(
            'open-assistant',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._openAssistant());

        // Push-to-talk (§5.7.5). IGNORE_AUTOREPEAT matters: holding a
        // key repeats it, and without this every repeat would fire the
        // handler and try to start a second recording on the same
        // microphone.
        Main.wm.addKeybinding(
            'push-to-talk',
            this._settings,
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._startTalking());

        // UI-control surface (dev.lisaos.Overlay1.UI): other shell
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
        Main.wm.removeKeybinding('open-assistant');
        Main.wm.removeKeybinding('push-to-talk');
        this._stopTalking();
        if (this._voiceSignals && this._voiceProxy) {
            for (const id of this._voiceSignals)
                this._voiceProxy.disconnectSignal(id);
        }
        this._voiceSignals = null;
        this._voiceProxy = null;
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

    // ---- push-to-talk (§5.7.5) ----
    //
    // `addKeybinding` fires on press and never tells us about release,
    // so on its own it can only build a TOGGLE: press to start, press
    // to stop. That is a different gesture with a different failure —
    // walking away from a machine that is still recording — so the
    // release is picked up separately, by watching stage events for the
    // key going up while a recording is running.
    //
    // The watcher is installed only for the duration of a recording.
    // A permanent capture handler on the stage sees every keystroke on
    // the system, which is not a thing to leave running for a feature
    // that is idle almost always.

    _voiceProxy_() {
        if (!this._voiceProxy) {
            this._voiceProxy = new VoiceProxy(
                Gio.DBus.session, VOICE_BUS_NAME, VOICE_OBJECT_PATH);
            this._voiceSignals = [
                this._voiceProxy.connectSignal('Transcribed',
                    (p, sender, [id, text]) => this._onTranscribed(id, text)),
                this._voiceProxy.connectSignal('Failed',
                    (p, sender, [id, reason]) => this._onVoiceFailed(id, reason)),
            ];
        }
        return this._voiceProxy;
    }

    _startTalking() {
        if (this._talkSession)
            return;
        let id;
        try {
            id = this._voiceProxy_().StartListeningSync()[0];
        } catch (e) {
            // The common causes are actionable and worth naming: no
            // recorder installed, or the backend could not start one.
            Main.notify('Lisa', `Cannot listen: ${e.message}`);
            return;
        }
        this._talkSession = id;
        this._showListening();

        // Watch for the key coming back up. The event is consumed only
        // when it is OUR key: swallowing anything else would eat
        // keystrokes for every other application while Lisa listens.
        const event = Clutter.get_current_event();
        this._talkKey = event ? event.get_key_symbol() : 0;
        this._talkWatch = global.stage.connect('captured-event', (actor, ev) => {
            if (ev.type() !== Clutter.EventType.KEY_RELEASE)
                return Clutter.EVENT_PROPAGATE;
            if (this._talkKey && ev.get_key_symbol() !== this._talkKey)
                return Clutter.EVENT_PROPAGATE;
            this._stopTalking();
            return Clutter.EVENT_PROPAGATE;
        });
    }

    _stopTalking() {
        if (this._handsFreeTimer) {
            GLib.source_remove(this._handsFreeTimer);
            this._handsFreeTimer = 0;
        }
        if (this._talkWatch) {
            global.stage.disconnect(this._talkWatch);
            this._talkWatch = 0;
        }
        const id = this._talkSession;
        this._talkSession = 0;
        this._talkKey = 0;
        this._hideListening();
        if (!id)
            return;
        try {
            this._voiceProxy_().StopListeningSync(id);
        } catch (e) {
            logError(e, 'push-to-talk: could not stop the recording');
        }
    }

    _onTranscribed(id, text) {
        this._hideListening();
        if (!text) {
            // Silence is not a question. Saying so beats summoning an
            // empty overlay, which reads as the key not having worked.
            Main.notify('Lisa', 'Did not catch that.');
            return;
        }
        // Straight into the overlay with the text already submitted —
        // the same handoff the launcher's "Ask Lisa" lane performs.
        this._summon(text, {});
    }

    // Escape while listening cancels the recording as well as dismissing
    // the layer. Without this the overlay closes and the microphone
    // stays open behind it — an indicator on a surface nobody is looking
    // at is not an indicator.
    _cancelListening() {
        if (this._talkSession) {
            const id = this._talkSession;
            this._stopTalking_noSubmit(id);
            return true;
        }
        return false;
    }

    _stopTalking_noSubmit(id) {
        if (this._handsFreeTimer) {
            GLib.source_remove(this._handsFreeTimer);
            this._handsFreeTimer = 0;
        }
        if (this._talkWatch) {
            global.stage.disconnect(this._talkWatch);
            this._talkWatch = 0;
        }
        this._talkSession = 0;
        this._talkKey = 0;
        this._hideListening();
        try {
            this._voiceProxy_().CancelSync(id);
        } catch (e) {
            logError(e, 'could not cancel the recording');
        }
    }

    _onVoiceFailed(id, reason) {
        this._hideListening();
        Main.notify('Lisa', `Voice: ${reason}`);
    }

    // A recording that is running must be visible. An invisible open
    // microphone is the single worst thing this feature could ship.
    _showListening() {
        if (this._listenOsd)
            return;
        this._listenOsd = new St.BoxLayout({
            style_class: 'lisa-listening',
            vertical: false,
            reactive: false,
        });
        this._listenOsd.add_child(new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'lisa-listening-icon',
        }));
        this._listenOsd.add_child(new St.Label({
            text: 'Listening…',
            y_align: Clutter.ActorAlign.CENTER,
        }));
        Main.layoutManager.addTopChrome(this._listenOsd);
        const mon = Main.layoutManager.primaryMonitor;
        this._listenOsd.set_position(
            mon.x + Math.floor((mon.width - this._listenOsd.width) / 2),
            mon.y + Math.floor(mon.height * 0.8));
    }

    _hideListening() {
        if (this._listenOsd) {
            this._listenOsd.destroy();
            this._listenOsd = null;
        }
    }

    // Open the persistent chat window, or raise it if already running
    // (app.lisaos.Assistant is single-instance, so activate() focuses it).
    _openAssistant() {
        const app = Shell.AppSystem.get_default()
            .lookup_app('app.lisaos.Assistant.desktop');
        if (app)
            app.activate();
        else
            Main.notify('Lisa Assistant', 'The assistant app is not installed.');
    }

    _toggle() {
        if (this._overlay)
            this._hide();
        else
            this._show();
    }

    // dev.lisaos.Overlay1.UI.Summon: show the layer (if hidden), preset
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
        // Hands-free lane (§5.7.5): summon AND open the microphone, the
        // shape people know from Siri. Double-tap-Shift asks for this;
        // the launcher's "Ask Lisa" hand-off does not, so it stays opt-in
        // per call rather than becoming what Summon always does.
        const listen = options?.listen;
        const wants = listen instanceof GLib.Variant
            ? listen.recursiveUnpack() : Boolean(listen);
        if (wants)
            this._listenHandsFree();
    }

    // Listening without a key held down. Push-to-talk ends when the key
    // comes up; this has no key, so it needs its own ending — and an
    // open microphone with no defined end is the thing this feature must
    // never be. Three of them: a ceiling, Escape, and a second
    // double-tap.
    //
    // The ceiling lives here rather than in the Voice1 service because
    // it is a UI decision. The service keeps its own much longer
    // stuck-key backstop, which is a safety net, not an interaction.
    _listenHandsFree() {
        if (this._talkSession) {
            // A second double-tap while listening means "stop", the same
            // way pressing the key again does on a phone.
            this._stopTalking();
            return;
        }
        let id;
        try {
            id = this._voiceProxy_().StartListeningSync()[0];
        } catch (e) {
            Main.notify('Lisa', `Cannot listen: ${e.message}`);
            return;
        }
        this._talkSession = id;
        this._showListening();
        this._handsFreeTimer = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT, HANDS_FREE_SECONDS, () => {
                this._handsFreeTimer = 0;
                if (this._talkSession === id)
                    this._stopTalking();
                return GLib.SOURCE_REMOVE;
            });
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
                // Listening first: closing the layer while the recording
                // continued would leave an open microphone behind a
                // surface nobody is looking at, and the indicator would
                // go with it. Then a live stream, then dismiss.
                if (this._cancelListening())
                    return Clutter.EVENT_STOP;
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
        // The layer going away must take the microphone with it, by
        // whatever route it went — Escape, Hide() over D-Bus, the
        // extension being disabled. Anything that can close this surface
        // and leave a recording running is an open microphone with no
        // indicator, which is the one outcome this feature may not have.
        this._cancelListening();
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
