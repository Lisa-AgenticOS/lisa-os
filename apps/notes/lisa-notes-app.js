#!/usr/bin/env -S gjs -m
// Notes — the window (PLAN §5.8, ADR-0048, ADR-0056).
//
// The FIRST consumer of `lisa_ui`, and the reason it was built against
// a real caller rather than invented: ADR-0056 step 4 says a widget set
// designed up front is a guess, one extracted from a real app is a
// fact.
//
// Notes had been an MCP server with no GUI since ADR-0013 — store,
// tools and tests all present, window missing. So this window is not a
// new app so much as the missing half of one, and it reaches its own
// data the same way the agent does: five tools over
// `app.lisaos.notes.sock`. What the person sees and what the model sees
// are the same list because it is the same call.
//
// Everything decidable without a widget lives in lib/model.js and is
// tested under node; this file is the part that needs a display.

import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';

import {lisaWindow, headerButton} from '../lisa_ui/ui/window.js';
import {McpClient} from '../lisa_ui/mcp/client.js';
import {ordered, preview, displayTitle, isWorthSaving, matches} from './lib/model.js';

const APP_ID = 'app.lisaos.Notes';
const BACKEND = 'app.lisaos.notes';

class NotesApp {
    constructor(app) {
        this._mcp = new McpClient(BACKEND);
        this._notes = [];
        this._query = '';

        const {window, header} = lisaWindow({
            app,
            title: 'Notes',
            width: 880,
            height: 620,
        });
        this.window = window;

        this._search = new Gtk.SearchEntry({placeholder_text: 'Search notes'});
        this._search.connect('search-changed', () => {
            this._query = this._search.text;
            this._render();
        });
        header.set_title_widget(this._search);

        header.pack_start(headerButton({
            icon: 'document-new-symbolic',
            tooltip: 'New note',
            onClick: () => this._compose(),
        }));
        header.pack_end(headerButton({
            icon: 'view-refresh-symbolic',
            tooltip: 'Reload',
            onClick: () => this.reload(),
        }));

        this._list = new Gtk.ListBox({
            selection_mode: Gtk.SelectionMode.NONE,
            css_classes: ['boxed-list'],
        });
        const scroll = new Gtk.ScrolledWindow({
            vexpand: true,
            child: new Adw.Clamp({maximum_size: 720, child: this._list, margin_top: 18,
                margin_bottom: 18, margin_start: 18, margin_end: 18}),
        });

        this._status = new Adw.StatusPage({
            icon_name: 'accessories-text-editor-symbolic',
            title: 'No notes yet',
            description: 'Notes you or the assistant create appear here.',
        });

        this._stack = new Gtk.Stack();
        this._stack.add_named(scroll, 'list');
        this._stack.add_named(this._status, 'empty');

        // The toast overlay wraps the stack and IS the window content;
        // a toast has to sit above whichever page is showing, not
        // inside one of them.
        this._toasts = new Adw.ToastOverlay({child: this._stack});
        this.window.content.content = this._toasts;
    }

    _toast(text) {
        this._toasts.add_toast(new Adw.Toast({title: text}));
    }

    /// Load from the backend. A daemon that is not running is said out
    /// loud rather than shown as an empty list — "you have no notes"
    /// and "I cannot reach your notes" are very different sentences to
    /// read when you know you wrote some.
    async reload() {
        if (!this._mcp.isAvailable()) {
            this._notes = [];
            this._status.title = 'Notes is not running';
            this._status.description =
                `No socket at ${this._mcp.path}. Start lisa-notes, then reload.`;
            this._status.icon_name = 'dialog-warning-symbolic';
            this._render();
            return;
        }
        try {
            const out = await this._mcp.call('list_notes', {});
            this._notes = Array.isArray(out) ? out : (out?.notes ?? []);
            this._status.title = 'No notes yet';
            this._status.description = 'Notes you or the assistant create appear here.';
            this._status.icon_name = 'accessories-text-editor-symbolic';
        } catch (e) {
            this._notes = [];
            this._status.title = 'Could not read your notes';
            this._status.description = String(e.message ?? e);
            this._status.icon_name = 'dialog-warning-symbolic';
        }
        this._render();
    }

    _render() {
        let row;
        while ((row = this._list.get_first_child()) !== null)
            this._list.remove(row);

        const visible = ordered(this._notes).filter((n) => matches(n, this._query));
        this._stack.visible_child_name = visible.length ? 'list' : 'empty';
        if (!visible.length && this._query)
            this._status.title = 'Nothing matches';

        for (const note of visible) {
            const r = new Adw.ActionRow({
                title: displayTitle(note),
                // list_notes returns {id,title,created} and no body, so
                // this is the created date rather than a body preview.
                // It said `preview(note.body)` until the first render on
                // hardware showed every subtitle blank — a preview of a
                // field the server does not send.
                subtitle: note.created ? String(note.created).slice(0, 10) : '',
                activatable: true,
            });
            // Rows were `activatable: true` with nothing connected, so
            // clicking a note did nothing at all. Comparing against
            // macOS Notes side by side is what made it obvious: theirs
            // has a reader pane, ours had a list you could only delete
            // from.
            r.connect('activated', () => this._open(note));
            r.add_suffix(headerButton({
                icon: 'user-trash-symbolic',
                tooltip: 'Delete',
                onClick: () => this._delete(note),
            }));
            this._list.append(r);
        }
    }

    /// Open one note for reading and editing.
    ///
    /// The body comes from `read_note`, which did not exist until
    /// today: `list_notes` and `search_notes` both return
    /// {id,title,created} and nothing else, so a note's content could
    /// be written and searched and never read back — by this window or
    /// by the model.
    async _open(note) {
        let full;
        try {
            full = await this._mcp.call('read_note', {id: note.id});
        } catch (e) {
            this._toast(`Could not open: ${e.message ?? e}`);
            return;
        }
        this._editor({
            title: displayTitle(full),
            heading: full.title ?? '',
            body: full.body ?? '',
            onSave: async (draft) => {
                // No update_note on the surface yet, so saving an edit
                // would silently create a second note. Rather than do
                // that, the editor is read-only for existing notes and
                // says so — a Save button that duplicates your note is
                // worse than no Save button.
                this._toast('Editing an existing note needs update_note (not built yet)');
            },
            readOnly: true,
        });
    }

    /// One editor, used for both a new note and an existing one.
    _editor({title, heading = '', body = '', onSave, readOnly = false}) {
        const dialog = new Adw.Window({
            transient_for: this.window,
            modal: true,
            title,
            default_width: 620,
            default_height: 480,
        });
        const view = new Adw.ToolbarView();
        const header = new Adw.HeaderBar();
        view.add_top_bar(header);

        const titleEntry = new Gtk.Entry({
            placeholder_text: 'Title', text: heading, editable: !readOnly,
        });
        const bodyView = new Gtk.TextView({
            wrap_mode: Gtk.WrapMode.WORD_CHAR, editable: !readOnly,
            top_margin: 8, bottom_margin: 8, left_margin: 8, right_margin: 8,
        });
        bodyView.buffer.set_text(body, -1);

        const box = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 12,
            margin_top: 12, margin_bottom: 12, margin_start: 12, margin_end: 12,
        });
        box.append(titleEntry);
        box.append(new Gtk.ScrolledWindow({vexpand: true, child: bodyView}));
        view.content = box;
        dialog.content = view;

        if (!readOnly) {
            const save = new Gtk.Button({label: 'Save', css_classes: ['suggested-action']});
            save.connect('clicked', async () => {
                const buf = bodyView.buffer;
                const text = buf.get_text(buf.get_start_iter(), buf.get_end_iter(), false);
                await onSave({title: titleEntry.text, body: text});
            });
            header.pack_end(save);
        }
        dialog.present();
        return dialog;
    }

    _compose() {
        const dialog = new Adw.Window({
            transient_for: this.window,
            modal: true,
            title: 'New note',
            default_width: 620,
            default_height: 480,
        });
        const view = new Adw.ToolbarView();
        const header = new Adw.HeaderBar();
        view.add_top_bar(header);

        const title = new Gtk.Entry({placeholder_text: 'Title'});
        const body = new Gtk.TextView({
            wrap_mode: Gtk.WrapMode.WORD_CHAR,
            top_margin: 8, bottom_margin: 8, left_margin: 8, right_margin: 8,
        });
        const box = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL, spacing: 12,
            margin_top: 12, margin_bottom: 12, margin_start: 12, margin_end: 12,
        });
        box.append(title);
        box.append(new Gtk.ScrolledWindow({vexpand: true, child: body}));
        view.content = box;
        dialog.content = view;

        const save = new Gtk.Button({label: 'Save', css_classes: ['suggested-action']});
        save.connect('clicked', async () => {
            const buf = body.buffer;
            const text = buf.get_text(buf.get_start_iter(), buf.get_end_iter(), false);
            const draft = {title: title.text, body: text};
            // An empty note is somebody who changed their mind, not a
            // note. Saving it leaves litter they then have to delete.
            if (!isWorthSaving(draft)) {
                dialog.close();
                return;
            }
            try {
                await this._mcp.call('create_note', draft);
                this._toast('Note saved');
                dialog.close();
                await this.reload();
            } catch (e) {
                this._toast(`Could not save: ${e.message ?? e}`);
            }
        });
        header.pack_end(save);
        dialog.present();
    }

    async _delete(note) {
        try {
            await this._mcp.call('delete_note', {id: note.id});
            // delete_note's undo is restore_note (the manifest says so),
            // so the toast offers it rather than asking first. A
            // confirmation dialog for something reversible is a speed
            // bump; an undo is an answer.
            const t = new Adw.Toast({title: 'Note deleted', button_label: 'Undo'});
            t.connect('button-clicked', async () => {
                try {
                    await this._mcp.call('restore_note', {id: note.id});
                    await this.reload();
                } catch (e) {
                    this._toast(`Could not restore: ${e.message ?? e}`);
                }
            });
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
    notes.window.present();
    notes.reload().catch((e) => logError(e));
});
app.run([imports.system.programInvocationName, ...ARGV]);
