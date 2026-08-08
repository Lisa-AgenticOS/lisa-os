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
import {lisaTripleWindow} from '../lisa.sdk/ui/window.js';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk?version=4.0';
import Pango from 'gi://Pango';
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
    sessionInfo, serializeSessionIndex, parseSession,
    serializeSession, sessionWithTurns, upsertIndex, removeFromIndex,
    displayIndex, formatSessionTime, migrateLegacyConversation,
    restorePlan, handoffPlan,
} from './lib/sessions.js';
import {toPangoMarkup} from './lib/markdown.js';
import {
    IMAGE_MIME_BY_EXT, imageMimeForName, attachmentsPayload, attachmentRefusal,
    attachmentSizeRefusal, stagedForSession,
} from './lib/attachments.js';
import {chosenPath, remoteLocationNote} from './lib/chooser.js';
import {
    MEMORY_IFACE_XML, parseNotes, provenanceNote, emptyText, forgetAllBody,
    sortNotes,
} from './lib/memory.js';
import {
    MODE_IDS, MODES, DEFAULT_MODE, modeById, wireMode, needsWorkspace,
} from './lib/modes.js';

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
// A second, deliberately narrow proxy for what the assistant remembers
// (#157). Separate from `HarnessProxy` because the shared interface node
// lives with the overlay and describes what the OVERLAY needs; memory is
// this window's business, and a pane that could also start runs is a
// pane with more reach than its job.
const MemoryProxy = Gio.DBusProxy.makeProxyWrapper(MEMORY_IFACE_XML);




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
        // Whether a read has authoritatively said what is stored. Until
        // it has, the index is never REWRITTEN — a failed read must not
        // become a destructive write (#228).
        this._indexKnown = false;
        this._indexPending = false;
        // A Spotlight hand-off that arrived while a reply was streaming
        // (#233). It starts its own conversation when the run ends
        // rather than being dropped on the floor.
        this._pendingHandoff = null;
        this._rows = [];        // sidebar rows, by position
        this._listUpdating = false;
        // Images staged for the next message (#209): {name, mime, b64,
        // texture}. Cleared on send — an attachment belongs to the
        // message it was attached to, not to the composer.
        this._attachments = [];

        this._http = new Soup.Session();
        // The mode the navrail is on. A mode is a real bundle of effects
        // (lib/modes.js): composer placeholder, this mode's chat list,
        // the `mode` on the wire, and Code's required working folder.
        this._mode = DEFAULT_MODE;
        this._modeButtons = {};   // id -> ToggleButton, for the rail
        // The shared chrome (#282, ADR-0056), now the three-pane shape
        // (ADR-0056 step 4, Mail's window): rail | this mode's chats |
        // the chat screen. The rail is a narrow glass pane; conversations
        // move to the middle list pane; the chat stays the content pane.
        const ui = lisaTripleWindow({
            app, title: 'Lisa Assistant',
            width: 1080, height: 760, sidebarWidth: 88,
        });
        this.window = ui.window;
        this._ui = ui;
        this._buildRail(ui);
        // The action handler reaches the controller through the window
        // GTK hands it back (app.activeWindow is a GtkWindow, not this).
        this.window.__lisa = this;

        const header = ui.contentHeader;
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
        this._sidebarBtn.connect('toggled', () => this._applySidebar());
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

        // What the assistant remembers about you, between conversations
        // (#157). In the header rather than behind a menu because the
        // whole justification for durable memory is that the person can
        // see it: a list one click away is the feature, a list three
        // clicks away is a compliance gesture.
        const memoryBtn =
            Gtk.Button.new_from_icon_name('view-list-bullet-symbolic');
        memoryBtn.tooltip_text = 'What the assistant remembers about you';
        memoryBtn.connect('clicked', () => this._showMemory());
        header.pack_end(memoryBtn);

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
        // Ctrl+V of an image (#209). The controller only claims the key
        // when the clipboard actually holds a picture; otherwise it
        // returns false and the entry pastes text exactly as before —
        // stealing Ctrl+V wholesale would break the commonest thing
        // anyone does in a text box.
        const keys = new Gtk.EventControllerKey();
        // CAPTURE, and this is the whole bug (#264). A controller added
        // with the default BUBBLE phase runs AFTER the focused widget —
        // and the focused widget here is the GtkText inside the
        // GtkEntry, which carries GTK's own Ctrl+V shortcut. That
        // shortcut matches on the KEY, not on what the clipboard holds:
        // it calls paste-clipboard, finds no *text* behind a pasted
        // screenshot, rings the error bell, and consumes the event. The
        // handler below never ran, and a person pasting a screenshot
        // heard a beep and saw nothing.
        //
        // Capture descends from the toplevel, so the entry sees the key
        // before its own GtkText child. Returning false when the
        // clipboard holds no texture hands the key straight back, so an
        // ordinary text paste is untouched — which the test asserts.
        keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
        keys.connect('key-pressed', (_c, keyval, _code, state) => {
            const ctrl = (state & Gdk.ModifierType.CONTROL_MASK) !== 0;
            if (ctrl && (keyval === Gdk.KEY_v || keyval === Gdk.KEY_V))
                return this._pasteImage();
            return false;
        });
        this._entry.add_controller(keys);

        this._attachBtn = new Gtk.Button({
            icon_name: 'mail-attachment-symbolic',
            tooltip_text: 'Attach an image',
        });
        this._attachBtn.connect('clicked', () => this._chooseAttachment());
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
        composer.append(this._attachBtn);
        composer.append(this._entry);
        composer.append(this._sendBtn);

        // Staged attachments, above the entry: visible, named, and
        // removable. An attachment you cannot see is one you send by
        // accident to a provider you did not mean to.
        this._attachBar = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL, spacing: 6,
            margin_top: 6, margin_start: 12, margin_end: 12,
            visible: false,
        });

        const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        box.append(this._scroll);
        box.append(this._attachBar);
        box.append(composer);

        ui.setContent(box);
        this._buildSidebar(ui);
        // Land the composer's placeholder and the rail selection on the
        // opening mode before anything is shown.
        this._entry.placeholder_text = modeById(this._mode).placeholder;
        // Narrow window: the conversation outranks the list — the
        // composer needs the width. Signals rather than setters (#339):
        // setters apply one way and their unapply-restore clobbers any
        // toggle made while narrow; a single _applySidebar() owns the
        // whole (toggle x narrow) truth table instead — including the
        // narrow+shown case, where the pane floats OVER the content
        // (margin 0) rather than squeezing it.
        this._narrow = false;
        const narrow = new Adw.Breakpoint({
            condition: Adw.BreakpointCondition.parse('max-width: 680px'),
        });
        narrow.connect('apply', () => {
            this._narrow = true;
            // Entering narrow starts with the list put away — the old
            // collapsed-split behaviour; the toggle can bring it back.
            this._sidebarBtn.active = false;
            this._applySidebar();
        });
        narrow.connect('unapply', () => {
            this._narrow = false;
            this._applySidebar();
        });
        this.window.add_breakpoint(narrow);

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

    /// The one owner of the conversation-list toggle. The list is the
    /// MIDDLE pane now (the rail is the leftmost), so the toggle drives
    /// the inner split's sidebar. The rail itself is always visible —
    /// switching modes is not something a toggle should hide.
    _applySidebar() {
        this._ui.inner.show_sidebar = this._sidebarBtn.active;
    }

    // ---- the mode navrail -----------------------------------------------

    /// The rail: one button per mode (lib/modes.js), top-to-bottom in
    /// MODE_IDS order, icon over label. A linked, single-select group —
    /// exactly one mode is active, shown by the button's selected state
    /// (no per-mode accent hue yet; that is a deferred design call).
    _buildRail(ui) {
        ui.sidebarHeader.set_title_widget(new Adw.WindowTitle({title: 'Lisa'}));
        const rail = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 4,
            margin_top: 8, margin_bottom: 8, margin_start: 6, margin_end: 6,
        });
        let group = null;
        for (const id of MODE_IDS) {
            const mode = MODES[id];
            const inner = new Gtk.Box({
                orientation: Gtk.Orientation.VERTICAL, spacing: 2,
            });
            inner.append(new Gtk.Image({icon_name: mode.icon, pixel_size: 20}));
            inner.append(new Gtk.Label({
                label: mode.label, css_classes: ['caption'],
            }));
            const btn = new Gtk.ToggleButton({
                child: inner,
                tooltip_text: mode.summary,
                css_classes: ['flat'],
                active: id === this._mode,
            });
            // One group: activating one releases the others, so the rail
            // is single-select and the model never runs "in two modes".
            if (group)
                btn.set_group(group);
            else
                group = btn;
            btn.connect('toggled', () => {
                if (btn.active)
                    this._setMode(id);
            });
            this._modeButtons[id] = btn;
            rail.append(btn);
        }
        ui.setSidebar(rail);
    }

    /// Switch modes: update the composer's prompt to the mode's job, show
    /// this mode's conversations, remember it for the wire, and — for
    /// Code — make sure there is a working folder, since that is the
    /// grant that turns on the file tools (ADR-0036 §6). A no-op if the
    /// mode did not actually change.
    _setMode(id) {
        if (id === this._mode)
            return;
        this._mode = id;
        const mode = modeById(id);
        this._entry.placeholder_text = mode.placeholder;
        if (this._modeButtons[id] && !this._modeButtons[id].active)
            this._modeButtons[id].active = true;
        // Per-mode conversation lists (a coding session and a research
        // thread not braiding) is the next increment — it needs the
        // session index to carry a mode non-lossily across reload. Today
        // the list is shared across modes; the rail changes the composer,
        // the wire, and the tools, not yet which chats show. Stated so
        // this is a known gap, not a silent one (rule 10).
        // Code needs a folder to be useful. Prompt when entering it with
        // none — never silently, and never for the other modes.
        if (needsWorkspace(id) && !this._workspace)
            this._chooseWorkspace();
    }

    // ---- conversation list ----------------------------------------------

    _buildSidebar(ui) {
        // The conversations are the MIDDLE (list) pane now — the rail is
        // the leftmost. So this builds into listHeader/setList, not the
        // sidebar pane the rail now owns.
        const header = ui.listHeader;
        header.set_title_widget(new Adw.WindowTitle({title: 'Conversations'}));
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

        ui.setList(new Gtk.ScrolledWindow({vexpand: true, child: this._list}));
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
        const read = await this._memoryGet(sessionKey(id));
        if (!read.ok) {
            // The read FAILED — which is not the same as the record
            // being gone, and dropping the conversation from the index
            // on the strength of it is how #228 lost conversations that
            // were sitting in the namespace the whole time. Say so, and
            // leave every stored byte exactly where it is.
            this._renderSessionList();  // put the selection back
            this._systemNote('That conversation could not be read right ' +
                `now (${read.error}). It has not been changed — try again.`);
            return;
        }
        const record = parseSession(read.value);
        if (!record) {
            // Authoritative this time: the store answered, and what it
            // holds is a tombstone or unusable. Say so rather than
            // silently opening an empty conversation.
            this._sessions = removeFromIndex(this._sessions, id);
            this._writeIndex();
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

    /// The Spotlight hand-off (#210): start a FRESH conversation and
    /// send `prompt` in it.
    ///
    /// Always a new session, never an append: the overlay's box is
    /// empty every time it opens, so what you type there is a new
    /// thought — silently continuing yesterday's thread would carry
    /// context the person cannot see.
    ///
    /// Mid-stream it is QUEUED, not dropped (#233). `_send` returns
    /// early while a run is in flight, so a hand-off that landed then
    /// used to overwrite the composer and go nowhere at all: no session,
    /// no turn, no error — and the overlay had already closed, so the
    /// question was gone. It now runs when the current reply finishes,
    /// and says so while it waits.
    ///
    /// The prompt no longer travels through `this._entry`: that is the
    /// person's draft for the conversation they are in, and the overlay
    /// has no business overwriting it.
    askInNewSession(prompt) {
        const plan = handoffPlan(prompt, {
            busy: this._activeQid !== null,
            hasTurns: this._turns.length > 0,
        });
        if (plan.action === 'ignore')
            return;
        if (plan.action === 'queue') {
            this._pendingHandoff = plan.prompt;
            this._systemNote(plan.note);
            this._scrollToBottom();
            return;
        }
        if (plan.newSession)
            this._showSession(newSession(), []);
        // The send is deferred to an idle tick because the model list
        // may still be loading when the action arrives (a cold start
        // goes activate -> window -> action within the same frame), and
        // _send refuses without a model. One retry window, then it gives
        // up with a visible note rather than looping.
        let tries = 0;
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 120, () => {
            if (this._model) {
                this._send(plan.prompt);
                return GLib.SOURCE_REMOVE;
            }
            if (++tries > 25) {          // ~3 s
                this._systemNote(`No model available yet — “${plan.prompt}” ` +
                    'was not sent. Pick a model and ask again.');
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    /// Make `info` the open conversation and render `turns` into the log.
    _showSession(info, turns) {
        // Staged attachments do not follow the person to another
        // conversation (#235). An image attached here and sent there
        // reaches THAT conversation's provider — which may be a cloud
        // one when this conversation's was local — so this is a
        // disclosure, not an untidy composer. Every switch in this
        // window comes through here, which is why it is here.
        const carried = this._attachments.length;
        if (carried > 0)
            this._clearAttachments();
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
        // After the log is emptied, or the note would be swept out with
        // the conversation it is about.
        if (carried > 0) {
            this._systemNote(`${carried} staged image${carried === 1 ? '' : 's'} ` +
                'stayed with the other conversation — attach again to send here.');
        }
        this._renderSessionList();
    }

    // ---- memory (#157) ---------------------------------------------------
    //
    // Cross-conversation memory carries facts about a person from one
    // conversation into the next, indefinitely, without being asked
    // again. That is a feature only while they can see the list and take
    // things off it — so this pane is part of the feature, not a
    // follow-up to it.
    //
    // The pane can only READ and DELETE. There is no "add" control and
    // no editing: what is remembered is a record of what the assistant
    // learned, and a record a person can rewrite is not one either of
    // them can rely on. Taking a note off the list is always available;
    // putting a different sentence in its place is not.

    _memoryProxy() {
        if (this._memoryIface)
            return this._memoryIface;
        try {
            this._memoryIface = MemoryProxy(Gio.DBus.session,
                HARNESS_BUS_NAME, HARNESS_OBJECT_PATH);
        } catch (e) {
            logError(e, 'assistant: memory proxy');
            this._memoryIface = null;
        }
        return this._memoryIface;
    }

    _showMemory() {
        const dialog = new Adw.Dialog({
            title: 'What Lisa remembers',
            content_width: 520, content_height: 560,
        });
        const list = new Gtk.ListBox({
            selection_mode: Gtk.SelectionMode.NONE,
            css_classes: ['boxed-list'],
            margin_top: 12, margin_bottom: 12,
            margin_start: 12, margin_end: 12,
        });
        const scroll = new Gtk.ScrolledWindow({vexpand: true, child: list});
        const header = new Adw.HeaderBar({
            title_widget: new Adw.WindowTitle({title: 'What Lisa remembers'}),
        });
        const forgetAll = new Gtk.Button({
            label: 'Forget everything',
            css_classes: ['destructive-action'],
        });
        header.pack_end(forgetAll);
        const view = new Adw.ToolbarView({content: scroll});
        view.add_top_bar(header);
        dialog.set_child(view);

        const refresh = () => {
            let child = list.get_first_child();
            while (child) {
                const next = child.get_next_sibling();
                list.remove(child);
                child = next;
            }
            const {notes, ok} = this._readMemory();
            forgetAll.sensitive = notes.length > 0;
            if (notes.length === 0) {
                const row = new Adw.ActionRow({title: emptyText(ok)});
                row.set_title_lines(0);
                list.append(row);
                return;
            }
            for (const note of sortNotes(notes)) {
                const row = new Adw.ActionRow({
                    title: note.text,
                    subtitle: provenanceNote(note),
                });
                row.set_title_lines(0);
                row.set_subtitle_lines(0);
                const del = Gtk.Button.new_from_icon_name('user-trash-symbolic');
                del.valign = Gtk.Align.CENTER;
                del.tooltip_text = 'Forget this';
                del.add_css_class('flat');
                // No confirmation for one note, deliberately. Forgetting
                // is the safe direction here: the cost of a mistaken
                // delete is that the assistant asks you something again,
                // and a dialog in front of that would be friction aimed
                // at the wrong outcome.
                del.connect('clicked', () => {
                    this._forgetMemory(note.id);
                    refresh();
                });
                row.add_suffix(del);
                list.append(row);
            }
        };

        forgetAll.connect('clicked', () => {
            const {notes} = this._readMemory();
            const confirm = new Adw.AlertDialog({
                heading: 'Forget everything?',
                body: forgetAllBody(notes.length),
            });
            confirm.add_response('cancel', 'Cancel');
            confirm.add_response('forget', 'Forget everything');
            confirm.set_response_appearance('forget',
                Adw.ResponseAppearance.DESTRUCTIVE);
            confirm.set_default_response('cancel');
            confirm.set_close_response('cancel');
            confirm.connect('response', (_d, response) => {
                if (response !== 'forget')
                    return;
                const proxy = this._memoryProxy();
                try {
                    proxy?.MemoryForgetAllSync();
                } catch (e) {
                    logError(e, 'assistant: forget all');
                }
                refresh();
            });
            confirm.present(dialog);
        });

        refresh();
        dialog.present(this.window);
    }

    /// Read the listing, keeping "empty" and "could not read" apart.
    ///
    /// The same distinction #228 cost us on the session index: a failed
    /// read that renders as an empty list is a person being told the
    /// assistant knows nothing about them, which may be false.
    _readMemory() {
        const proxy = this._memoryProxy();
        if (!proxy)
            return {notes: [], ok: false};
        try {
            const [payload] = proxy.MemoryListSync();
            return {notes: parseNotes(payload), ok: true};
        } catch (e) {
            logError(e, 'assistant: memory list');
            return {notes: [], ok: false};
        }
    }

    _forgetMemory(id) {
        try {
            this._memoryProxy()?.MemoryForgetSync(id);
        } catch (e) {
            logError(e, 'assistant: forget note');
        }
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
        if (!this._indexKnown) {
            // Deleting rewrites the index, and the index is not known
            // to be what we think it is (#228). Refusing visibly beats
            // tombstoning a record and then failing to say which of the
            // person's other conversations went with it.
            this._systemNote('Stored conversations could not be read this ' +
                'session, so deleting one is not safe — restart the ' +
                'assistant and try again.');
            return;
        }
        if (this._sessions.some(e => e.id === id)) {
            this._sessions = removeFromIndex(this._sessions, id);
            // Context1 has no per-key delete (only a namespace-wide wipe,
            // which would take the other conversations with it), so the
            // record is tombstoned with the empty string — what
            // SessionStore does too.
            this._memorySet(sessionKey(id), '');
            this._writeIndex();
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
            // Every way out of a run is a Finished signal — so a daemon
            // that dies mid-run is a run with no way out (#227). The
            // composer stays on "Stop", `_send` returns early for ever,
            // and the window is finished for the session with nothing
            // said. Watching the name is how the bus tells us.
            this._harnessWatch = Gio.bus_watch_name(
                Gio.BusType.SESSION, HARNESS_BUS_NAME,
                Gio.BusNameWatcherFlags.NONE,
                null,
                () => this._onBackendVanished());
        } catch (e) {
            this._harness = null;
            this._systemNote(`Assistant backend unavailable: ${e.message}`);
        }
    }

    /// The harness left the bus. If a run was in flight it is not coming
    /// back, so end it here rather than leaving the window stuck.
    _onBackendVanished() {
        if (this._activeQid === null)
            return;
        this._onFinished(this._activeQid, 'error',
            'The assistant backend stopped while this was running. ' +
            'It restarts on the next message.');
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
        // A hand-off that arrived while this was streaming (#233) gets
        // its own conversation now that there is one to give it.
        // Deferred by a tick so the persist above lands against the
        // conversation it belongs to before the switch.
        const queued = this._pendingHandoff;
        if (queued !== null) {
            this._pendingHandoff = null;
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                this.askInNewSession(queued);
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    // ---- attachments (#209) ---------------------------------------------
    //
    // An image reaches the model as an OpenAI content part on the user
    // turn: the window base64s the bytes, the harness puts the person's
    // words in front of them, and lisa-remoted rewrites the data URI for
    // providers that want their own shape. Nothing is uploaded anywhere
    // — the bytes ride inside the request, so there is no temporary
    // object with a URL to leak.

    /// Pick an image from disk. Filtered to what the wire carries: a
    /// chooser that offers a .pdf we would then refuse is a worse
    /// interaction than one that never offered it.
    _chooseAttachment() {
        const filter = new Gtk.FileFilter({name: 'Images'});
        for (const mime of new Set(Object.values(IMAGE_MIME_BY_EXT)))
            filter.add_mime_type(mime);
        const filters = new Gio.ListStore({item_type: Gtk.FileFilter.$gtype});
        filters.append(filter);
        const dialog = new Gtk.FileDialog({
            title: 'Attach an image', filters, default_filter: filter,
        });
        dialog.open(this.window, null, (d, res) => {
            let file = null;
            try {
                file = d.open_finish(res);
            } catch {
                return; // dismissed
            }
            // A Drive/sftp/camera pick has no local path. That is a
            // choice this window cannot honour, not a dismissal — say so
            // (#234), or the button just does nothing for ever.
            const chosen = chosenPath(file);
            if (chosen.kind === 'remote') {
                this._systemNote(remoteLocationNote('attach', chosen.uri));
                this._scrollToBottom();
                return;
            }
            if (chosen.kind === 'local')
                this._attachPath(chosen.path);
        });
    }

    /// Read a file, base64 it, stage it. Every failure says which file
    /// and why: a picture that silently fails to attach is one the
    /// person believes was sent.
    _attachPath(path) {
        if (!path) {
            // The chooser sorts locations out before we get here (#234);
            // anything reaching this branch is a caller bug, and a
            // silent return is how that stays invisible for a month.
            this._systemNote('Could not attach that — it has no file path.');
            return;
        }
        const name = path.split('/').filter(Boolean).pop() ?? path;
        const mime = imageMimeForName(path);
        if (!mime) {
            this._systemNote(`Cannot attach ${name} — images only ` +
                `(${Object.keys(IMAGE_MIME_BY_EXT).join(', ')}).`);
            return;
        }
        // Size BEFORE bytes (#226). Asking the filesystem how big it is
        // costs a stat; reading it first would mean loading whatever was
        // picked into this process to find out it was too big to send.
        let size = 0;
        try {
            size = Gio.File.new_for_path(path)
                .query_info('standard::size', Gio.FileQueryInfoFlags.NONE, null)
                .get_size();
        } catch (e) {
            this._systemNote(`Could not read ${name}: ${e.message}`);
            return;
        }
        const tooBig = attachmentSizeRefusal(name, size, this._attachments);
        if (tooBig) {
            this._systemNote(tooBig);
            this._scrollToBottom();
            return;
        }
        let bytes;
        try {
            const [ok, contents] = GLib.file_get_contents(path);
            if (!ok)
                throw new Error('unreadable');
            bytes = contents;
        } catch (e) {
            this._systemNote(`Could not read ${name}: ${e.message}`);
            return;
        }
        let texture = null;
        try {
            texture = Gdk.Texture.new_from_filename(path);
        } catch (e) {
            // No preview, but the bytes are fine and the model can still
            // see them — decoding is the toolkit's opinion, not the
            // provider's.
            logError(e, 'assistant: no thumbnail for the attachment');
        }
        this._addAttachment({
            name, mime, bytes: size, b64: GLib.base64_encode(bytes), texture,
        });
    }

    /// Ctrl+V. Returns true only when an image was taken off the
    /// clipboard, so a normal text paste still reaches the entry.
    _pasteImage() {
        const clipboard = this.window.get_clipboard();
        if (!clipboard.get_formats().contain_gtype(Gdk.Texture.$gtype))
            return false;
        clipboard.read_texture_async(null, (c, res) => {
            try {
                const texture = c.read_texture_finish(res);
                if (!texture)
                    return;
                // PNG because it is lossless and the one encoder every
                // GdkTexture has; the paste has no file name to inherit
                // a format from.
                const png = texture.save_to_png_bytes();
                const stamp = new Date().toISOString()
                    .replace(/[:.]/g, '-').slice(0, 19);
                const name = `pasted-${stamp}.png`;
                // A pasted screenshot is the biggest thing this window
                // ever attaches — full resolution, freshly encoded, and
                // it goes through the same budget as a picked file
                // (#226).
                const size = png.get_size();
                const tooBig = attachmentSizeRefusal(
                    name, size, this._attachments);
                if (tooBig) {
                    this._systemNote(tooBig);
                    this._scrollToBottom();
                    return;
                }
                this._addAttachment({
                    name,
                    mime: 'image/png',
                    bytes: size,
                    b64: GLib.base64_encode(png.toArray()),
                    texture,
                });
            } catch (e) {
                this._systemNote(`Could not paste that image: ${e.message}`);
            }
        });
        return true;
    }

    /// Stage an image against the conversation that is open (#235).
    ///
    /// The `session` tag is what makes an attachment belong to a
    /// conversation rather than to the composer. `_showSession` clears
    /// the strip on every switch; this is the second mechanism, so a
    /// switch path nobody remembered to clear still cannot put one
    /// conversation's picture on another's wire — and that wire may go
    /// to a different provider, which makes it a disclosure and not a
    /// stray widget.
    _addAttachment(item) {
        this._attachments.push({...item, session: this._session.id});
        this._renderAttachments();
    }

    _removeAttachment(item) {
        this._attachments = this._attachments.filter(a => a !== item);
        this._renderAttachments();
    }

    _clearAttachments() {
        this._attachments = [];
        this._renderAttachments();
    }

    /// Rebuild the strip wholesale — it holds a handful of chips and
    /// only attach/remove/send touch it.
    _renderAttachments() {
        let child = this._attachBar.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            this._attachBar.remove(child);
            child = next;
        }
        for (const item of this._attachments) {
            const chip = new Gtk.Box({
                orientation: Gtk.Orientation.HORIZONTAL, spacing: 6,
                css_classes: ['card'],
            });
            if (item.texture) {
                chip.append(new Gtk.Picture({
                    paintable: item.texture,
                    content_fit: Gtk.ContentFit.COVER,
                    width_request: 36, height_request: 36,
                    margin_start: 4, margin_top: 4, margin_bottom: 4,
                }));
            }
            chip.append(new Gtk.Label({
                label: item.name, css_classes: ['caption'],
                ellipsize: Pango.EllipsizeMode.MIDDLE, max_width_chars: 20,
                margin_start: item.texture ? 0 : 8,
            }));
            const drop = Gtk.Button.new_from_icon_name('window-close-symbolic');
            drop.tooltip_text = `Remove ${item.name}`;
            drop.valign = Gtk.Align.CENTER;
            drop.add_css_class('flat');
            drop.connect('clicked', () => this._removeAttachment(item));
            chip.append(drop);
            this._attachBar.append(chip);
        }
        this._attachBar.visible = this._attachments.length > 0;
    }

    // ---- sending -------------------------------------------------------

    /// Send the composer's contents, or `override` when something other
    /// than the person's typing supplies the text (the Spotlight
    /// hand-off, #233). An override never touches `this._entry`: that
    /// draft belongs to whoever typed it.
    _send(override = null) {
        const fromComposer = override === null;
        const prompt = (fromComposer ? this._entry.text : override).trim();
        if (this._activeQid !== null || !this._harness)
            return;
        // Only this conversation's attachments are ever in play (#235).
        const staged = stagedForSession(this._attachments, this._session.id);
        if (prompt === '' && staged.length === 0)
            return;
        if (!this._model) {
            this._systemNote('Pick a model first.');
            return;
        }
        // A local engine reads text only and lisa-inferenced refuses
        // content parts outright — correct, and five layers away. Say it
        // here, where the person can still change the model, rather than
        // letting them watch a spinner turn into a daemon error (#209).
        const picked = this._models.find(m => m.id === this._model) ??
            {id: this._model, label: this._model};
        const refusal = attachmentRefusal(picked, staged);
        if (refusal) {
            this._systemNote(refusal);
            this._scrollToBottom();
            return;
        }
        if (prompt === '') {
            this._systemNote('Add a message to send with the attachment.');
            this._scrollToBottom();
            return;
        }
        const parts = attachmentsPayload(staged);
        const attached = staged;
        const history = historyPayload(this._turns);
        if (fromComposer)
            this._entry.text = '';
        this._clearAttachments();
        this._addTurn('user', prompt, undefined, attached);
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
            // The navrail's mode (lib/modes.js). A validated known id, a
            // hint for the daemon's tool/policy selection — harmless if
            // harnessd ignores it today, and the contract the deeper
            // per-mode behaviour (ADR-0065/0067/0069, retrieval, artifacts)
            // will be built against rather than invented later.
            mode: GLib.Variant.new_string(wireMode(this._mode)),
        };
        if (this._workspace)
            options.workspace = GLib.Variant.new_string(this._workspace);
        // Attachments travel as a JSON string, the same way history
        // does. The daemon puts the message text in FRONT of these
        // parts; absent, the turn stays a plain string on the wire.
        if (parts.length > 0)
            options.attachments = GLib.Variant.new_string(JSON.stringify(parts));
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
        // Fire-and-forget: the daemon stops the loop between turns — and
        // mid-answer, since a half-generated sentence costs nothing to
        // abandon — then answers Finished with "Stopped.", which keeps
        // the words that did arrive and re-enables the composer (#227).
        //
        // It used to say the backend answered `Finished('cancelled')`.
        // Nothing did: the flag was set, read into a local variable, and
        // never acted on, so Stop was a no-op and the run carried on
        // through its whole turn budget.
        this._harness.CancelRemote(this._activeQid, () => {});
    }

    // ---- conversation widgets ------------------------------------------

    /// `attachments` is the live-session staging list for a user turn
    /// (#209) — the thumbnails are rendered from the GdkTextures it
    /// carries. Restored conversations pass none: the stored session
    /// shape is `{role, text, model}`, byte for byte what harness-core's
    /// SessionStore reads, and the image bytes are not in it.
    _addTurn(role, text, model, attachments) {
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
        // What was sent, shown as what it was. A turn that reads "what
        // is this?" with no picture above it is a transcript that has
        // lost half the question.
        for (const item of attachments ?? []) {
            if (!item?.texture)
                continue;
            card.append(new Gtk.Picture({
                paintable: item.texture,
                content_fit: Gtk.ContentFit.CONTAIN,
                can_shrink: true,
                height_request: 160,
                margin_start: 10, margin_end: 10, margin_top: 4,
                tooltip_text: item.name,
            }));
        }
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

    /// Write the conversation out as Markdown.
    ///
    /// Three outcomes, three answers (#234). The one `catch` used to
    /// cover all of them and call every one "Dismissed": a Drive
    /// destination (no local path), a full disk, a read-only folder —
    /// the file was simply not there afterwards and nothing had said so.
    _export() {
        const day = new Date().toISOString().slice(0, 10);
        const dialog = new Gtk.FileDialog({
            initial_name: `lisa-conversation-${day}.md`,
        });
        dialog.save(this.window, null, (d, res) => {
            let file = null;
            try {
                file = d.save_finish(res);
            } catch {
                return; // dismissed
            }
            const chosen = chosenPath(file);
            if (chosen.kind === 'remote') {
                this._systemNote(remoteLocationNote('save to', chosen.uri));
                this._scrollToBottom();
                return;
            }
            if (chosen.kind !== 'local')
                return;
            try {
                const ok = GLib.file_set_contents(chosen.path,
                    conversationMarkdown(this._turns, this._models));
                if (!ok)
                    throw new Error('the write did not complete');
                this._systemNote(`Saved to ${chosen.path}`);
            } catch (e) {
                this._systemNote(
                    `Could not save to ${chosen.path}: ${e.message}`);
            }
            this._scrollToBottom();
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

    /// Load the stored conversations, and decide whether this window is
    /// allowed to rewrite the listing at all (#228).
    ///
    /// One `MemoryList`, not a `MemoryGet` per key: it answers "what is
    /// stored" without conflating an empty namespace with an unreadable
    /// one, and it brings the records along, so an index a previous run
    /// clobbered is rebuilt out of the conversations themselves.
    async _restoreSessions() {
        const plan = restorePlan(await this._memoryList(), this._sessions);
        this._sessions = plan.sessions;
        this._indexKnown = plan.indexKnown;
        if (plan.note)
            this._systemNote(plan.note);
        // A write that was waiting on this now knows what it replaces.
        if (this._indexKnown && this._indexPending) {
            this._indexPending = false;
            this._writeIndex();
        }
        if (this._sessions.length === 0 && this._migrateLegacy(plan.legacy))
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
    ///
    /// `stored` is the value the restore already read — not a second
    /// fetch, which would be a second chance to read a failure as an
    /// empty key and migrate nothing.
    _migrateLegacy(stored) {
        const session = migrateLegacyConversation(stored);
        if (!session || this._turns.length > 0)
            return false;
        this._showSession(sessionInfo(session), session.turns);
        this._sessions = upsertIndex(this._sessions, session);
        this._memorySet(sessionKey(session.id), serializeSession(session));
        this._writeIndex();
        this._memorySet(LEGACY_CONVERSATION_KEY, '');
        this._renderSessionList();
        return true;
    }

    /// Pick the folder the assistant may work in.
    ///
    /// Nothing here is clever on purpose: a folder chooser, and the path
    /// goes to the daemon, which validates it and refuses the ones that
    /// would hand over too much. Clicking again re-picks.
    ///
    /// A grant is only ever REPLACED here, never dropped as a side
    /// effect. It used to be `folder ? folder.get_path() : null`, and
    /// `get_path()` is null for every folder that is not on this machine
    /// — so choosing a Drive folder revoked the working folder the
    /// person already had, said nothing, and left the assistant refusing
    /// to write files for reasons it could not explain (#234).
    _chooseWorkspace() {
        const dialog = new Gtk.FileDialog({title: 'Choose a working folder'});
        // The parent is the GtkWindow, not this controller: GJS cannot
        // marshal a plain JS object where a GtkWindow is expected, so
        // `this` here threw and the folder chooser never opened.
        dialog.select_folder(this.window, null, (d, res) => {
            let folder = null;
            try {
                folder = d.select_folder_finish(res);
            } catch {
                return; // dismissed — leave the current grant alone
            }
            const chosen = chosenPath(folder);
            if (chosen.kind === 'remote') {
                this._systemNote(remoteLocationNote('work in', chosen.uri));
                this._scrollToBottom();
                return; // and the grant they already had is untouched
            }
            if (chosen.kind === 'local')
                this._setWorkspace(chosen.path);
        });
    }

    /// The `else` branch has no caller today: the chooser only ever
    /// passes a real path (#234), and nothing else revokes a grant.
    /// Kept because it is the honest rendering of "no working folder"
    /// the moment a revoke exists — and NOT described in the README as
    /// something a person can do, which is how #234 got here.
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
        // The record first and unconditionally: writing it is additive,
        // it is the conversation itself, and a record with no index
        // entry is recoverable (`indexFromRecords`) where a lost record
        // is not.
        this._memorySet(sessionKey(record.id), serializeSession(record));
        this._writeIndex();
        this._renderSessionList();
    }

    /// Read one key. `{ok, value}` — NOT a bare string.
    ///
    /// This used to be `catch { return ''; }`, and that single line is
    /// #228's second and worse half. `''` is what a missing key and a
    /// tombstone look like, so every failure — `AccessDenied` most of
    /// all — arrived as "there is nothing stored here", and the callers
    /// act on that: `_openSession` drops the conversation from the index
    /// and `_persistSession` used to rewrite the index around a single
    /// entry. A read that failed became a destructive write. The #210
    /// shape, in a place with no undo: Context1 has no per-key delete.
    async _memoryGet(key) {
        try {
            const reply = await this._contextCall('MemoryGet',
                new GLib.Variant('(ss)', [APP_ID, key]), '(s)');
            const [value] = reply.deepUnpack();
            return {ok: true, value};
        } catch (e) {
            // contextd raises for a MISSING key by design, so an error
            // here is not necessarily a failure — but the window cannot
            // tell which from the error alone, and the safe reading of
            // "cannot tell" is "do not write". `_restoreSessions` asks
            // the unambiguous question (MemoryList) instead.
            return {ok: false, value: '', error: e?.message ?? String(e)};
        }
    }

    /// The whole namespace, key → value.
    ///
    /// Unambiguous where MemoryGet is not: an empty namespace is `{}`
    /// and a namespace that could not be read is an error, so "nothing
    /// stored" and "nothing readable" stop being the same answer. It
    /// also carries the session records, which is what makes recovering
    /// an index this bug already clobbered possible at all.
    async _memoryList() {
        try {
            const reply = await this._contextCall('MemoryList',
                new GLib.Variant('(s)', [APP_ID]), '(s)');
            const [json] = reply.deepUnpack();
            const map = JSON.parse(json);
            if (map === null || typeof map !== 'object' || Array.isArray(map))
                throw new Error('the namespace did not read as an object');
            return {ok: true, map};
        } catch (e) {
            return {ok: false, error: e?.message ?? String(e)};
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

    /// Write the index — the one destructive write this window makes.
    ///
    /// `sessions` REPLACES the listing of every conversation, so it may
    /// only be written once a read has authoritatively said what is
    /// stored (`_indexKnown`). Until then the write is remembered, not
    /// performed: a turn that completes before the restore lands still
    /// gets its RECORD written (that is additive and safe), and the
    /// listing follows when the restore says what it is replacing.
    _writeIndex() {
        if (!this._indexKnown) {
            this._indexPending = true;
            return;
        }
        this._memorySet(INDEX_KEY, serializeSessionIndex(this._sessions));
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

/// `ask(prompt)` — the Spotlight hand-off (#210).
///
/// The overlay is a one-shot surface by construction: it streams an
/// answer and then has nowhere to put your reply, which is exactly
/// what made it useless for anything that needed a second sentence.
/// So it stops being a chat and becomes a LAUNCHER: type, Enter, and
/// the conversation opens here — in a NEW session, never appended to
/// whatever was on screen, because the thing you just typed into an
/// empty box is a new thought, not a continuation.
///
/// A GAction rather than a CLI flag: the shell can call it over
/// org.gtk.Actions on an app that may not be running yet, and GTK
/// starts it, which is the whole activation contract.
const askAction = new Gio.SimpleAction({
    name: 'ask',
    parameter_type: new GLib.VariantType('s'),
});
askAction.connect('activate', (_a, param) => {
    const prompt = param?.deepUnpack() ?? '';
    const win = app.activeWindow?.__lisa ?? new AssistantWindow(app);
    win.window.present();
    win.askInNewSession(prompt);
});
app.add_action(askAction);

app.run([]);
