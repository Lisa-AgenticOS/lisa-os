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
            onClick: () => this._newNote().catch((e) => logError(e)),
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
        // Selection, not just activation: arrow keys and the
        // accessibility bus change the SELECTED row without ever
        // "activating" it, and a note list you can walk with the
        // keyboard but not read is only half a list. The id guard stops
        // the echo when _select re-selects the row it was given.
        this._list.connect('row-selected', (_lb, row) => {
            const note = row?._note;
            if (note && this._selected?.id !== note.id)
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
            // The daemon refuses longer titles (#319); stopping the
            // keystroke here beats a note that fails every autosave.
            max_length: 120,
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
        // Typing anything re-arms the save-before-close protection: a
        // standing "close again to discard" must not quietly apply to
        // words written after the warning.
        this._titleEntry.connect('changed', () => (this._discardOk = false));
        this._bodyView.buffer.connect('changed', () => (this._discardOk = false));

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

        // Closing the window is the third way to leave a note, and it
        // saves like the other two. The close is HELD until the write
        // finishes: returning false immediately would race process exit
        // against the socket, and a save that only sometimes survives
        // closing is worse than none.
        this.window.connect('close-request', () => {
            // After a failed save was reported, the next close is the
            // informed discard and goes straight through.
            if (this._closing || this._discardOk)
                return false;
            this._closing = true;
            // A FAILED save keeps the window open (#319): destroying
            // anyway would discard everything typed since the last good
            // save, silently, at the exact moment the person believes
            // they are done. They can close again to leave without the
            // edit — that second close is an informed one.
            this._maybeSave()
                .then(() => this.window.destroy())
                .catch((e) => {
                    this._closing = false;
                    this._discardOk = true;
                    this._toast(`Not saved — close again to discard: ${e.message ?? e}`);
                });
            return true;
        });
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
                // The body's opening when the server sends one (it does
                // since #155 closed); the date is the fallback for an
                // older daemon or a bodyless note — never a blank
                // subtitle, which is what the first version rendered.
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
        // A DRAFT (id null) has no row by design — clearing the
        // selection for it would wipe the editor mid-composition the
        // moment any background reload lands (#315), so a draft simply
        // keeps the editor and selects nothing.
        if (this._selected && this._selected.id != null) {
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
        // Leaving a note saves it — the Apple behaviour, and the only
        // one that makes an always-editable pane safe. A failure is
        // said out loud; navigation still happens, because trapping
        // someone in a note they cannot save is worse than telling
        // them the save failed.
        let savedOnLeave = false;
        try {
            savedOnLeave = await this._maybeSave();
        } catch (e) {
            // STAY on the note (#319): navigating away after a failed
            // save is how everything typed since the last good save
            // silently vanishes. The text is still on screen, which is
            // the one place it certainly survives. The list selection
            // already moved on click, so it is put back.
            this._toast(`Could not save — staying here: ${e.message ?? e}`);
            const back = this._selected.id != null
                ? this._rowsById.get(String(this._selected.id)) ?? null
                : null;
            this._list.select_row(back);
            return;
        }
        let full = note;
        try {
            full = await this._mcp.call('read_note', {id: note.id});
        } catch (e) {
            // read_note is newer than the daemon on some machines. Show
            // what the list already knows rather than an error page —
            // the title is real even when the body cannot be fetched.
            // Say it IN the pane, not only in a toast that fades. An
            // empty editor with no explanation reads as an empty note,
            // which is a lie about the person's data.
            this._toast(`Could not load the body: ${e.message ?? e}`);
            full = {
                ...note,
                body: `[This note's text could not be loaded.\n\n${e.message ?? e}]`,
                _unreadable: true,
            };
        }
        this._selected = full;
        this._titleEntry.text = full.title ?? '';
        // Cursor at the end, nothing selected. An entry that takes
        // focus selects its contents, so every note opened with its
        // title highlighted — one keystroke from being replaced, and it
        // reads as "this is about to be overwritten" rather than "this
        // is your note".
        this._titleEntry.select_region(-1, -1);
        this._bodyView.buffer.set_text(full.body ?? '', -1);
        // Editable now that update_note exists — except a note whose
        // body could not be loaded: letting someone "edit" the error
        // placeholder and save it over their real text would destroy
        // the very data the placeholder apologises for.
        const editable = !full._unreadable;
        this._titleEntry.editable = editable;
        this._bodyView.editable = editable;
        this._bodyView.cursor_visible = editable;
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
        // The row you just left still shows its old title and snippet;
        // a save that only shows up after a manual reload looks like a
        // save that did not happen. After the new note is on screen,
        // refresh the list (selection survives via _rowsById).
        if (savedOnLeave)
            await this.reload();
    }

    /// Start a new note: an empty editor, saved when it has content.
    ///
    /// Deliberately NOT created on the backend up front — an empty note
    /// that appears in the list the moment you click New, and stays
    /// there when you change your mind, is litter the person then has
    /// to delete.
    async _newNote() {
        // Same contract as switching notes: what you were writing is
        // saved, not discarded — and a save that FAILS keeps the editor
        // on what you wrote (#319) instead of replacing it with a blank
        // page.
        try {
            if (await this._maybeSave())
                await this.reload();
        } catch (e) {
            this._toast(`Could not save — staying here: ${e.message ?? e}`);
            return;
        }
        this._selected = {id: null, title: '', body: ''};
        this._titleEntry.text = '';
        this._bodyView.buffer.set_text('', -1);
        this._titleEntry.editable = true;
        this._bodyView.editable = true;
        this._bodyView.cursor_visible = true;
        this._dateLabel.label = 'New note';
        this._deleteBtn.sensitive = false;
        this._contentStack.visible_child_name = 'note';
        this._list.select_row(null);
        this._ui.showContent();
        this._titleEntry.grab_focus();
    }

    /// Write the editor's state to the backend if it changed.
    ///
    /// One save path for both directions: a draft with no id is created
    /// (and adopts the id the daemon hands back, so the NEXT save is an
    /// update rather than a duplicate); an existing note goes through
    /// update_note, whose undo is itself. Returns whether anything was
    /// written, so callers can toast only on a real write.
    async _maybeSave() {
        // One save at a time (#316): Ctrl+S starts a create, and a row
        // click before the reply lands would start a second — the id is
        // only adopted when the first returns, so the same draft would
        // land twice. Later requests wait for the running one, then
        // re-read the editor; the dirty check makes the retry a no-op
        // when the first save already wrote it.
        while (this._saving)
            await this._saving;
        this._saving = this._writeEditorState();
        try {
            const wrote = await this._saving;
            // Any save that gets through revokes a pending "close again
            // to discard" — the situation it warned about is over.
            this._discardOk = false;
            return wrote;
        } finally {
            this._saving = null;
        }
    }

    async _writeEditorState() {
        const s = this._selected;
        if (!s || s._unreadable)
            return false;
        const buf = this._bodyView.buffer;
        const body = buf.get_text(buf.get_start_iter(), buf.get_end_iter(), false);
        const title = this._titleEntry.text;
        if (s.id == null) {
            if (!isWorthSaving({title, body}))
                return false;
            const out = await this._mcp.call('create_note', {title, body});
            s.id = out?.id ?? null;
        } else {
            if (title === (s.title ?? '') && body === (s.body ?? ''))
                return false;
            await this._mcp.call('update_note', {id: s.id, title, body});
        }
        s.title = title;
        s.body = body;
        return true;
    }

    /// Ctrl+S: save, and say so.
    async _save() {
        const created = this._selected?.id == null;
        try {
            if (await this._maybeSave()) {
                this._toast(created ? 'Note saved' : 'Note updated');
                await this.reload();
            }
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
    fresh.connect('activate', () => notes._newNote().catch((e) => logError(e)));
    app.add_action(fresh);
    app.set_accels_for_action('app.new', ['<Primary>n']);

    notes.window.present();
    notes.reload().catch((e) => logError(e));
});
app.run([imports.system.programInvocationName, ...ARGV]);
