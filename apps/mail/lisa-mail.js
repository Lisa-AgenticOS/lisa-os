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
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk';

import {decodeWords, parseAddress, parseHeaders, readableBody, splitMessage} from './lib/rfc822.js';
import {listFolder, messagePath, previewOf} from './lib/maildir.js';
import {classify, grouped, unreadCount} from './lib/smart.js';
import {McpServer} from './lib/mcp.js';

const APP_ID = 'app.lisaos.Mail';

/// Where the Maildir lives. `LISA_MAILDIR` first so a second account,
/// or a test corpus, needs no rebuild.
function maildirRoot() {
    return GLib.getenv('LISA_MAILDIR') ||
        GLib.build_filenamev([GLib.get_home_dir(), 'Mail']);
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

/// The mail store: a Maildir on disk, read on demand.
///
/// Deliberately not a cache or a database. A folder is a directory
/// listing and a message is a file; the expensive thing (parsing a
/// body) happens when a message is opened, not when a folder is.
class Store {
    constructor(root) {
        this.root = root;
    }

    folders() {
        const found = listDir(this.root)
            .filter((e) => e.isDir && !e.name.startsWith('.'))
            .map((e) => e.name);
        const known = FOLDER_ORDER.filter((f) => found.includes(f));
        const rest = found.filter((f) => !FOLDER_ORDER.includes(f)).sort();
        return [...known, ...rest];
    }

    /// Message summaries for one folder, grouped and newest first.
    ///
    /// Headers are read for every message because the grouping needs
    /// them; the body is read too, for the preview line. That is one
    /// file read per message and it is what makes the list useful — a
    /// list without previews is a list of subjects.
    messages(folder) {
        const entries = [];
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${this.root}/${folder}/${dir}`)) {
                if (!e.isDir)
                    entries.push({dir, name: e.name});
            }
        }
        return listFolder(folder, entries).map((m) => {
            const raw = readFile(messagePath(this.root, folder, m.dir, m.filename) ?? '');
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
        });
    }

    /// One message, fully parsed.
    message(folder, unique) {
        for (const dir of ['cur', 'new']) {
            for (const e of listDir(`${this.root}/${folder}/${dir}`)) {
                if (!e.name.startsWith(unique))
                    continue;
                const path = messagePath(this.root, folder, dir, e.name);
                if (!path)
                    continue;
                const raw = readFile(path);
                const {headerText, body} = splitMessage(raw);
                const headers = parseHeaders(headerText);
                return {
                    folder,
                    id: `${folder}/${unique}`,
                    from: parseAddress(headers.get('from')),
                    to: headers.get('to'),
                    subject: decodeSubject(headers.get('subject')),
                    date: headers.get('date'),
                    body: bodyOf(raw, body),
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
let listBox = null;
let readerTitle = null;
let readerFrom = null;
let readerBody = null;
let folderList = null;

const app = new Adw.Application({application_id: APP_ID});

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

function loadFolder(folder) {
    currentFolder = folder;
    let child = listBox.get_first_child();
    while (child) {
        const next = child.get_next_sibling();
        listBox.remove(child);
        child = next;
    }
    const messages = store.messages(folder);
    if (messages.length === 0) {
        const empty = new Gtk.ListBoxRow({selectable: false});
        empty.set_child(new Adw.StatusPage({
            title: 'Nothing here',
            description: `No mail in ${folder}. Mail reads a Maildir at ${store.root} — ` +
                'sync one with mbsync or offlineimap and it will appear.',
            vexpand: true,
        }));
        listBox.append(empty);
        return;
    }
    for (const group of grouped(messages)) {
        listBox.append(groupHeader(group.name, group.items.length));
        for (const m of group.items)
            listBox.append(messageRow(m));
    }
}

function showMessage(msg) {
    const full = store.message(msg.folder, msg.unique) ?? msg;
    readerTitle.set_label(full.subject ?? '');
    // Both halves of the sender, always: a display name is
    // attacker-controlled, and `"security@yourbank.com" <evil@x.test>`
    // is a real shape. Showing the name alone is how that works.
    const name = full.from?.name;
    const addr = full.from?.address ?? '';
    readerFrom.set_label(name ? `${name}  ·  ${addr}` : addr);
    readerBody.buffer.set_text(full.body ?? '', -1);
}

app.connect('activate', () => {
    store = new Store(maildirRoot());

    const window = new Adw.ApplicationWindow({
        application: app, title: 'Mail', default_width: 1280, default_height: 820,
    });

    // Pane 1: folders.
    folderList = new Gtk.ListBox({css_classes: ['navigation-sidebar']});
    for (const folder of store.folders()) {
        const row = new Gtk.ListBoxRow();
        row._folder = folder;
        const box = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL, spacing: 8,
            margin_top: 6, margin_bottom: 6, margin_start: 6, margin_end: 6,
        });
        box.append(new Gtk.Label({label: folder, xalign: 0, hexpand: true}));
        const unread = unreadCount(store.messages(folder));
        if (unread > 0)
            box.append(new Gtk.Label({label: String(unread), css_classes: ['dim-label', 'caption']}));
        row.set_child(box);
        folderList.append(row);
    }
    folderList.connect('row-selected', (_l, row) => {
        if (row?._folder)
            loadFolder(row._folder);
    });

    const sidebar = new Adw.ToolbarView();
    sidebar.add_top_bar(new Adw.HeaderBar({title_widget: new Adw.WindowTitle({title: 'Mail'})}));
    sidebar.set_content(new Gtk.ScrolledWindow({child: folderList, vexpand: true}));

    // Pane 2: the grouped message list.
    listBox = new Gtk.ListBox({css_classes: ['navigation-sidebar']});
    listBox.connect('row-activated', (_l, row) => {
        if (row?._message)
            showMessage(row._message);
    });
    const listPane = new Adw.ToolbarView();
    const listHeader = new Adw.HeaderBar({show_title: false});
    const search = new Gtk.SearchEntry({placeholder_text: 'Search', hexpand: true});
    search.connect('search-changed', () => {
        const q = search.text.toLowerCase().trim();
        let child = listBox.get_first_child();
        while (child) {
            if (child._message) {
                const m = child._message;
                const hay = `${m.subject} ${m.from.name} ${m.from.address} ${m.preview}`.toLowerCase();
                child.set_visible(!q || hay.includes(q));
            }
            child = child.get_next_sibling();
        }
    });
    listHeader.set_title_widget(search);
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
    readerBox.append(readerScroll);
    const readerPane = new Adw.ToolbarView();
    readerPane.add_top_bar(new Adw.HeaderBar({show_title: false}));
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

    const first = folderList.get_row_at_index(0);
    if (first)
        folderList.select_row(first);
    else
        loadFolder('INBOX');

    // The Agent Bus surface. Started with the window and stopped with
    // it: socket presence IS tool availability (mcp-bus defers socket
    // activation), so a dead socket would be a tool that times out.
    const mcp = new McpServer({
        searchMail: (args) => searchMail(args),
        readMessage: (args) => readMessage(args),
    });
    mcp.start();
    window.connect('close-request', () => {
        mcp.stop();
        return false;
    });

    window.present();
});

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
        for (const m of store.messages(f)) {
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

app.run([imports.system.programInvocationName, ...ARGV]);
