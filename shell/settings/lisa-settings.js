#!/usr/bin/env -S gjs -m
// Lisa Settings — Providers page v1 (PLAN §5.11, §5.3 Settings panel;
// ADR-0008).
//
// Manages the lisa-remoted broker over D-Bus (org.lisa.Remote1):
// bring-your-own provider accounts (list/add/remove, key entry, Sign in
// with Claude), and the per-scope "may offload" switches. Everything
// that can leave the machine is rendered in the distinct amber
// "leaves your hardware" color; the default state — and the state shown
// whenever the broker is unreachable — is "Nothing leaves this
// machine." Keys are write-only: this app can store or clear them,
// never read them back (the broker refuses by construction).

import Adw from 'gi://Adw?version=1';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

import {
    EGRESS_CSS_CLASS, parseState, providerRows, consentRows,
    anythingLeaves, offloadSummary, validateCustomProvider,
} from './lib/model.js';

const BUS_NAME = 'org.lisa.Remoted';
const OBJECT_PATH = '/org/lisa/Remote1';
const IFACE = 'org.lisa.Remote1';

const CSS = `
.${EGRESS_CSS_CLASS} { color: #E66100; }
banner.${EGRESS_CSS_CLASS} { background-color: alpha(#E66100, 0.15); }
`;

/** Thin async wrapper over the broker's management interface. */
class RemoteService {
    constructor() {
        this._bus = null;
    }

    _connection() {
        this._bus ??= Gio.DBus.session;
        return this._bus;
    }

    _call(method, params = null) {
        return new Promise((resolve, reject) => {
            this._connection().call(
                BUS_NAME, OBJECT_PATH, IFACE, method, params, null,
                Gio.DBusCallFlags.NONE, 2000, null, (bus, res) => {
                    try {
                        resolve(bus.call_finish(res));
                    } catch (e) {
                        reject(e);
                    }
                });
        });
    }

    async state() {
        const reply = await this._call('State');
        return parseState(reply.deep_unpack()[0]);
    }

    addProvider(id, name, baseUrl) {
        return this._call('AddProvider',
            new GLib.Variant('(sss)', [id, name, baseUrl]));
    }

    removeProvider(id) {
        return this._call('RemoveProvider', new GLib.Variant('(s)', [id]));
    }

    setKey(id, key) {
        return this._call('SetKey', new GLib.Variant('(ss)', [id, key]));
    }

    clearKey(id) {
        return this._call('ClearKey', new GLib.Variant('(s)', [id]));
    }

    setConsent(scope, allowed) {
        return this._call('SetConsent', new GLib.Variant('(sb)', [scope, allowed]));
    }

    async claudeOauthStart() {
        const reply = await this._call('ClaudeOauthStart');
        return reply.deep_unpack()[0];
    }

    claudeOauthFinish(code) {
        return this._call('ClaudeOauthFinish', new GLib.Variant('(s)', [code]));
    }
}

class SettingsWindow {
    constructor(app) {
        this.service = new RemoteService();
        this.state = parseState(null); // safe default: nothing leaves
        this.offline = true;

        this.window = new Adw.ApplicationWindow({
            application: app,
            title: 'Lisa Settings',
            default_width: 720,
            default_height: 760,
        });

        const provider = new Gtk.CssProvider();
        provider.load_from_string(CSS);
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);

        this.toasts = new Adw.ToastOverlay();
        const header = new Adw.HeaderBar({
            title_widget: new Adw.WindowTitle({
                title: 'Lisa Settings',
                subtitle: 'Providers',
            }),
        });
        const refresh = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
        refresh.tooltip_text = 'Reload from the broker';
        refresh.connect('clicked', () => this.reload());
        header.pack_end(refresh);

        this.banner = new Adw.Banner({revealed: true});
        this.banner.add_css_class(EGRESS_CSS_CLASS);

        this.page = new Adw.PreferencesPage();
        const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        box.append(this.banner);
        box.append(this.page);
        const view = new Adw.ToolbarView({content: box});
        view.add_top_bar(header);
        this.toasts.child = view;
        this.window.content = this.toasts;

        this.reload();
    }

    toast(message) {
        this.toasts.add_toast(new Adw.Toast({title: message}));
    }

    async reload() {
        try {
            this.state = await this.service.state();
            this.offline = false;
        } catch (e) {
            this.state = parseState(null);
            this.offline = true;
            logError?.(e, 'lisa-remoted unreachable');
        }
        this._render();
    }

    _render() {
        // Rebuild the page from the view-model.
        if (this._groups)
            for (const g of this._groups)
                this.page.remove(g);
        this._groups = [];

        this.banner.title = this.offline
            ? 'lisa-remoted is not running — showing defaults; nothing leaves this machine.'
            : offloadSummary(this.state.mayOffload);
        if (!this.offline && !anythingLeaves(this.state.mayOffload))
            this.banner.remove_css_class(EGRESS_CSS_CLASS);
        else if (!this.offline)
            this.banner.add_css_class(EGRESS_CSS_CLASS);

        this._groups.push(this._providerGroup());
        this._groups.push(this._consentGroup());
        for (const g of this._groups)
            this.page.add(g);
    }

    _providerGroup() {
        const group = new Adw.PreferencesGroup({
            title: 'Remote providers',
            description: 'Bring-your-own accounts. Requests through a provider ' +
                'leave your hardware and are marked in the Ledger.',
        });
        const add = new Gtk.Button({
            icon_name: 'list-add-symbolic',
            valign: Gtk.Align.CENTER,
            tooltip_text: 'Add an OpenAI-compatible endpoint',
        });
        add.connect('clicked', () => this._addProviderDialog());
        group.header_suffix = add;

        for (const row of providerRows(this.state.providers)) {
            const item = new Adw.ActionRow({
                title: GLib.markup_escape_text(row.title, -1),
                subtitle: GLib.markup_escape_text(row.subtitle, -1),
            });
            const badge = new Gtk.Label({
                label: 'leaves your hardware',
                valign: Gtk.Align.CENTER,
            });
            badge.add_css_class(EGRESS_CSS_CLASS);
            badge.add_css_class('caption');
            item.add_suffix(badge);

            const keyBtn = new Gtk.Button({
                label: row.hasCredential ? 'Replace key…' : 'Set key…',
                valign: Gtk.Align.CENTER,
            });
            keyBtn.connect('clicked', () => this._keyDialog(row));
            item.add_suffix(keyBtn);

            if (row.hasCredential) {
                const clear = Gtk.Button.new_from_icon_name('edit-clear-symbolic');
                clear.valign = Gtk.Align.CENTER;
                clear.tooltip_text = 'Forget the stored key';
                clear.connect('clicked', async () => {
                    try {
                        await this.service.clearKey(row.id);
                        this.toast(`Key for ${row.title} forgotten`);
                    } catch (e) {
                        this.toast(e.message);
                    }
                    this.reload();
                });
                item.add_suffix(clear);
            }

            if (row.showsSignIn) {
                const signIn = new Gtk.Button({
                    label: 'Sign in with Claude',
                    valign: Gtk.Align.CENTER,
                    sensitive: row.signIn.enabled,
                    tooltip_text: row.signIn.enabled
                        ? 'Authorize with your Claude account'
                        : row.signIn.reason,
                });
                signIn.connect('clicked', () => this._signInWithClaude());
                item.add_suffix(signIn);
            }

            if (row.removable) {
                const remove = Gtk.Button.new_from_icon_name('user-trash-symbolic');
                remove.valign = Gtk.Align.CENTER;
                remove.tooltip_text = 'Remove this provider (and its key)';
                remove.connect('clicked', async () => {
                    try {
                        await this.service.removeProvider(row.id);
                        this.toast(`${row.title} removed`);
                    } catch (e) {
                        this.toast(e.message);
                    }
                    this.reload();
                });
                item.add_suffix(remove);
            }
            group.add(item);
        }
        return group;
    }

    _consentGroup() {
        const group = new Adw.PreferencesGroup({
            title: 'What may leave this machine',
            description: 'Per-scope offload consent (default: nothing). A remote ' +
                'request is refused unless every scope it carries is switched on — ' +
                'including the prompt itself.',
        });
        for (const row of consentRows(this.state.mayOffload)) {
            const item = new Adw.SwitchRow({
                title: row.label,
                subtitle: row.description,
                active: row.active,
                sensitive: !this.offline,
            });
            if (row.active)
                item.add_css_class(EGRESS_CSS_CLASS);
            item.connect('notify::active', async () => {
                try {
                    await this.service.setConsent(row.id, item.active);
                } catch (e) {
                    this.toast(e.message);
                }
                this.reload();
            });
            group.add(item);
        }
        return group;
    }

    _keyDialog(row) {
        const entry = new Adw.PasswordEntryRow({title: 'API key'});
        const group = new Adw.PreferencesGroup({
            description: 'Stored 0600 in the broker state dir; write-only — ' +
                'it can be replaced or forgotten, never read back.',
        });
        group.add(entry);
        this._dialog(`${row.title} key`, group, 'Save', async () => {
            const key = entry.text.trim();
            if (key === '')
                return 'Key must not be empty.';
            await this.service.setKey(row.id, key);
            this.toast(`Key for ${row.title} stored`);
            return null;
        });
    }

    _addProviderDialog() {
        const id = new Adw.EntryRow({title: 'Id (e.g. homelab)'});
        const name = new Adw.EntryRow({title: 'Name'});
        const url = new Adw.EntryRow({title: 'Base URL (OpenAI-compatible, …/v1)'});
        const group = new Adw.PreferencesGroup({
            description: 'Any OpenAI-compatible endpoint — your own box, or a ' +
                'service you have an account with (§5.11).',
        });
        for (const w of [id, name, url])
            group.add(w);
        this._dialog('Add provider', group, 'Add', async () => {
            const form = {
                id: id.text.trim(),
                displayName: name.text.trim(),
                baseUrl: url.text.trim(),
            };
            const errors = validateCustomProvider(
                form, this.state.providers.map(p => p.id));
            if (errors.length > 0)
                return errors.join(' ');
            await this.service.addProvider(form.id, form.displayName, form.baseUrl);
            this.toast(`${form.displayName} added`);
            return null;
        });
    }

    async _signInWithClaude() {
        let authorizeUrl;
        try {
            authorizeUrl = await this.service.claudeOauthStart();
        } catch (e) {
            this.toast(e.message);
            return;
        }
        Gtk.show_uri(this.window, authorizeUrl, Gdk.CURRENT_TIME);
        const code = new Adw.EntryRow({title: 'Authorization code'});
        const group = new Adw.PreferencesGroup({
            description: 'Complete the sign-in in your browser, then paste the ' +
                'code shown at the end.',
        });
        group.add(code);
        this._dialog('Sign in with Claude', group, 'Finish', async () => {
            const value = code.text.trim();
            if (value === '')
                return 'Paste the authorization code first.';
            await this.service.claudeOauthFinish(value);
            this.toast('Signed in with Claude');
            return null;
        });
    }

    /** Small modal helper: page + Cancel/confirm; confirm() returns an
     *  error string to keep the dialog open, or null on success. */
    _dialog(title, group, confirmLabel, confirm) {
        const page = new Adw.PreferencesPage();
        page.add(group);
        const dialog = new Adw.Dialog({
            title,
            content_width: 460,
            child: new Adw.ToolbarView({content: page}),
        });
        const bar = new Adw.HeaderBar({show_end_title_buttons: false});
        const cancel = new Gtk.Button({label: 'Cancel'});
        cancel.connect('clicked', () => dialog.close());
        const ok = new Gtk.Button({label: confirmLabel});
        ok.add_css_class('suggested-action');
        ok.connect('clicked', async () => {
            try {
                const error = await confirm();
                if (error) {
                    this.toast(error);
                    return;
                }
                dialog.close();
                this.reload();
            } catch (e) {
                this.toast(e.message);
            }
        });
        bar.pack_start(cancel);
        bar.pack_end(ok);
        dialog.child.add_top_bar(bar);
        dialog.present(this.window);
    }
}

const app = new Adw.Application({application_id: 'org.lisa.Settings'});
app.connect('activate', () => {
    (app.activeWindow ?? new SettingsWindow(app).window).present();
});
app.run([]);
