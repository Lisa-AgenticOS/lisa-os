#!/usr/bin/env -S gjs -m
// Surfer — the web as an agent surface (ADR-0037, issue #146).
//
// GJS + GTK4 + libadwaita + WebKit-6.0, the same stack as
// shell/assistant. The engine is the webkitgtk-6.0 the image already
// ships; this file is chrome around it.
//
// Structure: Adw.TabView owns the per-tab WebViews; the strip that
// SHOWS them is a Zen/Arc-style collapsible sidebar (#182) — rows in a
// ListBox bound to the TabView's pages, Ctrl+S to toggle. The pure
// modules own the decisions: lib/url.js decides what the address bar
// means, lib/extract.js (via evaluate_javascript) reads pages for the
// agent, lib/mcp.js serves the Agent Bus socket while a window is open.
//
// FOOTGUN, learned in Phase 0: an Adw.Window created WITHOUT
// `application: app` leaves WebKit loads parked at progress 0.1
// forever, with no error anywhere. Every window here sets it.

import Adw from 'gi://Adw?version=1';
import Gtk from 'gi://Gtk?version=4.0';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import WebKit from 'gi://WebKit?version=6.0';

import {resolveInput} from './lib/url.js';
import {EXTRACT_JS, pageResult} from './lib/extract.js';
import {McpServer} from './lib/mcp.js';
import {navigationTarget, clickScript, fillScript} from './lib/actions.js';
import {rowLabel} from './lib/tablist.js';
import {suggestionsFor} from './lib/omnibox.js';

const HOME = 'https://duckduckgo.com';

/// Surfer's own version, which appears in the user agent. Bumped by
/// hand: it is a product token, not a build number, and a site that
/// reports "broken in Surfer/0.1" should be able to mean something by
/// it.
const VERSION = '0.1';

const app = new Adw.Application({application_id: 'app.lisaos.Surfer'});
let win = null;
let session = null;
let tabView = null;
let urlBar = null;
let mcp = null;

/// The one network session every tab shares — and the reason logins
/// survive a restart.
///
/// WebKitGTK keeps cookies in MEMORY unless persistent storage is turned
/// on explicitly; a WebView built with no session at all also lands in a
/// data directory named after the process (`gjs`), shared with every
/// other GJS app that touches WebKit. Both were true here until the
/// first real test: signing into Google worked, and signed you straight
/// back out on restart.
function networkSession() {
    if (session) return session;
    const data = GLib.build_filenamev([GLib.get_user_data_dir(), 'lisa-surfer']);
    const cache = GLib.build_filenamev([GLib.get_user_cache_dir(), 'lisa-surfer']);
    GLib.mkdir_with_parents(data, 0o700);
    GLib.mkdir_with_parents(cache, 0o700);
    session = new WebKit.NetworkSession({
        data_directory: data,
        cache_directory: cache,
    });
    // The line that actually persists a login. SQLite rather than the
    // plain-text format: it is the one WebKit maintains, and a cookie
    // jar is a credential store in everything but name.
    session.get_cookie_manager().set_persistent_storage(
        GLib.build_filenamev([data, 'cookies.sqlite']),
        WebKit.CookiePersistentStorage.SQLITE);
    // Third-party cookies stay blocked. Logins that need them are rare
    // and the alternative is being tracked across every site you open.
    session.get_cookie_manager().set_accept_policy(
        WebKit.CookieAcceptPolicy.NO_THIRD_PARTY);
    // Sidebar favicons (#182): get_favicon() returns null forever
    // unless the favicon database is switched on — the rows would show
    // the fallback globe and nothing would ever say why.
    session.get_website_data_manager().set_favicons_enabled(true);
    return session;
}

/// The settings every WebView gets.
///
/// # The user agent, and why it is set at all
///
/// WebKitGTK's default announces itself as `Version/60.5 Safari/605.1.15`.
/// There is no Safari 60.5 — the number tracks WebKitGTK's own release,
/// not Safari's — and sites that branch on it read it as an unknown or
/// ancient browser. YouTube was the report: the page loads and video will
/// not play.
///
/// **The codec half of that diagnosis was wrong** (corrected 2026-08-02,
/// #146). This comment used to say "every codec is installed (checked on
/// the device: vp9, vp8, av1, opus, h264, aac all present)". Asked
/// element by element on the reference device instead of by grep:
///
///     vp8dec present   vp9dec present
///     opusdec MISSING  av1dec MISSING  dav1ddec MISSING
///     avdec_h264 MISSING  avdec_aac MISSING  faad MISSING
///
/// The only opus elements on the system are `rtpopusdepay` and
/// `rtpopuspay` — RTP payloaders from gst-plugins-good, not decoders.
/// A `gst-inspect-1.0 | grep -i opus` finds those two and reads as
/// "opus present", which is almost certainly how the original claim was
/// made. (I made the same mistake from the other direction the same day,
/// with `ls /usr/lib/gstreamer-1.0 | grep opus`.) Ask for the element by
/// name; a substring is not an answer.
///
/// So there were TWO independent reasons YouTube video did not play, and
/// fixing the user agent addressed one of them. The image now declares
/// gst-plugins-base, gst-libav and gst-plugins-bad (os/mkosi/mkosi.conf),
/// and ab-recovery asserts `vp9dec` AND `opusdec` register in the booted
/// image — because vp9 without opus plays silently, which looks like it
/// works.
///
/// So the version becomes one that exists, and Surfer names itself at
/// the end — the shape Epiphany uses. Everything before the product
/// token is left exactly as WebKitGTK sends it: this is a real WebKit,
/// and the one thing that was misleading was the number.
///
/// The token is a deliberate small risk. A site that allowlists known
/// browsers may read it the way YouTube read `Version/60.5`; the trade
/// is that a site can report a bug against us by name, and that we are
/// not pretending to be something we are not. If it turns out to cost
/// compatibility, dropping it is one line — and `LISA_SURFER_UA` makes
/// that testable without a rebuild, which is also how the next "what
/// does the site actually see" question gets answered.
function viewSettings() {
    const settings = new WebKit.Settings({
        // Present tense, and true: this engine is WebKit.
        user_agent: GLib.getenv('LISA_SURFER_UA') ||
            'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 ' +
            `(KHTML, like Gecko) Version/18.3 Safari/605.1.15 Surfer/${VERSION}`,
    });
    // NOT enabling encrypted-media (EME). It defaults off, and turning it
    // on without a Widevine CDM installed buys nothing except telling
    // sites we support DRM we cannot actually play. When there is a
    // reason to ship a CDM, that is its own decision with its own
    // consequences — not a flag flipped in passing.
    return settings;
}

function currentView() {
    const page = tabView.get_selected_page();
    return page ? page.get_child() : null;
}

/// Wire an EXISTING WebView into a tab. Split out because the `create`
/// signal hands us a view WebKit made itself, which must not be
/// re-created or pre-loaded.
function attachTab(view, focus = true) {
    // Without these the view is sized to its natural height and the rest
    // of the tab is dead space — the page renders as a band with black
    // below it (seen on the first real screenshot, 2026-07-29).
    view.set_vexpand(true);
    view.set_hexpand(true);
    const page = tabView.append(view);
    page.set_title('New Tab');
    view.connect('notify::title', () => {
        if (view.title) page.set_title(view.title);
    });
    view.connect('notify::uri', () => {
        if (tabView.get_selected_page() === page) urlBar.set_text(view.get_uri() ?? '');
    });
    view.connect('notify::estimated-load-progress', () => {
        page.set_loading(view.estimated_load_progress < 1);
    });
    // window.open / target=_blank / middle-click.
    //
    // WebKit REQUIRES the returned view to be constructed with
    // `related-view` — it shares the opener's web process and its
    // window-open relationship — and it must NOT be loaded here, because
    // WebKit performs the load itself on the view we hand back. Returning
    // a fresh unrelated WebView that had already loaded about:blank is
    // undefined behaviour, and it crashed the browser on the first real
    // popup it met: Google sign-in (2026-07-29).
    view.connect('create', (opener) => {
        // No network_session here on purpose: a related view inherits
        // the opener's session, and passing both is an error.
        const popup = new WebKit.WebView({
            related_view: opener,
            settings: viewSettings(),
        });
        // Attach only once WebKit says it is ready; attaching a view that
        // never becomes ready would leave an empty tab behind.
        popup.connect('ready-to-show', () => attachTab(popup, true));
        popup.connect('close', () => {
            const p = tabView.get_page(popup);
            if (p) tabView.close_page(p);
        });
        return popup;
    });
    if (focus) tabView.set_selected_page(page);
    return view;
}

/// A new tab we open ourselves, with a URL to load.
function newTab(url = HOME, focus = true) {
    const view = attachTab(new WebKit.WebView({
        network_session: networkSession(),
        settings: viewSettings(),
    }), focus);
    if (url) view.load_uri(url);
    return view;
}

function navigate(text) {
    const view = currentView();
    if (!view) return;
    const r = resolveInput(text);
    if (r.kind === 'refused') {
        // Say why in the URL bar's tooltip area rather than failing mute.
        urlBar.set_text('');
        urlBar.set_placeholder_text(r.reason);
        return;
    }
    view.load_uri(r.url);
}

/// The agent-facing read of the CURRENT tab. Promise resolves to the
/// extract.js page result; provenance tagging happens at the MCP edge.
function readCurrentPage() {
    return new Promise((resolve, reject) => {
        const view = currentView();
        if (!view) { reject(new Error('no open tab')); return; }
        view.evaluate_javascript(EXTRACT_JS, -1, null, null, null, (v, res) => {
            try {
                const value = v.evaluate_javascript_finish(res);
                resolve(pageResult(JSON.parse(value.to_string()), v.get_uri()));
            } catch (e) {
                reject(e);
            }
        });
    });
}

/// Write-tier agent actions (#166). agentd has already escalated these
/// through the consent surface before they reach this process — the
/// tier lives in the manifest and the guard in agentd, never here
/// (ADR-0029: a check reachable from inside is not a guardrail). What
/// lives here is only the doing.
function agentNavigate({url}) {
    const view = currentView();
    if (!view) return Promise.reject(new Error('no open tab'));
    // navigationTarget throws on javascript:/data:/etc — resolveInput's
    // refusal list, reused not re-implemented (#166).
    const target = navigationTarget(url);
    view.load_uri(target);
    return Promise.resolve({navigating: target});
}

function runPageScript(script) {
    return new Promise((resolve, reject) => {
        const view = currentView();
        if (!view) { reject(new Error('no open tab')); return; }
        view.evaluate_javascript(script, -1, null, null, null, (v, res) => {
            try {
                const value = v.evaluate_javascript_finish(res);
                resolve({...JSON.parse(value.to_string()), url: v.get_uri()});
            } catch (e) {
                reject(e);
            }
        });
    });
}

function agentClick({selector}) {
    return runPageScript(clickScript(selector));
}

function agentFill({selector, value}) {
    return runPageScript(fillScript(selector, value));
}

/// The agent-facing selection read.
function readSelection() {
    return new Promise((resolve, reject) => {
        const view = currentView();
        if (!view) { reject(new Error('no open tab')); return; }
        view.evaluate_javascript('window.getSelection().toString()', -1, null, null, null, (v, res) => {
            try {
                const value = v.evaluate_javascript_finish(res);
                resolve({selection: value.to_string(), url: v.get_uri()});
            } catch (e) {
                reject(e);
            }
        });
    });
}

/// Screenshot of the visible viewport as PNG bytes (capped upstream by
/// the visible region — a viewport is bounded where a full page is not).
function screenshotCurrent() {
    return new Promise((resolve, reject) => {
        const view = currentView();
        if (!view) { reject(new Error('no open tab')); return; }
        view.get_snapshot(
            WebKit.SnapshotRegion.VISIBLE, WebKit.SnapshotOptions.NONE, null,
            (v, res) => {
                try {
                    const texture = v.get_snapshot_finish(res);
                    const bytes = texture.save_to_png_bytes();
                    resolve({png: bytes.get_data(), url: v.get_uri()});
                } catch (e) {
                    reject(e);
                }
            });
    });
}

function buildWindow() {
    win = new Adw.Window({
        application: app, // NOT optional — see the footgun note up top.
        title: 'Surfer',
        default_width: 1280,
        default_height: 860,
    });

    const back = Gtk.Button.new_from_icon_name('go-previous-symbolic');
    const fwd = Gtk.Button.new_from_icon_name('go-next-symbolic');
    const reload = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
    back.connect('clicked', () => currentView()?.go_back());
    fwd.connect('clicked', () => currentView()?.go_forward());
    reload.connect('clicked', () => currentView()?.reload());

    urlBar = new Gtk.Entry({
        hexpand: true,
        placeholder_text: 'Search or enter address',
        input_purpose: Gtk.InputPurpose.URL,
    });
    urlBar.connect('activate', () => { suggestPopover?.popdown(); navigate(urlBar.get_text()); });

    // Address-bar suggestions (#182): open tabs + the search row —
    // sources that exist. History-backed completion waits for a
    // history feature; provider suggest-as-you-type is egress per
    // keystroke and waits for its own toggle (lib/omnibox.js).
    const suggestList = new Gtk.ListBox({css_classes: ['lisa-suggest']});
    const suggestPopover = new Gtk.Popover({
        child: suggestList,
        autohide: false,
        has_arrow: false,
        can_focus: false,
    });
    suggestPopover.set_parent(urlBar);
    const rebuildSuggestions = () => {
        const text = urlBar.get_text();
        const tabs = [];
        for (let i = 0; i < tabView.get_n_pages(); i++) {
            const v = tabView.get_nth_page(i).get_child();
            tabs.push({title: v?.title ?? '', uri: v?.get_uri?.() ?? ''});
        }
        const items = suggestionsFor(text, tabs);
        let row = suggestList.get_first_child();
        while (row) { const next = row.get_next_sibling(); suggestList.remove(row); row = next; }
        if (!items.length || !urlBar.has_focus) { suggestPopover.popdown(); return; }
        for (const item of items) {
            const label = new Gtk.Label({xalign: 0, ellipsize: 3, margin_start: 8, margin_end: 8, margin_top: 4, margin_bottom: 4});
            if (item.kind === 'url') label.set_text(`Go to ${item.url}`);
            else if (item.kind === 'tab') label.set_text(`Switch to: ${item.title || item.uri}`);
            else label.set_text(`Search DuckDuckGo for “${item.query}”`);
            const r = new Gtk.ListBoxRow({child: label});
            r._item = item;
            suggestList.append(r);
        }
        suggestPopover.set_size_request(urlBar.get_width(), -1);
        suggestPopover.popup();
    };
    urlBar.connect('changed', rebuildSuggestions);
    suggestList.connect('row-activated', (_l, r) => {
        const item = r._item;
        suggestPopover.popdown();
        if (item.kind === 'tab') tabView.set_selected_page(tabView.get_nth_page(item.index));
        else if (item.kind === 'url') currentView()?.load_uri(item.url);
        else navigate(item.query);
    });

    // Zen/Arc anatomy (#182 v2, from the owner's references): the
    // SIDEBAR owns navigation and the address bar; there is no top
    // header at all, and the page floats as a rounded card in the
    // tinted frame. WindowHandle keeps the chrome draggable without a
    // HeaderBar; WindowControls keeps close/minimize.
    const navRow = new Gtk.Box({spacing: 2, margin_start: 6, margin_end: 6, margin_top: 6});
    navRow.append(back);
    navRow.append(fwd);
    navRow.append(reload);
    const navSpacer = new Gtk.Box({hexpand: true});
    navRow.append(navSpacer);
    navRow.append(new Gtk.WindowControls({side: Gtk.PackType.END}));
    urlBar.add_css_class('lisa-urlbar');
    urlBar.set_margin_start(10);
    urlBar.set_margin_end(10);
    urlBar.set_margin_top(6);
    urlBar.set_margin_bottom(4);

    tabView = new Adw.TabView({vexpand: true, hexpand: true});
    tabView.connect('notify::selected-page', () => {
        const view = currentView();
        urlBar.set_text(view?.get_uri() ?? '');
    });
    // Closing the last tab closes the window; Adw.TabView handles
    // neighbour focus on close by itself.
    tabView.connect('close-page', (_tv, page) => {
        tabView.close_page_finish(page, true);
        if (tabView.get_n_pages() === 0) win.close();
        return true;
    });
    // Zen/Arc-shaped: tabs live in a collapsible SIDEBAR, not a strip
    // (#182). Adw.TabView stays the model — rows below are only a
    // different presentation of the same pages, so drag-out, agent tab
    // handles and close-page semantics are untouched.
    const tabList = new Gtk.ListBox({css_classes: ['navigation-sidebar', 'lisa-tablist']});
    tabList.set_selection_mode(Gtk.SelectionMode.SINGLE);
    const rows = new Map(); // Adw.TabPage → {row, label, close, favicon}

    const rowFor = (page) => {
        const label = new Gtk.Label({xalign: 0, hexpand: true, ellipsize: 3 /* END */});
        // The VIEW's title, not the page's (#189): attachTab seeds the
        // page title with 'New Tab', so rowLabel's title argument was
        // never empty and its host fallback was unreachable — untitled
        // pages read "New Tab" forever with a live URI in hand. The
        // view is the source; uri changes resync too, so a titled →
        // untitled navigation updates instead of going stale.
        const view = page.get_child();
        const favicon = new Gtk.Image({
            icon_name: 'web-browser-symbolic',
            pixel_size: 16,
        });
        const sync = () => {
            label.set_text(rowLabel(view?.title ?? '', view?.get_uri?.() ?? ''));
            // webkit6 hands a Gdk.Texture directly; null between
            // navigations, so the globe holds the slot.
            const tex = view?.get_favicon?.();
            if (tex) favicon.set_from_paintable(tex);
            else favicon.set_from_icon_name('web-browser-symbolic');
        };
        view?.connect('notify::title', sync);
        view?.connect('notify::uri', sync);
        view?.connect('notify::favicon', sync);
        sync();
        const close = new Gtk.Button({
            icon_name: 'window-close-symbolic',
            css_classes: ['flat', 'circular'],
            valign: Gtk.Align.CENTER,
        });
        close.connect('clicked', () => tabView.close_page(page));
        const box = new Gtk.Box({spacing: 8, margin_top: 4, margin_bottom: 4});
        box.append(favicon);
        box.append(label);
        box.append(close);
        const row = new Gtk.ListBoxRow({child: box});
        // Middle-click closes, the way every tab strip has always worked.
        const mid = new Gtk.GestureClick({button: 2});
        mid.connect('pressed', () => tabView.close_page(page));
        row.add_controller(mid);
        return {row, label, close, favicon};
    };

    tabView.connect('page-attached', (_tv, page) => {
        const entry = rowFor(page);
        rows.set(page, entry);
        applyRail(entry);
        tabList.append(entry.row);
    });
    tabView.connect('page-detached', (_tv, page) => {
        const entry = rows.get(page);
        if (entry) tabList.remove(entry.row);
        rows.delete(page);
    });
    tabView.connect('notify::selected-page', () => {
        const entry = rows.get(tabView.get_selected_page());
        if (entry && tabList.get_selected_row() !== entry.row)
            tabList.select_row(entry.row);
    });
    tabList.connect('row-selected', (_l, row) => {
        if (!row) return;
        for (const [page, entry] of rows) {
            if (entry.row === row && tabView.get_selected_page() !== page)
                tabView.set_selected_page(page);
        }
    });

    const newTabBtn = new Gtk.Button({
        child: new Adw.ButtonContent({icon_name: 'tab-new-symbolic', label: 'New Tab', halign: Gtk.Align.START}),
        css_classes: ['flat', 'lisa-newtab'],
        margin_start: 8, margin_end: 8,
    });
    newTabBtn.connect('clicked', () => newTab());

    const top = new Gtk.WindowHandle({
        child: (() => {
            const b = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
            b.append(navRow);
            b.append(urlBar);
            return b;
        })(),
    });

    // Bottom mini-bar, reference-shaped: just the things that exist —
    // the rail toggle. Spaces and downloads join when THEY exist
    // (rule 10 applies to buttons too).
    //
    // COLLAPSE MEANS RAIL, NEVER GONE (owner, at the machine, having
    // hidden the sidebar with no visible way back): the sidebar
    // narrows to a favicon column with this toggle still in it. The
    // way back must live inside the thing that shrank.
    let rail = false;
    const applyRail = (entry) => {
        entry.label.set_visible(!rail);
        entry.close.set_visible(!rail);
    };
    const collapseBtn = Gtk.Button.new_from_icon_name('sidebar-show-symbolic');
    collapseBtn.add_css_class('flat');
    const bottomBar = new Gtk.Box({margin_start: 8, margin_end: 8, margin_bottom: 8, margin_top: 4});
    bottomBar.append(collapseBtn);

    const scroller = new Gtk.ScrolledWindow({child: tabList, vexpand: true});
    const sidebar = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, css_classes: ['lisa-sidebar']});
    sidebar.append(top);
    sidebar.append(scroller);
    sidebar.append(newTabBtn);
    sidebar.append(bottomBar);

    // The page as a floating rounded card inside the tinted frame.
    const contentCard = new Gtk.Box({
        css_classes: ['lisa-content-card'],
        margin_top: 10, margin_end: 10, margin_bottom: 10,
    });
    contentCard.append(tabView);

    const split = new Adw.OverlaySplitView({
        sidebar,
        content: contentCard,
        min_sidebar_width: 210,
        max_sidebar_width: 250,
    });
    const newTabIcon = Gtk.Image.new_from_icon_name('tab-new-symbolic');
    const newTabFull = newTabBtn.get_child();
    const setRail = (on) => {
        rail = on;
        urlBar.set_visible(!rail);
        back.set_visible(!rail);
        fwd.set_visible(!rail);
        reload.set_visible(!rail);
        newTabBtn.set_child(rail ? newTabIcon : newTabFull);
        for (const entry of rows.values()) applyRail(entry);
        split.set_min_sidebar_width(rail ? 56 : 210);
        split.set_max_sidebar_width(rail ? 56 : 250);
    };
    collapseBtn.connect('clicked', () => setRail(!rail));
    win.set_content(split);

    // The active row carries the brand accent (tokens: violet-500 —
    // the gate in os/repo-tools/check-tokens.py sanctions every hex
    // here).
    const css = new Gtk.CssProvider();
    css.load_from_string(`
        window { background: mix(#4F378B, #1B1917, 0.72); } /* tokens: violet-700 into dark-base */
        .lisa-sidebar { background: transparent; }
        .lisa-urlbar {
            border-radius: 10px;
            background: alpha(#FFF1E9, 0.08); /* token: warm-white */
            color: #FFF1E9;
            border: none;
            min-height: 30px;
        }
        .lisa-tablist { background: transparent; }
        .lisa-tablist row { border-radius: 10px; margin: 1px 8px; color: alpha(#FFF1E9, 0.82); }
        .lisa-tablist row:hover { background: alpha(#FFF1E9, 0.07); }
        .lisa-tablist row:selected { background: alpha(#9B7BE8, 0.28); } /* token: violet-300 */
        .lisa-tablist row:selected label { color: #FFF1E9; font-weight: 600; }
        .lisa-newtab { color: alpha(#FFF1E9, 0.65); border-radius: 10px; }
        .lisa-suggest { background: transparent; }
        .lisa-suggest row { border-radius: 8px; padding: 2px; }
        .lisa-suggest row:hover { background: alpha(#9B7BE8, 0.25); } /* token: violet-300 */
        .lisa-content-card {
            background: #FFFFFF; /* token: surface */
            border-radius: 14px;
        }
    `);
    Gtk.StyleContext.add_provider_for_display(
        win.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);

    // Shortcuts: the three everyone's hands already know.
    const add = (accel, fn) => {
        const action = new Gio.SimpleAction({name: accel.replace(/\W/g, '')});
        action.connect('activate', fn);
        app.add_action(action);
        app.set_accels_for_action(`app.${accel.replace(/\W/g, '')}`, [accel]);
    };
    add('<Control>t', () => newTab());
    add('<Control>w', () => {
        const page = tabView.get_selected_page();
        if (page) tabView.close_page(page);
    });
    add('<Control>l', () => urlBar.grab_focus());
    add('<Control>s', () => setRail(!rail));

    win.present();
}

app.connect('activate', () => {
    if (win) { win.present(); return; }
    buildWindow();
    newTab(ARGV[0] && resolveInput(ARGV[0]).kind === 'load' ? resolveInput(ARGV[0]).url : HOME);
    // The Agent Bus socket lives exactly as long as a window does
    // (mcp-bus defers socket activation, so presence == usability).
    mcp = new McpServer({
        readCurrentPage, readSelection, screenshotCurrent,
        agentNavigate, agentClick, agentFill,
    });
    mcp.start();
});
app.connect('shutdown', () => mcp?.stop());
app.run([]);
