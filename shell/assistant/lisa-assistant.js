#!/usr/bin/env -S gjs -m
// Lisa Assistant — the persistent chat window (this session's ADR; PLAN
// §5.7.1). A second thin frontend of the org.lisa.Overlay1 backend: it sends
// a multi-turn chat Ask (lane:"chat") and renders the streamed Token signals,
// exactly as the transient overlay does — but with history, a model picker
// (local + cloud), and an egress marker on turns that leave the machine.
//
// Models: local from lisa-inferenced `GET /v1/models`; cloud from
// org.lisa.Remote1 (providers that are signed in or hold a key → their
// ListModels). Cloud turns route as `remote:<provider>:<model>` and are
// ledgered `remote.*` by the broker. This app renders; the daemons enforce.

import Adw from 'gi://Adw?version=1';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
import Soup from 'gi://Soup?version=3.0';

import {
    OVERLAY_IFACE_XML, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH,
} from '../overlay-extension/lib/iface.js';
import {
    parseLocalModels, usableProviders, cloudEntries, mergeModelList,
    historyPayload, isRemote,
} from './lib/model.js';

Gio._promisify(Soup.Session.prototype, 'send_and_read_async');
Gio._promisify(Gio.DBusConnection.prototype, 'call');

const INFERENCED_URL =
    GLib.getenv('LISA_INFERENCED_URL') ?? 'http://127.0.0.1:7778';
const REMOTED_NAME = 'org.lisa.Remoted';        // well-known name (≠ iface)
const REMOTED_PATH = '/org/lisa/Remote1';
const REMOTED_IFACE = 'org.lisa.Remote1';
const EGRESS_COLOR = '#E66100';                 // the Ledger "leaves" colour

const OverlayProxy = Gio.DBusProxy.makeProxyWrapper(OVERLAY_IFACE_XML);

class AssistantWindow {
    constructor(app) {
        this._turns = [];       // {role, text, widget, body}
        this._models = [];      // {id, label, kind, provider?}
        this._model = null;     // selected model id
        this._activeQid = null; // in-flight query id
        this._current = null;   // the streaming assistant turn

        this._http = new Soup.Session();
        this.window = new Adw.ApplicationWindow({
            application: app,
            title: 'Lisa Assistant',
            default_width: 720,
            default_height: 760,
        });

        const header = new Adw.HeaderBar();
        this._modelDrop = new Gtk.DropDown({
            model: Gtk.StringList.new(['Loading models…']),
            tooltip_text: 'Model — local runs here, cloud leaves the machine',
        });
        this._modelDrop.connect('notify::selected', () => this._onModelPicked());
        header.pack_start(this._modelDrop);

        const clear = Gtk.Button.new_from_icon_name('document-new-symbolic');
        clear.tooltip_text = 'New conversation';
        clear.connect('clicked', () => this._reset());
        header.pack_end(clear);

        // Conversation.
        this._log = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 10,
            margin_top: 12, margin_bottom: 12, margin_start: 12, margin_end: 12,
        });
        this._scroll = new Gtk.ScrolledWindow({vexpand: true, child: this._log});

        // Composer.
        this._entry = new Gtk.Entry({
            hexpand: true, placeholder_text: 'Message Lisa…',
        });
        this._entry.connect('activate', () => this._send());
        this._sendBtn = new Gtk.Button({
            label: 'Send', css_classes: ['suggested-action'],
        });
        this._sendBtn.connect('clicked', () => this._send());
        const composer = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL, spacing: 6,
            margin_top: 6, margin_bottom: 12, margin_start: 12, margin_end: 12,
        });
        composer.append(this._entry);
        composer.append(this._sendBtn);

        const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        box.append(this._scroll);
        box.append(composer);

        const view = new Adw.ToolbarView({content: box});
        view.add_top_bar(header);
        this.window.set_content(view);

        this._connectBackend();
        this._loadModels().catch(e => logError(e, 'model list'));
        this._systemNote('Ask a local model, or sign in to a cloud provider ' +
            'in Settings → Intelligence for Claude / GPT.');
    }

    // ---- backend (org.lisa.Overlay1) -----------------------------------

    _connectBackend() {
        try {
            this._overlay = OverlayProxy(Gio.DBus.session,
                OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH);
            this._overlay.connectSignal('Token',
                (_p, _s, [qid, text]) => this._onToken(Number(qid), text));
            this._overlay.connectSignal('Finished',
                (_p, _s, [qid, status, detail]) =>
                    this._onFinished(Number(qid), status, detail));
        } catch (e) {
            this._overlay = null;
            this._systemNote(`Assistant backend unavailable: ${e.message}`);
        }
    }

    _onToken(qid, text) {
        if (qid !== this._activeQid || !this._current)
            return;
        this._current.text += text;
        this._current.body.label = this._current.text;
        this._scrollToBottom();
    }

    _onFinished(qid, status, detail) {
        if (qid !== this._activeQid)
            return;
        if (this._current && !['ok', 'executed', 'cancelled'].includes(status)) {
            const why = detail || status;
            this._current.text = this._current.text
                ? `${this._current.text}\n\n⚠ ${why}` : `⚠ ${why}`;
            this._current.body.label = this._current.text;
            this._current.body.add_css_class('error');
        }
        this._activeQid = null;
        this._current = null;
        this._setBusy(false);
        this._scrollToBottom();
    }

    // ---- sending -------------------------------------------------------

    _send() {
        const prompt = this._entry.text.trim();
        if (prompt === '' || this._activeQid !== null || !this._overlay)
            return;
        if (!this._model) {
            this._systemNote('Pick a model first.');
            return;
        }
        const history = historyPayload(this._turns);
        this._entry.text = '';
        this._addTurn('user', prompt);
        this._current = this._addTurn('assistant', '', this._model);
        this._setBusy(true);

        const options = {
            lane: GLib.Variant.new_string('chat'),
            model_hint: GLib.Variant.new_string(this._model),
            history_json: GLib.Variant.new_string(JSON.stringify(history)),
        };
        // Sync so the query id is set before any Token signal is dispatched
        // (the main loop can't deliver a signal until this returns).
        try {
            const [qid] = this._overlay.AskSync(prompt, options);
            this._activeQid = Number(qid);
        } catch (e) {
            this._onFinished(-1, 'error', e.message);
        }
    }

    // ---- conversation widgets ------------------------------------------

    _addTurn(role, text, model) {
        const isUser = role === 'user';
        const card = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 2,
            halign: isUser ? Gtk.Align.END : Gtk.Align.START,
            css_classes: ['card'],
            margin_start: isUser ? 48 : 0, margin_end: isUser ? 0 : 48,
        });
        const heading = new Gtk.Label({
            xalign: 0, css_classes: ['caption', 'dim-label'],
            margin_top: 6, margin_start: 10, margin_end: 10,
            use_markup: true,
            label: isUser ? 'You' : this._assistantHeading(model),
        });
        const body = new Gtk.Label({
            xalign: 0, wrap: true, selectable: true, label: text,
            margin_bottom: 8, margin_start: 10, margin_end: 10, margin_top: 2,
        });
        card.append(heading);
        card.append(body);
        this._log.append(card);
        const turn = {role, text, widget: card, body};
        this._turns.push(turn);
        this._scrollToBottom();
        return turn;
    }

    _assistantHeading(model) {
        if (!model)
            return 'Lisa';
        if (isRemote(model)) {
            const label = this._models.find(m => m.id === model)?.label ?? model;
            return `${GLib.markup_escape_text(label, -1)} · ` +
                `<span foreground="${EGRESS_COLOR}">leaves this machine</span>`;
        }
        const label = this._models.find(m => m.id === model)?.label ?? model;
        return `${GLib.markup_escape_text(label, -1)} · stays on this machine`;
    }

    _systemNote(text) {
        const label = new Gtk.Label({
            label: text, wrap: true, xalign: 0.5,
            css_classes: ['dim-label', 'caption'],
            margin_top: 6, margin_bottom: 6, margin_start: 24, margin_end: 24,
        });
        this._log.append(label);
    }

    _reset() {
        if (this._activeQid !== null)
            return; // don't drop a stream mid-flight
        this._turns = [];
        let child = this._log.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            this._log.remove(child);
            child = next;
        }
    }

    _setBusy(busy) {
        this._sendBtn.sensitive = !busy;
        this._entry.sensitive = !busy;
    }

    _scrollToBottom() {
        // Defer until the new row is laid out.
        GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            const adj = this._scroll.get_vadjustment();
            adj.set_value(adj.get_upper() - adj.get_page_size());
            return GLib.SOURCE_REMOVE;
        });
    }

    // ---- model list ----------------------------------------------------

    async _loadModels() {
        const [local, cloud] = await Promise.all([
            this._fetchLocalModels(), this._fetchCloudModels(),
        ]);
        this._models = mergeModelList(local, cloud);
        const labels = this._models.length > 0
            ? this._models.map(m => m.label)
            : ['No models — is lisa-inferenced running?'];
        this._modelDrop.set_model(Gtk.StringList.new(labels));
        this._modelDrop.set_selected(0);
        this._onModelPicked();
    }

    _onModelPicked() {
        const i = this._modelDrop.selected;
        this._model = this._models[i]?.id ?? null;
    }

    async _fetchLocalModels() {
        try {
            const msg = Soup.Message.new('GET', `${INFERENCED_URL}/v1/models`);
            const bytes = await this._http.send_and_read_async(
                msg, GLib.PRIORITY_DEFAULT, null);
            if (msg.get_status() !== Soup.Status.OK)
                return [];
            return parseLocalModels(
                JSON.parse(new TextDecoder().decode(bytes.toArray())));
        } catch {
            return [];
        }
    }

    async _fetchCloudModels() {
        let stateJson;
        try {
            const reply = await this._remoteCall('State', null, '(s)');
            [stateJson] = reply.deepUnpack();
        } catch {
            return []; // broker not up → local-only, no error to the user
        }
        const providers = usableProviders(JSON.parse(stateJson));
        const entries = [];
        for (const p of providers) {
            try {
                const reply = await this._remoteCall(
                    'ListModels', new GLib.Variant('(s)', [p.id]), '(s)');
                const [modelsJson] = reply.deepUnpack();
                entries.push(...cloudEntries(
                    p.id, p.display_name, JSON.parse(modelsJson)));
            } catch {
                // provider listing failed (offline/revoked) — skip it.
            }
        }
        return entries;
    }

    _remoteCall(method, params, replyType) {
        return Gio.DBus.session.call(
            REMOTED_NAME, REMOTED_PATH, REMOTED_IFACE, method, params,
            replyType ? new GLib.VariantType(replyType) : null,
            Gio.DBusCallFlags.NONE, 4000, null);
    }
}

const app = new Adw.Application({application_id: 'org.lisa.Assistant'});
app.connect('activate', () => {
    (app.activeWindow ?? new AssistantWindow(app).window).present();
});
app.run([]);
