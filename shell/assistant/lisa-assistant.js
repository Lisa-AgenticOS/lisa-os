#!/usr/bin/env -S gjs -m
// Lisa Assistant — the persistent chat window (this session's ADR; PLAN
// §5.7.1). A second thin frontend of the dev.lisaos.Overlay1 backend: it sends
// a multi-turn chat Ask (lane:"chat") and renders the streamed Token signals,
// exactly as the transient overlay does — but with history, a model picker
// (local + cloud), and an egress marker on turns that leave the machine.
//
// Models: local from lisa-inferenced `GET /v1/models`; cloud from
// dev.lisaos.Remote1 (providers that are signed in or hold a key → their
// ListModels). Cloud turns route as `remote:<provider>:<model>` and are
// ledgered `remote.*` by the broker. This app renders; the daemons enforce.
//
// Conversations persist across restarts in dev.lisaos.Context1 app memory
// (namespace `app.lisaos.Assistant`), one key per session under the key
// layout harness-core's SessionStore uses (lib/sessions.js, issue #25) —
// fail-soft when contextd is absent, which is still the shipped state on
// devices without lisa-contextd. Send flips to Stop while a reply streams
// (Overlay1.Cancel, #11); the header exports Markdown via a pure helper
// (conversationMarkdown, #8).

import Adw from 'gi://Adw?version=1';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk?version=4.0';
import Soup from 'gi://Soup?version=3.0';

import {
    OVERLAY_IFACE_XML, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH,
    HARNESS_IFACE_XML, HARNESS_BUS_NAME, HARNESS_OBJECT_PATH,
} from '../overlay-extension/lib/iface.js';
import {
    parseLocalModels, usableProviders, cloudEntries, mergeModelList,
    historyPayload, isRemote, conversationMarkdown,
} from './lib/model.js';
import {
    INDEX_KEY, LEGACY_CONVERSATION_KEY, UNTITLED, sessionKey, newSession,
    sessionInfo, parseSessionIndex, serializeSessionIndex, parseSession,
    serializeSession, sessionWithTurns, upsertIndex, removeFromIndex,
    displayIndex, formatSessionTime, migrateLegacyConversation,
} from './lib/sessions.js';
import {toPangoMarkup} from './lib/markdown.js';

Gio._promisify(Soup.Session.prototype, 'send_and_read_async');
Gio._promisify(Gio.DBusConnection.prototype, 'call');

const INFERENCED_URL =
    GLib.getenv('LISA_INFERENCED_URL') ?? 'http://127.0.0.1:7778';
const REMOTED_NAME = 'dev.lisaos.Remoted';        // well-known name (≠ iface)
const REMOTED_PATH = '/dev/lisaos/Remote1';
const REMOTED_IFACE = 'dev.lisaos.Remote1';
const CONTEXTD_NAME = 'dev.lisaos.Context1';      // name = iface (contextd)
const CONTEXTD_PATH = '/dev/lisaos/Context1';
const CONTEXTD_IFACE = 'dev.lisaos.Context1';
const APP_ID = 'app.lisaos.Assistant';            // Context1 memory namespace
const EGRESS_COLOR = '#E66100';                 // the Ledger "leaves" colour

const OverlayProxy = Gio.DBusProxy.makeProxyWrapper(OVERLAY_IFACE_XML);
const HarnessProxy = Gio.DBusProxy.makeProxyWrapper(HARNESS_IFACE_XML);

/// Breakpoint setters take a GValue; box it here rather than lean on
/// GJS's implicit conversion, which is not the same across versions.
function boolValue(b) {
    const value = new GObject.Value();
    value.init(GObject.TYPE_BOOLEAN);
    value.set_boolean(b);
    return value;
}


/// Put text into a turn's label as rendered Markdown.
///
/// Falls back to plain text if Pango refuses the markup. That fallback
/// is not defensive padding: invalid markup makes a GtkLabel render
/// NOTHING — not an error, an empty bubble — so the failure mode without
/// it is a reply that silently disappears. `toPangoMarkup` is written so
/// this should never fire; it fires anyway, because "should never" is
/// not a property you get to assert about a model's output.
function setRendered(label, text) {
    try {
        label.set_markup(toPangoMarkup(text));
    } catch (e) {
        logError(e, 'assistant: markup refused, showing plain text');
        label.set_use_markup(false);
        label.set_label(text);
    }
}

class AssistantWindow {
    constructor(app) {
        this._turns = [];       // {role, text, widget, body}
        this._models = [];      // {id, label, kind, provider?}
        this._model = null;     // selected model id
        this._activeQid = null; // in-flight query id
        this._current = null;   // the streaming assistant turn
        this._persistWarned = false; // one note max when contextd is absent
        // The open conversation, and the stored index behind the list.
        // A new session lives only here until its first completed turn,
        // so abandoning one leaves nothing in app memory.
        this._session = newSession();
        this._sessions = [];
        this._rows = [];        // sidebar rows, by position
        this._listUpdating = false;

        this._http = new Soup.Session();
        this.window = new Adw.ApplicationWindow({
            application: app,
            title: 'Lisa Assistant',
            default_width: 980,
            default_height: 760,
        });

        const header = new Adw.HeaderBar();
        this._title = new Adw.WindowTitle({
            title: 'Lisa Assistant', subtitle: UNTITLED,
        });
        header.title_widget = this._title;
        this._modelDrop = new Gtk.DropDown({
            model: Gtk.StringList.new(['Loading models…']),
            tooltip_text: 'Model — local runs here, cloud leaves the machine',
        });
        this._modelDrop.connect('notify::selected', () => this._onModelPicked());
        this._sidebarBtn = new Gtk.ToggleButton({
            icon_name: 'sidebar-show-symbolic',
            tooltip_text: 'Conversations',
            active: true,
        });
        header.pack_start(this._sidebarBtn);
        header.pack_start(this._modelDrop);
        // Signing in to a provider happens in Settings, in another
        // window — so re-read the model list whenever this window comes
        // back to the front. Without this the picker keeps whatever it
        // saw at startup and a fresh Claude sign-in only appears after
        // restarting the app (field-found on v30).
        this.window.connect('notify::is-active', () => {
            // Never mid-stream: swapping the picker's model would
            // relabel the turn that is still arriving.
            if (this.window.is_active && this._activeQid === null)
                this._refreshModels().catch(() => {});
        });

        const fresh = Gtk.Button.new_from_icon_name('document-new-symbolic');
        fresh.tooltip_text = 'New conversation';
        fresh.connect('clicked', () => this._newSession());
        header.pack_end(fresh);

        const exportBtn =
            Gtk.Button.new_from_icon_name('document-save-symbolic');
        exportBtn.tooltip_text = 'Export conversation as Markdown';
        exportBtn.connect('clicked', () => this._export());
        header.pack_end(exportBtn);

        // The working folder. A button, because the grant has to come
        // from a person: the model gets no file tools until one exists,
        // cannot choose one, and cannot widen the one it has. Same shape
        // Claude Desktop uses, and the same reason — a capability handed
        // in from outside the loop (ADR-0030).
        this._workspace = null;
        this._folderBtn = new Gtk.Button({
            icon_name: 'folder-symbolic',
            tooltip_text: 'No working folder — the assistant cannot read or write files',
        });
        this._folderBtn.connect('clicked', () => this._chooseWorkspace());
        header.pack_end(this._folderBtn);

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
        this._sendBtn.connect('clicked', () => {
            // Doubles as Stop while a reply streams (issue #11).
            if (this._activeQid !== null)
                this._stop();
            else
                this._send();
        });
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

        this._split = new Adw.OverlaySplitView({
            sidebar: this._buildSidebar(),
            content: view,
            show_sidebar: true,
            min_sidebar_width: 200,
            max_sidebar_width: 280,
        });
        this._sidebarBtn.bind_property('active', this._split, 'show-sidebar',
            GObject.BindingFlags.BIDIRECTIONAL | GObject.BindingFlags.SYNC_CREATE);
        // Narrow window: the list overlays the conversation instead of
        // squeezing it — the composer needs the width more than the list.
        const narrow = new Adw.Breakpoint({
            condition: Adw.BreakpointCondition.parse('max-width: 680px'),
        });
        narrow.add_setter(this._split, 'collapsed', boolValue(true));
        narrow.add_setter(this._split, 'show-sidebar', boolValue(false));
        this.window.add_breakpoint(narrow);
        this.window.set_content(this._split);

        this._connectBackend();
        this._systemNote('Ask a local model, or sign in to a cloud provider ' +
            'in Settings → Intelligence for Claude / GPT.');
        this._renderSessionList();
        // Models first so restored headings resolve to picker labels;
        // then the stored sessions from Context1 app memory.
        this._loadModels()
            .catch(e => logError(e, 'model list'))
            .then(() => this._restoreSessions())
            .catch(e => logError(e, 'restore sessions'));
    }

    // ---- conversation list ----------------------------------------------

    _buildSidebar() {
        const header = new Adw.HeaderBar({
            title_widget: new Adw.WindowTitle({title: 'Conversations'}),
            show_end_title_buttons: false,
        });
        const add = Gtk.Button.new_from_icon_name('document-new-symbolic');
        add.tooltip_text = 'New conversation';
        add.connect('clicked', () => this._newSession());
        header.pack_end(add);

        this._list = new Gtk.ListBox({
            selection_mode: Gtk.SelectionMode.SINGLE,
            css_classes: ['navigation-sidebar'],
        });
        this._list.connect('row-selected', (_l, row) => {
            if (this._listUpdating || !row)
                return;
            const info = this._rows[row.get_index()];
            if (info)
                this._openSession(info.id).catch(e => logError(e, 'open session'));
        });

        const sidebar = new Adw.ToolbarView({
            content: new Gtk.ScrolledWindow({vexpand: true, child: this._list}),
        });
        sidebar.add_top_bar(header);
        return sidebar;
    }

    /// Rebuild the list from the stored index plus the open conversation.
    /// Cheap enough to redo wholesale: the index is a handful of rows and
    /// only activity, switching, and deletion touch it.
    _renderSessionList() {
        this._rows = displayIndex(this._sessions, this._session);
        this._listUpdating = true;
        let child = this._list.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            this._list.remove(child);
            child = next;
        }
        for (const info of this._rows) {
            const row = new Adw.ActionRow({
                title: GLib.markup_escape_text(info.title, -1),
                subtitle: formatSessionTime(info.updated_ts),
                tooltip_text: info.title,
            });
            const del = Gtk.Button.new_from_icon_name('user-trash-symbolic');
            del.tooltip_text = 'Delete conversation';
            del.valign = Gtk.Align.CENTER;
            del.add_css_class('flat');
            del.connect('clicked', () => this._confirmDelete(info));
            row.add_suffix(del);
            this._list.append(row);
            if (info.id === this._session.id)
                this._list.select_row(row);
        }
        this._listUpdating = false;
        this._title.subtitle = this._session.title;
    }

    /// Switch to a stored conversation. Refuses mid-stream: the reply
    /// belongs to the conversation that asked for it.
    async _openSession(id) {
        if (id === this._session.id)
            return;
        if (this._activeQid !== null) {
            this._renderSessionList();  // put the selection back
            return;
        }
        const record = parseSession(await this._memoryGet(sessionKey(id)));
        if (!record) {
            // The index knows about it but the record is gone or corrupt;
            // say so rather than silently opening an empty conversation.
            this._sessions = removeFromIndex(this._sessions, id);
            this._memorySet(INDEX_KEY, serializeSessionIndex(this._sessions));
            this._showSession(newSession(), []);
            this._systemNote('That conversation could not be read — it has ' +
                'been removed from the list.');
            return;
        }
        this._showSession(sessionInfo(record), record.turns);
    }

    _newSession() {
        if (this._activeQid !== null)
            return;             // don't drop a stream mid-flight
        if (this._turns.length === 0)
            return;             // already on a blank conversation
        this._showSession(newSession(), []);
    }

    /// Make `info` the open conversation and render `turns` into the log.
    _showSession(info, turns) {
        this._session = sessionInfo(info);
        this._turns = [];
        let child = this._log.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            this._log.remove(child);
            child = next;
        }
        for (const t of turns)
            this._addTurn(t.role, t.text, t.model ?? undefined);
        this._renderSessionList();
    }

    _confirmDelete(info) {
        const dialog = new Adw.AlertDialog({
            heading: 'Delete conversation?',
            body: `“${info.title}” and its turns are removed from this ` +
                'machine. The Ledger entries for its turns remain — this ' +
                'deletes the transcript, not the record that it happened.',
        });
        dialog.add_response('cancel', 'Cancel');
        dialog.add_response('delete', 'Delete');
        dialog.set_response_appearance('delete',
            Adw.ResponseAppearance.DESTRUCTIVE);
        dialog.set_default_response('cancel');
        dialog.set_close_response('cancel');
        dialog.connect('response', (_d, response) => {
            if (response === 'delete')
                this._deleteSession(info.id);
        });
        dialog.present(this.window);
    }

    _deleteSession(id) {
        if (this._activeQid !== null && id === this._session.id)
            return;
        if (this._sessions.some(e => e.id === id)) {
            this._sessions = removeFromIndex(this._sessions, id);
            // Context1 has no per-key delete (only a namespace-wide wipe,
            // which would take the other conversations with it), so the
            // record is tombstoned with the empty string — what
            // SessionStore does too.
            this._memorySet(sessionKey(id), '');
            this._memorySet(INDEX_KEY, serializeSessionIndex(this._sessions));
        }
        if (id !== this._session.id) {
            this._renderSessionList();
            return;
        }
        const next = this._sessions[0];
        this._showSession(newSession(), []);
        if (next)
            this._openSession(next.id).catch(e => logError(e, 'open session'));
    }

    // ---- backend (dev.lisaos.Harness1) -----------------------------------
    //
    // The Assistant drives the HARNESS, not Overlay1's chat lane. That
    // lane skips the Agent Bus by construction, so the assistant had no
    // tools at all — asked about the page you were looking at, Claude
    // could only answer that it had no way to look (seen on the device,
    // 2026-07-29).
    //
    // The overlay keeps its own lane and its own job: small things, fast,
    // one action, Siri-shaped. Real work happens here.

    _connectBackend() {
        try {
            this._harness = HarnessProxy(Gio.DBus.session,
                HARNESS_BUS_NAME, HARNESS_OBJECT_PATH);
            this._harness.connectSignal('Token',
                (_p, _s, [rid, text]) => this._onToken(Number(rid), text));
            // Tool calls are narrated in the transcript: an assistant
            // that silently reads your browser is worse than one that
            // says it did.
            this._harness.connectSignal('Tool',
                (_p, _s, [rid, name, detail]) =>
                    this._onTool(Number(rid), name, detail));
            this._harness.connectSignal('Finished',
                (_p, _s, [rid, ok, summary]) =>
                    this._onFinished(Number(rid), ok ? 'ok' : 'error', summary));
        } catch (e) {
            this._harness = null;
            this._systemNote(`Assistant backend unavailable: ${e.message}`);
        }
    }

    /// Narrate a tool call as a distinct line, not as assistant prose —
    /// what the model DID and what it SAID should not read the same.
    _onTool(rid, name, detail) {
        if (rid !== this._activeQid)
            return;
        const pretty = name.replace(/^app_lisaos_/, '').replace(/__/, ' · ');
        this._systemNote(`⚙ ${pretty}${detail ? ` ${detail}` : ''}`);
        this._scrollToBottom();
    }

    _onToken(qid, text) {
        if (qid !== this._activeQid || !this._current)
            return;
        this._current.text += text;
        // Re-rendered per token rather than appended, because Markdown
        // is not resolvable one character at a time: `**bo` is not bold
        // until the closing pair arrives.
        setRendered(this._current.body, this._current.text);
        this._scrollToBottom();
    }

    _onFinished(qid, status, detail) {
        if (qid !== this._activeQid)
            return;
        if (this._current && !['ok', 'executed', 'cancelled'].includes(status)) {
            const why = detail || status;
            this._current.text = this._current.text
                ? `${this._current.text}\n\n⚠ ${why}` : `⚠ ${why}`;
            setRendered(this._current.body, this._current.text);
            this._current.body.add_css_class('error');
        }
        this._activeQid = null;
        this._current = null;
        this._setBusy(false);
        this._scrollToBottom();
        this._persistSession();
    }

    // ---- sending -------------------------------------------------------

    _send() {
        const prompt = this._entry.text.trim();
        if (prompt === '' || this._activeQid !== null || !this._harness)
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

        // `trigger: prompt` — a person typed this. The daemon clamps it
        // against what this caller is allowed to claim; a surface can
        // only ever narrow its own trust, never widen it (ADR-0036 §1).
        // The working folder, if one has been granted. Absent means the
        // daemon offers no file tools at all — the assistant then says
        // it needs a folder rather than pretending to save something.
        // The path comes from a file chooser the PERSON drove; the model
        // never picks it and cannot widen it (ADR-0030).
        // History travels WITH the run. The daemon keeps no sessions of
        // its own — it would then be one store holding every user's and
        // every surface's conversations — so this window keeps its
        // sessions where it always has (per-user contextd) and hands the
        // relevant ones over per run.
        const options = {
            model: GLib.Variant.new_string(this._model),
            trigger: GLib.Variant.new_string('prompt'),
            history: GLib.Variant.new_string(JSON.stringify(history)),
        };
        if (this._workspace)
            options.workspace = GLib.Variant.new_string(this._workspace);
        // Sync so the run id is set before any Token signal is dispatched
        // (the main loop can't deliver a signal until this returns).
        try {
            const [rid] = this._harness.RunSync(prompt, options);
            this._activeQid = Number(rid);
        } catch (e) {
            // Match the guard in _onFinished so the failure renders and
            // the composer un-sticks.
            this._activeQid = -1;
            this._onFinished(-1, 'error', e.message);
        }
    }

    _stop() {
        if (this._activeQid === null || !this._harness)
            return;
        // Fire-and-forget: the backend answers with Finished('cancelled'),
        // which keeps the partial text and re-enables the composer.
        this._harness.CancelRemote(this._activeQid, () => {});
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
            xalign: 0, wrap: true, selectable: true, use_markup: true,
            margin_bottom: 8, margin_start: 10, margin_end: 10, margin_top: 2,
        });
        setRendered(body, text);
        card.append(heading);
        card.append(body);
        this._log.append(card);
        const turn = {role, text, model: model ?? null, widget: card, body};
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

    _export() {
        const day = new Date().toISOString().slice(0, 10);
        const dialog = new Gtk.FileDialog({
            initial_name: `lisa-conversation-${day}.md`,
        });
        dialog.save(this.window, null, (d, res) => {
            try {
                const file = d.save_finish(res);
                if (!file)
                    return;
                GLib.file_set_contents(file.get_path(),
                    conversationMarkdown(this._turns, this._models));
            } catch {
                // Dismissed — nothing to do.
            }
        });
    }

    // The button flips Send ↔ Stop; the entry stays usable for typing the
    // next message while a reply streams — only sending is gated (#11).
    _setBusy(busy) {
        this._sendBtn.label = busy ? 'Stop' : 'Send';
        if (busy) {
            this._sendBtn.remove_css_class('suggested-action');
            this._sendBtn.add_css_class('destructive-action');
        } else {
            this._sendBtn.remove_css_class('destructive-action');
            this._sendBtn.add_css_class('suggested-action');
        }
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

    /// Re-read the model list, keeping the user's pick if it survived.
    /// Silent when nothing changed — a window focus must not disturb the
    /// picker mid-conversation.
    async _refreshModels() {
        const [local, cloud] = await Promise.all([
            this._fetchLocalModels(), this._fetchCloudModels(),
        ]);
        const models = mergeModelList(local, cloud);
        const same = models.length === this._models?.length &&
            models.every((m, i) => m.id === this._models[i].id);
        if (same || models.length === 0)
            return;
        const chosen = this._model;
        this._models = models;
        this._modelDrop.set_model(
            Gtk.StringList.new(models.map(m => m.label)));
        const keep = models.findIndex(m => m.id === chosen);
        this._modelDrop.set_selected(keep >= 0 ? keep : 0);
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

    // ---- sessions (dev.lisaos.Context1 app memory) ---------------------
    //
    // Every call fails soft: with lisa-contextd absent the app behaves
    // exactly as it always has — conversations live for the run of the
    // window, and the user is told once.

    async _restoreSessions() {
        this._sessions = parseSessionIndex(await this._memoryGet(INDEX_KEY));
        if (this._sessions.length === 0 && await this._migrateLegacy())
            return;
        this._renderSessionList();
        if (this._turns.length > 0)
            return; // the user beat the restore to it — don't interleave
        const recent = this._sessions[0];
        if (recent)
            await this._openSession(recent.id);
    }

    /// Fold the pre-sessions `conversation` key into session one, then
    /// tombstone it so an upgrade happens exactly once. Returns whether
    /// anything was migrated.
    async _migrateLegacy() {
        const session = migrateLegacyConversation(
            await this._memoryGet(LEGACY_CONVERSATION_KEY));
        if (!session || this._turns.length > 0)
            return false;
        this._showSession(sessionInfo(session), session.turns);
        this._sessions = upsertIndex(this._sessions, session);
        this._memorySet(sessionKey(session.id), serializeSession(session));
        this._memorySet(INDEX_KEY, serializeSessionIndex(this._sessions));
        this._memorySet(LEGACY_CONVERSATION_KEY, '');
        this._renderSessionList();
        return true;
    }

    /// Pick the folder the assistant may work in.
    ///
    /// Nothing here is clever on purpose: a folder chooser, and the path
    /// goes to the daemon, which validates it and refuses the ones that
    /// would hand over too much. Clicking again re-picks; picking
    /// nothing clears the grant, so there is always a way to take it
    /// back that is as easy as giving it.
    _chooseWorkspace() {
        const dialog = new Gtk.FileDialog({title: 'Choose a working folder'});
        dialog.select_folder(this, null, (d, res) => {
            let folder = null;
            try {
                folder = d.select_folder_finish(res);
            } catch {
                return; // dismissed — leave the current grant alone
            }
            this._setWorkspace(folder ? folder.get_path() : null);
        });
    }

    _setWorkspace(path) {
        this._workspace = path;
        if (path) {
            const name = path.split('/').filter(Boolean).pop() ?? path;
            this._folderBtn.tooltip_text = `Working in ${path}`;
            this._folderBtn.add_css_class('suggested-action');
            this._systemNote(`📁 Working folder: ${name}`);
        } else {
            this._folderBtn.tooltip_text =
                'No working folder — the assistant cannot read or write files';
            this._folderBtn.remove_css_class('suggested-action');
        }
    }

    /// Write the open conversation and re-file it at the top of the index.
    /// Called when a turn completes — a conversation nobody spoke in is
    /// never written.
    _persistSession() {
        if (this._turns.length === 0)
            return;
        const record = sessionWithTurns(this._session, this._turns);
        this._session = sessionInfo(record);
        this._sessions = upsertIndex(this._sessions, record);
        this._memorySet(sessionKey(record.id), serializeSession(record));
        this._memorySet(INDEX_KEY, serializeSessionIndex(this._sessions));
        this._renderSessionList();
    }

    /// A missing key and an absent daemon are both '' — the same thing
    /// as far as the window is concerned, and the same thing a tombstone
    /// means.
    async _memoryGet(key) {
        try {
            const reply = await this._contextCall('MemoryGet',
                new GLib.Variant('(ss)', [APP_ID, key]), '(s)');
            const [value] = reply.deepUnpack();
            return value;
        } catch {
            return '';
        }
    }

    _memorySet(key, value) {
        this._contextCall('MemorySet',
            new GLib.Variant('(sss)', [APP_ID, key, value]), null)
            .catch(() => {
                if (this._persistWarned)
                    return;
                this._persistWarned = true;
                this._systemNote('Context daemon unavailable — ' +
                    'conversations will not survive a restart.');
            });
    }

    _contextCall(method, params, replyType) {
        return Gio.DBus.session.call(
            CONTEXTD_NAME, CONTEXTD_PATH, CONTEXTD_IFACE, method, params,
            replyType ? new GLib.VariantType(replyType) : null,
            Gio.DBusCallFlags.NONE, 4000, null);
    }
}

const app = new Adw.Application({application_id: APP_ID});
app.connect('activate', () => {
    (app.activeWindow ?? new AssistantWindow(app).window).present();
});
app.run([]);
