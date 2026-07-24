#!/usr/bin/env -S gjs -m
// Lisa Ledger app — the transparency centerpiece (PLAN §5.7.6).
//
// Renders the append-only audit DB: a timeline of every model call,
// context retrieval, and (M5) tool execution — filter by app, kind,
// day; activate an entry for the prompt envelope detail. Usage stats
// (tokens by app) in the footer; export writes the filtered view as
// JSON. Data comes from `lisa ledger --json` (CLAUDE.md rule 7: the
// CLI is the one command center; this app renders, it never writes —
// the DB's triggers would refuse anyway).

import Adw from 'gi://Adw?version=1';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

import {
    buildTimeline, filterRows, distinctValues, usageStats,
    formatTs, dayOf, describeRow,
} from './lib/model.js';

Gio._promisify(Gio.Subprocess.prototype, 'communicate_utf8_async');

const TAIL = 1000;
const ALL = 'all';

class LedgerWindow {
    constructor(app) {
        this._entries = [];
        this._rows = [];
        this._filter = {app: ALL, kind: ALL, day: ALL};

        this.window = new Adw.ApplicationWindow({
            application: app,
            title: 'Ledger',
            default_width: 860,
            default_height: 640,
        });

        const header = new Adw.HeaderBar();
        this._appDrop = this._makeDropdown('app');
        this._kindDrop = this._makeDropdown('kind');
        this._dayDrop = this._makeDropdown('day');
        header.pack_start(this._appDrop);
        header.pack_start(this._kindDrop);
        header.pack_start(this._dayDrop);

        const refresh = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
        refresh.tooltip_text = 'Reload the ledger';
        refresh.connect('clicked', () => this.reload());
        header.pack_end(refresh);

        const exportBtn = Gtk.Button.new_from_icon_name('document-save-symbolic');
        exportBtn.tooltip_text = 'Export the filtered view as JSON';
        exportBtn.connect('clicked', () => this._export());
        header.pack_end(exportBtn);

        this._list = new Gtk.ListBox({
            selection_mode: Gtk.SelectionMode.NONE,
            css_classes: ['boxed-list'],
            margin_top: 12, margin_bottom: 12, margin_start: 12, margin_end: 12,
        });
        this._list.connect('row-activated', (_l, row) => {
            this._showDetail(this._visible[row.get_index()]);
        });

        const scroll = new Gtk.ScrolledWindow({vexpand: true, child: this._list});

        this._stats = new Gtk.Label({
            xalign: 0, wrap: true, css_classes: ['dim-label'],
            margin_top: 6, margin_bottom: 10, margin_start: 14, margin_end: 14,
        });

        const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        box.append(scroll);
        box.append(this._stats);

        const view = new Adw.ToolbarView({content: box});
        view.add_top_bar(header);
        this.window.set_content(view);

        this.reload();
    }

    _makeDropdown(field) {
        const drop = new Gtk.DropDown({
            model: Gtk.StringList.new([`${field}: ${ALL}`]),
            tooltip_text: `Filter by ${field}`,
        });
        drop.connect('notify::selected', () => {
            const item = drop.selected_item?.get_string() ?? ALL;
            this._filter[field] = item.replace(`${field}: `, '');
            this._render();
        });
        return drop;
    }

    async reload() {
        try {
            const cli = GLib.getenv('LISA_CLI') ?? 'lisa';
            const proc = Gio.Subprocess.new(
                [cli, 'ledger', '--json', '--tail', String(TAIL)],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            const [stdout, stderr] = await proc.communicate_utf8_async(null, null);
            if (!proc.get_successful())
                throw new Error(stderr.trim() || 'lisa ledger failed');
            this._entries = JSON.parse(stdout);
        } catch (e) {
            this._entries = [];
            this._stats.label = `Cannot read the ledger: ${e.message} — ` +
                'is the lisa CLI on PATH and has lisa-inferenced run once?';
        }
        this._rows = buildTimeline(this._entries);
        this._refreshDropdowns();
        this._render();
    }

    _refreshDropdowns() {
        const fill = (drop, field, values) => {
            const model = Gtk.StringList.new([
                `${field}: ${ALL}`,
                ...values.map(v => `${field}: ${v}`),
            ]);
            drop.set_model(model);
            drop.set_selected(0);
        };
        fill(this._appDrop, 'app', distinctValues(this._rows, 'app_id'));
        fill(this._kindDrop, 'kind', distinctValues(this._rows, 'kind'));
        fill(this._dayDrop, 'day',
            [...new Set(this._rows.map(r => dayOf(r.ts)))].sort().reverse());
    }

    _render() {
        this._list.remove_all();
        this._visible = filterRows(this._rows, this._filter);
        for (const row of this._visible) {
            const item = new Adw.ActionRow({
                title: GLib.markup_escape_text(
                    row.preview || describeRow(row), -1),
                subtitle: GLib.markup_escape_text(
                    `${formatTs(row.ts)} · ${describeRow(row)}`, -1),
                activatable: true,
            });
            this._list.append(item);
        }
        if (this._entries.length > 0) {
            const {tokensByApp} = usageStats(this._entries);
            const byApp = tokensByApp.length > 0
                ? tokensByApp.map(([a, n]) => `${a}: ${n}`).join(' · ')
                : 'none yet';
            this._stats.label =
                `${this._visible.length} of ${this._rows.length} events shown · ` +
                `output tokens by app — ${byApp}`;
        }
    }

    _showDetail(row) {
        if (!row)
            return;
        // The prompt envelope as the Ledger knows it today: input hash +
        // bounded preview + detail. M2's portal work attaches the full
        // envelope (context chunks + provenance); this pane grows with it.
        const fields = [
            ['When', formatTs(row.ts)],
            ['Kind', row.kind],
            ['App', row.app_id],
            ['Model', row.model || '—'],
            ['Status', row.effective_status],
            ['Input hash (blake3)', row.input_hash || '—'],
            ['Preview', row.preview || '—'],
            ['Detail', row.detail || '—'],
            ['Output tokens', String(row.effective_tokens)],
            ['Duration', `${row.effective_duration_ms} ms`],
            ['Ledger ids', row.completion
                ? `${row.id} → ${row.completion.id}` : String(row.id)],
        ];
        const page = new Adw.PreferencesPage();
        const group = new Adw.PreferencesGroup({
            title: 'Prompt envelope',
            description: 'Everything recorded before this action was allowed to run.',
        });
        for (const [title, value] of fields) {
            group.add(new Adw.ActionRow({
                title,
                subtitle: GLib.markup_escape_text(value, -1),
                subtitle_selectable: true,
            }));
        }
        page.add(group);
        const dialog = new Adw.Dialog({
            title: `Entry ${row.id}`,
            content_width: 560,
            child: new Adw.ToolbarView({content: page}),
        });
        dialog.child.add_top_bar(new Adw.HeaderBar());
        dialog.present(this.window);
    }

    _export() {
        const dialog = new Gtk.FileDialog({
            initial_name: `lisa-ledger-${dayOf(Date.now())}.json`,
        });
        dialog.save(this.window, null, (d, res) => {
            try {
                const file = d.save_finish(res);
                if (!file)
                    return;
                GLib.file_set_contents(file.get_path(),
                    JSON.stringify(this._visible, null, 2));
            } catch {
                // Dismissed — nothing to do.
            }
        });
    }
}

const app = new Adw.Application({application_id: 'app.lisaos.LedgerApp'});
app.connect('activate', () => {
    (app.activeWindow ?? new LedgerWindow(app).window).present();
});
app.run([]);
