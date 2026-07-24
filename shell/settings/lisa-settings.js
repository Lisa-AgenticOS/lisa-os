#!/usr/bin/env -S gjs -m
// Lisa Settings — AI / Intelligence panel (PLAN §5.3 Settings panel,
// §5.11, §8; ADR-0008). A two-section libadwaita window:
//
//   • Local models — the §8 hardware-aware catalog from
//     `lisa models catalog --json`: what runs on THIS machine, what's
//     installed, and a one-click Get for pinned models that fit. Local
//     inference never leaves the machine, so nothing here is
//     egress-marked.
//   • Providers — the lisa-remoted broker over D-Bus (org.lisa.Remote1):
//     one card per provider with its brand logo and a key-state pill,
//     write-only key entry (store/replace/forget), Sign in with Claude,
//     and — once a key is set and the `prompt` scope is on — a live
//     model dropdown fed by the broker's `ListModels` (the provider's
//     own /models, never an invented list). Picking a model copies its
//     ready-to-use route `remote:<id>:<model>`. Because the broker
//     refuses any remote request whose `prompt` scope is not consented
//     (default: off), a keyed provider with `prompt` off raises a
//     prominent amber warning; when the broker is unreachable the
//     add/save actions are disabled with the reason instead of throwing.
//     Everything that can leave the machine is rendered in the distinct
//     amber "leaves your hardware" color; the default — and the state
//     shown whenever the broker is unreachable — is "Nothing leaves
//     this machine." Keys are write-only: this app stores or clears
//     them, never reads them back.

import Adw from 'gi://Adw?version=1';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

import {
    EGRESS_CSS_CLASS, parseState, providerRows, consentRows,
    anythingLeaves, offloadSummary, validateCustomProvider,
    parseCatalog, localModelRows, profileSummary, providerModelHelp,
    remoteReadiness, providersDisabledReason, providerLogoFile,
    modelHintFor, parseModelList,
} from './lib/model.js';

const BUS_NAME = 'org.lisa.Remoted';
const OBJECT_PATH = '/org/lisa/Remote1';
const IFACE = 'org.lisa.Remote1';

// Brand logos shipped alongside this app (assets/provider-logos/),
// resolved relative to this file so the dev run works from the repo.
const LOGO_DIR = Gio.File.new_for_uri(import.meta.url)
    .get_parent().get_child('assets').get_child('provider-logos');

const CSS = `
@define-color lisa_violet #7A55D1;
.${EGRESS_CSS_CLASS} { color: #E66100; }
banner.${EGRESS_CSS_CLASS} { background-color: alpha(#E66100, 0.15); }
.provider-pill { border-radius: 99px; padding: 2px 10px; font-weight: 600; }
.provider-pill.has-key { background-color: alpha(@lisa_violet, 0.16); color: @lisa_violet; }
.provider-pill.no-key {
    background-color: alpha(@window_fg_color, 0.08);
    color: alpha(@window_fg_color, 0.55);
}
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

    _call(method, params = null, timeout = 2000) {
        return new Promise((resolve, reject) => {
            this._connection().call(
                BUS_NAME, OBJECT_PATH, IFACE, method, params, null,
                Gio.DBusCallFlags.NONE, timeout, null, (bus, res) => {
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

    /** The provider's own live model list (its /models, keyed). Longer
     *  timeout: this is a real egress roundtrip, unlike local state. */
    async listModels(provider) {
        const reply = await this._call('ListModels',
            new GLib.Variant('(s)', [provider]), 15000);
        return parseModelList(reply.deep_unpack()[0]);
    }
}

/**
 * Local-model access via the `lisa` CLI (the documented, no-egress data
 * source for modeld — §8). We shell out rather than reimplement the
 * hardware-fit logic, keeping one source of truth in Rust.
 */
class LocalModels {
    /** Run `lisa <args…>`, resolving {ok, stdout, stderr}. */
    _run(args) {
        return new Promise((resolve, reject) => {
            let proc;
            try {
                proc = Gio.Subprocess.new(
                    ['lisa', ...args],
                    Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            } catch (e) {
                reject(e);
                return;
            }
            proc.communicate_utf8_async(null, null, (p, res) => {
                try {
                    const [, stdout, stderr] = p.communicate_utf8_finish(res);
                    resolve({ok: p.get_successful(), stdout, stderr});
                } catch (e) {
                    reject(e);
                }
            });
        });
    }

    /** The §8 catalog annotated by this machine's fit, or null if the
     *  CLI/daemon is unavailable (the page then says so). */
    async catalog() {
        const {ok, stdout} = await this._run(['models', 'catalog', '--json']);
        if (!ok)
            return null;
        return parseCatalog(stdout);
    }

    /** Download a pinned catalog model. Long-running (GiB); the caller
     *  shows progress. Rejects with stderr on failure. */
    async get(id) {
        const {ok, stderr} = await this._run(['models', 'get', id]);
        if (!ok)
            throw new Error(stderr.trim() || `could not get ${id}`);
    }
}

class SettingsWindow {
    constructor(app) {
        this.service = new RemoteService();
        this.models = new LocalModels();
        this.state = parseState(null); // safe default: nothing leaves
        this.offline = true;
        this.catalog = null;           // null until modeld answers
        this._logoCache = new Map();   // `${id}:${dark}` → Gdk.Texture

        // Logo SVGs are recolored with the foreground color, so a theme
        // flip means re-rendering the cards with fresh textures.
        this._styleManager = Adw.StyleManager.get_default();
        this._styleManager.connect('notify::dark', () => this._render());

        this.window = new Adw.ApplicationWindow({
            application: app,
            title: 'Lisa Settings',
            default_width: 760,
            default_height: 800,
        });

        const provider = new Gtk.CssProvider();
        provider.load_from_string(CSS);
        Gtk.StyleContext.add_provider_for_display(
            Gdk.Display.get_default(), provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);

        this.toasts = new Adw.ToastOverlay();

        // Two top-level sections, switched from the header (and a bottom
        // bar when the window is narrow) — the libadwaita idiom for a
        // small settings app.
        this.stack = new Adw.ViewStack();
        this.localPage = new Adw.PreferencesPage();
        this.providersPage = new Adw.PreferencesPage();
        this.banner = new Adw.Banner({revealed: true});
        this.banner.add_css_class(EGRESS_CSS_CLASS);
        // The consent trap, made loud: a key is stored but the `prompt`
        // scope is off, so every remote request is silently refused.
        this.consentBanner = new Adw.Banner({
            title: 'Add a key, then enable the ‘prompt’ scope below to ' +
                'actually use remote models.',
            revealed: false,
        });
        this.consentBanner.add_css_class(EGRESS_CSS_CLASS);

        // The egress banner belongs to Providers (the only egress path);
        // Local models get their own scrolling page with no banner.
        const providersBox = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        providersBox.append(this.banner);
        providersBox.append(this.consentBanner);
        providersBox.append(this.providersPage);

        const localItem = this.stack.add_titled(
            this.localPage, 'local', 'Local models');
        localItem.icon_name = 'application-x-firmware-symbolic';
        const provItem = this.stack.add_titled(
            providersBox, 'providers', 'Providers');
        provItem.icon_name = 'network-transmit-receive-symbolic';

        const header = new Adw.HeaderBar({
            title_widget: new Adw.ViewSwitcher({
                stack: this.stack,
                policy: Adw.ViewSwitcherPolicy.WIDE,
            }),
        });
        const refresh = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
        refresh.tooltip_text = 'Reload models and providers';
        refresh.connect('clicked', () => this.reload());
        header.pack_end(refresh);

        // Narrow-window fallback switcher, revealed by a breakpoint.
        const switcherBar = new Adw.ViewSwitcherBar({stack: this.stack});
        const view = new Adw.ToolbarView({content: this.stack});
        view.add_top_bar(header);
        view.add_bottom_bar(switcherBar);

        const breakpoint = new Adw.Breakpoint({
            condition: Adw.BreakpointCondition.parse('max-width: 550px'),
        });
        breakpoint.add_setter(header.title_widget, 'visible', false);
        breakpoint.add_setter(switcherBar, 'reveal', true);
        this.window.add_breakpoint(breakpoint);

        this.toasts.child = view;
        this.window.content = this.toasts;

        this.reload();
    }

    toast(message) {
        this.toasts.add_toast(new Adw.Toast({title: message}));
    }

    async reload() {
        // Providers/consent over D-Bus; local catalog over the CLI. The
        // two are independent — one being down never blanks the other.
        try {
            this.state = await this.service.state();
            this.offline = false;
        } catch (e) {
            this.state = parseState(null);
            this.offline = true;
            logError?.(e, 'lisa-remoted unreachable');
        }
        try {
            this.catalog = await this.models.catalog();
        } catch (e) {
            this.catalog = null;
            logError?.(e, 'lisa models catalog failed');
        }
        this._render();
    }

    _render() {
        this._renderProviders();
        this._renderLocalModels();
    }

    _renderProviders() {
        if (this._provGroups)
            for (const g of this._provGroups)
                this.providersPage.remove(g);
        this._provGroups = [];

        const readiness = remoteReadiness(this.state);
        const disabledReason = providersDisabledReason({offline: this.offline});

        this.banner.title = this.offline
            ? `${disabledReason} Showing defaults; nothing leaves this machine.`
            : offloadSummary(this.state.mayOffload);
        if (!this.offline && !anythingLeaves(this.state.mayOffload))
            this.banner.remove_css_class(EGRESS_CSS_CLASS);
        else if (!this.offline)
            this.banner.add_css_class(EGRESS_CSS_CLASS);

        // Keyed provider but `prompt` consent off → every remote request
        // is refused. Say so at the top of the page, not in a log.
        this.consentBanner.revealed =
            !this.offline && readiness.hasKeyedProvider && !readiness.promptAllowed;

        this._provGroups.push(this._providersHeader(readiness, disabledReason));
        for (const row of providerRows(this.state.providers))
            this._provGroups.push(this._providerCard(row, readiness));
        this._provGroups.push(this._consentGroup());
        for (const g of this._provGroups)
            this.providersPage.add(g);
    }

    _renderLocalModels() {
        if (this._localGroups)
            for (const g of this._localGroups)
                this.localPage.remove(g);
        this._localGroups = [this._localModelGroup()];
        for (const g of this._localGroups)
            this.localPage.add(g);
    }

    /** Section header: title, description, and the add-provider button. */
    _providersHeader(readiness, disabledReason) {
        const group = new Adw.PreferencesGroup({
            title: 'Remote providers',
            description: 'Bring-your-own accounts. Requests through a provider ' +
                'leave your hardware and are marked in the Ledger.' +
                (readiness.usable
                    ? ' Ready: a key is set and the ‘prompt’ scope is on.'
                    : ''),
        });
        const add = new Gtk.Button({
            icon_name: 'list-add-symbolic',
            valign: Gtk.Align.CENTER,
            sensitive: disabledReason === null,
            tooltip_text: disabledReason ?? 'Add an OpenAI-compatible endpoint',
        });
        add.connect('clicked', () => this._addProviderDialog());
        group.header_suffix = add;
        return group;
    }

    /**
     * One card per provider (the construct pattern, libadwaita-native):
     * a header row with the brand logo, the key-state pill and removal
     * for custom endpoints; the write-only key row; then the live model
     * dropdown when the card can actually serve models, otherwise the
     * routing guidance.
     */
    _providerCard(row, readiness) {
        const card = new Adw.PreferencesGroup();

        const header = new Adw.ActionRow({
            title: GLib.markup_escape_text(row.title, -1),
            subtitle: GLib.markup_escape_text(row.subtitle, -1),
        });
        header.add_prefix(this._logoImage(row.id));

        const badge = new Gtk.Label({
            label: 'leaves your hardware',
            valign: Gtk.Align.CENTER,
        });
        badge.add_css_class(EGRESS_CSS_CLASS);
        badge.add_css_class('caption');
        header.add_suffix(badge);

        const pill = new Gtk.Label({
            label: row.hasCredential ? 'key set' : 'no key',
            valign: Gtk.Align.CENTER,
        });
        pill.add_css_class('provider-pill');
        pill.add_css_class('caption');
        pill.add_css_class(row.hasCredential ? 'has-key' : 'no-key');
        header.add_suffix(pill);

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
            header.add_suffix(remove);
        }
        card.add(header);

        // Keys are write-only: store/replace/forget, never read back.
        const keyRow = new Adw.ActionRow({
            title: 'API key',
            subtitle: row.hasCredential
                ? 'Stored by the broker — replace or forget it here.'
                : 'Not set — the provider refuses requests until you add one.',
        });
        const keyBtn = new Gtk.Button({
            label: row.hasCredential ? 'Replace key…' : 'Set key…',
            valign: Gtk.Align.CENTER,
        });
        keyBtn.connect('clicked', () => this._keyDialog(row));
        keyRow.add_suffix(keyBtn);

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
            keyRow.add_suffix(clear);
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
            keyRow.add_suffix(signIn);
        }
        card.add(keyRow);

        // Models: the live list when the card can serve models; the
        // routing guidance otherwise (never an invented list — rule 8).
        if (row.hasCredential && readiness.promptAllowed) {
            card.add(this._modelsRow(row));
        } else {
            card.add(this._modelsGuidanceRow(row));
            const raw = this.state.providers.find(p => p.id === row.id) ?? {id: row.id};
            const help = providerModelHelp(raw);
            const routeRow = new Adw.ActionRow({
                title: 'Use models as',
                subtitle: GLib.markup_escape_text(help.route, -1),
                subtitle_selectable: true,
            });
            const copy = Gtk.Button.new_from_icon_name('edit-copy-symbolic');
            copy.valign = Gtk.Align.CENTER;
            copy.tooltip_text = 'Copy the routing prefix';
            copy.connect('clicked', () => {
                this.window.get_clipboard().set(help.route);
                this.toast('Routing prefix copied');
            });
            routeRow.add_suffix(copy);
            card.add(routeRow);
            if (help.hint && help.hint !== help.route)
                card.add(new Adw.ActionRow({
                    title: 'Notes',
                    subtitle: GLib.markup_escape_text(help.hint, -1),
                    subtitle_selectable: true,
                }));
        }
        return card;
    }

    /**
     * The provider's brand logo, recolored with the foreground color:
     * the shipped SVGs draw in `currentColor`, which a texture does not
     * inherit from the widget tree — so we substitute the actual color
     * for the current light/dark theme before rendering.
     */
    _logoImage(providerId) {
        const key = `${providerId}:${this._styleManager.dark}`;
        if (!this._logoCache.has(key)) {
            let texture = null;
            try {
                const file = LOGO_DIR.get_child(providerLogoFile(providerId));
                const [, bytes] = file.load_bytes(null);
                const svg = new TextDecoder().decode(bytes.toArray())
                    .replaceAll('currentColor',
                        this._styleManager.dark ? '#ffffff' : '#1a1a1a');
                texture = Gdk.Texture.new_from_bytes(new GLib.Bytes(svg));
            } catch (e) {
                logError?.(e, `provider logo for ${providerId}`);
            }
            this._logoCache.set(key, texture);
        }
        const texture = this._logoCache.get(key);
        const image = texture
            ? Gtk.Image.new_from_paintable(texture)
            : new Gtk.Image({icon_name: 'network-server-symbolic'});
        image.pixel_size = 24;
        image.valign = Gtk.Align.CENTER;
        return image;
    }

    /**
     * The live "Models" row: fetch → spinner → a dropdown of the
     * provider's real model ids. Picking one copies its ready-to-use
     * route (`remote:<id>:<model>`) to the clipboard and shows it in the
     * row. Errors render inline with a Retry — never thrown.
     */
    _modelsRow(row) {
        const item = new Adw.ActionRow({
            title: 'Models',
            subtitle: 'Fetch the provider’s own /models list with your key — ' +
                'the request leaves this machine.',
        });
        const box = new Gtk.Box({spacing: 6, valign: Gtk.Align.CENTER});
        item.add_suffix(box);
        const swap = widget => {
            let child = box.get_first_child();
            while (child) {
                const next = child.get_next_sibling();
                box.remove(child);
                child = next;
            }
            box.append(widget);
        };
        const fetch = new Gtk.Button({
            label: 'Fetch models',
            valign: Gtk.Align.CENTER,
        });
        fetch.connect('clicked', () => this._loadModels(row, item, swap));
        box.append(fetch);
        return item;
    }

    async _loadModels(row, item, swap) {
        const spinner = new Gtk.Spinner({
            spinning: true,
            valign: Gtk.Align.CENTER,
        });
        swap(spinner);
        item.subtitle = 'Asking the provider for its model list…';
        const fail = message => {
            const retry = new Gtk.Button({
                label: 'Retry',
                valign: Gtk.Align.CENTER,
            });
            retry.connect('clicked', () => this._loadModels(row, item, swap));
            swap(retry);
            item.subtitle = GLib.markup_escape_text(message, -1);
        };
        let models;
        try {
            models = await this.service.listModels(row.id);
        } catch (e) {
            fail(e.message);
            return;
        }
        if (models.length === 0) {
            fail('The provider returned an empty model list.');
            return;
        }
        const list = new Gtk.StringList();
        for (const model of models)
            list.append(model);
        const drop = new Gtk.DropDown({model: list, valign: Gtk.Align.CENTER});
        // Nothing pre-selected — selecting is what copies the route.
        drop.selected = Gtk.INVALID_LIST_POSITION;
        drop.connect('notify::selected-item', () => {
            const model = drop.selected_item?.string;
            if (!model)
                return;
            const hint = modelHintFor(row.id, model);
            this.window.get_clipboard().set(hint);
            item.subtitle = GLib.markup_escape_text(hint, -1);
            this.toast(`Copied ${hint}`);
        });
        swap(drop);
        item.subtitle_selectable = true;
        item.subtitle = `${models.length} models — pick one to copy its route.`;
    }

    /** Why the live model list is unavailable on this card right now. */
    _modelsGuidanceRow(row) {
        const reason = !row.hasCredential
            ? 'Set a key above to fetch this provider’s live model list.'
            : 'Enable the ‘prompt’ scope below to fetch the live model list.';
        return new Adw.ActionRow({
            title: 'Models',
            subtitle: GLib.markup_escape_text(reason, -1),
        });
    }

    _localModelGroup() {
        const group = new Adw.PreferencesGroup({
            title: 'Local models',
            description: this.catalog
                ? `${profileSummary(this.catalog.profile)} Local inference never ` +
                    'leaves this machine.'
                : 'Could not read the local catalog (`lisa models catalog --json`). ' +
                    'Is the lisa CLI on PATH and up to date?',
        });
        if (!this.catalog)
            return group;

        for (const row of localModelRows(this.catalog.models)) {
            const item = new Adw.ActionRow({
                title: GLib.markup_escape_text(row.title, -1),
                subtitle: GLib.markup_escape_text(row.subtitle, -1),
            });
            const badge = new Gtk.Label({
                label: row.badge.label,
                valign: Gtk.Align.CENTER,
            });
            badge.add_css_class('caption');
            badge.add_css_class(row.installed ? 'success' : 'dim-label');
            item.add_suffix(badge);

            if (row.canGet) {
                const get = new Gtk.Button({
                    label: 'Get',
                    valign: Gtk.Align.CENTER,
                    tooltip_text: 'Download this model to run it locally',
                });
                get.add_css_class('suggested-action');
                get.connect('clicked', () => this._getModel(row, get));
                item.add_suffix(get);
            }
            group.add(item);
        }
        return group;
    }

    async _getModel(row, button) {
        button.sensitive = false;
        button.label = 'Downloading…';
        this.toast(`Downloading ${row.id} — this can take a while.`);
        try {
            await this.models.get(row.id);
            this.toast(`${row.id} installed`);
        } catch (e) {
            this.toast(e.message);
            button.sensitive = true;
            button.label = 'Get';
        }
        this.reload();
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
            if (row.primary) {
                // The scope every remote request carries; set it apart.
                const badge = new Gtk.Label({
                    label: 'required for remote',
                    valign: Gtk.Align.CENTER,
                });
                badge.add_css_class('caption');
                badge.add_css_class('dim-label');
                item.add_suffix(badge);
            }
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
