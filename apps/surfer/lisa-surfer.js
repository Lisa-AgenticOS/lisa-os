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
import Gdk from 'gi://Gdk?version=4.0';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import WebKit from 'gi://WebKit?version=6.0';

import {resolveInput, addressBarAction, DEFAULT_PLACEHOLDER} from './lib/url.js';
import {EXTRACT_JS, pageResult} from './lib/extract.js';
import {McpServer} from './lib/mcp.js';
import {navigationTarget, clickScript, fillScript} from './lib/actions.js';
import {evaluateInAgentWorld} from './lib/world.js';
import {pinTarget} from './lib/target.js';
import {rowLabel} from './lib/tablist.js';
import {suggestionsFor} from './lib/omnibox.js';
import {START_URI, START_PAGE_HTML, goQuery} from './lib/startpage.js';

/// The Unix-signal half of GLib, which moved.
///
/// `GLib.unix_signal_add` still works and prints a deprecation warning
/// on every current gjs ("has been moved to a separate platform-
/// specific library"), and `GLibUnix` does not exist on older ones. So
/// ask for it, the same way `apps/mail` does.
let GLibUnix = null;
try {
    GLibUnix = imports.gi.GLibUnix;
} catch {
    // Older GLib: the function is still on GLib itself.
}

/// Run `handler` when this process is sent `signal`, on the main loop.
function onUnixSignal(signal, handler) {
    if (GLibUnix?.signal_add)
        return GLibUnix.signal_add(GLib.PRIORITY_HIGH, signal, handler);
    return GLib.unix_signal_add(GLib.PRIORITY_HIGH, signal, handler);
}

const HOME = START_URI; // the local start page (lib/startpage.js)

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

/// Every open tab, in tab order, as lib/target.js wants them plus the
/// page itself. The snapshot is taken when an action is REQUESTED —
/// which is the whole point of #213: what is in front of the user at
/// that instant is not what the confirmation described.
function openTabs() {
    const selected = tabView.get_selected_page();
    const out = [];
    for (let i = 0; i < tabView.get_n_pages(); i++) {
        const page = tabView.get_nth_page(i);
        out.push({
            page,
            url: page.get_child()?.get_uri() ?? '',
            selected: page === selected,
        });
    }
    return out;
}

/// The view a write-tier action names, or a throw explaining why not
/// (lib/target.js owns the rule). Never falls back to "whatever is in
/// front of the user" — that fallback IS the bug.
function pinnedView(args) {
    const tabs = openTabs();
    return tabs[pinTarget(tabs, args)].page.get_child();
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
    // The start page's form navigates to lisa-go:?q=… — intercepted
    // here and routed through navigate(), the SAME resolveInput brain
    // as the address bar, so words search and addresses navigate in
    // both places (lib/startpage.js).
    view.connect('decide-policy', (v, decision, type) => {
        if (type !== WebKit.PolicyDecisionType.NAVIGATION_ACTION) return false;
        const uri = decision.get_navigation_action()?.get_request()?.get_uri() ?? '';
        const q = goQuery(uri);
        if (q === null) return false;
        decision.ignore();
        applyAddress(v, q);
        return true;
    });
    // Rail toggling resizes the content card, WebKit re-rasterizes its
    // GL surface, and the engine's backdrop for not-yet-painted frames
    // is opaque WHITE — dark pages flash bright for a frame (owner saw
    // it as flicker on collapse/expand). Painting the backdrop in the
    // scheme's own tone makes those frames invisible.
    const rgba = new Gdk.RGBA();
    rgba.parse(Adw.StyleManager.get_default().dark
        ? '#1B1917'   /* token: dark-base */
        : '#FFFFFF'); /* token: surface */
    view.set_background_color(rgba);
    const page = tabView.append(view);
    page.set_title('New Tab');
    view.connect('notify::title', () => {
        if (view.title) page.set_title(view.title);
    });
    view.connect('notify::uri', () => {
        if (tabView.get_selected_page() === page) {
            const uri = view.get_uri() ?? '';
            urlBar.set_text(uri.startsWith('lisa:') ? '' : uri);
        }
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
    if (url === START_URI) view.load_html(START_PAGE_HTML, START_URI);
    else if (url) view.load_uri(url);
    return view;
}

/// Enter in the address bar, and the start page's form — one brain, and
/// all THREE of resolveInput's outcomes (#220). The empty-bar case used
/// to reach `load_uri(null)`, which throws `Argument uri may not be
/// null` inside a signal handler where nobody sees it.
function applyAddress(view, text) {
    if (!view) return;
    const a = addressBarAction(text);
    // Always reset the placeholder: a refusal used to leave its reason
    // in the entry for the rest of the session.
    urlBar.set_placeholder_text(a.placeholder);
    if (a.act === 'refuse') {
        urlBar.set_text('');
        return;
    }
    if (a.act === 'load') view.load_uri(a.url);
}

function navigate(text) {
    applyAddress(currentView(), text);
}

/// The agent-facing read of the CURRENT tab. Promise resolves to the
/// extract.js page result; provenance tagging happens at the MCP edge.
///
/// The script runs in the agent world (lib/world.js, #212): in the
/// page's own world a page that redefines JSON.stringify forges this
/// entire result, title and text included.
async function readCurrentPage() {
    const view = currentView();
    if (!view) throw new Error('no open tab');
    const raw = await evaluateInAgentWorld(view, EXTRACT_JS);
    return pageResult(JSON.parse(raw), view.get_uri());
}

/// Write-tier agent actions (#166).
///
/// The tier lives in the manifest and the consent surface is agentd's,
/// not ours (ADR-0029: a check reachable from inside is not a
/// guardrail). But "somebody else escalates it" is not the same as
/// "this process may do as it is told": anything that can open the
/// socket reaches these handlers directly, which is exactly how #212,
/// #213 and #214 were reproduced on the device. So the rules that can
/// only be enforced HERE are enforced here, in deterministic code with
/// no model in the loop —
///
///   - what a page script may see:      lib/world.js  (#212)
///   - which tab an action may touch:   lib/target.js (#213)
///   - where an agent may navigate:     lib/actions.js (#214)
///
/// and each of them refuses rather than doing something approximate.

/// `navigate` acts on the tab the user is looking at, and its argument
/// IS its target — the consent dialog shows the address it will open,
/// so there is nothing about this call the approver cannot see. The
/// tab is read once, here, rather than at some later point.
async function agentNavigate({url}) {
    const view = currentView();
    if (!view) throw new Error('no open tab');
    // navigationTarget throws on javascript:/data:/etc AND on anything
    // that is not http/https — the agent allowlist, which is not the
    // address bar's rule (#214).
    const target = navigationTarget(url);
    const from = view.get_uri() ?? '';
    view.load_uri(target);
    return {navigating: target, from};
}

/// Run a page script on a PINNED view (never `currentView()` — #213)
/// and in the agent world (never the page's own — #212).
async function runPageScript(view, script) {
    const raw = await evaluateInAgentWorld(view, script);
    return {...JSON.parse(raw), url: view.get_uri()};
}

async function agentClick(args) {
    // pinnedView throws when the named page is not open any more; the
    // action is refused rather than landing on whatever replaced it.
    return runPageScript(pinnedView(args), clickScript(args.selector));
}

async function agentFill(args) {
    return runPageScript(pinnedView(args), fillScript(args.selector, args.value));
}

/// The agent-facing selection read.
async function readSelection() {
    const view = currentView();
    if (!view) throw new Error('no open tab');
    const selection = await evaluateInAgentWorld(
        view, 'window.getSelection().toString()');
    return {selection, url: view.get_uri()};
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
    // The socket lives exactly as long as a window does. `shutdown`
    // would reach it anyway when the last window closes; releasing here
    // as well means the tools stop advertising themselves before GTK
    // starts tearing the window down, rather than after (#219).
    win.connect('close-request', () => {
        release();
        return false;
    });

    const back = Gtk.Button.new_from_icon_name('go-previous-symbolic');
    const fwd = Gtk.Button.new_from_icon_name('go-next-symbolic');
    const reload = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
    back.connect('clicked', () => currentView()?.go_back());
    fwd.connect('clicked', () => currentView()?.go_forward());
    reload.connect('clicked', () => currentView()?.reload());

    urlBar = new Gtk.Entry({
        hexpand: true,
        placeholder_text: DEFAULT_PLACEHOLDER,
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
        // min == max: a RANGE lets allocation settle through
        // intermediate widths, and every step re-rasterizes the
        // webview. One width, one jump.
        min_sidebar_width: 240,
        max_sidebar_width: 240,
    });
    const newTabIcon = Gtk.Image.new_from_icon_name('tab-new-symbolic');
    const newTabFull = newTabBtn.get_child();
    // Width constraints accept updates only in one order per
    // direction: shrinking sets min first, growing sets max first.
    // The first animated attempt set min before max EVERY tick, so
    // every grow step tripped min>max, GTK clamped silently, and the
    // expand never happened — a clean journal, because a clamp is not
    // an error. This is the one place widths change; keep it that way.
    const setSidebarWidth = (w) => {
        if (w <= split.get_min_sidebar_width()) {
            split.set_min_sidebar_width(w);
            split.set_max_sidebar_width(w);
        } else {
            split.set_max_sidebar_width(w);
            split.set_min_sidebar_width(w);
        }
    };

    const showExtras = (visible) => {
        urlBar.set_visible(visible);
        back.set_visible(visible);
        fwd.set_visible(visible);
        reload.set_visible(visible);
        newTabBtn.set_child(visible ? newTabFull : newTabIcon);
        for (const entry of rows.values()) applyRail(entry);
    };

    let railAnim = null;
    const setRail = (on) => {
        if (rail === on) return;
        rail = on;
        railAnim?.pause();
        // Collapsing hides the wide chrome first; expanding reveals it
        // only once the width is there — text sliding under a growing
        // edge reads as clutter, ellipsis or not.
        if (rail) showExtras(false);
        railAnim = new Adw.TimedAnimation({
            widget: split,
            value_from: rail ? 240 : 56,
            value_to: rail ? 56 : 240,
            duration: 220,
            easing: Adw.Easing.EASE_OUT_CUBIC,
            target: Adw.CallbackAnimationTarget.new(setSidebarWidth),
        });
        if (!rail) railAnim.connect('done', () => showExtras(true));
        railAnim.play();
    };
    collapseBtn.connect('clicked', () => setRail(!rail));
    win.set_content(split);

    // The active row carries the brand accent (tokens: violet-500 —
    // the gate in os/repo-tools/check-tokens.py sanctions every hex
    // here).
    const css = new Gtk.CssProvider();
    const styleMgr = Adw.StyleManager.get_default();
    const loadCss = () => css.load_from_string(`
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
            /* tokens: dark-base / surface — the card follows the
               scheme; a white card in a dark session was the bug the
               owner saw as "opens white". */
            background: ${styleMgr.dark ? '#1B1917' : '#FFFFFF'};
            border-radius: 14px;
        }
    `);
    loadCss();
    styleMgr.connect('notify::dark', loadCss);
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

/// Give the socket back, once, whatever ended the process.
///
/// This hung off GApplication `shutdown` alone, which covers a clean
/// exit and nothing else — SIGTERM from systemd, a logout that kills
/// the session's units, `pkill`, a crash of the outer process — so a
/// killed Surfer left the socket file sitting in
/// `$XDG_RUNTIME_DIR/lisa/mcp`. mcp-bus defers socket activation and
/// reads PRESENCE AS AVAILABILITY, so a dead browser went on
/// advertising `read_page`, `navigate` and `click`, and agentd got
/// ECONNREFUSED where it should have been told "Surfer is not running"
/// (#219). Reproduced on the reference machine before this landed:
/// SIGTERM, no process, socket still there, `connect()` → ECONNREFUSED.
///
/// Idempotent, because more than one of these paths can fire: a window
/// close that quits the app reaches `shutdown` too.
let released = false;
function release() {
    if (released)
        return;
    released = true;
    try {
        mcp?.stop();
    } catch (e) {
        logError(e, 'lisa-surfer: releasing the agent socket');
    }
}
app.connect('shutdown', () => release());
// …and the signals that end a process without a `shutdown`. The handler
// runs on the main loop, so it may do real work; `quit()` then unwinds
// the app the ordinary way. SOURCE_REMOVE because a second SIGTERM
// should kill us rather than queue another quit.
for (const signal of [1 /* SIGHUP */, 2 /* SIGINT */, 15 /* SIGTERM */]) {
    onUnixSignal(signal, () => {
        release();
        app.quit();
        return GLib.SOURCE_REMOVE;
    });
}
app.run([]);
