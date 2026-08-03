// Mail — your mail, and the assistant's view of it (PLAN §5.3, §5.8).
//
// Three panes: folders, a grouped message list, and a reading pane. That
// layout is what every mail client has converged on, and the grouped
// middle pane is the part worth taking seriously — newsletters,
// automated notifications and mail a person wrote are three different
// reading modes, and mixing them is why inboxes feel like work.
//
// # What this is for
//
// Lisa's reason to have a mail app is not that the world needs another
// one. It is that `mail` is the context source PLAN §5.3 cares most
// about and the one nothing has ever fed: the ACL has had a `mail`
// provenance since M3 with no source to put anything in it, and the
// prompt-injection machinery escalates on mail provenance with no mail
// to escalate on. This app is that source.
//
// So the Agent Bus tools are not an afterthought bolted on the side.
// They are half the point, and everything they emit is tagged `mail`
// (lib/mcp-protocol.js) — which is what makes "read my mail and then do
// something privileged" ask first.
//
// # Sync is somebody else's job
//
// Mail reads a **Maildir**. It does not speak IMAP, and will not: an OS
// whose defining constraint is egress control should not grow its own
// network mail client when `mbsync`, `offlineimap` and `notmuch` are
// mature and already write the format. See lib/maildir.js.

imports.gi.versions.Gtk = '4.0';
imports.gi.versions.Adw = '1';

import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk';

import {
    decodeWords, parseAddress, parseHeaders, readableBody, renderableBody, splitMessage,
} from './lib/rfc822.js';
import {
    listFolder, messageId, messagePath, parseFilename, previewOf, uniqueMatchesId,
} from './lib/maildir.js';
import {classify, grouped, unreadCount} from './lib/smart.js';
import {actionsFor, flagChange, moveTo} from './lib/actions.js';
import {parseMessageId, planFor} from './lib/agent-actions.js';
import {buildMessage, forwardFields, messageIdFor, replyFields} from './lib/compose.js';
import {msmtpArgv, sendOutcome, sentFilename} from './lib/send.js';
import {McpServer} from './lib/mcp.js';
import {linkAction} from './lib/links.js';

import {
    CONFIG_NAME, accountRows, parseConfig, resolveMaildir, serializeConfig,
    storeSummary, syncStatus, validateMaildir,
} from './lib/settings.js';

/// WebKit, if this system has it.
///
/// Loaded dynamically because the app has to open on a machine without
/// it — a dev checkout on another distro, the minimal image — where the
/// right behaviour is plain text, not a stack trace. `null` means the
/// reading pane stays a TextView, which is what it was before HTML
/// rendering existed and is a perfectly good mail reader.
///
/// NOT `await import()`, which is what this was and what silently killed
/// every Agent Bus tool this app declares. A top-level await makes the
/// module an async evaluation, so `app.run()` drives the main loop from
/// inside a continuation that never finished: the MCP socket binds and
/// appears in $XDG_RUNTIME_DIR/lisa/mcp — and since mcp-bus treats
/// socket presence AS tool availability, `search_mail` and
/// `read_message` were advertised and answered nothing. Measured on the
/// reference device: connect succeeds, `initialize` times out, nothing
/// in any log. `imports.gi` is synchronous and keeps the module plain.
let WebKit = null;
try {
    imports.gi.versions.WebKit = '6.0';
    WebKit = imports.gi.WebKit;
} catch {
    // No WebKit here. readerHtml stays null and the TextView is used.
}

const APP_ID = 'app.lisaos.Mail';

function configPath() {
    return GLib.build_filenamev([GLib.get_user_config_dir(), CONFIG_NAME]);
}

function loadConfig() {
    return parseConfig(readFile(configPath()));
}

/// Write the config, and say so if it fails.
///
/// Returns a boolean rather than throwing: a settings page that cannot
/// save is a thing the user needs told, not an exception in a log they
/// will never read.
function saveConfig(config) {
    const path = configPath();
    try {
        const dir = Gio.File.new_for_path(path).get_parent();
        if (dir && !dir.query_exists(null))
            dir.make_directory_with_parents(null);
        return GLib.file_set_contents(path, serializeConfig(config));
    } catch (e) {
        logError(e, 'mail: could not save settings');
        return false;
    }
}

/// Where the Maildir lives: `LISA_MAILDIR`, then the saved setting,
/// then `~/Mail`. Which one won is carried along so the settings page
/// can show a path *and* the reason it is that path.
function maildirRoot(config = null) {
    return resolveMaildir({
        env: GLib.getenv('LISA_MAILDIR'),
        config: config ?? loadConfig(),
        home: GLib.get_home_dir(),
    });
}

/// Folders, in the order a person thinks of them rather than
/// alphabetically: the inbox, then what you sent, then the rest.
const FOLDER_ORDER = ['INBOX', 'Sent', 'Drafts', 'Archive', 'Spam', 'Trash'];

function listDir(path) {
    const out = [];
    let dir;
    try {
        dir = Gio.File.new_for_path(path).enumerate_children(
            'standard::name,standard::type', Gio.FileQueryInfoFlags.NONE, null);
    } catch {
        return out;
    }
    let info;
    while ((info = dir.next_file(null)) !== null)
        out.push({name: info.get_name(), isDir: info.get_file_type() === Gio.FileType.DIRECTORY});
    return out;
}

/// How many messages the list draws before you ask for more.
///
/// 50 is a screenful and a bit. The number that matters is not this one
/// but the one it replaced: "all of them", which on a real inbox meant
/// 3,758 file reads before the first pixel.
const PAGE = 50;

/// Bytes read per message when searching.
///
/// Headers are at the top of the file and a preview needs one paragraph.
/// 16 KiB covers both with room to spare, and turns a full-folder search
/// from "read every byte on disk" into "read the front of each file".
const SEARCH_HEAD = 16384;

/// Read at most `max` bytes from a file.
///
/// Whole-file reads are what made a real mailbox unusable: an HTML
/// newsletter with an inline image is megabytes, and the list needs its
/// first line.
function readHead(path, max) {
    try {
        const stream = Gio.File.new_for_path(path).read(null);
        const bytes = stream.read_bytes(max, null);
        stream.close(null);
        return new TextDecoder('utf-8').decode(bytes.get_data() ?? new Uint8Array());
    } catch {
        return '';
    }
}

function readFile(path) {
    try {
        const [ok, bytes] = GLib.file_get_contents(path);
        if (!ok)
            return '';
        return new TextDecoder('utf-8').decode(bytes);
    } catch {
        return '';
    }
}

/// The accounts this Maildir holds, and where each one's folders are.
///
/// Two layouts exist and both must work. `lisa mail setup` writes a flat
/// tree for one account (`~/Mail/INBOX`) and a per-account tree for
/// several (`~/Mail/you_at_example.com/INBOX`). The flat one is what is
/// already synced on this machine, so it is not a legacy shape to
/// tolerate — it is the common case, and silently relocating somebody's
/// Maildir would not be a migration, it would be a disappearance.
///
/// Detection is by structure, not by config: a directory holding
/// `cur/` is a folder, and a directory holding folders is an account.
function discoverAccounts(root) {
    const dirs = listDir(root).filter((e) => e.isDir && !e.name.startsWith('.'));
    const isFolder = (path) => listDir(path).some((e) => e.isDir && (e.name === 'cur' || e.name === 'new'));

    // Flat: the root itself contains folders.
    if (dirs.some((d) => isFolder(`${root}/${d.name}`)))
        return [{name: accountLabel() ?? 'Mail', root}];

    // Nested: each subdirectory that contains folders is an account.
    const out = [];
    for (const d of dirs) {
        const path = `${root}/${d.name}`;
        if (listDir(path).some((e) => e.isDir && isFolder(`${path}/${e.name}`)))
            out.push({name: d.name.replace('_at_', '@'), root: path});
    }
    return out;
}

/// The mail store: a Maildir on disk, read on demand.
///
/// Deliberately not a cache or a database. A folder is a directory
/// listing and a message is a file; the expensive thing (parsing a
/// body) happens when a message is opened, not when a folder is.
class Store {
    constructor(root) {
        this.root = root;
        // Every account under this Maildir, and the subtree each one
        // owns. `root` stays the *current* account's root so nothing
        // below this line had to learn about accounts.
        this.accounts = discoverAccounts(root);
        this.base = root;
        if (this.accounts.length > 0)
            this.root = this.accounts[0].root;
    }

    /// Point the store at one account's subtree.
    use(account) {
        const hit = this.accounts.find((a) => a.name === account);
        if (hit)
            this.root = hit.root;
        return !!hit;
    }

    /// Folder names across every account, for the sidebar's outer level.
    allFolders() {
        const seen = [];
        for (const a of this.accounts) {
            for (const f of listDir(a.root).filter((e) => e.isDir).map((e) => e.name)) {
                if (!seen.includes(f))
                    seen.push(f);
            }
        }
        const known = FOLDER_ORDER.filter((f) => seen.includes(f));
        return [...known, ...seen.filter((f) => !FOLDER_ORDER.includes(f)).sort()];
    }

    /// Unread in one folder of one account, without changing `root`.
    countsIn(accountRoot, folder) {
        const was = this.root;
        this.root = accountRoot;
        try {
            return this.counts(folder);
        } finally {
            this.root = was;
        }
    }

    folders() {
        const found = listDir(this.root)
            .filter((e) => e.isDir && !e.name.startsWith('.'))
            .map((e) => e.name);
        const known = FOLDER_ORDER.filter((f) => found.includes(f));
        const rest = found.filter((f) => !FOLDER_ORDER.includes(f)).sort();
        return [...known, ...rest];
    }

    /// The cheap half: what the filenames alone can tell us.
    ///
    /// Opens nothing. Flags, delivery time and identity all live in the
    /// Maildir name, so ordering a folder and counting its unread costs
    /// one directory listing however large the folder is. A real inbox
    /// is 3,758 messages — reading every one of them to draw a window
    /// froze the app until it was killed.
    index(folder) {
        const entries = [];
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${this.root}/${folder}/${dir}`)) {
                if (!e.isDir)
                    entries.push({dir, name: e.name});
            }
        }
        return listFolder(folder, entries);
    }

    /// The expensive half, for one message: headers, sender, preview.
    ///
    /// `head` bounds the read. Headers live at the top of the file and a
    /// preview only needs the first paragraph, so a 4 MB message with a
    /// base64 attachment costs the same as a short one. Opening the
    /// message for real still reads all of it — see `message()`.
    summary(folder, m, head = 0) {
        const path = messagePath(this.root, folder, m.dir, m.filename) ?? '';
        const raw = head > 0 ? readHead(path, head) : readFile(path);
        const {headerText, body} = splitMessage(raw);
        const headers = parseHeaders(headerText);
        const from = parseAddress(headers.get('from'));
        const full = {
            ...m,
            from,
            subject: decodeSubject(headers.get('subject')),
            date: headers.get('date'),
            preview: previewOf(bodyOf(raw, body)),
        };
        return {...full, group: classify(full, headers)};
    }

    /// Message summaries for one folder, newest first, at most `limit`.
    ///
    /// Paged on purpose. The list is the first thing drawn and nobody
    /// reads the four-thousandth row before the first.
    messages(folder, limit = PAGE) {
        return this.index(folder)
            .slice(0, Math.max(0, limit))
            .map((m) => this.summary(folder, m));
    }

    /// How many messages the folder holds, and how many are unread —
    /// both from the index, so the sidebar tells the truth about the
    /// whole folder while the list shows a page of it.
    counts(folder) {
        const index = this.index(folder);
        return {total: index.length, unread: unreadCount(index)};
    }

    /// Search the WHOLE folder, not the page.
    ///
    /// A search that only looked at what had already been drawn would be
    /// a filter wearing a search's clothes, and would answer "no
    /// results" for a message sitting on disk. Every message is
    /// considered; each costs a bounded header read rather than a whole
    /// file.
    search(folder, query, limit = 200) {
        const q = String(query).toLowerCase().trim();
        if (!q)
            return [];
        const out = [];
        for (const m of this.index(folder)) {
            const full = this.summary(folder, m, SEARCH_HEAD);
            const hay = `${full.subject} ${full.from.name} ${full.from.address} ${full.preview}`;
            if (hay.toLowerCase().includes(q)) {
                out.push(full);
                if (out.length >= limit)
                    break;
            }
        }
        return out;
    }

    /// One message, fully parsed.
    message(folder, unique) {
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${this.root}/${folder}/${dir}`)) {
                // Compare SANITISED to sanitised. `startsWith` against
                // the raw name never matched on a synced Maildir, where
                // mbsync's `,U=<uid>` suffix is sanitised in the id and
                // not on disk (see uniqueMatchesId).
                if (!uniqueMatchesId(parseFilename(e.name, dir).unique, unique))
                    continue;
                const path = messagePath(this.root, folder, dir, e.name);
                if (!path)
                    continue;
                const raw = readFile(path);
                const {headerText, body} = splitMessage(raw);
                const headers = parseHeaders(headerText);
                const meta = parseFilename(e.name, dir);
                return {
                    folder,
                    id: `${folder}/${unique}`,
                    // The ACTION shape as well as the reading shape:
                    // flagChange/moveTo need the filename, the dir and
                    // the current flags, and returning only the reading
                    // fields is why the write tools could not act on
                    // what read_message could find (#167).
                    filename: e.name, dir,
                    seen: meta.seen, flagged: meta.flagged,
                    from: parseAddress(headers.get('from')),
                    to: headers.get('to'),
                    subject: decodeSubject(headers.get('subject')),
                    date: headers.get('date'),
                    body: bodyOf(raw, body),
                    // The HTML as sent, for the window. Tools never see
                    // this: a model is handed `body`, which is prose.
                    html: renderableBody(raw).html,
                };
            }
        }
        return null;
    }
}

function bodyOf(raw, fallback) {
    const text = readableBody(raw);
    return text.trim() ? text : fallback;
}

/// A subject, decoded and never empty.
///
/// Written first as `parseAddress(value).address`, which produced the
/// right string for the wrong reason — `parseAddress` decodes on the
/// way in and returns the whole text when there is no `<…>` — and would
/// have mangled any subject containing angle brackets. A subject is not
/// an address.
function decodeSubject(value) {
    return decodeWords(value).trim() || '(no subject)';
}

let store = null;
let currentFolder = 'INBOX';
/// How many of the current folder are drawn. Reset by loadFolder, grown
/// by the "load more" row.
let shownLimit = PAGE;
let listBox = null;
let readerTitle = null;
let readerFrom = null;
let readerBody = null;
let folderList = null;
let readerHeader = null;
let readerStack = null;
let readerHtml = null;
/// Remote loads are refused unless the person asked for this message.
/// Reset on every open: consent is per message, not a mode you leave on.
/// Whether remote content in the CURRENT message may load.
///
/// Seeded per message from the `showRemoteImages` setting (default on,
/// see lib/settings.js for the trade). The per-message reset stays:
/// turning the setting off must take effect on the very next message,
/// not on the next launch.
let allowRemote = true;
let remoteBanner = null;
let currentAccountName = null;
/// The message currently open, so the banner can re-render it.
let openMessageFull = null;
let readerActions = null;
let openMessage = null;

const app = new Adw.Application({application_id: APP_ID});

/// A window that fills most of the screen, within reason.
///
/// Clamped at the top so it does not become unusable on an ultrawide,
/// and at the bottom so it stays a three-pane window on a laptop.
function window_default_size() {
    let w = 1600;
    let h = 1000;
    try {
        const monitor = Gdk.Display.get_default()?.get_monitors()?.get_item(0);
        const geo = monitor?.get_geometry();
        if (geo) {
            w = Math.max(1100, Math.min(2000, Math.round(geo.width * 0.8)));
            h = Math.max(700, Math.min(1300, Math.round(geo.height * 0.85)));
        }
    } catch {
        // No display yet, or a backend that will not say: the defaults
        // above are a reasonable window on any of them.
    }
    return {width: w, height: h};
}

/// One row in the message list: sender, time, subject, preview.
///
/// Unread is carried by weight and a dot rather than colour alone —
/// colour is the first thing lost to a theme or to a person who cannot
/// distinguish it.
function messageRow(msg) {
    const row = new Gtk.ListBoxRow();
    row._message = msg;
    const box = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL,
        margin_top: 8, margin_bottom: 8, margin_start: 12, margin_end: 12, spacing: 2,
    });

    const top = new Gtk.Box({orientation: Gtk.Orientation.HORIZONTAL, spacing: 6});
    if (!msg.seen) {
        const dot = new Gtk.Label({label: '●'});
        dot.add_css_class('accent');
        top.append(dot);
    }
    const sender = new Gtk.Label({
        label: msg.from.name || msg.from.address || '(unknown sender)',
        xalign: 0, hexpand: true, ellipsize: 3,
    });
    sender.add_css_class(msg.seen ? 'dim-label' : 'heading');
    top.append(sender);
    top.append(new Gtk.Label({label: shortTime(msg.date), css_classes: ['dim-label', 'caption']}));
    box.append(top);

    const subject = new Gtk.Label({label: msg.subject, xalign: 0, ellipsize: 3});
    if (!msg.seen)
        subject.add_css_class('heading');
    box.append(subject);

    if (msg.preview) {
        const preview = new Gtk.Label({
            label: msg.preview, xalign: 0, ellipsize: 3, lines: 2, wrap: true,
            css_classes: ['dim-label', 'caption'],
        });
        box.append(preview);
    }
    row.set_child(box);
    return row;
}

function shortTime(date) {
    if (!date)
        return '';
    const m = String(date).match(/(\d{1,2}):(\d{2})/);
    return m ? `${m[1]}:${m[2]}` : String(date).split(' ').slice(1, 3).join(' ');
}

function groupHeader(name, count) {
    const row = new Gtk.ListBoxRow({selectable: false, activatable: false});
    const box = new Gtk.Box({
        orientation: Gtk.Orientation.HORIZONTAL, spacing: 8,
        margin_top: 14, margin_bottom: 4, margin_start: 12, margin_end: 12,
    });
    const label = new Gtk.Label({label: name, xalign: 0, hexpand: true});
    label.add_css_class('heading');
    box.append(label);
    box.append(new Gtk.Label({label: String(count), css_classes: ['dim-label', 'caption']}));
    row.set_child(box);
    return row;
}

function loadFolder(folder, limit = PAGE) {
    currentFolder = folder;
    shownLimit = limit;
    let child = listBox.get_first_child();
    while (child) {
        const next = child.get_next_sibling();
        listBox.remove(child);
        child = next;
    }
    const messages = store.messages(folder, limit);
    const {total} = store.counts(folder);
    if (messages.length === 0) {
        const empty = new Gtk.ListBoxRow({selectable: false});
        empty.set_child(new Adw.StatusPage({
            title: 'Nothing here',
            description: `No mail in ${folder}. Mail reads a Maildir at ${store.root} — ` +
                'sync one with mbsync or offlineimap and it will appear.',
            vexpand: true,
        }));
        listBox.append(empty);
        clearReader();
        return;
    }
    for (const group of grouped(messages)) {
        listBox.append(groupHeader(group.name, group.items.length));
        for (const m of group.items)
            listBox.append(messageRow(m));
    }

    // …and a way to see the rest. It says how many are left rather than
    // just "more", because "3,708 older" is the number that tells you
    // whether to scroll or to search.
    if (total > messages.length) {
        const more = new Gtk.ListBoxRow({selectable: false});
        const button = new Gtk.Button({
            label: `Load ${Math.min(PAGE, total - messages.length)} more ` +
                `(${total - messages.length} older)`,
            margin_top: 8, margin_bottom: 8, margin_start: 12, margin_end: 12,
            css_classes: ['flat'],
        });
        button.connect('clicked', () => loadFolder(folder, shownLimit + PAGE));
        more._loadMore = true;
        more.set_child(button);
        listBox.append(more);
    }

    // Open the first message. Every mail client does this, and here it
    // is load-bearing rather than a nicety: the action buttons live in
    // the reading pane's header, so a pane with nothing open is an app
    // that appears to have no actions at all. That is exactly how this
    // was reported — "still no icons" — and no amount of checking icon
    // names would have found it.
    //
    // Opening does NOT mark the message read. `S` is set by the button,
    // by a person deciding they have read it; a client that flips the
    // flag because a pane happened to render it is a client that eats
    // your unread count.
    let row = listBox.get_first_child();
    while (row && !row._message)
        row = row.get_next_sibling();
    if (row) {
        listBox.select_row(row);
        showMessage(row._message);
    } else {
        clearReader();
    }
}

/// Empty the reading pane and take its toolbar away with it.
///
/// The toolbar goes too: buttons for a message that is no longer open
/// would still act on it, and the first one clicked would rename a file
/// the user cannot see.
function clearReader() {
    openMessage = null;
    readerTitle.set_label('');
    readerFrom.set_label('');
    readerBody.buffer.set_text('', -1);
    allowRemote = loadConfig().showRemoteImages !== false;
    if (readerStack)
        readerStack.set_visible_child_name('text');
    if (readerActions) {
        readerHeader.remove(readerActions);
        readerActions = null;
    }
}

/// Rebuild the reading-pane toolbar for the open message.
///
/// Built from `actionsFor` rather than eight hand-written buttons: what
/// each one does is then testable without a window, and the labels
/// cannot drift from the state they describe.
/// A WebView that renders mail and does nothing else.
///
/// The safety here is the renderer's configuration, not the markup —
/// sanitising HTML with string surgery is a promise you cannot keep
/// against a real parser. Two settings carry it:
///
/// **JavaScript is off.** Mail is a document, not a program. Nothing in
/// a message has any business executing, and leaving it on would put an
/// attacker-controlled script in the same process as somebody's inbox.
///
/// **Remote loads are refused.** A remote image is a read receipt the
/// sender never asked permission for: fetching it tells them the
/// message was opened, when, and from which address. Every mail client
/// blocks these by default and the ones that did not were rightly
/// shamed into it. `data:` URIs are allowed because the bytes are
/// already in the message, and `cid:` because that is how a message
/// refers to its own attached parts.
/// Put a message body on screen, in whichever renderer suits it.
///
/// HTML when the message has an HTML part and this system has WebKit;
/// text otherwise. The text path is not a degraded mode — most personal
/// mail is plain, and it reads better as text than as a document.
/// Does this HTML reach for anything off this machine?
///
/// Only worth asking so the "show images" offer appears on messages
/// that would actually change — a banner on a message with nothing
/// remote in it teaches people to click banners.
function hasRemoteContent(html) {
    return /(?:src|background|href)\s*=\s*["']?https?:/i.test(String(html ?? '')) ||
        /url\(\s*["']?https?:/i.test(String(html ?? ''));
}

function renderBody(full) {
    const rich = readerHtml && full.html;
    if (!rich) {
        readerBody.buffer.set_text(full.body ?? '', -1);
        readerStack?.set_visible_child_name('text');
        return;
    }
    readerHtml.load_html(htmlDocument(full.html), null);
    readerStack.set_visible_child_name('html');
    remoteBanner?.set_revealed(!allowRemote && hasRemoteContent(full.html));
}

function buildWebView() {
    const settings = new WebKit.Settings({
        enable_javascript: false,
        enable_javascript_markup: false,
        enable_webgl: false,
        enable_media: false,
        enable_html5_database: false,
        enable_html5_local_storage: false,
        enable_developer_extras: false,
    });
    const view = new WebKit.WebView({settings, vexpand: true});
    // The widget's own backdrop, not just the document's: without this
    // the WebView paints the GTK dark surface for the fraction of a
    // second before the document's white lands, and any margin outside
    // the page box stays dark forever (#211).
    view.set_background_color(new Gdk.RGBA({red: 1, green: 1, blue: 1, alpha: 1}));

    view.connect('decide-policy', (_v, decision, type) => {
        if (type === WebKit.PolicyDecisionType.NAVIGATION_ACTION) {
            const uri = decision.get_navigation_action()?.get_request()?.get_uri() ?? '';
            // lib/links.js owns the rules and is tested without a
            // display. `data:` used to be allowed here — it is needed
            // for inline image BYTES, which is the RESPONSE branch
            // below, not for navigation, and allowing it let a clicked
            // `data:text/html,…` link redraw the reading pane as
            // anything the sender liked.
            const decided = linkAction(uri);
            if (decided.action === 'in-place')
                return false;
            decision.ignore();
            if (decided.action === 'external') {
                // A clicked link opens in the browser, where a person
                // can see where they are going. It never navigates this
                // view: a reading pane that can be steered elsewhere is
                // a phishing surface wearing a mail client.
                try {
                    Gtk.show_uri(null, decided.uri, Gdk.CURRENT_TIME);
                } catch (e) {
                    logError(e, 'mail: could not open link');
                }
            } else {
                // Refused, and said so. A link that does nothing when
                // clicked reads as a broken app; this reads as a
                // decision, which is what it is.
                //
                // There is no toast overlay in this window — checked,
                // after writing `toast?.(…)` here and finding the
                // binding does not exist at all, which is a
                // ReferenceError rather than the no-op the `?.` implies.
                // The banner is the surface this window already has.
                if (remoteBanner) {
                    remoteBanner.set_title(decided.reason);
                    remoteBanner.set_revealed(true);
                } else {
                    log(`mail: refused link ${decided.uri}`);
                }
            }
            return true;
        }
        if (type === WebKit.PolicyDecisionType.RESPONSE) {
            const uri = decision.get_request()?.get_uri() ?? '';
            const local = uri.startsWith('data:') || uri.startsWith('cid:') ||
                uri.startsWith('about:');
            if (!local && !allowRemote) {
                decision.ignore();
                return true;
            }
        }
        return false;
    });
    return view;
}

/// Wrap a message body so it renders like mail rather than like a
/// document dumped in a browser.
///
/// The base tag is `about:blank` on purpose: a relative URL in a message
/// then resolves to nothing instead of to something on this machine.
function htmlDocument(body) {
    return `<!DOCTYPE html><html><head><meta charset="utf-8">` +
        `<base href="about:blank">` +
        `<meta name="referrer" content="no-referrer">` +
        `<style>
          html { -webkit-text-size-adjust: 100%; }
          body { font-family: system-ui, sans-serif; font-size: 14px; line-height: 1.5;
                 margin: 16px; word-wrap: break-word; }
          img, table { max-width: 100% !important; height: auto; }
          pre { white-space: pre-wrap; }
          a { color: #6D45C9; } /* token: violet-500 */
          /* NO dark-mode inversion, deliberately (#211).
             Email HTML is authored for a WHITE canvas — that assumption
             is universal, and senders set their own inline colours
             against it. This used to render on a transparent background
             in dark mode, so an invoice whose inline colours are a
             near-black on a pale card, with no background set on the
             outer wrapper, painted dark text onto our dark surface: the
             message looked EMPTY (seen on the device, Negenet invoice
             F-2026-0007). A reader that hides mail is worse than one
             that looks unfashionable, and every other mail client makes
             the same call: the message renders on paper, the app chrome
             around it stays dark. */
          body { background: #FFFFFF; color: #2B2320; } /* token: ink-900 */
        </style></head><body>${body}</body></html>`;
}

/// Which account this Maildir holds mail for.
///
/// Read from the sync config `lisa mail setup` wrote, not from Online
/// Accounts. That is the more truthful source: GOA lists every account
/// connected to the desktop, while this Maildir contains the mail of
/// exactly one of them, and naming a different one would be worse than
/// naming none. It is also synchronous, which the async GOA lookup was
/// not — called during `activate` its promise never settled, because
/// nothing was spinning the main loop yet.
///
/// Returns null when there is no config, and the header stays "Mail".
function accountLabel() {
    const path = GLib.build_filenamev([GLib.get_user_config_dir(), 'lisa', 'mbsyncrc']);
    const text = readFile(path);
    for (const line of text.split('\n')) {
        const m = line.match(/^\s*User\s+(.+?)\s*$/);
        if (m)
            return m[1];
    }
    return null;
}

function buildToolbar(msg) {
    if (readerActions) {
        readerHeader.remove(readerActions);
        readerActions = null;
    }
    readerActions = new Gtk.Box({orientation: Gtk.Orientation.HORIZONTAL, spacing: 4});
    readerActions.add_css_class('linked');
    // The icon theme is not ours, and Adwaita has been retiring legacy
    // names for several releases — `box-symbolic` was never in it at
    // all. An icon name that does not resolve gives you a button with
    // nothing in it, which is indistinguishable from no button. So ask
    // first, and fall back to the label: an "Archive" button is worse
    // than an icon and far better than a gap.
    const icons = Gtk.IconTheme.get_for_display(Gdk.Display.get_default());
    for (const action of actionsFor(msg, store.folders(), canSend())) {
        const known = action.icon && icons?.has_icon(action.icon);
        const button = new Gtk.Button({
            ...(known ? {icon_name: action.icon} : {label: action.label}),
            tooltip_text: action.why ? `${action.label} — ${action.why}` : action.label,
            sensitive: action.enabled !== false,
        });
        if (!known)
            printerr(`mail: no icon ${JSON.stringify(action.icon)}, showing the label`);
        if (action.active)
            button.add_css_class('suggested-action');
        button.connect('clicked', () => runAction(msg, action));
        readerActions.append(button);
    }
    readerHeader.pack_start(readerActions);
}

/// Perform one action, then reload so the list reflects it.
///
/// Reloading the whole folder rather than patching the row: a rename
/// changes the message's filename, which is its identity on disk, and
/// a half-updated row that still remembers the old name is a row whose
/// next action fails.
function runAction(msg, action) {
    try {
        if (action.kind === 'flag') {
            const change = flagChange(msg, action.flag, action.on);
            if (change)
                applyMove(msg.folder, change, msg.folder);
        } else if (action.kind === 'send' && action.enabled !== false) {
            const me = defaultIdentity();
            openCompose(action.id === 'reply'
                ? replyFields(msg, me)
                : forwardFields(msg, me));
            return;
        } else if (action.kind === 'move' && action.enabled !== false) {
            const mv = moveTo(msg, action.folder);
            if (mv)
                applyMove(msg.folder, mv, mv.toFolder);
        } else {
            return;
        }
    } catch (e) {
        logError(e, `mail: ${action.id} failed`);
        return;
    }
    // Reload, which re-opens whatever is now at the top — the message
    // after the one just filed. Clearing the pane instead would punish
    // the user for acting: archive three messages and you would be
    // staring at a blank pane three times.
    loadFolder(currentFolder);
}

/// The rename itself. One `Gio.File.move`, no fallback copy: a copy
/// that half-succeeds leaves the message in two folders, and a maildir
/// with a duplicate is worse than one with a failed action.
function applyMove(fromFolder, change, toFolder) {
    const from = messagePath(store.root, fromFolder, change.fromDir, change.fromName);
    const to = messagePath(store.root, toFolder, change.toDir, change.toName);
    if (!from || !to)
        throw new Error('refusing a path outside the maildir');
    GLib.mkdir_with_parents(`${store.root}/${toFolder}/${change.toDir}`, 0o700);
    Gio.File.new_for_path(from).move(
        Gio.File.new_for_path(to), Gio.FileCopyFlags.NONE, null, null);
}

function showMessage(msg) {
    openMessage = msg;
    buildToolbar(msg);
    // `?? msg` used to be silent, and that silence WAS the bug (#210):
    // a list row carries subject and sender but no body, so a failed
    // lookup filled the header and left the pane empty — indis-
    // tinguishable from an empty message, and impossible to report.
    // A reader that cannot find the file on disk must say so.
    const loaded = store.message(msg.folder, msg.unique);
    const full = loaded ?? {
        ...msg,
        body: `Could not read this message from the Maildir.\n\n` +
              `folder: ${msg.folder}\nid: ${msg.unique}\n\n` +
              `The list row was built from the index, so the message ` +
              `exists — the file lookup under the folder above did not ` +
              `match it.`,
        html: null,
    };
    readerTitle.set_label(full.subject ?? '');
    // Both halves of the sender, always: a display name is
    // attacker-controlled, and `"security@yourbank.com" <evil@x.test>`
    // is a real shape. Showing the name alone is how that works.
    const name = full.from?.name;
    const addr = full.from?.address ?? '';
    readerFrom.set_label(name ? `${name}  ·  ${addr}` : addr);
    // Consent is per message: opening a different one must not inherit
    // the last one's permission to phone home.
    allowRemote = loadConfig().showRemoteImages !== false;
    openMessageFull = full;
    renderBody(full);
}

/// Rebuild the folder sidebar from the current store.
///
/// Separate from `activate` because changing the Maildir in settings
/// has to take effect without a restart — an app that needs relaunching
/// to notice its own preference is one people stop trusting the
/// preference of.
/// Rebuild the folder sidebar.
///
/// Folder-major, accounts nested inside — the shape Spark uses and the
/// one in the screenshot that prompted this. INBOX is a group holding
/// one row per account rather than a single row, because "which of my
/// inboxes" is the question a person with two accounts is actually
/// asking, and a merged list cannot answer it.
///
/// With one account the expanders collapse to plain rows: nesting a
/// single mailbox under itself is ceremony, not structure.
function reloadFolders() {
    let child = folderList.get_first_child();
    while (child) {
        const next = child.get_next_sibling();
        folderList.remove(child);
        child = next;
    }

    const accounts = store.accounts;
    const multi = accounts.length > 1;

    for (const folder of store.allFolders()) {
        const present = accounts.filter(
            (a) => listDir(`${a.root}/${folder}`).some((e) => e.isDir));
        if (present.length === 0)
            continue;

        if (!multi) {
            const a = present[0];
            folderList.append(folderRow(folder, a, store.countsIn(a.root, folder).unread, false));
            continue;
        }

        // One expander per folder, one row per account inside it. The
        // badge on the expander is the folder's total across accounts,
        // which is the number you want when it is collapsed.
        const total = present.reduce((n, a) => n + store.countsIn(a.root, folder).unread, 0);
        const expander = new Adw.ExpanderRow({
            title: folder,
            expanded: folder === 'INBOX',
        });
        if (total > 0) {
            expander.add_suffix(new Gtk.Label({
                label: String(total), css_classes: ['dim-label', 'caption'],
            }));
        }
        for (const a of present)
            expander.add_row(folderRow(folder, a, store.countsIn(a.root, folder).unread, true));
        folderList.append(expander);
    }

    const first = folderList.get_row_at_index(0);
    if (first && first._folder)
        folderList.select_row(first);
    else
        selectFolder(accounts[0], currentFolder || 'INBOX');
}

/// One selectable row: a folder within an account.
function folderRow(folder, account, unread, nested) {
    const row = nested ? new Adw.ActionRow({title: account.name, activatable: true})
        : new Gtk.ListBoxRow();
    row._folder = folder;
    row._account = account;
    if (nested) {
        if (unread > 0) {
            row.add_suffix(new Gtk.Label({
                label: String(unread), css_classes: ['dim-label', 'caption'],
            }));
        }
        row.connect('activated', () => selectFolder(account, folder));
        return row;
    }
    const box = new Gtk.Box({
        orientation: Gtk.Orientation.HORIZONTAL, spacing: 8,
        margin_top: 6, margin_bottom: 6, margin_start: 6, margin_end: 6,
    });
    box.append(new Gtk.Label({label: folder, xalign: 0, hexpand: true}));
    if (unread > 0)
        box.append(new Gtk.Label({label: String(unread), css_classes: ['dim-label', 'caption']}));
    row.set_child(box);
    return row;
}

/// Switch to a folder, in an account.
function selectFolder(account, folder) {
    if (account)
        store.use(account.name);
    currentAccountName = account ? account.name : null;
    loadFolder(folder, PAGE);
}

app.connect('activate', () => {
    const resolved = maildirRoot();
    store = new Store(resolved.path);
    store.source = resolved.source;

    // Sized against the monitor rather than a fixed guess: 1280x820 is
    // a dialog on the 4K panel this was first seen on. Three panes need
    // room, and a mail window that opens smaller than its own content
    // reads as a preview of an app.
    const display = window_default_size();
    const window = new Adw.ApplicationWindow({
        application: app, title: 'Mail',
        default_width: display.width, default_height: display.height,
    });

    // Pane 1: folders. Populated by reloadFolders() once the rest of
    // the window exists — it selects a row, which loads a folder, which
    // needs the list and reading panes to be there already.
    folderList = new Gtk.ListBox({css_classes: ['navigation-sidebar']});
    folderList.connect('row-selected', (_l, row) => {
        if (row?._folder)
            selectFolder(row._account, row._folder);
    });

    const sidebar = new Adw.ToolbarView();
    // The account is named in the sidebar itself — as the header's
    // subtitle when there is one account, and as the rows inside each
    // folder's expander when there are several. Either way the answer to
    // "whose mail am I reading" is on screen, which it was not.
    const sidebarTitle = new Adw.WindowTitle({title: 'Mail'});
    sidebar.add_top_bar(new Adw.HeaderBar({title_widget: sidebarTitle}));
    const who = accountLabel();
    if (who)
        sidebarTitle.set_subtitle(who);
    sidebar.set_content(new Gtk.ScrolledWindow({child: folderList, vexpand: true}));

    // Pane 2: the grouped message list.
    listBox = new Gtk.ListBox({css_classes: ['navigation-sidebar']});
    listBox.connect('row-activated', (_l, row) => {
        if (row?._message)
            showMessage(row._message);
    });
    const listPane = new Adw.ToolbarView();
    // NOT show_title:false. That hides the title WIDGET too, so the
    // search entry below was created, packed, and invisible — which no
    // test would have caught and one screenshot did.
    const listHeader = new Adw.HeaderBar();
    const search = new Gtk.SearchEntry({placeholder_text: 'Search', hexpand: true});
    // Searches the WHOLE folder, not the page on screen. Filtering drawn
    // rows would answer "no results" for a message that is on disk and
    // simply had not been read yet — a filter wearing a search's
    // clothes. Each candidate costs a bounded header read, so a folder
    // of thousands is a pause, not a freeze.
    search.connect('search-changed', () => {
        const q = search.text.trim();
        if (!q) {
            loadFolder(currentFolder, PAGE);
            return;
        }
        let child = listBox.get_first_child();
        while (child) {
            const next = child.get_next_sibling();
            listBox.remove(child);
            child = next;
        }
        const hits = store.search(currentFolder, q);
        if (hits.length === 0) {
            const none = new Gtk.ListBoxRow({selectable: false});
            none.set_child(new Adw.StatusPage({
                title: 'No matches',
                description: `Nothing in ${currentFolder} matches ${JSON.stringify(q)}.`,
                vexpand: true,
            }));
            listBox.append(none);
            return;
        }
        listBox.append(groupHeader(`Results in ${currentFolder}`, hits.length));
        for (const m of hits)
            listBox.append(messageRow(m));
    });
    listHeader.set_title_widget(search);
    const menu = new Gio.Menu();
    menu.append('Preferences', 'app.preferences');
    menu.append('About Mail', 'app.about');
    listHeader.pack_end(new Gtk.MenuButton({
        icon_name: 'open-menu-symbolic', menu_model: menu, tooltip_text: 'Main menu',
    }));
    listPane.add_top_bar(listHeader);
    listPane.set_content(new Gtk.ScrolledWindow({child: listBox, vexpand: true}));

    // Pane 3: the reading pane.
    readerTitle = new Gtk.Label({xalign: 0, wrap: true, css_classes: ['title-2']});
    readerFrom = new Gtk.Label({xalign: 0, css_classes: ['dim-label']});
    readerBody = new Gtk.TextView({
        editable: false, cursor_visible: false, wrap_mode: Gtk.WrapMode.WORD_CHAR,
        left_margin: 16, right_margin: 16, top_margin: 12, bottom_margin: 16,
        monospace: false,
    });
    const readerBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL, spacing: 6,
        margin_top: 16, margin_start: 16, margin_end: 16,
    });
    readerBox.append(readerTitle);
    readerBox.append(readerFrom);
    const readerScroll = new Gtk.ScrolledWindow({child: readerBody, vexpand: true});

    // Two ways to show a body, chosen per message: the TextView for
    // plain mail and as the fallback everywhere, a WebView for HTML.
    // Says what is being withheld and offers to stop withholding it —
    // for this message only. Adw.Banner is the shape GNOME uses for
    // exactly this: a thing the app did on your behalf, with the undo.
    remoteBanner = new Adw.Banner({
        title: 'Images in this message are on someone else\u2019s server',
        button_label: 'Show images',
        revealed: false,
    });
    remoteBanner.connect('button-clicked', () => {
        allowRemote = true;
        remoteBanner.set_revealed(false);
        if (openMessageFull)
            renderBody(openMessageFull);
    });
    readerBox.append(remoteBanner);

    readerStack = new Gtk.Stack({vexpand: true});
    readerStack.add_named(readerScroll, 'text');
    if (WebKit) {
        readerHtml = buildWebView();
        readerStack.add_named(readerHtml, 'html');
    }
    readerBox.append(readerStack);
    const readerPane = new Adw.ToolbarView();
    readerHeader = new Adw.HeaderBar();
    readerHeader.set_title_widget(new Gtk.Label({label: '', visible: false}));
    readerPane.add_top_bar(readerHeader);
    readerPane.set_content(readerBox);

    // Two nested split views: sidebar | (list | reader).
    const inner = new Adw.OverlaySplitView({
        sidebar: listPane, content: readerPane,
        min_sidebar_width: 320, max_sidebar_width: 460, sidebar_width_fraction: 0.34,
    });
    const outer = new Adw.OverlaySplitView({
        sidebar, content: inner,
        min_sidebar_width: 200, max_sidebar_width: 280, sidebar_width_fraction: 0.18,
    });
    window.set_content(outer);
    reloadFolders();

    const preferences = new Gio.SimpleAction({name: 'preferences'});
    preferences.connect('activate', () => openPreferences(window));
    app.add_action(preferences);
    app.set_accels_for_action('app.preferences', ['<Control>comma']);

    const about = new Gio.SimpleAction({name: 'about'});
    about.connect('activate', () => {
        const dialog = new Adw.AboutDialog({
            application_name: 'Mail', application_icon: APP_ID, version: '0.1',
            comments: 'Reads a Maildir, and gives the assistant a view of it.',
        });
        dialog.present(window);
    });
    app.add_action(about);

    // The Agent Bus surface. Started with the window and stopped with
    // it: socket presence IS tool availability (mcp-bus defers socket
    // activation), so a dead socket would be a tool that times out.
    const mcp = new McpServer({
        searchMail: (args) => searchMail(args),
        readMessage: (args) => readMessage(args),
        writeAction: (tool, args) => writeAction(tool, args),
    });
    mcp.start();
    window.connect('close-request', () => {
        mcp.stop();
        return false;
    });

    window.present();
});

// ---------------------------------------------------------------------
// Settings — a diagnostic before it is a preference sheet.
//
// A user connects Google in Settings, opens Mail, and sees nothing.
// Every layer is behaving exactly as designed and not one of them can
// say so: GOA holds an account it cannot get a token for, the Maildir is
// empty because nothing fills it, and an empty folder is the correct
// thing to draw. The failure lives in the gaps between them, which is
// where nothing is looking. This page looks there.
// ---------------------------------------------------------------------

/// Is there a Secret Service on the bus?
///
/// Asked because its absence is invisible everywhere else: Online
/// Accounts will accept an account and then fail to hand out a token
/// for it, with an error only the consumer ever sees (issue #154).
///
/// Both questions matter — a name with no owner may still be
/// activatable, and a service that starts on demand is present as far
/// as anything using it is concerned.
function hasSecretService() {
    try {
        const bus = Gio.DBus.session;
        const owned = bus.call_sync(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'NameHasOwner', new GLib.Variant('(s)', ['org.freedesktop.secrets']),
            new GLib.VariantType('(b)'), Gio.DBusCallFlags.NONE, 1000, null);
        if (owned.deepUnpack()[0])
            return true;
        const names = bus.call_sync(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'ListActivatableNames', null,
            new GLib.VariantType('(as)'), Gio.DBusCallFlags.NONE, 1000, null);
        return names.deepUnpack()[0].includes('org.freedesktop.secrets');
    } catch (e) {
        logError(e, 'mail: could not ask the bus about a secret service');
        return false;
    }
}

/// Connected accounts, as plain facts.
///
/// Loaded through a dynamic import so a system without the GOA typelib
/// — a dev checkout on another distro, the minimal image — shows "no
/// account connected" instead of failing to open the settings page. The
/// page's whole job is to work on the machine that is broken.
/// One property off a D-Bus proxy, unwrapped, or `null`.
function cached(proxy, name) {
    try {
        return proxy?.get_cached_property(name)?.deepUnpack() ?? null;
    } catch {
        return null;
    }
}

async function goaAccounts() {
    let Goa;
    try {
        Goa = (await import('gi://Goa?version=1.0')).default;
    } catch {
        return [];
    }
    try {
        const client = Goa.Client.new_sync(null);
        return client.get_accounts().map((obj) => {
            const account = obj.get_account();
            const mail = obj.get_mail();
            if (!account)
                return null;
            // Read off the D-Bus proxy, not through the GObject
            // properties. GOA's generated proxies confuse gjs's
            // property-accessor fast path, which warns per read: "Type
            // GITypeInfo of property Goa.Account::provider-name does not
            // match type GITypeInfo of first argument of setter
            // complete_remove. Falling back to slow path". There are no
            // get_provider_name()-style methods to use instead — the
            // cached property is the interface.
            return {
                provider: cached(account, 'ProviderName'),
                identity: cached(account, 'PresentationIdentity') || cached(account, 'Identity'),
                imapUser: mail ? cached(mail, 'ImapUserName') : '',
                // An account with no Mail interface at all is a calendar
                // or contacts account; from here that is the same thing
                // as mail being switched off.
                mailDisabled: cached(account, 'MailDisabled') === true || !mail,
            };
        }).filter(Boolean);
    } catch (e) {
        logError(e, 'mail: could not read online accounts');
        return [];
    }
}

/// Open Settings at the Online Accounts panel.
///
/// Launched rather than reimplemented: adding an account is GNOME's
/// job, involves a browser and an OAuth flow, and duplicating any part
/// of that here would be a second place for it to be wrong.
function openOnlineAccounts() {
    try {
        GLib.spawn_async(null, ['gnome-control-center', 'online-accounts'], null,
            GLib.SpawnFlags.SEARCH_PATH, null);
        return true;
    } catch (e) {
        logError(e, 'mail: could not open Settings');
        return false;
    }
}

function openPreferences(parent) {
    const dialog = new Adw.PreferencesDialog({title: 'Mail Settings'});
    const page = new Adw.PreferencesPage();
    dialog.add(page);

    // --- Where the mail is.
    const location = new Adw.PreferencesGroup({
        title: 'Maildir',
        description: 'The folder Mail reads. Something else writes it — see below.',
    });
    const resolved = maildirRoot();
    const pathRow = new Adw.EntryRow({title: 'Folder', text: resolved.path});
    // An environment variable that a saved setting can override is a
    // debugging trap, so when one is set the row says so and stops
    // pretending to be editable.
    if (resolved.source === 'env') {
        pathRow.set_sensitive(false);
        pathRow.set_title('Folder — set by LISA_MAILDIR');
    }
    location.add(pathRow);

    const counts = {};
    const folders = store.folders();
    for (const f of folders)
        counts[f] = store.messages(f).length;
    location.add(new Adw.ActionRow({
        title: 'On disk now',
        subtitle: storeSummary(folders, counts),
    }));
    page.add(location);

    // --- Reading: the preferences that are actually preferences.
    //
    // This page began as a diagnostic ("why is there no mail?") and had
    // no settings in it at all, which made "Settings" a slightly odd
    // name for it. The switch below is the first real one.
    const reading = new Adw.PreferencesGroup({
        title: 'Reading',
        description: 'Remote images are how senders know you opened a message. ' +
            'Turning this off shows a banner instead, and you decide per message.',
    });
    const remoteRow = new Adw.SwitchRow({
        title: 'Show images in messages',
        subtitle: 'Loads pictures and other content from the sender’s servers',
        active: loadConfig().showRemoteImages !== false,
    });
    remoteRow.connect('notify::active', () => {
        const config = loadConfig();
        config.showRemoteImages = remoteRow.get_active();
        if (!saveConfig(config)) {
            // Say so rather than leaving a switch that springs back
            // with no explanation.
            remoteRow.set_subtitle('Could not save this preference');
            return;
        }
        // Take effect on what is on screen now, not on next launch.
        allowRemote = config.showRemoteImages;
        // openMessageFull, not an invented `currentMessage` — line 1069
        // (the “show images” banner) already re-renders through it.
        if (openMessageFull) renderBody(openMessageFull);
    });
    reading.add(remoteRow);
    page.add(reading);

    // --- Why there is, or is not, mail arriving.
    const sync = new Adw.PreferencesGroup({title: 'Syncing'});
    const status = syncStatus({
        mbsync: GLib.find_program_in_path('mbsync') !== null,
        secretService: hasSecretService(),
        accounts: [],
    });
    const statusRow = new Adw.ActionRow({title: status.title, subtitle: status.detail});
    statusRow.add_prefix(new Gtk.Image({
        icon_name: status.kind === 'ok'
            ? 'emblem-ok-symbolic'
            : 'dialog-warning-symbolic',
    }));
    sync.add(statusRow);
    page.add(sync);

    // --- Accounts, filled in once GOA answers.
    const accountsGroup = new Adw.PreferencesGroup({
        title: 'Accounts',
        description: 'Connected in Settings, under Online Accounts.',
    });
    const openRow = new Adw.ActionRow({
        title: 'Open Online Accounts', activatable: true,
    });
    openRow.add_suffix(new Gtk.Image({icon_name: 'go-next-symbolic'}));
    openRow.connect('activated', () => openOnlineAccounts());
    accountsGroup.add(openRow);
    page.add(accountsGroup);

    // GOA is read asynchronously so a slow or absent daemon cannot hold
    // the dialog shut. The rows land when they land; the page is useful
    // without them.
    const added = [];
    goaAccounts().then((accounts) => {
        for (const row of added)
            accountsGroup.remove(row);
        added.length = 0;
        for (const a of accountRows(accounts)) {
            const row = new Adw.ActionRow({title: a.title, subtitle: a.subtitle});
            if (!a.usable)
                row.add_prefix(new Gtk.Image({icon_name: 'dialog-warning-symbolic'}));
            accountsGroup.add(row);
            added.push(row);
        }
        // The status depends on the accounts, so it is recomputed rather
        // than left showing the answer from before they were known.
        const next = syncStatus({
            mbsync: GLib.find_program_in_path('mbsync') !== null,
            secretService: hasSecretService(),
            accounts,
        });
        statusRow.set_title(next.title);
        statusRow.set_subtitle(next.detail);
    }).catch((e) => logError(e, 'mail: could not list accounts'));

    // Applied on close rather than per keystroke: a half-typed path is
    // not a path, and rebuilding the folder list on every character
    // would read every message in a folder that does not exist yet.
    dialog.connect('closed', () => {
        if (resolved.source === 'env')
            return;
        const check = validateMaildir(pathRow.text);
        if (!check.ok) {
            printerr(`mail: not saving that folder — ${check.reason}`);
            return;
        }
        if (check.path === resolved.path)
            return;
        const config = loadConfig();
        config.maildir = check.path;
        if (!saveConfig(config))
            return;
        const next = maildirRoot(config);
        store = new Store(next.path);
        store.source = next.source;
        reloadFolders();
    });

    dialog.present(parent);
}

/// `search_mail` — subjects, senders and previews across folders.
///
/// Returns summaries, never whole bodies: a search that dumped every
/// matching message into the model's context would spend the window on
/// the first query and hand over far more than was asked for.
function searchMail({query = '', folder = '', limit = 20} = {}) {
    const q = String(query).toLowerCase().trim();
    const folders = folder ? [String(folder)] : store.folders();
    const out = [];
    for (const f of folders) {
        // The index first, then a bounded read per candidate. Calling
        // `messages()` here would either page the tool (a model asking
        // about mail would be answered from the newest 50 and never
        // told) or read every byte of a four-thousand-message folder
        // while the caller waits.
        for (const entry of store.index(f)) {
            const m = store.summary(f, entry, SEARCH_HEAD);
            const hay = `${m.subject} ${m.from.name} ${m.from.address} ${m.preview}`.toLowerCase();
            if (q && !hay.includes(q))
                continue;
            out.push({
                id: m.id, folder: f, subject: m.subject,
                from: m.from.address, from_name: m.from.name,
                date: m.date, unread: !m.seen, group: m.group, preview: m.preview,
            });
            if (out.length >= Math.min(Number(limit) || 20, 50))
                return {messages: out};
        }
    }
    return {messages: out};
}

/// `read_message` — one message, by the id `search_mail` returned.
function readMessage({id = ''} = {}) {
    const [folder, ...rest] = String(id).split('/');
    const unique = rest.join('/');
    if (!folder || !unique)
        return {error: 'id must be "<folder>/<message>" as returned by search_mail'};
    const msg = store.message(folder, unique);
    if (!msg)
        return {error: `no message ${id}`};
    return {
        id: msg.id, folder: msg.folder, subject: msg.subject,
        from: msg.from.address, from_name: msg.from.name, to: msg.to, date: msg.date,
        body: msg.body,
    };
}

/// The Write-tier tools (#167). lib/agent-actions.js decides; this
/// performs, with the SAME `applyMove` the toolbar uses.
///
/// Every outcome is named. A tool that reports success having done
/// nothing is the defect this repo keeps finding, so "already archived"
/// comes back as `changed: false` with a reason rather than as a
/// cheerful ok.
function writeAction(tool, args = {}) {
    const parsed = parseMessageId(args.id);
    if (parsed.error)
        return {error: parsed.error};
    const message = store.message(parsed.folder, parsed.unique);
    const plan = planFor(tool, message, args, store.folders());
    if (plan.error)
        return {error: plan.error};
    if (plan.noop)
        return {changed: false, reason: plan.noop, id: args.id};
    try {
        applyMove(message.folder, plan.change, plan.toFolder);
    } catch (e) {
        // The rename failed: say so. Returning ok here would tell the
        // model the mail was filed while it sat where it was.
        return {error: `could not ${tool}: ${e.message}`};
    }
    // The list on screen is now stale — the filename IS the identity,
    // and a row remembering the old one fails its next action.
    if (currentFolder === parsed.folder || plan.toFolder === currentFolder)
        loadFolder(currentFolder);
    // The id an agent can USE next, not the raw filename. Returning
    // `folder/1785529483.3297_1.lisa,U=8407:2,FPS` would hand back a
    // string that the very next lookup rejects — the same mismatch
    // between id-space and disk-space this commit exists to fix, and I
    // reintroduced it in my own return value until the device test
    // showed the commas.
    return {
        changed: true,
        id: messageId(plan.toFolder, parseFilename(plan.change.toName, plan.change.toDir).unique),
        from: args.id, folder: plan.toFolder,
    };
}

// ---------------------------------------------------------------------
// Sending (#168).
//
// `lisa mail setup` already writes ~/.config/lisa/msmtprc — one account
// block per identity, credentials and XOAUTH2 included. So this is a
// composer and a subprocess, not a mail transport.

function msmtprcPath() {
    return GLib.build_filenamev([GLib.get_user_config_dir(), 'lisa', 'msmtprc']);
}

/// Can this machine send? The config existing is the honest test —
/// having msmtp installed says nothing about having an account.
function canSend() {
    return GLib.file_test(msmtprcPath(), GLib.FileTest.EXISTS)
        && GLib.find_program_in_path('msmtp') !== null;
}

/// Write a message into a Maildir folder, atomically enough.
///
/// tmp/ then rename into cur/, which is the Maildir contract: a reader
/// that scans mid-write must never see a partial file.
function writeToMaildir(folder, text) {
    const name = sentFilename(Math.floor(Date.now() / 1000),
        `${GLib.random_int()}_${GLib.getenv('USER') ?? 'lisa'}`);
    const dir = `${store.root}/${folder}`;
    GLib.mkdir_with_parents(`${dir}/tmp`, 0o700);
    GLib.mkdir_with_parents(`${dir}/cur`, 0o700);
    const tmp = `${dir}/tmp/${name}`;
    if (!GLib.file_set_contents(tmp, text))
        throw new Error(`could not write ${tmp}`);
    const to = messagePath(store.root, folder, 'cur', name);
    if (!to)
        throw new Error('refusing a path outside the maildir');
    Gio.File.new_for_path(tmp).move(Gio.File.new_for_path(to), Gio.FileCopyFlags.NONE, null, null);
    return name;
}

/// Compose → send. Returns the outcome; the caller shows it.
///
/// DRAFT FIRST. The message is written to Drafts before msmtp is
/// invoked and removed only after a success, so a crash, a power cut or
/// a refusal leaves the text on disk rather than nowhere. Losing a
/// state flag is annoying; losing what somebody wrote is not a trade
/// anybody would accept.
function sendMessage(fields, account) {
    let text;
    try {
        text = buildMessage({
            ...fields,
            date: new Date().toUTCString().replace('GMT', '+0000'),
            messageId: messageIdFor(fields.from, GLib.random_int(), Date.now()),
        });
    } catch (e) {
        return {sent: false, retryable: false, message: e.message};
    }

    let draft = null;
    try {
        draft = writeToMaildir('Drafts', text);
    } catch (e) {
        // Not fatal: the send can still go. But say so, because the
        // safety net is what makes the next step safe to attempt.
        logError(e, 'mail: could not save a draft before sending');
    }

    let status = -1;
    let stderr = '';
    try {
        const proc = Gio.Subprocess.new(msmtpArgv(msmtprcPath(), account),
            Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
        const [, , err] = proc.communicate_utf8(text, null);
        stderr = err ?? '';
        status = proc.get_exit_status();
    } catch (e) {
        return {sent: false, retryable: true, message: `Could not run msmtp: ${e.message}`};
    }

    const outcome = sendOutcome(status, stderr);
    if (!outcome.sent)
        return outcome; // the draft stays; that is the point of it

    try {
        writeToMaildir('Sent', text);
    } catch (e) {
        // Sent but not filed. Say exactly that — claiming failure would
        // invite a second send of a message already delivered.
        logError(e, 'mail: sent but could not save a copy');
        return {sent: true, message: 'Sent, but the copy in Sent could not be written'};
    }
    if (draft) {
        const path = messagePath(store.root, 'Drafts', 'cur', draft);
        if (path) GLib.unlink(path);
    }
    if (currentFolder === 'Sent' || currentFolder === 'Drafts')
        loadFolder(currentFolder);
    return outcome;
}

/// The address messages are sent from.
///
/// The first `from` in the msmtprc — the same file msmtp will use, so
/// the From header and the envelope cannot disagree. Reading the config
/// rather than keeping a second copy is the point: one account list.
function defaultIdentity() {
    const text = readFile(msmtprcPath());
    const m = String(text ?? '').match(/^\s*from\s+(\S+)/m);
    return m ? m[1] : '';
}

/// The compose window. One window per message being written.
function openCompose(fields) {
    const win = new Adw.Window({
        title: 'New message', default_width: 700, default_height: 560,
        // app.get_active_window() rather than a module-level handle:
        // the main window is a `const` inside its builder, and adding a
        // second place that remembers it is a second place to get stale.
        modal: false, transient_for: app.get_active_window(),
    });
    const toasts = new Adw.ToastOverlay();
    const header = new Adw.HeaderBar();
    const send = new Gtk.Button({label: 'Send'});
    send.add_css_class('suggested-action');
    header.pack_end(send);

    const rows = new Adw.PreferencesGroup();
    const to = new Adw.EntryRow({title: 'To'});
    const cc = new Adw.EntryRow({title: 'Cc'});
    const subject = new Adw.EntryRow({title: 'Subject'});
    to.set_text(fields.to ?? '');
    subject.set_text(fields.subject ?? '');
    [to, cc, subject].forEach((r) => rows.add(r));

    const body = new Gtk.TextView({wrap_mode: Gtk.WrapMode.WORD_CHAR, monospace: false});
    body.buffer.set_text(fields.body ?? '', -1);
    const scroller = new Gtk.ScrolledWindow({vexpand: true, child: body, margin_start: 12,
        margin_end: 12, margin_bottom: 12});

    const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 12});
    box.append(rows);
    box.append(scroller);
    const view = new Adw.ToolbarView({content: box});
    view.add_top_bar(header);
    toasts.set_child(view);
    win.set_content(toasts);

    send.connect('clicked', () => {
        const [start, end] = [body.buffer.get_start_iter(), body.buffer.get_end_iter()];
        const outcome = sendMessage({
            from: fields.from || defaultIdentity(),
            to: to.get_text(), cc: cc.get_text(), subject: subject.get_text(),
            body: body.buffer.get_text(start, end, false),
            inReplyTo: fields.inReplyTo ?? '', references: fields.references ?? '',
        });
        if (outcome.sent) {
            win.close();
            return;
        }
        // The window STAYS OPEN on failure, with the text in it and the
        // reason on screen. Closing it and showing a toast somewhere
        // else is how people lose what they wrote.
        send.set_sensitive(!!outcome.retryable);
        toasts.add_toast(new Adw.Toast({title: outcome.message, timeout: 8}));
        if (!outcome.retryable && outcome.detail)
            printerr(`mail: ${outcome.detail}`);
    });

    win.present();
}

app.run([imports.system.programInvocationName, ...ARGV]);
