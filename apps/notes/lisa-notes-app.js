#!/usr/bin/env -S gjs -m
// Notes — the window (PLAN §5.8, ADR-0048, ADR-0056).
//
// The first consumer of `lisa_ui`, and the reason it was built against a
// real caller rather than invented: ADR-0056 step 4 says a widget set
// designed up front is a guess, one extracted from a real app is a fact.
// `lisaSplitWindow` exists because THIS app needed two header bars.
//
// Shape follows macOS Notes where that is genuinely better and stops
// where it would be decoration. Two panes, not three: Apple's first
// pane is folders and accounts, and Notes has neither, so a third pane
// would be an empty promise. The list is grouped by period the way
// theirs is, because "Previous 7 Days" is how people look for a note.
//
// Everything decidable without a widget lives in lib/model.js and is
// tested under node; this file is the part that needs a display.

import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';

import {lisaSplitWindow, headerButton} from '../lisa_ui/ui/window.js';
import {McpClient} from '../lisa_ui/mcp/client.js';
import {
    groupByPeriod, preview, displayTitle, isWorthSaving, matches, timeOf,
} from './lib/model.js';

const APP_ID = 'app.lisaos.Notes';
const BACKEND = 'app.lisaos.notes';

class NotesApp {
    constructor(app) {
        this._mcp = new McpClient(BACKEND);
        this._notes = [];
        this._query = '';
        this._selected = null;
        this._rowsById = new Map();

        const ui = lisaSplitWindow({
            app, title: 'Notes', width: 940, height: 640, sidebarWidth: 320,
            // A floating frosted sidebar over the note. The blur only
            // means anything when there is content behind the pane —
            // beside-content there is nothing to blur but a flat
            // background (ADR-0056, Mutter#3023 for the window-over-
            // desktop case, which is still upstream-blocked).
            overlay: true,
        });
        this.window = ui.window;
        this._ui = ui;

        // --- sidebar: search, new note, the grouped list --------------
        this._search = new Gtk.SearchEntry({placeholder_text: 'Search'});
        this._search.connect('search-changed', () => {
            this._query = this._search.text;
            this._renderList();
        });
        ui.sidebarHeader.set_title_widget(this._search);
        ui.sidebarHeader.pack_start(headerButton({
            icon: 'document-new-symbolic',
            tooltip: 'New note',
            onClick: () => this._newNote(),
        }));
        ui.sidebarHeader.pack_end(headerButton({
            icon: 'view-refresh-symbolic',
            tooltip: 'Reload',
            onClick: () => this.reload(),
        }));

        this._list = new Gtk.ListBox({
            selection_mode: Gtk.SelectionMode.SINGLE,
            css_classes: ['navigation-sidebar'],
        });
        this._list.connect('row-activated', (_lb, row) => {
            const note = row?._note;
            if (note)
                this._select(note);
        });
        this._listScroll = new Gtk.ScrolledWindow({vexpand: true, child: this._list});

        this._listEmpty = new Adw.StatusPage({
            icon_name: 'accessories-text-editor-symbolic',
            title: 'No notes yet',
            description: 'Notes you or the assistant create appear here.',
        });
        this._listStack = new Gtk.Stack();
        this._listStack.add_named(this._listScroll, 'list');
        this._listStack.add_named(this._listEmpty, 'empty');
        ui.setSidebar(this._listStack);

        // --- content: the note itself --------------------------------
        this._titleEntry = new Gtk.Entry({
            placeholder_text: 'Title',
            css_classes: ['title-2', 'flat'],
            has_frame: false,
        });
        this._bodyView = new Gtk.TextView({
            wrap_mode: Gtk.WrapMode.WORD_CHAR,
            // Zero left margin: the entry above already provides the
            // inset, and a second one made the body sit a few pixels
            // right of its own title. Text that does not line up with
            // its heading reads as a rendering fault.
            top_margin: 6, bottom_margin: 12, left_margin: 0, right_margin: 0,
            css_classes: ['lisa-note-body'],
        });
        // No frame: a boxed text area inside a pane that is already a
        // pane is a border around a border. Apple's note body has no
        // edge at all, and neither should this.
        const bodyScroll = new Gtk.ScrolledWindow({
            vexpand: true, child: this._bodyView, has_frame: false,
        });
        const editorBox = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 6,
            margin_top: 12, margin_bottom: 12, margin_start: 18, margin_end: 18,
        });
        editorBox.append(this._titleEntry);
        editorBox.append(bodyScroll);
        this._editor = editorBox;

        this._contentEmpty = new Adw.StatusPage({
            icon_name: 'document-open-symbolic',
            title: 'No note selected',
            description: 'Pick a note on the left, or start a new one.',
        });
        this._contentStack = new Gtk.Stack();
        this._contentStack.add_named(this._contentEmpty, 'none');
        this._contentStack.add_named(this._editor, 'note');
        this._contentStack.visible_child_name = 'none';

        this._toasts = new Adw.ToastOverlay({child: this._contentStack});
        ui.setContent(this._toasts);

        this._dateLabel = new Gtk.Label({css_classes: ['dim-label', 'caption']});
        ui.contentHeader.set_title_widget(this._dateLabel);
        this._deleteBtn = headerButton({
            icon: 'user-trash-symbolic',
            tooltip: 'Delete this note',
            onClick: () => this._delete(this._selected),
        });
        this._deleteBtn.sensitive = false;
        ui.contentHeader.pack_end(this._deleteBtn);
    }

    _toast(text) {
        this._toasts.add_toast(new Adw.Toast({title: text}));
    }

    /// Load from the backend.
    ///
    /// A daemon that is not running is said out loud rather than shown
    /// as an empty list — "you have no notes" and "I cannot reach your
    /// notes" are very different sentences to read when you know you
    /// wrote some.
    async reload() {
        if (!this._mcp.isAvailable()) {
            this._notes = [];
            this._listEmpty.title = 'Notes is not running';
            this._listEmpty.description =
                `No socket at ${this._mcp.path}. Start lisa-notes, then reload.`;
            this._listEmpty.icon_name = 'dialog-warning-symbolic';
            this._renderList();
            return;
        }
        try {
            const out = await this._mcp.call('list_notes', {});
            this._notes = Array.isArray(out) ? out : (out?.notes ?? []);
            this._listEmpty.title = 'No notes yet';
            this._listEmpty.description = 'Notes you or the assistant create appear here.';
            this._listEmpty.icon_name = 'accessories-text-editor-symbolic';
        } catch (e) {
            this._notes = [];
            this._listEmpty.title = 'Could not read your notes';
            this._listEmpty.description = String(e.message ?? e);
            this._listEmpty.icon_name = 'dialog-warning-symbolic';
        }
        this._renderList();
    }

    _renderList() {
        let child;
        while ((child = this._list.get_first_child()) !== null)
            this._list.remove(child);
        this._rowsById.clear();

        const visible = this._notes.filter((n) => matches(n, this._query));
        this._listStack.visible_child_name = visible.length ? 'list' : 'empty';
        if (!visible.length && this._query) {
            this._listEmpty.title = 'Nothing matches';
            this._listEmpty.description = `No note contains “${this._query}”.`;
        }

        for (const group of groupByPeriod(visible)) {
            // A non-selectable header row, the way a sectioned list
            // reads on both this desktop and Apple's.
            const head = new Gtk.ListBoxRow({
                selectable: false, activatable: false,
                child: new Gtk.Label({
                    label: group.label, xalign: 0,
                    css_classes: ['heading', 'dim-label'],
                    margin_top: 12, margin_bottom: 4,
                    margin_start: 12, margin_end: 12,
                }),
            });
            this._list.append(head);

            for (const note of group.notes) {
                const box = new Gtk.Box({
                    orientation: Gtk.Orientation.VERTICAL, spacing: 2,
                    margin_top: 8, margin_bottom: 8, margin_start: 12, margin_end: 12,
                });
                box.append(new Gtk.Label({
                    label: displayTitle(note), xalign: 0, ellipsize: 3,
                    css_classes: ['heading'],
                }));
                // `snippet` when the server sends one; list_notes does
                // not today, so this is usually the date alone rather
                // than a preview of a field nobody sent (which is what
                // the first version rendered — every subtitle blank).
                const sub = note.snippet ? preview(note.snippet, 60) : this._dateOf(note);
                box.append(new Gtk.Label({
                    label: sub, xalign: 0, ellipsize: 3,
                    css_classes: ['caption', 'dim-label'],
                }));
                const row = new Gtk.ListBoxRow({child: box});
                row._note = note;
                this._list.append(row);
                this._rowsById.set(String(note.id), row);
            }
        }

        // Keep the selection across a reload when the note still exists.
        if (this._selected) {
            const row = this._rowsById.get(String(this._selected.id));
            if (row)
                this._list.select_row(row);
            else
                this._clearSelection();
        }
    }

    _dateOf(note) {
        const ms = timeOf(note);
        if (!ms)
            return '';
        const d = new Date(ms);
        return d.toLocaleDateString(undefined,
            {year: 'numeric', month: 'short', day: 'numeric'});
    }

    _clearSelection() {
        this._selected = null;
        this._contentStack.visible_child_name = 'none';
        this._dateLabel.label = '';
        this._deleteBtn.sensitive = false;
    }

    /// Show one note in the content pane.
    async _select(note) {
        let full = note;
        try {
            full = await this._mcp.call('read_note', {id: note.id});
        } catch (e) {
            // read_note is newer than the daemon on some machines. Show
            // what the list already knows rather than an error page —
            // the title is real even when the body cannot be fetched.
            this._toast(`Could not load the body: ${e.message ?? e}`);
            full = {...note, body: ''};
        }
        this._selected = full;
        this._titleEntry.text = full.title ?? '';
        this._bodyView.buffer.set_text(full.body ?? '', -1);
        this._dateLabel.label = this._dateOf(full);
        this._deleteBtn.sensitive = true;
        this._contentStack.visible_child_name = 'note';
        // Highlight the row too. Selecting a note from anywhere other
        // than a click (a reload, a fresh save) otherwise left the list
        // showing nothing selected while the pane showed a note.
        const row = this._rowsById.get(String(full.id));
        if (row)
            this._list.select_row(row);
        this._ui.showContent();
    }

    /// Start a new note: an empty editor, saved when it has content.
    ///
    /// Deliberately NOT created on the backend up front — an empty note
    /// that appears in the list the moment you click New, and stays
    /// there when you change your mind, is litter the person then has
    /// to delete.
    _newNote() {
        this._selected = {id: null, title: '', body: ''};
        this._titleEntry.text = '';
        this._bodyView.buffer.set_text('', -1);
        this._dateLabel.label = 'New note';
        this._deleteBtn.sensitive = false;
        this._contentStack.visible_child_name = 'note';
        this._list.select_row(null);
        this._ui.showContent();
        this._titleEntry.grab_focus();
    }

    /// Save whatever is in the editor.
    ///
    /// Creating works. UPDATING DOES NOT: the tool surface has no
    /// update_note, so saving an edit to an existing note would create
    /// a second copy. Rather than do that silently, an edit says so and
    /// changes nothing. The alternative — a Save button that quietly
    /// duplicates your note — is the kind of thing you only discover
    /// after it has happened twenty times.
    async _save() {
        const buf = this._bodyView.buffer;
        const body = buf.get_text(buf.get_start_iter(), buf.get_end_iter(), false);
        const draft = {title: this._titleEntry.text, body};
        if (!isWorthSaving(draft))
            return;
        if (this._selected?.id != null) {
            this._toast('Editing an existing note needs update_note, which is not built yet');
            return;
        }
        try {
            await this._mcp.call('create_note', draft);
            this._toast('Note saved');
            await this.reload();
        } catch (e) {
            this._toast(`Could not save: ${e.message ?? e}`);
        }
    }

    async _delete(note) {
        if (!note?.id)
            return;
        try {
            await this._mcp.call('delete_note', {id: note.id});
            // delete_note's undo is restore_note — the manifest says so
            // — so the toast offers it rather than asking first. A
            // confirmation for something reversible is a speed bump; an
            // undo is an answer.
            const t = new Adw.Toast({title: 'Note deleted', button_label: 'Undo'});
            t.connect('button-clicked', async () => {
                try {
                    await this._mcp.call('restore_note', {id: note.id});
                    await this.reload();
                } catch (e) {
                    this._toast(`Could not restore: ${e.message ?? e}`);
                }
            });
            this._clearSelection();
            this._toasts.add_toast(t);
            await this.reload();
        } catch (e) {
            this._toast(`Could not delete: ${e.message ?? e}`);
        }
    }
}

const app = new Adw.Application({
    application_id: APP_ID,
    flags: Gio.ApplicationFlags.DEFAULT_FLAGS,
});
app.connect('activate', () => {
    const notes = new NotesApp(app);

    // Ctrl+S saves; Ctrl+N starts a new note. Named actions rather than
    // raw accels so the shortcuts are discoverable by the shell.
    const save = new Gio.SimpleAction({name: 'save'});
    save.connect('activate', () => notes._save().catch((e) => logError(e)));
    app.add_action(save);
    app.set_accels_for_action('app.save', ['<Primary>s']);

    const fresh = new Gio.SimpleAction({name: 'new'});
    fresh.connect('activate', () => notes._newNote());
    app.add_action(fresh);
    app.set_accels_for_action('app.new', ['<Primary>n']);

    notes.window.present();
    notes.reload().catch((e) => logError(e));
});
app.run([imports.system.programInvocationName, ...ARGV]);
