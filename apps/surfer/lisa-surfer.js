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
import {
    AGENT_PROFILE, DEFAULT_PROFILE, dataDirFor,
} from './lib/profiles.js';
import {EXTRACT_JS, pageResult} from './lib/extract.js';
import {McpServer} from './lib/mcp.js';
import {navigationTarget, clickScript, fillScript} from './lib/actions.js';
import {evaluateInAgentWorld} from './lib/world.js';
import {pinTarget} from './lib/target.js';
import {rowLabel} from './lib/tablist.js';
import {suggestionsFor} from './lib/omnibox.js';
import {START_URI, START_PAGE_HTML, goQuery} from './lib/startpage.js';
import {
    decodeSettings, decodeStore, encodeSettings, encodeStore, profileStorePath,
} from './lib/store.js';
import {
    agentDriven, clearFinished, completeDownload, destinationFor,
    downloadFraction, downloadLabel, failDownload, persistableDownloads,
    removeDownload, resolveConflict, startedDownload, trimDownloads,
    updateDownload,
} from './lib/downloads.js';
import {
    addVisit, clearHistory, forgetSince, forgetUrl, historyLabel, recordable,
    retitle, searchHistory,
} from './lib/history.js';
import {
    bookmarkLabel, isBookmarked, removeBookmark, searchBookmarks, toggleBookmark,
} from './lib/bookmarks.js';
import {
    restoreEnabled, selectedIndex, sessionSnapshot, tabsToRestore,
} from './lib/session.js';
import {MAX_MATCH_COUNT, findOptions, matchLabel, searchable} from './lib/find.js';
import {zoomIn, zoomLabel, zoomOut, zoomReset} from './lib/zoom.js';

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
let tabView = null;
let urlBar = null;
let mcp = null;

/// The network session a tab browses in — and the reason logins survive
/// a restart.
///
/// WebKitGTK keeps cookies in MEMORY unless persistent storage is turned
/// on explicitly; a WebView built with no session at all also lands in a
/// data directory named after the process (`gjs`), shared with every
/// other GJS app that touches WebKit. Both were true here until the
/// first real test: signing into Google worked, and signed you straight
/// back out on restart.
/// One `NetworkSession` per profile (#259).
///
/// Keyed by name, because a WebView is constructed against a session and
/// two views in the same profile must share one — a second session on
/// the same directory is two cookie jars over one file.
const sessions = new Map();

function networkSession(profile = DEFAULT_PROFILE) {
    const existing = sessions.get(profile);
    if (existing) return existing;
    const base = GLib.build_filenamev([GLib.get_user_data_dir(), 'lisa-surfer']);
    const cacheBase = GLib.build_filenamev([GLib.get_user_cache_dir(), 'lisa-surfer']);
    const data = dataDirFor(profile, base);
    const cache = dataDirFor(profile, cacheBase);
    if (!data || !cache) {
        // An unsafe name never reaches a path. Fall back to the agent
        // profile rather than to the person's: the failure mode of a
        // wrong-but-safe session is a logged-out browser.
        return profile === AGENT_PROFILE ? null : networkSession(AGENT_PROFILE);
    }
    GLib.mkdir_with_parents(data, 0o700);
    GLib.mkdir_with_parents(cache, 0o700);
    const session = new WebKit.NetworkSession({
        data_directory: data,
        cache_directory: cache,
    });
    sessions.set(profile, session);
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
    // Downloads. **The signal is on the SESSION, not the WebView** —
    // `WebKitWebView` has no `download-started` in the 6.0 API; the GIR
    // on the reference device puts it on `WebKitNetworkSession`
    // (WebKitNetworkSession.cpp:203). Connecting it here rather than per
    // view also means a download started from a popup, a redirect or a
    // link with `download=` is caught the same way as one the person
    // clicked.
    session.connect('download-started', (_s, download) => {
        try {
            beginDownload(download);
        } catch (e) {
            logError(e, 'lisa-surfer: starting a download');
            try { download.cancel(); } catch { /* already gone */ }
        }
    });
    return session;
}

// ---------------------------------------------------------------------
// Per-profile storage
// ---------------------------------------------------------------------
//
// History, bookmarks, the downloads list, the session snapshot and the
// handful of settings all live in the profile's own directory. The path
// comes from lib/store.js and NOWHERE else — that module is the only
// place a profile name becomes a path, which is what stops the agent
// profile's browsing from being appended to the person's history.

const DATA_BASE = GLib.build_filenamev([GLib.get_user_data_dir(), 'lisa-surfer']);

/// The profile this window browses in. One for now; `lib/profiles.js`
/// already knows how to name more, and everything below is keyed by this
/// variable rather than by a constant so adding a switcher is a UI job
/// and not a storage job.
const activeProfile = DEFAULT_PROFILE;

function storePath(kind) {
    return profileStorePath(activeProfile, DATA_BASE, kind);
}

/// Read a store file. A missing file and a corrupt one are the same
/// answer — an empty list — because a browser that will not start
/// because its history is truncated is worse than one with no history.
function storeLoad(kind, key) {
    const path = storePath(kind);
    if (!path) return [];
    try {
        const [ok, bytes] = GLib.file_get_contents(path);
        if (!ok) return [];
        return decodeStore(new TextDecoder().decode(bytes), key);
    } catch {
        return [];
    }
}

function storeSave(kind, key, items) {
    const path = storePath(kind);
    if (!path) return;
    try {
        GLib.mkdir_with_parents(GLib.path_get_dirname(path), 0o700);
        GLib.file_set_contents(path, encodeStore(key, items));
    } catch (e) {
        logError(e, `lisa-surfer: writing ${kind}`);
    }
}

function settingsLoad() {
    const path = storePath('settings');
    if (!path) return {};
    try {
        const [ok, bytes] = GLib.file_get_contents(path);
        if (!ok) return {};
        return decodeSettings(new TextDecoder().decode(bytes));
    } catch {
        return {};
    }
}

function settingsSave(next) {
    const path = storePath('settings');
    if (!path) return;
    try {
        GLib.mkdir_with_parents(GLib.path_get_dirname(path), 0o700);
        GLib.file_set_contents(path, encodeSettings(next));
    } catch (e) {
        logError(e, 'lisa-surfer: writing settings');
    }
}

let settings = {};
let history = [];
let bookmarks = [];
let downloads = [];

/// Redraw hooks, set by buildWindow. Held as variables rather than
/// called directly so this half of the file has no idea what a ListBox
/// is — the same split the pure modules get.
let onDownloadsChanged = () => {};
let onBookmarksChanged = () => {};

/// Say something, briefly, where the person is looking. Set by
/// buildWindow; a no-op before there is a window, which is when nobody
/// could read it anyway.
let showToast = () => {};

// ---------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------

/// Where files land. XDG's Downloads directory, with `$HOME/Downloads`
/// as the fallback for a machine whose user-dirs were never generated.
///
/// `LISA_SURFER_DOWNLOAD_DIR` overrides it. That exists so the download
/// path can be exercised on a real machine without writing into
/// somebody's actual Downloads folder — the same reason
/// `LISA_SURFER_UA` exists.
function downloadsDir() {
    const override = GLib.getenv('LISA_SURFER_DOWNLOAD_DIR');
    if (override && override !== '') return override;
    return GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_DOWNLOAD) ||
        GLib.build_filenamev([GLib.get_home_dir(), 'Downloads']);
}

const fileExists = (path) => GLib.file_test(path, GLib.FileTest.EXISTS);

let downloadSeq = 0;

/// The GTK half of a download. Every decision in here comes from
/// lib/downloads.js; what is left is signals and a dialog.
function beginDownload(download) {
    const view = download.get_web_view();
    const uri = download.get_request()?.get_uri() ?? '';

    // THE AGENT BOUNDARY. Surfer exposes no `download` tool, but
    // `navigate` and `click` are enough on their own: an http address
    // that answers `Content-Disposition: attachment` writes a file. A
    // download that starts while an agent-driven action is still in
    // flight is refused, in deterministic code with no model in it
    // (CLAUDE.md 6a) — the rule itself is `agentDriven`, tested.
    if (agentDriven({agentTouchedAt: view?._agentTouchedAt, now: Date.now()})) {
        download.cancel();
        log(`lisa-surfer: refused an agent-driven download of ${uri}`);
        showToast('Refused a download an agent asked for');
        return;
    }

    const id = `d${++downloadSeq}`;
    let decided = false;

    download.connect('decide-destination', (d, suggested) => {
        // Answered exactly once. WebKit re-emits on a redirect, and a
        // second destination for a transfer already in flight is a file
        // written somewhere nobody was told about.
        if (decided) return true;
        decided = true;
        const dir = downloadsDir();
        GLib.mkdir_with_parents(dir, 0o700);
        let decision;
        try {
            decision = destinationFor({suggested, uri, dir, exists: fileExists});
        } catch (e) {
            logError(e, 'lisa-surfer: deciding a download destination');
            d.cancel();
            return true;
        }
        if (decision.action === 'save') {
            d.set_destination(decision.path);
            addDownloadRow(id, d, uri, decision);
            return true;
        }
        // A conflict is a question, and returning TRUE without setting a
        // destination is how WebKit ≥2.40 is told to wait for the
        // answer. The transfer does not proceed until `set_destination`
        // or `cancel` — so an unanswered dialog is a stalled download,
        // never an overwritten file.
        askAboutConflict(d, id, uri, decision);
        return true;
    });

    download.connect('received-data', (d) => {
        const total = d.get_response()?.get_content_length() ?? 0;
        downloads = updateDownload(downloads, id, {
            received: d.get_received_data_length(),
            total,
        });
        onDownloadsChanged();
    });
    download.connect('created-destination', (_d, destination) => {
        downloads = updateDownload(downloads, id, {path: destination});
        onDownloadsChanged();
    });
    download.connect('finished', () => {
        // `finished` fires after `failed` too, so a row already marked
        // failed must not be promoted to done.
        const row = downloads.find(e => e.id === id);
        if (row && row.state === 'running')
            downloads = completeDownload(downloads, id, Date.now());
        persistDownloads();
        onDownloadsChanged();
    });
    download.connect('failed', (_d, error) => {
        downloads = failDownload(downloads, id, error?.message ?? 'failed', Date.now());
        persistDownloads();
        onDownloadsChanged();
    });
}

function addDownloadRow(id, download, uri, decision) {
    downloads = trimDownloads([
        startedDownload({
            id,
            uri,
            filename: GLib.path_get_basename(decision.path ?? decision.suggestion ?? ''),
            path: decision.path,
            startedAt: Date.now(),
        }),
        ...downloads,
    ]);
    downloads = updateDownload(downloads, id, {
        total: download.get_response()?.get_content_length() ?? 0,
    });
    onDownloadsChanged();
}

function persistDownloads() {
    storeSave('downloads', 'downloads', persistableDownloads(downloads));
}

/// "Something is already called that." Three answers, and every way of
/// dismissing the dialog is the third one (lib/downloads.js's
/// `resolveConflict` defaults to cancel).
function askAboutConflict(download, id, uri, decision) {
    const name = GLib.path_get_basename(decision.path);
    const alt = GLib.path_get_basename(decision.suggestion);
    const dialog = new Adw.AlertDialog({
        heading: 'A file called that is already there',
        body: `${name} already exists in ${GLib.path_get_dirname(decision.path)}.`,
    });
    dialog.add_response('cancel', 'Cancel');
    dialog.add_response('replace', 'Replace');
    dialog.add_response('keep-both', `Keep both (${alt})`);
    dialog.set_response_appearance('replace', Adw.ResponseAppearance.DESTRUCTIVE);
    dialog.set_response_appearance('keep-both', Adw.ResponseAppearance.SUGGESTED);
    dialog.set_default_response('keep-both');
    // Escape closes with this response, which `resolveConflict` treats
    // as a cancel like every other answer it does not recognise.
    dialog.set_close_response('cancel');
    dialog.choose(win, null, (d, res) => {
        let answer = 'cancel';
        try {
            answer = d.choose_finish(res);
        } catch (e) {
            logError(e, 'lisa-surfer: download conflict dialog');
        }
        const outcome = resolveConflict(answer, decision);
        if (outcome.action !== 'save') {
            download.cancel();
            return;
        }
        // `allow-overwrite` defaults FALSE, and a replace that is not
        // granted it fails with WEBKIT_DOWNLOAD_ERROR_DESTINATION —
        // which looks, from the outside, like a download that broke for
        // no reason. It is set from the decision, never by default.
        download.set_allow_overwrite(outcome.allowOverwrite);
        download.set_destination(outcome.path);
        addDownloadRow(id, download, uri, {path: outcome.path});
    });
}

/// Open the folder a finished download landed in.
///
/// The FOLDER, not the file selected inside it: selecting a file is
/// `org.freedesktop.FileManager1.ShowItems`, which is a D-Bus call to
/// whatever file manager happens to own that name, and this does not
/// make it. What it does is open the directory.
///
/// The URI comes from `Gio.File`, never from string concatenation — a
/// download called `my report #2.pdf` has two characters in it that mean
/// something else in a URI.
function revealDownload(entry) {
    const path = entry?.path;
    if (!path || !fileExists(path)) return;
    launchUri(Gio.File.new_for_path(GLib.path_get_dirname(path)).get_uri(),
        'opening the downloads folder');
}

function openDownload(entry) {
    const path = entry?.path;
    if (!path || !fileExists(path)) return;
    launchUri(Gio.File.new_for_path(path).get_uri(), 'opening a downloaded file');
}

function launchUri(uri, what) {
    try {
        Gio.AppInfo.launch_default_for_uri(uri, null);
    } catch (e) {
        logError(e, `lisa-surfer: ${what}`);
        showToast(`Could not open ${uri}`);
    }
}

// ---------------------------------------------------------------------
// History
// ---------------------------------------------------------------------

/// Write down a page the person actually landed on.
///
/// `recordable` decides, and it takes the profile: the agent's browsing
/// is not the person's history (lib/history.js).
function recordVisit(uri, title) {
    if (!recordable(uri, activeProfile)) return;
    history = addVisit(history, {url: uri, title, at: Date.now()});
    storeSave('history', 'history', history);
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
        // A response the engine cannot render is a file, not a blank
        // page. Without this, clicking a .zip or a .tar.zst shows an
        // empty tab and nothing anywhere says why — WebKit only starts
        // a download of its own accord for `Content-Disposition:
        // attachment`, and most servers do not send it.
        if (type === WebKit.PolicyDecisionType.RESPONSE) {
            if (decision.is_mime_type_supported()) return false;
            decision.download();
            return true;
        }
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
    // History. The visit is recorded when the load FINISHES — a URL that
    // is still loading may still redirect, and a history full of
    // intermediate hops is a history nobody can use. The title arrives
    // later than that on most pages, so it is corrected in place rather
    // than counted as a second visit (lib/history.js `retitle`).
    view.connect('load-changed', (v, event) => {
        if (event !== WebKit.LoadEvent.FINISHED) return;
        recordVisit(v.get_uri() ?? '', v.title ?? '');
        if (tabView.get_selected_page() === page) onBookmarksChanged();
    });
    view.connect('notify::title', () => {
        const uri = view.get_uri() ?? '';
        if (!recordable(uri, activeProfile) || !view.title) return;
        history = retitle(history, uri, view.title);
        storeSave('history', 'history', history);
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
        // The agent stamp is INHERITED. An agent `click` that opens a
        // popup which then answers with an attachment would otherwise be
        // an agent-caused write to disk on a view nothing had stamped —
        // the boundary has to follow the causation, not the widget.
        popup._agentTouchedAt = opener._agentTouchedAt;
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
    stampAgent(view);
    view.load_uri(target);
    return {navigating: target, from};
}

/// Mark a view as having just been driven by an agent.
///
/// `beginDownload` reads this stamp: an http address that answers with
/// `Content-Disposition: attachment` starts a download, so `navigate`
/// and `click` are a write to disk even though neither is called one.
/// The rule that reads the stamp is `agentDriven` in lib/downloads.js,
/// which is tested; this is only the clock.
function stampAgent(view) {
    if (view) view._agentTouchedAt = Date.now();
}

/// Run a page script on a PINNED view (never `currentView()` — #213)
/// and in the agent world (never the page's own — #212).
async function runPageScript(view, script) {
    // A click can land on a download link, so the view is stamped before
    // the script runs, not after it returns.
    stampAgent(view);
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

/// Ctrl+D, and the star. One function so the two cannot disagree about
/// what "already bookmarked" means (lib/bookmarks.js decides).
function toggleCurrentBookmark() {
    const view = currentView();
    const uri = view?.get_uri() ?? '';
    if (uri === '') return;
    bookmarks = toggleBookmark(bookmarks, {
        url: uri, title: view?.title ?? '', at: Date.now(),
    });
    storeSave('bookmarks', 'bookmarks', bookmarks);
    onBookmarksChanged();
}

/// The history and bookmarks windows, which are the same window twice.
///
/// A search box, a list, open-on-activate, delete-per-row, and whatever
/// bulk actions the caller wants at the bottom. Deleting is a first-class
/// button rather than a menu item three levels down: a history a person
/// cannot clear is surveillance, and one they cannot clear EASILY is
/// surveillance with a defence.
function openLibraryWindow({title, rowsFor, onActivate, onDelete, bulk = []}) {
    const listBox = new Gtk.ListBox({
        css_classes: ['boxed-list'],
        selection_mode: Gtk.SelectionMode.NONE,
        margin_start: 12, margin_end: 12, margin_top: 12, margin_bottom: 12,
    });
    const search = new Gtk.SearchEntry({
        hexpand: true, placeholder_text: 'Search',
        margin_start: 12, margin_end: 12, margin_top: 12,
    });
    const empty = new Gtk.Label({
        label: 'Nothing here yet',
        css_classes: ['dim-label'],
        margin_top: 24, margin_bottom: 24,
    });

    const refresh = () => {
        let child = listBox.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            listBox.remove(child);
            child = next;
        }
        const rows = rowsFor(search.get_text());
        empty.set_visible(rows.length === 0);
        for (const row of rows) {
            const label = new Gtk.Label({
                label: row.label, xalign: 0, hexpand: true, ellipsize: 3,
            });
            const sub = new Gtk.Label({
                label: row.sublabel, xalign: 0, css_classes: ['dim-label'], ellipsize: 3,
            });
            const text = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, hexpand: true});
            text.append(label);
            text.append(sub);
            const open = new Gtk.Button({
                label: 'Open', css_classes: ['flat'], valign: Gtk.Align.CENTER,
            });
            open.connect('clicked', () => onActivate(row));
            const drop = Gtk.Button.new_from_icon_name('user-trash-symbolic');
            drop.add_css_class('flat');
            drop.set_valign(Gtk.Align.CENTER);
            drop.set_tooltip_text('Forget this');
            drop.connect('clicked', () => { onDelete(row); refresh(); });
            const box = new Gtk.Box({
                spacing: 8,
                margin_start: 10, margin_end: 10, margin_top: 6, margin_bottom: 6,
            });
            box.append(text);
            box.append(open);
            box.append(drop);
            listBox.append(new Gtk.ListBoxRow({child: box, activatable: false}));
        }
    };
    search.connect('search-changed', refresh);

    const content = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
    content.append(search);
    content.append(empty);
    const scroller = new Gtk.ScrolledWindow({child: listBox, vexpand: true});
    content.append(scroller);
    if (bulk.length > 0) {
        const bar = new Gtk.Box({
            spacing: 6, halign: Gtk.Align.END,
            margin_start: 12, margin_end: 12, margin_bottom: 12,
        });
        for (const b of bulk) {
            const button = new Gtk.Button({label: b.label});
            if (b.destructive) button.add_css_class('destructive-action');
            button.connect('clicked', () => { b.run(); refresh(); });
            bar.append(button);
        }
        content.append(bar);
    }

    const view = new Adw.ToolbarView({content});
    view.add_top_bar(new Adw.HeaderBar());
    const window = new Adw.Window({
        // `application: app` — the footgun at the top of this file. No
        // WebView lives in here, but an unattached window is a shape this
        // app does not use anywhere else and should not start now.
        application: app,
        transient_for: win,
        title,
        default_width: 720,
        default_height: 560,
        content: view,
    });
    refresh();
    window.present();
    return window;
}

function showHistoryWindow() {
    const hour = 60 * 60 * 1000;
    openLibraryWindow({
        title: 'History',
        rowsFor: (query) => searchHistory(history, query).map(e => ({
            label: historyLabel(e), sublabel: e.url, url: e.url,
        })),
        onActivate: (row) => newTab(row.url),
        onDelete: (row) => {
            history = forgetUrl(history, row.url);
            storeSave('history', 'history', history);
        },
        bulk: [
            {
                label: 'Clear last hour',
                run: () => {
                    history = forgetSince(history, Date.now() - hour);
                    storeSave('history', 'history', history);
                },
            },
            {
                label: 'Clear all history',
                destructive: true,
                run: () => {
                    history = clearHistory();
                    storeSave('history', 'history', history);
                },
            },
        ],
    });
}

function showBookmarksWindow() {
    openLibraryWindow({
        title: 'Bookmarks',
        rowsFor: (query) => searchBookmarks(bookmarks, query).map(e => ({
            label: bookmarkLabel(e), sublabel: e.url, url: e.url,
        })),
        onActivate: (row) => newTab(row.url),
        onDelete: (row) => {
            // `removeBookmark`, not `toggleBookmark`: a delete button
            // that adds the row back when the list and the file have
            // drifted is a delete button nobody can trust.
            bookmarks = removeBookmark(bookmarks, row.url);
            storeSave('bookmarks', 'bookmarks', bookmarks);
            onBookmarksChanged();
        },
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
        // `release` writes the session as its first act, while the tabs
        // still exist — see its note.
        release();
        return false;
    });

    const back = Gtk.Button.new_from_icon_name('go-previous-symbolic');
    const fwd = Gtk.Button.new_from_icon_name('go-next-symbolic');
    const reload = Gtk.Button.new_from_icon_name('view-refresh-symbolic');
    back.connect('clicked', () => currentView()?.go_back());
    fwd.connect('clicked', () => currentView()?.go_forward());
    reload.connect('clicked', () => currentView()?.reload());

    // The star. `onBookmarksChanged` is what keeps it honest across tab
    // switches, navigations and the bookmarks window — one redraw hook,
    // so the icon can never disagree with the file.
    const star = Gtk.Button.new_from_icon_name('non-starred-symbolic');
    star.add_css_class('flat');
    star.set_tooltip_text('Bookmark this page (Ctrl+D)');
    star.connect('clicked', () => toggleCurrentBookmark());
    onBookmarksChanged = () => {
        const uri = currentView()?.get_uri() ?? '';
        const on = isBookmarked(bookmarks, uri);
        star.set_icon_name(on ? 'starred-symbolic' : 'non-starred-symbolic');
        star.set_tooltip_text(on
            ? 'Remove bookmark (Ctrl+D)'
            : 'Bookmark this page (Ctrl+D)');
    };

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
    // GTK4: a widget attached with set_parent() must be unparented
    // before its parent is disposed, or GTK finalizes the entry with a
    // child still on it. Two Surfer sessions on the device ended with
    // exactly that warning and no coredump, no signal and no JS error —
    // the process shut down, which from the user's side is
    // indistinguishable from a crash (#258). Whether that caused the
    // disappearance is unproven; unclean teardown is a bug either way,
    // and it is the kind that behaves differently each run.
    win.connect('close-request', () => {
        suggestPopover.popdown();
        suggestPopover.unparent();
        return false;
    });
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
    navRow.append(star);
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
        onBookmarksChanged();
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
    const bottomBar = new Gtk.Box({
        spacing: 2,
        margin_start: 8, margin_end: 8, margin_bottom: 8, margin_top: 4,
    });
    bottomBar.append(collapseBtn);

    // Downloads live behind a button that only exists once something has
    // been downloaded — an empty tray is chrome that has never told
    // anybody anything.
    const downloadsBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL,
        spacing: 4,
        margin_top: 6, margin_bottom: 6, margin_start: 6, margin_end: 6,
    });
    const downloadsClear = new Gtk.Button({
        label: 'Clear finished',
        css_classes: ['flat'],
        halign: Gtk.Align.END,
    });
    downloadsClear.connect('clicked', () => {
        downloads = clearFinished(downloads);
        persistDownloads();
        onDownloadsChanged();
    });
    const downloadsScroller = new Gtk.ScrolledWindow({
        child: downloadsBox,
        propagate_natural_height: true,
        max_content_height: 420,
        hscrollbar_policy: Gtk.PolicyType.NEVER,
    });
    const downloadsPane = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 4});
    downloadsPane.append(downloadsScroller);
    downloadsPane.append(downloadsClear);
    const downloadsPopover = new Gtk.Popover({child: downloadsPane, width_request: 380});
    const downloadsBtn = new Gtk.MenuButton({
        icon_name: 'folder-download-symbolic',
        popover: downloadsPopover,
        css_classes: ['flat'],
        tooltip_text: 'Downloads (Ctrl+J)',
        visible: false,
    });
    bottomBar.append(downloadsBtn);

    const rebuildDownloads = () => {
        let child = downloadsBox.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            downloadsBox.remove(child);
            child = next;
        }
        downloadsBtn.set_visible(downloads.length > 0);
        downloadsClear.set_visible(downloads.some(e => e.state !== 'running'));
        if (downloads.length === 0) return;
        for (const entry of downloads) {
            const name = new Gtk.Label({
                label: entry.filename || entry.uri,
                xalign: 0, hexpand: true, ellipsize: 3 /* END */,
            });
            const sub = new Gtk.Label({
                label: downloadLabel(entry),
                xalign: 0, css_classes: ['dim-label'], ellipsize: 3,
            });
            const text = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, hexpand: true});
            text.append(name);
            text.append(sub);
            if (entry.state === 'running') {
                const bar = new Gtk.ProgressBar();
                const fraction = downloadFraction(entry);
                // A server that sent no Content-Length gets a pulsing
                // bar. A bar pinned at zero for a transfer that is
                // moving is a bug report waiting to happen.
                if (fraction === null) bar.pulse();
                else bar.set_fraction(fraction);
                text.append(bar);
            }
            const row = new Gtk.Box({spacing: 6});
            row.append(text);
            if (entry.state === 'done') {
                const open = Gtk.Button.new_from_icon_name('document-open-symbolic');
                open.add_css_class('flat');
                open.set_tooltip_text('Open');
                open.connect('clicked', () => openDownload(entry));
                const folder = Gtk.Button.new_from_icon_name('folder-symbolic');
                folder.add_css_class('flat');
                folder.set_tooltip_text('Show in folder');
                folder.connect('clicked', () => revealDownload(entry));
                row.append(open);
                row.append(folder);
            }
            if (entry.state !== 'running') {
                const drop = Gtk.Button.new_from_icon_name('window-close-symbolic');
                drop.add_css_class('flat');
                // Said out loud, because a downloads list that deletes
                // files when you tidy it is one nobody tidies twice.
                drop.set_tooltip_text('Remove from this list (the file stays)');
                drop.connect('clicked', () => {
                    downloads = removeDownload(downloads, entry.id);
                    persistDownloads();
                    onDownloadsChanged();
                });
                row.append(drop);
            }
            downloadsBox.append(row);
        }
    };
    onDownloadsChanged = rebuildDownloads;

    // The menu: the things that have no button, and the one setting
    // Surfer has.
    const menuModel = new Gio.Menu();
    const pageSection = new Gio.Menu();
    pageSection.append('Find in Page', 'app.find');
    pageSection.append('Print…', 'app.print');
    menuModel.append_section(null, pageSection);
    const zoomSection = new Gio.Menu();
    zoomSection.append('Zoom In', 'app.zoomin');
    zoomSection.append('Zoom Out', 'app.zoomout');
    zoomSection.append('Normal Size', 'app.zoomreset');
    menuModel.append_section(null, zoomSection);
    const libSection = new Gio.Menu();
    libSection.append('History', 'app.history');
    libSection.append('Bookmarks', 'app.bookmarks');
    libSection.append('Downloads', 'app.downloads');
    menuModel.append_section(null, libSection);
    const prefSection = new Gio.Menu();
    prefSection.append('Reopen Tabs on Launch', 'app.restoresession');
    menuModel.append_section(null, prefSection);
    const menuBtn = new Gtk.MenuButton({
        icon_name: 'open-menu-symbolic',
        menu_model: menuModel,
        css_classes: ['flat'],
        tooltip_text: 'Menu',
    });
    const bottomSpacer = new Gtk.Box({hexpand: true});
    bottomBar.append(bottomSpacer);
    bottomBar.append(menuBtn);

    const scroller = new Gtk.ScrolledWindow({child: tabList, vexpand: true});
    const sidebar = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, css_classes: ['lisa-sidebar']});
    sidebar.append(top);
    sidebar.append(scroller);
    sidebar.append(newTabBtn);
    sidebar.append(bottomBar);

    // Find in page (Ctrl+F). `WebKit.FindController` belongs to a VIEW,
    // so the bar drives whichever view is in front at the moment a key
    // is pressed — and every view it has touched gets its signals
    // connected exactly once, tracked on the view itself. Connecting
    // them per search would fire the handler once per Ctrl+F ever
    // pressed, which reads as a counter that jumps.
    const findEntry = new Gtk.SearchEntry({
        hexpand: true,
        placeholder_text: 'Find in page',
    });
    const findCount = new Gtk.Label({css_classes: ['dim-label'], width_chars: 12, xalign: 1});
    const findPrev = Gtk.Button.new_from_icon_name('go-up-symbolic');
    const findNext = Gtk.Button.new_from_icon_name('go-down-symbolic');
    const findClose = Gtk.Button.new_from_icon_name('window-close-symbolic');
    for (const b of [findPrev, findNext, findClose]) b.add_css_class('flat');
    const findBar = new Gtk.Box({
        spacing: 6,
        margin_start: 10, margin_end: 10, margin_top: 8, margin_bottom: 4,
    });
    findBar.append(findEntry);
    findBar.append(findCount);
    findBar.append(findPrev);
    findBar.append(findNext);
    findBar.append(findClose);
    const findRevealer = new Gtk.Revealer({child: findBar, reveal_child: false});

    const findControllerFor = (view) => {
        if (!view) return null;
        const controller = view.get_find_controller();
        if (!view._findWired) {
            view._findWired = true;
            controller.connect('found-text', (_c, count) => {
                findCount.set_text(matchLabel(findEntry.get_text(), count));
            });
            controller.connect('failed-to-find-text', () => {
                findCount.set_text(matchLabel(findEntry.get_text(), 0));
            });
            controller.connect('counted-matches', (_c, count) => {
                findCount.set_text(matchLabel(findEntry.get_text(), count));
            });
        }
        return controller;
    };
    // Stepping through matches is `search_next`/`search_previous`, NOT a
    // second `search()` with the same text — that restarts from the top,
    // so Enter and the arrows would land on the first match forever.
    let findActive = false;
    const runFind = ({mode = 'new'} = {}) => {
        const controller = findControllerFor(currentView());
        if (!controller) return;
        const text = findEntry.get_text();
        if (!searchable(text)) {
            controller.search_finish();
            findActive = false;
            findCount.set_text('');
            return;
        }
        // Stepping only means anything once a search is running. Before
        // that — Ctrl+G with text left in the box from another tab — it
        // silently does nothing, so start one instead.
        if (mode !== 'new' && findActive) {
            if (mode === 'prev') controller.search_previous();
            else controller.search_next();
            return;
        }
        findCount.set_text(matchLabel(text, null));
        controller.count_matches(text, findOptions({}), MAX_MATCH_COUNT);
        controller.search(text, findOptions({}), MAX_MATCH_COUNT);
        findActive = true;
    };
    const closeFind = () => {
        findControllerFor(currentView())?.search_finish();
        findActive = false;
        findRevealer.set_reveal_child(false);
        findCount.set_text('');
        currentView()?.grab_focus();
    };
    const openFind = () => {
        findRevealer.set_reveal_child(true);
        findEntry.grab_focus();
        findEntry.select_region(0, -1);
        if (searchable(findEntry.get_text())) runFind();
    };
    findEntry.connect('search-changed', () => runFind());
    findEntry.connect('activate', () => runFind({mode: 'next'}));
    findEntry.connect('stop-search', closeFind);
    findNext.connect('clicked', () => runFind({mode: 'next'}));
    findPrev.connect('clicked', () => runFind({mode: 'prev'}));
    findClose.connect('clicked', closeFind);

    // The page as a floating rounded card inside the tinted frame, with
    // the find bar inside the card so it scrolls with nothing and sits
    // over the page it is searching.
    const contentCard = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL,
        css_classes: ['lisa-content-card'],
        margin_top: 10, margin_end: 10, margin_bottom: 10,
    });
    contentCard.append(findRevealer);
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
    // Toasts are where the things with no widget of their own speak:
    // the zoom level, a refused download, "nothing has been downloaded
    // yet". A refusal that says nothing is indistinguishable from a
    // browser that is broken.
    const toasts = new Adw.ToastOverlay({child: split});
    showToast = (text) => toasts.add_toast(new Adw.Toast({title: text, timeout: 3}));
    win.set_content(toasts);

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

    // Shortcuts. Named actions rather than names derived from the accel,
    // because the menu above references them by name and
    // `app.Controlt` is not a name anybody can read in a menu model.
    const add = (name, accels, fn) => {
        const action = new Gio.SimpleAction({name});
        action.connect('activate', fn);
        app.add_action(action);
        if (accels.length > 0) app.set_accels_for_action(`app.${name}`, accels);
    };
    add('newtab', ['<Control>t'], () => newTab());
    add('closetab', ['<Control>w'], () => {
        const page = tabView.get_selected_page();
        if (page) tabView.close_page(page);
    });
    add('focusurl', ['<Control>l'], () => urlBar.grab_focus());
    add('togglesidebar', ['<Control>s'], () => setRail(!rail));
    add('find', ['<Control>f'], openFind);
    add('findnext', ['<Control>g'], () => runFind({mode: 'next'}));
    add('findprev', ['<Control><Shift>g'], () => runFind({mode: 'prev'}));
    add('bookmark', ['<Control>d'], () => toggleCurrentBookmark());
    add('history', ['<Control>h'], () => showHistoryWindow());
    add('bookmarks', ['<Control><Shift>o'], () => showBookmarksWindow());
    add('downloads', ['<Control>j'], () => {
        // The button hides itself when nothing has been downloaded, and
        // popping up a popover on a hidden widget is a GTK warning and
        // no popover. Say so instead.
        if (!downloadsBtn.get_visible()) {
            showToast('Nothing downloaded yet');
            return;
        }
        downloadsPopover.popup();
    });
    // Zoom is per TAB: two tabs at different sizes is what people
    // expect, and a global zoom means one hard-to-read site resizes
    // every other one.
    const setZoom = (fn) => {
        const view = currentView();
        if (!view) return;
        const level = fn(view.get_zoom_level());
        view.set_zoom_level(level);
        showToast(`Zoom ${zoomLabel(level)}`);
    };
    // Both spellings of the plus key: GTK reports `plus` from the main
    // row and `equal` from an unshifted press, and a Ctrl+= that does
    // nothing is the single most common zoom bug there is.
    add('zoomin', ['<Control>plus', '<Control>equal', '<Control>KP_Add'],
        () => setZoom(zoomIn));
    add('zoomout', ['<Control>minus', '<Control>KP_Subtract'], () => setZoom(zoomOut));
    add('zoomreset', ['<Control>0', '<Control>KP_0'], () => setZoom(zoomReset));
    add('print', ['<Control>p'], () => {
        const view = currentView();
        if (!view) return;
        // WebKit owns the print dialog; it renders the page the engine
        // laid out rather than a screenshot of it.
        WebKit.PrintOperation.new(view).run_dialog(win);
    });

    // The one setting Surfer has, as a stateful action so the menu shows
    // a check mark rather than a line of text that lies.
    const restoreAction = Gio.SimpleAction.new_stateful(
        'restoresession', null, GLib.Variant.new_boolean(restoreEnabled(settings)));
    restoreAction.connect('activate', () => {
        const next = !restoreAction.get_state().get_boolean();
        restoreAction.set_state(GLib.Variant.new_boolean(next));
        settings = {...settings, restoreSession: next};
        settingsSave(settings);
        // Turning it OFF discards the snapshot as well as declining to
        // read it. A saved session that survives being switched off is
        // still on disk, and "off" has to mean the tabs are not there.
        if (!next) clearSessionFile();
    });
    app.add_action(restoreAction);

    onBookmarksChanged();
    onDownloadsChanged();
    win.present();
}

/// Remove the session snapshot. Used both when restore is switched off
/// and when there is nothing worth saving — leaving yesterday's file
/// behind is how a closed tab comes back tomorrow.
function clearSessionFile() {
    const path = storePath('session');
    if (path) GLib.unlink(path);
}

/// Write down what is open, if the person wants it written down.
///
/// Refuses to run without a live TabView. Called after the window is
/// torn down it would snapshot nothing, and "nothing" is written as
/// "you had no tabs open" — which deletes a perfectly good session.
function saveSession() {
    if (!tabView) return;
    if (!restoreEnabled(settings)) { clearSessionFile(); return; }
    const snapshot = sessionSnapshot(openTabs().map(t => ({
        url: t.url,
        title: t.page.get_child()?.title ?? '',
        selected: t.selected,
    })), {at: Date.now(), profile: activeProfile});
    if (!snapshot) { clearSessionFile(); return; }
    const path = storePath('session');
    if (!path) return;
    try {
        GLib.mkdir_with_parents(GLib.path_get_dirname(path), 0o700);
        GLib.file_set_contents(path, JSON.stringify(snapshot));
    } catch (e) {
        logError(e, 'lisa-surfer: writing the session');
    }
}

function loadSessionSnapshot() {
    const path = storePath('session');
    if (!path) return null;
    try {
        const [ok, bytes] = GLib.file_get_contents(path);
        if (!ok) return null;
        return JSON.parse(new TextDecoder().decode(bytes));
    } catch {
        return null;
    }
}

app.connect('activate', () => {
    if (win) { win.present(); return; }
    // The stores are read BEFORE the window is built: the star, the
    // downloads button and the restore check mark are all drawn from
    // them, and a window built first shows an empty browser for a frame
    // and then contradicts itself.
    settings = settingsLoad();
    history = storeLoad('history', 'history');
    bookmarks = storeLoad('bookmarks', 'bookmarks');
    downloads = storeLoad('downloads', 'downloads');
    const snapshot = loadSessionSnapshot();
    buildWindow();
    // An address on the command line wins over a restored session: the
    // person asked for that page, now.
    const argument = ARGV[0] ? resolveInput(ARGV[0]) : null;
    const restored = argument?.kind === 'load'
        ? []
        : tabsToRestore(snapshot, {settings, profile: activeProfile});
    if (argument?.kind === 'load') {
        newTab(argument.url);
    } else if (restored.length > 0) {
        for (const tab of restored) newTab(tab.url, false);
        const index = selectedIndex(snapshot, restored.length);
        if (index >= 0) tabView.set_selected_page(tabView.get_nth_page(index));
    } else {
        newTab(HOME);
    }
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
    // The session first, because every path that ends this process
    // comes through here and only SOME of them are a window close.
    // `shutdown` alone was never enough: a logout, a `pkill` or systemd
    // stopping the session unit ends the browser without ever emitting
    // `close-request`, and a session that survives only a polite quit is
    // a session restore nobody can rely on.
    try {
        saveSession();
    } catch (e) {
        logError(e, 'lisa-surfer: writing the session on exit');
    }
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
