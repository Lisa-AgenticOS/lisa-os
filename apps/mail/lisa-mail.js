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
    messageText, parseAddress, parseHeaders, readableBody, splitMessage,
} from './lib/rfc822.js';
import {
    FOLDER_ORDER, discoverAccounts as accountsUnder, foldersIn, isMaildirFolder,
    listFolder, messageId, messagePath, parseFilename, previewOf, uniqueMatchesId,
} from './lib/maildir.js';
import {decodeSubject, messageView} from './lib/message.js';
import {classify, grouped, unreadCount} from './lib/smart.js';
import {actionsFor, flagChange, movePaths, moveTo} from './lib/actions.js';
import {agentMayRead, parseMessageId, planFor} from './lib/agent-actions.js';
import {buildMessage, forwardFields, messageIdFor, replyFields} from './lib/compose.js';
import {msmtpArgv, sendOutcome, sentFilename} from './lib/send.js';
import {McpServer} from './lib/mcp.js';
import {linkAction} from './lib/links.js';
import {attachmentBytes as partBytes, safeFilename} from './lib/attachments.js';
import {orderedReaderChildren, spaceAction} from './lib/reader.js';

// Preview is a sibling app in the same tree — `lisa-app` resolves
// `mail/lisa-mail.js` and `preview/lisa-preview.js` under one base, and
// the PKGBUILD copies both, so this relative path is the same in a
// checkout and on the image. Importing beats copying: the previewer's
// bus name has already been gotten wrong once (a "2" that belongs only
// on the interface, see lib/previewer-protocol.js), and what Preview can
// render is a list that changes when Preview changes.
import {BUS_NAME as PREVIEWER_NAME, OBJECT_PATH as PREVIEWER_PATH}
    from '../preview/lib/previewer-protocol.js';
import {kindOf} from '../preview/lib/formats.js';

import {
    CONFIG_NAME, accountRows, bannerText, lastSynced, parseConfig, resolveMaildir,
    serializeConfig, storeSummary, syncStatus, validateMaildir,
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

/// The Unix-signal half of GLib, which moved.
///
/// `GLib.unix_signal_add` still works and prints a deprecation warning
/// on every current gjs ("has been moved to a separate platform-
/// specific library"), and `GLibUnix` does not exist on older ones. So
/// ask, the same way WebKit is asked for above, and keep the warning
/// out of the log of an app whose job includes being diagnosable.
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

/// Read at most `max` bytes from the front of a MESSAGE file.
///
/// Whole-file reads are what made a real mailbox unusable: an HTML
/// newsletter with an inline image is megabytes, and the list needs its
/// first line.
///
/// Decoded with `messageText`, not `TextDecoder` — see `readMessageFile`
/// below. A bounded read makes the UTF-8 decoder worse, not better: the
/// bound lands mid-character on a message that is fine, and every byte
/// after it becomes U+FFFD.
function readHead(path, max) {
    try {
        const stream = Gio.File.new_for_path(path).read(null);
        const bytes = stream.read_bytes(max, null);
        stream.close(null);
        return messageText(bytes.get_data() ?? new Uint8Array());
    } catch {
        return '';
    }
}

/// Read one of OUR OWN files — the config, the msmtprc. UTF-8, because
/// we wrote them and they are text.
///
/// Never a message: a message is a byte stream and this decode is lossy
/// (#232). `readMessageFile` is the one below.
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

/// The largest message file an attachment is extracted from.
///
/// A bound on the window, not on mail: a 256 MB message is not a
/// message anyone is reading, and decoding one would freeze the app
/// while it tried.
const MAX_MESSAGE_BYTES = 128 * 1024 * 1024;

/// A message file, as one character per byte.
///
/// THE ONLY WAY THIS APP READS MAIL, and it used not to be: everything
/// except the attachment path went through `readFile`'s
/// `TextDecoder('utf-8')`, which replaces every byte sequence that is
/// not valid UTF-8 with U+FFFD. That decode happens BEFORE the message
/// has said what its charset is, so `charset=ISO-8859-1; 8bit` mail was
/// destroyed on the way in and `decodeBytes`' Latin-1 branch could
/// never run: `Përshëndetje` arrived as `P<FFFD>rsh<FFFD>ndetje`, on
/// 198 of the reference device's 25,207 messages (#232).
///
/// Latin-1 is the only decoding that is a bijection on bytes, so it is
/// the only one that may be applied before the charset is known — the
/// parser looks at ASCII (boundaries, header names, base64) and the
/// part bodies come back out with every byte intact, for `rfc822.js` to
/// decode once it knows what they are.
///
/// `null` when the file cannot be read or is larger than the bound
/// above — the caller says so rather than pretending it was empty.
function readMessageFile(path) {
    try {
        const [ok, bytes] = GLib.file_get_contents(path);
        if (!ok || bytes.length > MAX_MESSAGE_BYTES)
            return null;
        return messageText(bytes);
    } catch {
        return null;
    }
}

/// The accounts this Maildir holds, and where each one's folders are.
///
/// The rules live in `lib/maildir.js` where they are testable against
/// the tree the reference device actually has — a flat set of folders
/// AND two per-account subtrees in the same root, which is the state
/// that made both real accounts invisible (#222). This is the reader.
function discoverAccounts(root) {
    return accountsUnder(root, listDir, {label: accountLabel()});
}

/// The mail store: a Maildir on disk, read on demand.
///
/// Deliberately not a cache or a database. A folder is a directory
/// listing and a message is a file; the expensive thing (parsing a
/// body) happens when a message is opened, not when a folder is.
///
/// # It has no current account, and that is the point (#264)
///
/// This class used to carry a mutable `root` naming whichever account
/// was being looked at, and a `use(account)` that overwrote it. Two
/// things then shared one answer to "which Maildir": a second window
/// replaced the store the first was using, and one click in the sidebar
/// repointed every action in flight. `root` is what `messagePath` builds
/// on, so a move, a trash, an archive or a Save As could resolve under
/// an account the message was never in — which is a bug about somebody's
/// mail, not about window management. With a single account both answers
/// were always the same string, so it stayed invisible until #67/#222
/// made multi-account real.
///
/// So **every method takes the account root it is to read**, and every
/// message that comes out of one carries the root it was read from
/// (`listFolder`, `message`). Nothing here remembers a "current"
/// anything; the window remembers what it is showing, and an ACTION
/// resolves against the message rather than against the window.
class Store {
    constructor(base) {
        /// The Maildir root itself — what the setting points at, and
        /// the fallback when it holds no recognisable account at all.
        this.base = base;
        // Every account under this Maildir, and the subtree each owns.
        this.accounts = discoverAccounts(base);
    }

    /// The subtree one account owns, by name, or `null`.
    rootOf(name) {
        return this.accounts.find((a) => a.name === name)?.root ?? null;
    }

    /// Folder names across every account, for the sidebar's outer level.
    ///
    /// Real folders only — a directory that holds no `cur/` is not one,
    /// and listing every subdirectory is how an ACCOUNT was drawn as an
    /// empty folder (#222).
    allFolders() {
        const seen = [];
        for (const a of this.accounts) {
            for (const f of foldersIn(a.root, listDir)) {
                if (!seen.includes(f))
                    seen.push(f);
            }
        }
        const known = FOLDER_ORDER.filter((f) => seen.includes(f));
        return [...known, ...seen.filter((f) => !FOLDER_ORDER.includes(f)).sort()];
    }

    folders(root) {
        return foldersIn(root, listDir);
    }

    /// The cheap half: what the filenames alone can tell us.
    ///
    /// Opens nothing. Flags, delivery time and identity all live in the
    /// Maildir name, so ordering a folder and counting its unread costs
    /// one directory listing however large the folder is. A real inbox
    /// is 3,758 messages — reading every one of them to draw a window
    /// froze the app until it was killed.
    index(root, folder) {
        const entries = [];
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${root}/${folder}/${dir}`)) {
                if (!e.isDir)
                    entries.push({dir, name: e.name});
            }
        }
        // Every row remembers which account it came out of, so an action
        // on it cannot land in another one (#264).
        return listFolder(folder, entries, {root});
    }

    /// The expensive half, for one message: headers, sender, preview.
    ///
    /// `head` bounds the read. Headers live at the top of the file and a
    /// preview only needs the first paragraph, so a 4 MB message with a
    /// base64 attachment costs the same as a short one. Opening the
    /// message for real still reads all of it — see `message()`.
    summary(root, folder, m, head = 0) {
        const path = messagePath(root, folder, m.dir, m.filename) ?? '';
        const raw = (head > 0 ? readHead(path, head) : readMessageFile(path)) ?? '';
        const {headerText} = splitMessage(raw);
        const headers = parseHeaders(headerText);
        const from = parseAddress(headers.get('from'));
        const full = {
            ...m,
            from,
            subject: decodeSubject(headers.get('subject')),
            date: headers.get('date'),
            // The readable body and nothing else. This used to fall
            // back to the RAW body when the readable one came out
            // empty, which is how `%PDF-1.4 %Ǭ… /FlateDecode` ended up
            // in the preview column of the message list (#221). A
            // message with nothing to say in the list says nothing.
            preview: previewOf(readableBody(raw)),
        };
        return {...full, group: classify(full, headers)};
    }

    /// Message summaries for one folder, newest first, at most `limit`.
    ///
    /// Paged on purpose. The list is the first thing drawn and nobody
    /// reads the four-thousandth row before the first.
    messages(root, folder, limit = PAGE) {
        return this.index(root, folder)
            .slice(0, Math.max(0, limit))
            .map((m) => this.summary(root, folder, m));
    }

    /// How many messages the folder holds, and how many are unread —
    /// both from the index, so the sidebar tells the truth about the
    /// whole folder while the list shows a page of it.
    counts(root, folder) {
        const index = this.index(root, folder);
        return {total: index.length, unread: unreadCount(index)};
    }

    /// Search the WHOLE folder, not the page.
    ///
    /// A search that only looked at what had already been drawn would be
    /// a filter wearing a search's clothes, and would answer "no
    /// results" for a message sitting on disk. Every message is
    /// considered; each costs a bounded header read rather than a whole
    /// file.
    search(root, folder, query, limit = 200) {
        const q = String(query).toLowerCase().trim();
        if (!q)
            return [];
        const out = [];
        for (const m of this.index(root, folder)) {
            const full = this.summary(root, folder, m, SEARCH_HEAD);
            const hay = `${full.subject} ${full.from.name} ${full.from.address} ${full.preview}`;
            if (hay.toLowerCase().includes(q)) {
                out.push(full);
                if (out.length >= limit)
                    break;
            }
        }
        return out;
    }

    /// Where one message's file is, or null.
    ///
    /// Split out of `message()` so the attachment reader finds the same
    /// file by the same rule. Two lookups with two ideas of which file a
    /// message id names is how an attachment ends up coming from the
    /// wrong message.
    locate(root, folder, unique) {
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${root}/${folder}/${dir}`)) {
                // Compare SANITISED to sanitised. `startsWith` against
                // the raw name never matched on a synced Maildir, where
                // mbsync's `,U=<uid>` suffix is sanitised in the id and
                // not on disk (see uniqueMatchesId).
                if (!uniqueMatchesId(parseFilename(e.name, dir).unique, unique))
                    continue;
                const path = messagePath(root, folder, dir, e.name);
                if (path)
                    return {path, dir, name: e.name};
            }
        }
        return null;
    }

    /// One message, fully parsed.
    ///
    /// `messageView` (lib/message.js) does the parsing, so the object
    /// the reading pane, the tools and Reply all share has ONE producer
    /// and a test can run it over the bytes of a message file. It used
    /// to be built here, where nothing can reach it — which is how
    /// every reply went out with no In-Reply-To for as long as replying
    /// has existed (#223).
    message(root, folder, unique) {
        const found = this.locate(root, folder, unique);
        if (!found)
            return null;
        const raw = readMessageFile(found.path);
        if (raw === null)
            return null;
        const meta = parseFilename(found.name, found.dir);
        return messageView(raw, {
            folder,
            // The account this file was READ FROM, carried on the object
            // the toolbar, the composer and the write tools all act on
            // (#264). An action resolves against this, never against
            // whatever the window is showing by then.
            root,
            id: `${folder}/${unique}`,
            unique,
            // The ACTION shape as well as the reading shape:
            // flagChange/moveTo need the filename, the dir and
            // the current flags, and returning only the reading
            // fields is why the write tools could not act on
            // what read_message could find (#167).
            filename: found.name, dir: found.dir,
            seen: meta.seen, flagged: meta.flagged,
        });
    }

    /// The bytes of one attachment, read from the message file on
    /// demand.
    ///
    /// Read as Latin-1 rather than UTF-8, and that is not a detail: a
    /// message file is a byte stream, and `readFile`'s UTF-8 decode
    /// turns every invalid sequence into U+FFFD. base64 survives that
    /// (it is ASCII), an `8bit` or `binary` part does not — it would
    /// save a corrupt file that still opens far enough to look like the
    /// app's fault. (Every read is that way round now — see
    /// `readMessageFile` — but this one is where it started.)
    ///
    /// `maxBytes` is the size of the message file itself, not the
    /// 64 MiB part bound: this is a save or an open a PERSON asked for,
    /// the whole file is already in memory, and a decoded part cannot
    /// exceed the file it came out of (base64 and quoted-printable both
    /// shrink). `attachmentBytes` returns null rather than a truncated
    /// buffer when even that is not enough (#238) — the caller says so
    /// out loud instead of writing three quarters of somebody's file.
    attachmentBytes(root, folder, unique, path) {
        const found = this.locate(root, folder, unique);
        if (!found)
            return null;
        const raw = readMessageFile(found.path);
        if (raw === null)
            return null;
        return partBytes(raw, path, {maxBytes: MAX_MESSAGE_BYTES});
    }
}

/// The Maildir, and the accounts under it.
///
/// Module-level, and legitimately so now: this app has exactly ONE
/// window (see `activate`), the store has no mutable "current account"
/// left to alias, and it is replaced only when the Maildir *path*
/// changes in settings — which is a whole-app change, not a per-window
/// one. What made the old global dangerous was not that it was global,
/// it was that two windows could write to it and `use()` could repoint
/// it under a message somebody was acting on.
let store = null;
/// The account the window is SHOWING — `{name, root}` from
/// `store.accounts`, or `null` for a Maildir with no recognisable
/// account in it.
///
/// A view-state variable, deliberately not an action-state one: nothing
/// resolves a message's path through this. Rows carry their own root.
let currentAccount = null;
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
let readerActions = null;
/// The message on screen, FULLY PARSED — the object `store.message`
/// returned, not the list row that was clicked. The toolbar, the
/// attachment reader, the composer and the "show images" banner all act
/// on this one, because a list row has no body, no Message-ID and no
/// References (#223).
let openMessage = null;
/// The window, for the file dialogs the attachment bar puts up.
let mainWindow = null;
/// The attachment bar: hidden on a message with nothing attached.
let attachmentBox = null;
let attachmentList = null;
/// Attachments already written to a temp file, keyed by message id and
/// part path.
///
/// Load-bearing for the Space gesture, not an optimisation: the peek
/// toggles closed when it is shown the URI it is already showing
/// (`CloseIfAlreadyVisible`), so a second press must hand it the SAME
/// path. Extracting into a fresh temp directory each time would swap
/// the peek for an identical copy of itself and never close.
const attachmentFiles = new Map();

const app = new Adw.Application({application_id: APP_ID});

/// Which account the window is LISTING from.
///
/// Only the list, the sidebar and the search box ask this. An action
/// never does — it asks the message (`message.root`), which is the
/// whole of #264. Falls back to the Maildir root itself, which is what
/// a directory holding no recognisable account still is.
function browsingRoot() {
    return currentAccount?.root ?? store.base;
}

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
    const root = browsingRoot();
    const messages = store.messages(root, folder, limit);
    const {total} = store.counts(root, folder);
    if (messages.length === 0) {
        const empty = new Gtk.ListBoxRow({selectable: false});
        empty.set_child(new Adw.StatusPage({
            title: 'Nothing here',
            description: `No mail in ${folder}. Mail reads a Maildir at ${root} — ` +
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
    buildAttachments({attachments: []});
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

/// What the reading pane shows when a message has no text at all.
///
/// A message whose only content is a document now has an empty body
/// rather than the document's bytes (#221), and an empty pane under a
/// subject reads as a broken app — that is exactly how #210 was
/// reported. So the pane says which it is, and points at the row above
/// it that has the file in it.
function emptyBodyNotice(full) {
    const count = full.attachments?.length ?? 0;
    if (count === 0)
        return 'This message has no text.';
    return count === 1
        ? 'This message has no text — what it carries is the attachment above.'
        : `This message has no text — what it carries is the ${count} attachments above.`;
}

function renderBody(full) {
    const rich = readerHtml && full.html;
    if (!rich) {
        const body = String(full.body ?? '');
        readerBody.buffer.set_text(body.trim() ? body : emptyBodyNotice(full), -1);
        readerStack?.set_visible_child_name('text');
        return;
    }
    readerHtml.load_html(htmlDocument(full.html), null);
    readerStack.set_visible_child_name('html');
    remoteBanner?.set_revealed(!allowRemote && hasRemoteContent(full.html));
}

// ---------------------------------------------------------------------
// Attachments (#211, #169).
//
// Before this, a message with a PDF invoice looked exactly like a
// message without one: `collectParts` decoded every leaf as text and
// `renderableBody` kept the two it could read. The invoice said
// "Dokumenti është bashkëngjitur si PDF" and the reader had thrown the
// document away.
//
// Three things happen here and nothing else does:
//
//   **Save As…** — a Gtk.FileDialog, so the directory is chosen by the
//   person. The sender's filename is a suggestion in the name field and
//   nothing more; `safeFilename` has already taken the basename off it.
//
//   **Open** — Preview, the app, by its desktop id. NOT the system's
//   default handler: `Gtk.show_uri` on a file a stranger emailed hands
//   an arbitrary type to whatever claims it, and "attachment.pdf.desktop"
//   is not a document. Preview opens images, PDFs, media and text, and
//   says so about anything else.
//
//   **Space** — the quick look Files already has, via the same
//   `org.gnome.NautilusPreviewer` interface (apps/preview/lib/
//   previewer-protocol.js). A transient peek, not the app.
//
// The bytes never go to a model. `read_message` lists what is attached —
// name, type, size — and stops there.
// ---------------------------------------------------------------------

/// Say something in the reading pane's banner.
///
/// The banner is the only notice surface this window has (there is no
/// toast overlay — checked, after writing `toast?.(…)` here and finding
/// that is a ReferenceError rather than the no-op the `?.` suggests).
function showNotice(text) {
    if (!remoteBanner) {
        log(`mail: ${text}`);
        return;
    }
    remoteBanner.set_title(text);
    remoteBanner.set_revealed(true);
}

/// Rebuild the attachment bar for the open message.
function buildAttachments(full) {
    if (!attachmentList)
        return;
    let child = attachmentList.get_first_child();
    while (child) {
        const next = child.get_next_sibling();
        attachmentList.remove(child);
        child = next;
    }
    const items = full.attachments ?? [];
    attachmentBox.set_visible(items.length > 0);
    if (items.length === 0)
        return;

    const icons = Gtk.IconTheme.get_for_display(Gdk.Display.get_default());
    for (const att of items) {
        const name = safeFilename(att.filename, att.mimeType);
        const row = new Adw.ActionRow({
            // ESCAPED, because AdwPreferencesRow renders its title as
            // Pango markup and the sender wrote this name: a filename of
            // `<b>x</b>.pdf` is not a formatting instruction. Escaping
            // rather than `use-markup: false` so this does not depend on
            // which libadwaita is installed.
            title: GLib.markup_escape_text(att.filename || name, -1),
            subtitle: GLib.markup_escape_text(
                `${att.mimeType} · ${GLib.format_size(att.size)}`, -1),
            activatable: true,
            // What Preview can actually render, asked of Preview's own
            // list rather than claimed here (apps/preview/lib/
            // formats.js). An unrecognised type still peeks — it gets
            // the generic file card, name, type and size — and saying
            // so beats a person pressing Space and wondering.
            tooltip_text: kindOf(name)
                ? 'Space to quick look, Enter to open in Preview'
                : 'Preview has no viewer for this type — Space shows a file card',
        });
        row._attachment = att;

        const buttons = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL, spacing: 4, valign: Gtk.Align.CENTER,
        });
        for (const spec of [
            {icon: 'document-save-as-symbolic', label: 'Save As…',
                tip: 'Save this attachment', run: () => saveAttachment(att)},
            {icon: 'document-open-symbolic', label: 'Open',
                tip: 'Open in Preview', run: () => openAttachment(att)},
        ]) {
            // Same fallback as the toolbar: an icon name that does not
            // resolve gives an empty button, which is indistinguishable
            // from no button.
            const known = icons?.has_icon(spec.icon);
            const button = new Gtk.Button({
                ...(known ? {icon_name: spec.icon} : {label: spec.label}),
                tooltip_text: spec.tip,
                css_classes: ['flat'],
            });
            button.connect('clicked', () => spec.run());
            buttons.append(button);
        }
        row.add_suffix(buttons);
        attachmentList.append(row);
    }

    // Start on the first attachment (#247). Two reasons, and neither is
    // decoration: it is what makes the selected state *visible* — a
    // `boxed-list` draws selection perfectly well, but before this
    // nothing was ever selected long enough to see it, because a single
    // click opened the attachment instead of selecting it — and it is
    // what makes Space work on arrival, since the Space handler falls
    // back to the selected row when focus is on the list itself.
    //
    // `select_row` emits `row-selected`, never `row-activated`, so this
    // cannot open anything.
    attachmentList.select_row(attachmentList.get_row_at_index(0));
}

/// Write one attachment to a private temp directory, and return the
/// path.
///
/// THE ONE EXTRACTION ROUTE. Open and Space both come here, so there is
/// a single place where a sender's filename meets a filesystem — and
/// that place builds the path out of a directory this app just made and
/// a name `safeFilename` has already reduced to a basename. The parent
/// check afterwards is belt and braces: if a name ever slipped a
/// separator past the sanitiser, the file would land outside the
/// directory we made and this refuses instead.
function materialise(att) {
    // Keyed by the ACCOUNT too: two accounts can hold the same folder
    // name, and a key that cannot tell them apart is a key that hands
    // back the wrong file (#264).
    const key = `${openMessage?.root}/${openMessage?.folder}/` +
        `${openMessage?.unique}#${att.path.join('.')}`;
    const cached = attachmentFiles.get(key);
    if (cached && GLib.file_test(cached, GLib.FileTest.EXISTS))
        return cached;

    const bytes = attachmentBytesOf(att);
    if (!bytes)
        return null;
    try {
        const dir = GLib.Dir.make_tmp('lisa-mail-XXXXXX');
        const path = GLib.build_filenamev([dir, safeFilename(att.filename, att.mimeType)]);
        if (Gio.File.new_for_path(path).get_parent()?.get_path() !== dir) {
            showNotice('That attachment has a filename this app will not write.');
            return null;
        }
        // PRIVATE: 0600. A stranger's attachment in a world-readable
        // temp file is a stranger's attachment in everyone's temp file.
        Gio.File.new_for_path(path).replace_contents(
            bytes, null, false, Gio.FileCreateFlags.PRIVATE, null);
        attachmentFiles.set(key, path);
        return path;
    } catch (e) {
        showNotice(`Could not unpack that attachment: ${e.message}`);
        return null;
    }
}

function attachmentBytesOf(att) {
    if (!openMessage) {
        showNotice('No message is open.');
        return null;
    }
    // REFUSED OUT LOUD, rather than saved short (#238). The row's size
    // and the bytes that get written are now the same number or there
    // are no bytes: a file that saves at three quarters of its length
    // is worse than one that does not save, because it opens.
    if (att.size > MAX_MESSAGE_BYTES) {
        showNotice(`That attachment is ${GLib.format_size(att.size)}, larger than the ` +
            `${GLib.format_size(MAX_MESSAGE_BYTES)} this app will unpack. ` +
            'Nothing was written — a partial file would be worse.');
        return null;
    }
    // `openMessage.root` — the account the message was read from, not
    // the one the sidebar is on (#264). Save As on a message you are
    // still reading after clicking another account's inbox used to
    // extract from the wrong Maildir, or from nothing.
    const bytes = store.attachmentBytes(
        openMessage.root, openMessage.folder, openMessage.unique, att.path);
    if (!bytes || bytes.length === 0) {
        showNotice('That attachment could not be read from the message file.');
        return null;
    }
    if (bytes.length !== att.size) {
        // Cannot happen — `attachmentBytes` refuses rather than
        // truncates — and is checked anyway, because the failure it
        // guards against is silent by nature and this is the last place
        // that can see both numbers.
        showNotice(`That attachment did not come back whole (${bytes.length} of ` +
            `${att.size} bytes), so nothing was written.`);
        return null;
    }
    return bytes;
}

/// Save As… — the person chooses the directory; we only suggest a name.
function saveAttachment(att) {
    const bytes = attachmentBytesOf(att);
    if (!bytes)
        return;
    const dialog = new Gtk.FileDialog({
        title: 'Save attachment',
        initial_name: safeFilename(att.filename, att.mimeType),
        modal: true,
    });
    const downloads = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_DOWNLOAD);
    if (downloads)
        dialog.set_initial_folder(Gio.File.new_for_path(downloads));
    dialog.save(mainWindow, null, (source, result) => {
        try {
            const file = source.save_finish(result);
            if (!file)
                return;
            file.replace_contents(bytes, null, false, Gio.FileCreateFlags.NONE, null);
            showNotice(`Saved ${file.get_basename()}`);
        } catch (e) {
            // Cancelling a dialog is not an error worth a banner.
            if (!dialogDismissed(e))
                showNotice(`Could not save: ${e.message}`);
        }
    });
}

/// Did the person just close the dialog?
///
/// Cancelling is not a failure and must not raise a banner. Asked of
/// the error domain first, with the message as a fallback — the same
/// two-step Preview uses, because the domain check needs a GError and
/// not every binding hands one over.
function dialogDismissed(e) {
    try {
        if (typeof e?.matches === 'function')
            return e.matches(Gtk.dialog_error_quark(), Gtk.DialogError.DISMISSED);
    } catch {
        // Fall through to the message check.
    }
    return /dismiss/i.test(String(e?.message ?? ''));
}

/// Open — hand the file to Preview, the app.
function openAttachment(att) {
    const path = materialise(att);
    if (!path)
        return;
    try {
        const info = Gio.DesktopAppInfo.new('app.lisaos.Preview.desktop');
        if (!info) {
            showNotice('Preview is not installed, so there is nothing to open this with.');
            return;
        }
        info.launch([Gio.File.new_for_path(path)], null);
    } catch (e) {
        showNotice(`Could not open that attachment: ${e.message}`);
    }
}

/// Space — the transient peek, the same one Files gets.
///
/// `CloseIfAlreadyVisible: true` is what makes Space a toggle: the peek
/// closes when it is shown the URI it is already showing. The bus name
/// is VERSIONLESS and only the interface carries the "2" — owning the
/// versioned name produced zero bus traffic on the device, which is why
/// the constants are imported from lib/previewer-protocol.js rather than
/// spelled again here.
function peekAttachment(att) {
    const path = materialise(att);
    if (!path)
        return;
    const uri = Gio.File.new_for_path(path).get_uri();
    try {
        Gio.DBus.session.call(
            PREVIEWER_NAME, PREVIEWER_PATH, 'org.gnome.NautilusPreviewer2', 'ShowFile',
            new GLib.Variant('(ssbs)', [uri, '', true, '']),
            null, Gio.DBusCallFlags.NONE, 5000, null,
            (bus, result) => {
                try {
                    bus.call_finish(result);
                } catch (e) {
                    showNotice(`Quick look is unavailable: ${e.message}`);
                }
            });
    } catch (e) {
        showNotice(`Quick look is unavailable: ${e.message}`);
    }
}

/// Remove the temp copies this session made.
///
/// Only the paths in `attachmentFiles` — every one of them a file this
/// app wrote into a directory it created with `make_tmp` — and the
/// directory that held it. Nothing recursive, nothing from a name a
/// sender chose.
function clearAttachmentFiles() {
    for (const path of attachmentFiles.values()) {
        try {
            const file = Gio.File.new_for_path(path);
            const dir = file.get_parent();
            file.delete(null);
            if (dir && GLib.path_get_basename(dir.get_path()).startsWith('lisa-mail-'))
                dir.delete(null);
        } catch {
            // A temp file that has already gone is not a problem worth
            // reporting on the way out.
        }
    }
    attachmentFiles.clear();
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
function mbsyncrcPath() {
    return GLib.build_filenamev([GLib.get_user_config_dir(), 'lisa', 'mbsyncrc']);
}

function accountLabel() {
    const text = readFile(mbsyncrcPath());
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
    // The folders of the message's OWN account: whether Archive exists
    // is a question about where this message lives, not about where the
    // sidebar is pointing.
    for (const action of actionsFor(msg, store.folders(msg.root), canSend())) {
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
                applyMove(msg, change, msg.folder);
        } else if (action.kind === 'send' && action.enabled !== false) {
            const me = defaultIdentity();
            openCompose(action.id === 'reply'
                ? replyFields(msg, me)
                : forwardFields(msg, me));
            return;
        } else if (action.kind === 'move' && action.enabled !== false) {
            const mv = moveTo(msg, action.folder);
            if (mv)
                applyMove(msg, mv, mv.toFolder);
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
///
/// **Takes the message, not a folder name** (#264). Both paths come out
/// of `movePaths`, which resolves them against the account the message
/// was listed from — the tested decision in lib/actions.js. This used to
/// build them from `store.root`, one mutable module-level string that a
/// second window replaced and a sidebar click overwrote, so the file
/// somebody archived could be renamed inside a different account.
function applyMove(message, change, toFolder) {
    const paths = movePaths(message, change, toFolder);
    if (!paths)
        throw new Error('refusing a move that names no account, or a path outside the maildir');
    GLib.mkdir_with_parents(paths.dir, 0o700);
    Gio.File.new_for_path(paths.from).move(
        Gio.File.new_for_path(paths.to), Gio.FileCopyFlags.NONE, null, null);
}

function showMessage(msg) {
    // `?? msg` used to be silent, and that silence WAS the bug (#210):
    // a list row carries subject and sender but no body, so a failed
    // lookup filled the header and left the pane empty — indis-
    // tinguishable from an empty message, and impossible to report.
    // A reader that cannot find the file on disk must say so.
    const loaded = store.message(msg.root, msg.folder, msg.unique);
    const full = loaded ?? {
        ...msg,
        body: `Could not read this message from the Maildir.\n\n` +
              `folder: ${msg.folder}\nid: ${msg.unique}\n\n` +
              `The list row was built from the index, so the message ` +
              `exists — the file lookup under the folder above did not ` +
              `match it.`,
        html: null,
    };
    // THE TOOLBAR ACTS ON THE FULL MESSAGE, not on the list row. A row
    // carries a preview and no body, no Message-ID and no References —
    // so Reply quoted nothing and threaded nowhere (#223). Same object
    // for the buttons, the attachment reader and the composer.
    openMessage = full;
    buildToolbar(full);
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
    buildAttachments(full);
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
        // A folder is present in an account when that account really
        // has one — `cur/` or `new/`, not merely a directory of that
        // name (#222).
        const present = accounts.filter(
            (a) => isMaildirFolder(`${a.root}/${folder}`, listDir));
        if (present.length === 0)
            continue;

        if (!multi) {
            const a = present[0];
            folderList.append(folderRow(folder, a, store.counts(a.root, folder).unread, false));
            continue;
        }

        // One expander per folder, one row per account inside it. The
        // badge on the expander is the folder's total across accounts,
        // which is the number you want when it is collapsed.
        const total = present.reduce((n, a) => n + store.counts(a.root, folder).unread, 0);
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
            expander.add_row(folderRow(folder, a, store.counts(a.root, folder).unread, true));
        folderList.append(expander);
    }

    const first = folderList.get_row_at_index(0);
    if (first && first._folder) {
        folderList.select_row(first);
    } else {
        // `currentAccount` first: a rebuild after a sync must not bounce
        // somebody out of the account they were reading and back to the
        // first one. Only a window that has not chosen yet falls through
        // to accounts[0].
        selectFolder(currentAccount ?? accounts[0], currentFolder || 'INBOX');
    }
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
///
/// This moves the VIEW and nothing else (#264). It used to call
/// `store.use(account.name)`, which repointed the one root every action
/// in the app resolved against — including the message still open in
/// the reading pane, whose next button press then renamed a file in
/// this account instead of its own.
function selectFolder(account, folder) {
    currentAccount = account ?? null;
    loadFolder(folder, PAGE);
}

// ---------------------------------------------------------------------
// Sync: the button you press, and the failure you could not see (#265).
//
// On the reference device `lisa-mail-sync.service` had been failing
// every five minutes since boot — the login keyring was locked, so
// `lisa mail token` had nothing to hand mbsync — and Mail said nothing
// at all. A person saw mail that was silently hours stale and no reason
// why. The reasoning already existed and was already tested
// (lib/settings.js `syncStatus`) and this file already imported it;
// nothing rendered it.
//
// What "blocked" means is decided in ONE place, and it is not here.
// This section gathers facts, hands them to `syncStatus`, and draws the
// answer. #249 will render the same function per account in Settings.
// ---------------------------------------------------------------------

/// The banner, the refresh button and the quiet timestamp.
let syncBanner = null;
let syncAction = null;
let refreshButton = null;
let refreshSpinner = null;
let refreshStack = null;
let lastSyncedLabel = null;
/// One pass at a time. A second press while mbsync is running would
/// start a second syncer over the same Maildir, which is how a mailbox
/// gets two writers and a lock file.
let syncing = false;

/// Has anything written the config that joins an account to mbsync?
///
/// The file existing is the honest test, and it is the one input
/// `syncStatus` had no source for: the settings page never passed
/// `bridged`, so on a device where `lisa mail setup` had run it still
/// answered "Nothing is syncing yet — issue #155" and pointed at the
/// wrong layer.
function isBridged() {
    return GLib.file_test(mbsyncrcPath(), GLib.FileTest.EXISTS);
}

/// Is the login keyring locked?
///
/// A property READ, never an unlock: reading `Locked` does not prompt,
/// so this is safe to call while drawing a window. The same question
/// `lisa mail token` asks before it refuses (cli/lisa/src/mail.rs), and
/// asked the same way, so the app and the CLI cannot disagree about the
/// state of the machine they are both on.
///
/// An error is "not locked". A system with no Secret Service at all is
/// a different, earlier answer that `syncStatus` gives first; failing
/// open here means a broken property read degrades to the old silence
/// rather than inventing a refusal.
function loginKeyringLocked() {
    try {
        const reply = Gio.DBus.session.call_sync(
            'org.freedesktop.secrets', '/org/freedesktop/secrets/collection/login',
            'org.freedesktop.DBus.Properties', 'Get',
            new GLib.Variant('(ss)', ['org.freedesktop.Secret.Collection', 'Locked']),
            new GLib.VariantType('(v)'), Gio.DBusCallFlags.NONE, 1000, null);
        const value = reply.deepUnpack()[0];
        // gjs hands a boxed `v` back as a GLib.Variant on some versions
        // and as the unpacked value on others.
        return typeof value === 'boolean' ? value : value?.get_boolean?.() === true;
    } catch {
        return false;
    }
}

/// Everything `syncStatus` needs to answer, observed once.
///
/// Gathered here so the banner and the settings page ask the same
/// questions in the same order; the ANSWER is lib/settings.js's.
function syncFacts(accounts) {
    return {
        mbsync: GLib.find_program_in_path('mbsync') !== null,
        secretService: hasSecretService(),
        keyringLocked: loginKeyringLocked(),
        accounts: accounts ?? [],
        bridged: isBridged(),
    };
}

/// Put the current answer on the banner.
function showSyncStatus(status) {
    if (!syncBanner)
        return;
    syncAction = status.action ?? null;
    syncBanner.set_title(bannerText(status));
    // An empty label is how AdwBanner draws no button. Where there is
    // nothing to offer, the sentence stands on its own rather than
    // sitting next to a control that does nothing.
    syncBanner.set_button_label(syncAction?.label ?? '');
    syncBanner.set_revealed(status.kind !== 'ok');
}

/// Re-ask every layer, and redraw the banner.
///
/// GOA is asynchronous, so this lands a moment after the window does.
/// That is the right way round: a mail window that waits for a D-Bus
/// daemon before it draws is a mail window that does not open on the
/// machine that is broken.
function refreshSyncStatus() {
    goaAccounts()
        .then((accounts) => showSyncStatus(syncStatus(syncFacts(accounts))))
        .catch((e) => logError(e, 'mail: could not work out why sync is stuck'));
}

/// The banner's button, for the states that have one.
function runSyncAction() {
    if (syncAction?.id === 'online-accounts') {
        openOnlineAccounts();
        return;
    }
    if (syncAction?.id === 'unlock-keyring')
        unlockKeyring();
}

/// Ask the Secret Service to unlock the login collection.
///
/// The issue's point, and the difference between a banner and a help
/// page: the fix for a locked keyring is that something touches a
/// credential so the prompt appears, and this app can be that something
/// instead of describing it. `Unlock` returns either "already open" or
/// a Prompt object that has to be shown — gnome-keyring draws the
/// password dialog only when somebody calls `Prompt`.
function unlockKeyring() {
    const bus = Gio.DBus.session;
    bus.call('org.freedesktop.secrets', '/org/freedesktop/secrets',
        'org.freedesktop.Secret.Service', 'Unlock',
        new GLib.Variant('(ao)', [['/org/freedesktop/secrets/collection/login']]),
        new GLib.VariantType('(aoo)'), Gio.DBusCallFlags.NONE, 30000, null,
        (source, result) => {
            let prompt = '/';
            try {
                [, prompt] = source.call_finish(result).deepUnpack();
            } catch (e) {
                syncBanner?.set_title(`Could not ask the keyring to unlock: ${e.message}`);
                return;
            }
            if (prompt && prompt !== '/') {
                showUnlockPrompt(prompt);
                return;
            }
            // Nothing to prompt for: it was already open, so re-ask
            // rather than leaving a banner about a state that has gone.
            refreshSyncStatus();
        });
}

/// Draw the prompt the Secret Service handed back, and re-ask when it
/// finishes — dismissed or not.
function showUnlockPrompt(path) {
    const bus = Gio.DBus.session;
    // Subscribed BEFORE the call: `Completed` can arrive while the
    // reply to `Prompt` is still in flight, and a listener attached
    // afterwards would miss it and leave a stale banner.
    const id = bus.signal_subscribe(
        null, 'org.freedesktop.Secret.Prompt', 'Completed', path, null,
        Gio.DBusSignalFlags.NONE, () => {
            bus.signal_unsubscribe(id);
            refreshSyncStatus();
        });
    bus.call('org.freedesktop.secrets', path, 'org.freedesktop.Secret.Prompt', 'Prompt',
        new GLib.Variant('(s)', ['']), null, Gio.DBusCallFlags.NONE, 120000, null,
        (source, result) => {
            try {
                source.call_finish(result);
            } catch (e) {
                bus.signal_unsubscribe(id);
                syncBanner?.set_title(`The unlock prompt could not be shown: ${e.message}`);
            }
        });
}

/// When mail last actually arrived from the server.
///
/// mbsync rewrites `.mbsyncstate` in every folder it syncs, so the
/// newest of those across every account is the last pass that completed
/// — which is the number a person wants and is not the same as "when
/// did new mail last arrive". Read off the disk rather than remembered
/// in this process, so it survives a restart and counts the timer's
/// passes as well as the button's.
///
/// `0` when nothing has ever synced, which `lastSynced` renders as
/// "Never synced" rather than as a very old date.
function lastSyncSeconds() {
    const roots = store.accounts.length > 0
        ? store.accounts.map((a) => a.root)
        : [store.base];
    let newest = 0;
    for (const root of roots) {
        for (const folder of foldersIn(root, listDir)) {
            try {
                const info = Gio.File.new_for_path(`${root}/${folder}/.mbsyncstate`)
                    .query_info('time::modified', Gio.FileQueryInfoFlags.NONE, null);
                newest = Math.max(newest, Number(info.get_attribute_uint64('time::modified')));
            } catch {
                // No state file in this folder: nothing synced it.
            }
        }
    }
    return newest;
}

function updateLastSynced() {
    lastSyncedLabel?.set_label(lastSynced(lastSyncSeconds(), Date.now() / 1000));
}

/// Show or hide the spinner in the refresh button.
function setSyncing(busy) {
    syncing = busy;
    refreshButton?.set_sensitive(!busy);
    refreshButton?.set_tooltip_text(busy ? 'Syncing…' : 'Sync now');
    if (!refreshStack)
        return;
    refreshStack.set_visible_child_name(busy ? 'busy' : 'idle');
    if (busy)
        refreshSpinner?.start();
    else
        refreshSpinner?.stop();
}

/// Refresh: one sync pass, then re-read the Maildir.
///
/// `lisa mail sync` rather than mbsync directly — it is the same one
/// command the timer runs, it passes our config rather than
/// `~/.mbsyncrc`, and it indexes what it fetched into retrieval (#170).
/// Reimplementing any part of that here would be a second thing to keep
/// in step.
function runSync() {
    if (syncing)
        return;
    if (!GLib.find_program_in_path('lisa')) {
        syncBanner?.set_title('The `lisa` command is not on PATH, so Mail cannot start a sync.');
        syncBanner?.set_button_label('');
        syncBanner?.set_revealed(true);
        return;
    }
    setSyncing(true);
    let proc;
    try {
        proc = Gio.Subprocess.new(['lisa', 'mail', 'sync'],
            Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_PIPE);
    } catch (e) {
        setSyncing(false);
        syncBanner?.set_title(`Could not run a sync: ${e.message}`);
        syncBanner?.set_revealed(true);
        return;
    }
    proc.communicate_utf8_async(null, null, (source, result) => {
        setSyncing(false);
        let stderr = '';
        let ok = false;
        try {
            const [, , err] = source.communicate_utf8_finish(result);
            stderr = err ?? '';
            // `get_successful`, not `get_exit_status`: the latter is
            // documented as an error to call on a process that was
            // signalled, and mbsync is exactly the kind of long
            // subprocess a session teardown kills.
            ok = source.get_successful();
        } catch (e) {
            stderr = e.message;
        }
        // The Maildir has changed under us: a new account subtree can
        // appear on a first sync, so the store is rebuilt rather than
        // just the folder redrawn.
        store = new Store(store.base);
        // Keep looking at the account the person was looking at — the
        // fresh object from the new store, never the old one, which
        // belongs to a Store that no longer exists.
        currentAccount = store.accounts.find((a) => a.name === currentAccount?.name) ?? null;
        reloadFolders();
        updateLastSynced();
        if (ok) {
            refreshSyncStatus();
            return;
        }
        // A failed pass says what the command said, on the banner. Then
        // `syncStatus` gets the last word: if the reason is a state it
        // knows — a locked keyring, no account — its sentence is the
        // better one and it replaces this.
        const said = stderr.split('\n').map((l) => l.trim())
            .filter(Boolean).slice(-1)[0] ?? 'no reason given';
        syncBanner?.set_title(`Sync failed: ${said}`);
        syncBanner?.set_button_label('');
        syncBanner?.set_revealed(true);
        syncAction = null;
        refreshSyncStatus();
    });
}

app.connect('activate', () => {
    // ONE WINDOW (#264).
    //
    // GApplication fires `activate` on first launch AND every time a
    // second instance starts: the new process registers, forwards
    // `activate` to the running one and exits. This handler used to
    // build a window unconditionally, so every launch stacked another —
    // three of them on the reference device, all reading and writing the
    // same module-level state. Present what is there instead.
    //
    // `get_windows().find(…)` rather than the Assistant's
    // `app.activeWindow ?? …` one-liner: `activeWindow` is the window
    // with toplevel FOCUS, which is null when the app has none, and
    // "nothing is focused" is exactly the moment a second launch
    // happens. The tag is what tells our window from the composer's,
    // which is an Adw.Window and never registers with the application.
    const existing = app.get_windows().find((w) => w.__lisaMail);
    if (existing) {
        existing.present();
        return;
    }

    const resolved = maildirRoot();
    store = new Store(resolved.path);

    // Sized against the monitor rather than a fixed guess: 1280x820 is
    // a dialog on the 4K panel this was first seen on. Three panes need
    // room, and a mail window that opens smaller than its own content
    // reads as a preview of an app.
    const display = window_default_size();
    const window = new Adw.ApplicationWindow({
        application: app, title: 'Mail',
        default_width: display.width, default_height: display.height,
    });
    // The file dialogs the attachment bar puts up need a parent.
    mainWindow = window;
    // How the next `activate` finds this one instead of building a
    // second. Set before anything can fail below it.
    window.__lisaMail = true;

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
        const hits = store.search(browsingRoot(), currentFolder, q);
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

    // Refresh, next to the search box (#265). One sync pass, a spinner
    // while it runs, and the list re-read when it finishes — the app had
    // no way at all to fetch mail, which on a machine whose timer has
    // been failing since boot means no way to find out either.
    const listIcons = Gtk.IconTheme.get_for_display(Gdk.Display.get_default());
    refreshSpinner = new Gtk.Spinner();
    refreshStack = new Gtk.Stack();
    // Same fallback rule as every other button in this app: an icon
    // name that does not resolve draws an empty button, which is
    // indistinguishable from no button.
    refreshStack.add_named(listIcons?.has_icon('view-refresh-symbolic')
        ? new Gtk.Image({icon_name: 'view-refresh-symbolic'})
        : new Gtk.Label({label: 'Refresh'}), 'idle');
    refreshStack.add_named(refreshSpinner, 'busy');
    refreshButton = new Gtk.Button({child: refreshStack, tooltip_text: 'Sync now'});
    refreshButton.connect('clicked', () => runSync());
    listHeader.pack_start(refreshButton);

    const menu = new Gio.Menu();
    menu.append('Preferences', 'app.preferences');
    menu.append('About Mail', 'app.about');
    listHeader.pack_end(new Gtk.MenuButton({
        icon_name: 'open-menu-symbolic', menu_model: menu, tooltip_text: 'Main menu',
    }));
    listPane.add_top_bar(listHeader);

    // Why nothing is arriving, when nothing is arriving — in
    // `syncStatus`'s words, over the list rather than buried in a
    // settings page nobody opens when the app looks fine.
    syncBanner = new Adw.Banner({revealed: false});
    syncBanner.connect('button-clicked', () => runSyncAction());
    listPane.add_top_bar(syncBanner);

    // …and how stale what IS on screen is. Quiet, and always there:
    // stale mail with a timestamp is a different experience from stale
    // mail that looks current.
    lastSyncedLabel = new Gtk.Label({
        label: '', xalign: 0, margin_top: 4, margin_bottom: 4, margin_start: 12, margin_end: 12,
        css_classes: ['dim-label', 'caption'],
    });
    listPane.add_bottom_bar(lastSyncedLabel);

    listPane.set_content(new Gtk.ScrolledWindow({child: listBox, vexpand: true}));

    // Pane 3: the reading pane.
    readerTitle = new Gtk.Label({xalign: 0, wrap: true, css_classes: ['title-2']});
    readerFrom = new Gtk.Label({xalign: 0, css_classes: ['dim-label']});
    readerBody = new Gtk.TextView({
        editable: false, cursor_visible: false, wrap_mode: Gtk.WrapMode.WORD_CHAR,
        left_margin: 16, right_margin: 16, top_margin: 12, bottom_margin: 16,
        monospace: false,
    });
    // Not itself scrolled: the scrolling lives inside the body stack,
    // which is what lets the attachment bar pin under it.
    const readerBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL, spacing: 6,
        margin_top: 16, margin_start: 16, margin_end: 16,
    });

    // The attachment bar. Where it goes is `READER_ORDER`'s business,
    // not this line's — see lib/reader.js, and the append loop below.
    attachmentList = new Gtk.ListBox({
        selection_mode: Gtk.SelectionMode.SINGLE,
        css_classes: ['boxed-list'],
        // SINGLE CLICK SELECTS; A DOUBLE CLICK OPENS (#247).
        //
        // `Gtk.ListBox` defaults this to true, so `row-activated` fired
        // on the FIRST click and every click opened the attachment. The
        // comment below has always claimed otherwise; the property was
        // never set, so the comment described an intention rather than
        // the behaviour.
        //
        // This is also what makes the Space peek reachable at all: with
        // single-click activation there was no way to select a row
        // without opening it, so the selected-row branch of the Space
        // handler could never be reached by mouse.
        activate_on_single_click: false,
    });
    // Enter (or a double click) opens; the buttons in the row do the
    // same two things for a mouse.
    attachmentList.connect('row-activated', (_l, row) => {
        if (row?._attachment)
            openAttachment(row._attachment);
    });
    // WHICH WIDGET OWNS SPACE — the reasoning is in `spaceAction`
    // (lib/reader.js), including why this controller must stay on the
    // attachment list and nowhere else. This is the binding; the
    // decision it defers to is unit-tested.
    const attachmentKeys = new Gtk.EventControllerKey({
        propagation_phase: Gtk.PropagationPhase.CAPTURE,
    });
    attachmentKeys.connect('key-pressed', (_c, keyval) => {
        if (keyval !== Gdk.KEY_space)
            return false;
        const focus = mainWindow?.get_focus?.() ?? null;
        // Walk out to the row: focus may be on a label inside it.
        let node = focus;
        while (node && node._attachment === undefined)
            node = node.get_parent?.() ?? null;
        const focusedAttachment = node?._attachment ?? null;
        const selectedAttachment =
            attachmentList.get_selected_row()?._attachment ?? null;
        // `spaceAction` returns WHICH attachment, so the precedence
        // between a focused row and a stale selection lives in one
        // tested place rather than being repeated here.
        const {action, attachment} = spaceAction({
            focusIsButton: focus instanceof Gtk.Button,
            focusedAttachment,
            selectedAttachment,
        });
        if (action !== 'peek')
            return false;
        peekAttachment(attachment);
        return true;
    });
    attachmentList.add_controller(attachmentKeys);
    attachmentBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL, margin_top: 6, margin_bottom: 6, visible: false,
    });
    attachmentBox.append(attachmentList);

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
        if (openMessage)
            renderBody(openMessage);
    });

    // Only the body expands (`READER_EXPANDING_SLOT`), so everything
    // appended after it is pinned under it rather than scrolling away
    // with the message. `readerBox` is not itself inside a
    // ScrolledWindow — the scrolling happens inside this stack.
    readerStack = new Gtk.Stack({vexpand: true});
    readerStack.add_named(readerScroll, 'text');
    if (WebKit) {
        readerHtml = buildWebView();
        readerStack.add_named(readerHtml, 'html');
    }

    // The reading pane's order is DATA, in lib/reader.js, because an
    // append sequence here is untestable and this one was wrong (#247).
    // `orderedReaderChildren` throws if a slot has no widget, so a
    // dropped attachment bar is a crash rather than a silent absence.
    for (const child of orderedReaderChildren({
        'title': readerTitle,
        'from': readerFrom,
        'remote-banner': remoteBanner,
        'body': readerStack,
        'attachments': attachmentBox,
    }))
        readerBox.append(child);

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
    updateLastSynced();
    // Asked once the window exists, and answered when GOA answers.
    refreshSyncStatus();

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

    /// Give the socket back, once, whatever ended the process.
    ///
    /// This used to hang off `close-request` alone, so every exit that
    /// is not a person clicking the window's X — SIGTERM from systemd,
    /// a logout that kills the session's units, `pkill` — left the
    /// socket file behind. mcp-bus defers socket activation and reads
    /// PRESENCE AS AVAILABILITY, so a dead app went on advertising
    /// `search_mail` and `read_message` and agentd got ECONNREFUSED
    /// instead of "Mail is not running" (#219, filed against Surfer;
    /// the same shape was here).
    ///
    /// Idempotent, because more than one of these paths can fire: a
    /// close-request that quits the app reaches `shutdown` too.
    let released = false;
    const release = () => {
        if (released)
            return;
        released = true;
        mcp.stop();
        // The temp copies of attachments go with it. Somebody else's
        // invoice should not outlive the app that unpacked it.
        clearAttachmentFiles();
    };
    window.connect('close-request', () => {
        release();
        return false;
    });
    // `shutdown` covers every clean exit — `app.quit()`, the last
    // window closing, a Gio.Application decision this file never makes.
    app.connect('shutdown', () => release());
    // …and the signals that end a process without one. The handler runs
    // on the main loop, so it may do real work; `quit()` then unwinds
    // the rest of the app the ordinary way. SOURCE_REMOVE because a
    // second SIGTERM should kill us rather than queue another quit.
    for (const signal of [1 /* SIGHUP */, 2 /* SIGINT */, 15 /* SIGTERM */]) {
        onUnixSignal(signal, () => {
            release();
            app.quit();
            return GLib.SOURCE_REMOVE;
        });
    }

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
    const browsing = browsingRoot();
    const folders = store.folders(browsing);
    // `counts().total`, not `messages().length` — `messages` is PAGED
    // (50 by default), so this row reported "50 messages" for every
    // folder with more than fifty in it, on a page whose entire purpose
    // is to say truthfully what is on disk.
    for (const f of folders)
        counts[f] = store.counts(browsing, f).total;
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
        // `openMessage`, not an invented `currentMessage` — the “show
        // images” banner already re-renders through it. There used to
        // be a second global holding the same message under a different
        // name, which is a second place to get stale.
        if (openMessage) renderBody(openMessage);
    });
    reading.add(remoteRow);
    page.add(reading);

    // --- Why there is, or is not, mail arriving.
    //
    // The same `syncStatus` the main window's banner renders, off the
    // same facts (#265). Two surfaces with two opinions about whether
    // sync is blocked is how a person ends up debugging a layer that is
    // fine.
    const sync = new Adw.PreferencesGroup({title: 'Syncing'});
    const status = syncStatus(syncFacts([]));
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
        const next = syncStatus(syncFacts(accounts));
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
        // A different Maildir has different accounts; the one the window
        // was showing is not in it. `reloadFolders` picks the first.
        currentAccount = null;
        reloadFolders();
        refreshSyncStatus();
    });

    dialog.present(parent);
}

/// `search_mail` — subjects, senders and previews across folders.
///
/// Returns summaries, never whole bodies: a search that dumped every
/// matching message into the model's context would spend the window on
/// the first query and hand over far more than was asked for.
///
/// Spam, Junk and Trash are not searched (#237). That is contextd's
/// decision from #185 — "a corpus that exists to be hostile", and mail
/// the person already chose to forget — and the Agent Bus was reaching
/// the same corpus through a second door with no index in between. The
/// WINDOW still shows them: a person opening their own Spam folder is
/// their business (ADR-0029).
function searchMail({query = '', folder = '', limit = 20} = {}) {
    const q = String(query).toLowerCase().trim();
    if (folder && !agentMayRead(folder))
        return {messages: [], skipped: `${folder} is not served to agents`};
    // The account the window is on. The Agent Bus surface has always
    // been scoped to one account — it had no way to name a second — and
    // it stays that way here rather than quietly widening what a model
    // can read while fixing where an action lands (#264).
    const root = browsingRoot();
    const folders = (folder ? [String(folder)] : store.folders(root)).filter(agentMayRead);
    const out = [];
    for (const f of folders) {
        // The index first, then a bounded read per candidate. Calling
        // `messages()` here would either page the tool (a model asking
        // about mail would be answered from the newest 50 and never
        // told) or read every byte of a four-thousand-message folder
        // while the caller waits.
        for (const entry of store.index(root, f)) {
            const m = store.summary(root, f, entry, SEARCH_HEAD);
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
    // The same folders `search_mail` will not list (#237). Asked here
    // too, because an id is a string a model can construct: refusing to
    // list Spam while answering a hand-built `Spam/…` id would be a
    // guardrail with a door in it.
    if (!agentMayRead(folder))
        return {error: `${folder} is not served to agents — open it in Mail instead`};
    const msg = store.message(browsingRoot(), folder, unique);
    if (!msg)
        return {error: `no message ${id}`};
    return {
        id: msg.id, folder: msg.folder, subject: msg.subject,
        from: msg.from.address, from_name: msg.from.name, to: msg.to, date: msg.date,
        body: msg.body,
        // WHAT is attached, never the attachment. A model is handed
        // `body`, which is prose; the bytes of a file a stranger sent
        // are not prose and are not summarised here. Listing them is
        // what lets an assistant answer "is the invoice attached?" and
        // point at Preview, which is where a document gets read
        // (#211). Filenames are the sender's, so they arrive
        // sanitised — a model that has read one is a model that can be
        // asked to repeat it into a path.
        attachments: (msg.attachments ?? []).map((a) => ({
            filename: safeFilename(a.filename, a.mimeType),
            mime_type: a.mimeType,
            size: a.size,
        })),
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
    const root = browsingRoot();
    const message = store.message(root, parsed.folder, parsed.unique);
    const plan = planFor(tool, message, args, store.folders(root));
    if (plan.error)
        return {error: plan.error};
    if (plan.noop)
        return {changed: false, reason: plan.noop, id: args.id};
    try {
        // `message`, which carries the root it was READ from — so a
        // tool call that arrives while the person is clicking around
        // the sidebar still renames the file it looked at (#264).
        applyMove(message, plan.change, plan.toFolder);
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
/// The account a composed message is filed under is the one being
/// browsed — Sent and Drafts belong to the mailbox you are working in.
/// Which is exactly the ambiguity that put them in the orphan tree
/// (#222); it is at least a stated rule now, and it is the same string
/// the list on screen came from.
function writeToMaildir(folder, text) {
    const root = browsingRoot();
    const name = sentFilename(Math.floor(Date.now() / 1000),
        `${GLib.random_int()}_${GLib.getenv('USER') ?? 'lisa'}`);
    const dir = `${root}/${folder}`;
    GLib.mkdir_with_parents(`${dir}/tmp`, 0o700);
    GLib.mkdir_with_parents(`${dir}/cur`, 0o700);
    const tmp = `${dir}/tmp/${name}`;
    if (!GLib.file_set_contents(tmp, text))
        throw new Error(`could not write ${tmp}`);
    const to = messagePath(root, folder, 'cur', name);
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
        const path = messagePath(browsingRoot(), 'Drafts', 'cur', draft);
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
