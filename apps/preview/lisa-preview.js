#!/usr/bin/env -S gjs -m
// Preview — look at a file, and let the assistant look with you.
//
// GJS + GTK4 + libadwaita, the Surfer stack minus WebKit. Images render
// through GdkPixbuf (22 formats on the shipped image, AVIF and JXL
// among them); PDFs render through Poppler, which the image already
// carries as libpoppler-glib.
//
// WHY THIS EXISTS AT ALL: on 2026-08-02 nothing on the system claimed
// image/* — double-clicking a photo in Files did nothing, on a machine
// whose libraries decode more image formats than its browser does. The
// capability was there and the door was missing.
//
// The pure modules own every decision that can be got wrong quietly:
// lib/formats.js decides what we claim (and generates the .desktop MIME
// list from the same source, so we cannot register for something we
// cannot open), lib/view.js owns zoom/fit/rotation arithmetic, and
// lib/mcp-protocol.js owns the JSON-RPC surface and the provenance tag.

import Adw from 'gi://Adw?version=1';
import Gtk from 'gi://Gtk?version=4.0';
import Gdk from 'gi://Gdk?version=4.0';
import GdkPixbuf from 'gi://GdkPixbuf?version=2.0';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';

import {kindOf, siblings} from './lib/formats.js';
import {zoomStep, fitScale, fitWidthScale, step, rotate} from './lib/view.js';
import {McpServer} from './lib/mcp.js';
import {Previewer} from './lib/previewer.js';

/// Poppler is optional at RUNTIME, not at build time. A dev host
/// without it should still open images rather than failing to start —
/// and the failure, when a PDF is opened anyway, should name the
/// missing piece instead of showing an empty page.
///
/// NOT `await import()`. The first version used top-level await here,
/// and the cost was invisible until the app ran on hardware: the MCP
/// socket bound, accepted connections, and never answered one. A
/// top-level await makes the whole module an async evaluation, and
/// `app.run()` then drives the main loop from inside a continuation
/// that has not finished — GIO accepts the connection at the C level
/// while the JS `await` on read_line_async never resolves. Symptom:
/// every agent call times out with nothing in any log.
///
/// The legacy `imports.gi` accessor is synchronous, so the module stays
/// a plain one and the try/catch still gives the optional behaviour.
let Poppler = null;
try {
    imports.gi.versions.Poppler = '0.18';
    Poppler = imports.gi.Poppler;
} catch (e) {
    Poppler = null;
}

const app = new Adw.Application({
    application_id: 'app.lisaos.Preview',
    // Without HANDLES_OPEN, GTK routes `preview file.png` to activate()
    // with the argument silently dropped — the app opens empty and the
    // user assumes the file is broken.
    flags: Gio.ApplicationFlags.HANDLES_OPEN,
});

let win = null, mcp = null, previewer = null;
const state = {
    path: null, kind: null, rotation: 0, zoom: 1, fitMode: 'fit',
    pageIndex: 0, pageCount: 1, doc: null, pixbuf: null,
    files: [], fileIndex: -1,
};

let picture = null, drawing = null, stack = null, pageLabel = null, zoomLabel = null, titleLabel = null;

function contentSize() {
    if (state.kind === 'image' && state.pixbuf)
        return {width: state.pixbuf.get_width(), height: state.pixbuf.get_height()};
    if (state.kind === 'document' && state.doc) {
        const page = state.doc.get_page(state.pageIndex);
        const [w, h] = page.get_size();
        return {width: w, height: h};
    }
    return {width: 0, height: 0};
}

function viewportSize() {
    if (!win) return {width: 800, height: 600};
    // Subtract the header bar; an image "fitted" to the whole window is
    // always a scrollbar taller than the space it has.
    return {width: Math.max(1, win.get_width() - 24), height: Math.max(1, win.get_height() - 96)};
}

function effectiveScale() {
    if (state.fitMode === 'fit') return fitScale(contentSize(), viewportSize(), state.rotation);
    if (state.fitMode === 'width') return fitWidthScale(contentSize(), viewportSize(), state.rotation);
    return state.zoom;
}

function render() {
    const scale = effectiveScale();
    if (zoomLabel) zoomLabel.label = `${Math.round(scale * 100)}%`;
    if (pageLabel) {
        pageLabel.label = state.kind === 'document'
            ? `${state.pageIndex + 1} / ${state.pageCount}` : '';
        pageLabel.visible = state.kind === 'document';
    }
    if (titleLabel && state.path)
        titleLabel.label = state.path.split('/').pop();

    if (state.kind === 'image' && state.pixbuf) {
        const {width, height} = contentSize();
        let pb = state.pixbuf;
        if (state.rotation) {
            // GdkPixbuf rotates anticlockwise; our model is clockwise.
            const angle = (360 - state.rotation) % 360;
            pb = pb.rotate_simple(angle) ?? pb;
        }
        picture.set_paintable(Gdk.Texture.new_for_pixbuf(pb));
        picture.set_size_request(Math.round(width * scale), Math.round(height * scale));
        stack.set_visible_child_name('image');
    } else if (state.kind === 'document' && state.doc) {
        drawing.queue_draw();
        stack.set_visible_child_name('document');
    }
}

function drawPage(area, cr, _w, _h) {
    if (!state.doc) return;
    const page = state.doc.get_page(state.pageIndex);
    const [pw, ph] = page.get_size();
    const scale = effectiveScale();
    const quarter = state.rotation % 180 !== 0;
    const outW = Math.round((quarter ? ph : pw) * scale);
    const outH = Math.round((quarter ? pw : ph) * scale);
    area.set_size_request(outW, outH);
    // White under the page: a PDF with transparent regions on a dark
    // theme renders as unreadable light-on-light without it.
    cr.setSourceRGB(1, 1, 1);
    cr.rectangle(0, 0, outW, outH);
    cr.fill();
    cr.save();
    if (state.rotation) {
        cr.translate(outW / 2, outH / 2);
        cr.rotate(state.rotation * Math.PI / 180);
        cr.translate(-(pw * scale) / 2, -(ph * scale) / 2);
    }
    cr.scale(scale, scale);
    page.render(cr);
    cr.restore();
    cr.$dispose?.();
}

function loadFile(path) {
    const kind = kindOf(path);
    if (!kind) {
        toast(`Preview does not open ${path.split('/').pop()}`);
        return false;
    }
    try {
        if (kind === 'image') {
            state.pixbuf = GdkPixbuf.Pixbuf.new_from_file(path);
            state.doc = null;
            state.pageCount = 1;
        } else {
            if (!Poppler) {
                toast('PDF support needs poppler-glib, which is not installed');
                return false;
            }
            state.doc = Poppler.Document.new_from_gfile(Gio.File.new_for_path(path), null, null);
            state.pixbuf = null;
            state.pageCount = state.doc.get_n_pages();
        }
    } catch (e) {
        // Name the file AND the reason. "Could not open" alone sends
        // people to check permissions on a file that is simply corrupt.
        toast(`Could not open ${path.split('/').pop()}: ${e.message}`);
        return false;
    }
    state.path = path;
    state.kind = kind;
    state.pageIndex = 0;
    state.rotation = 0;
    state.fitMode = 'fit';

    // Folder browsing, best-effort: an unreadable directory costs the
    // ← → keys, not the file the user actually asked for.
    try {
        const dir = Gio.File.new_for_path(path).get_parent();
        const en = dir.enumerate_children('standard::name', Gio.FileQueryInfoFlags.NONE, null);
        const names = [];
        let info;
        while ((info = en.next_file(null)) !== null) names.push(info.get_name());
        const sib = siblings(path, names);
        state.files = sib.files;
        state.fileIndex = sib.index;
    } catch (e) {
        state.files = [path];
        state.fileIndex = 0;
    }
    render();
    return true;
}

function toast(text) {
    if (win?.__toasts) win.__toasts.add_toast(new Adw.Toast({title: text, timeout: 4}));
    else printerr(text);
}

function goFile(delta) {
    if (state.fileIndex < 0 || state.files.length === 0) return;
    const next = step(state.fileIndex, state.files.length, delta);
    if (next !== state.fileIndex) loadFile(state.files[next]);
}

function goPage(delta) {
    if (state.kind !== 'document') { goFile(delta); return; }
    const next = step(state.pageIndex, state.pageCount, delta);
    if (next !== state.pageIndex) { state.pageIndex = next; render(); }
}

function setZoom(z) { state.fitMode = 'free'; state.zoom = z; render(); }

function buildWindow() {
    win = new Adw.ApplicationWindow({application: app, default_width: 1000, default_height: 720});
    const toasts = new Adw.ToastOverlay();
    win.__toasts = toasts;

    const header = new Adw.HeaderBar();
    titleLabel = new Gtk.Label({label: 'Preview', ellipsize: 3});
    header.set_title_widget(titleLabel);

    const open = new Gtk.Button({icon_name: 'document-open-symbolic', tooltip_text: 'Open (Ctrl+O)'});
    open.connect('clicked', chooseFile);
    header.pack_start(open);

    const zoomOut = new Gtk.Button({icon_name: 'zoom-out-symbolic', tooltip_text: 'Zoom out (-)'});
    zoomOut.connect('clicked', () => setZoom(zoomStep(effectiveScale(), -1)));
    const zoomIn = new Gtk.Button({icon_name: 'zoom-in-symbolic', tooltip_text: 'Zoom in (+)'});
    zoomIn.connect('clicked', () => setZoom(zoomStep(effectiveScale(), +1)));
    const fit = new Gtk.Button({icon_name: 'zoom-fit-best-symbolic', tooltip_text: 'Fit (0)'});
    fit.connect('clicked', () => { state.fitMode = 'fit'; render(); });
    const rot = new Gtk.Button({icon_name: 'object-rotate-right-symbolic', tooltip_text: 'Rotate (R)'});
    rot.connect('clicked', () => { state.rotation = rotate(state.rotation, 90); render(); });
    zoomLabel = new Gtk.Label({label: '100%', width_chars: 5});
    pageLabel = new Gtk.Label({label: '', width_chars: 8});
    [zoomOut, zoomIn, fit, rot].forEach(b => header.pack_start(b));
    header.pack_end(pageLabel);
    header.pack_end(zoomLabel);

    picture = new Gtk.Picture({can_shrink: false, halign: Gtk.Align.CENTER, valign: Gtk.Align.CENTER});
    drawing = new Gtk.DrawingArea({halign: Gtk.Align.CENTER, valign: Gtk.Align.CENTER});
    drawing.set_draw_func(drawPage);

    stack = new Gtk.Stack();
    stack.add_named(picture, 'image');
    stack.add_named(drawing, 'document');

    const scroller = new Gtk.ScrolledWindow({hexpand: true, vexpand: true, child: stack});
    const view = new Adw.ToolbarView({content: scroller});
    view.add_top_bar(header);
    toasts.set_child(view);
    win.set_content(toasts);

    const keys = new Gtk.EventControllerKey();
    keys.connect('key-pressed', (_c, keyval, _code, mods) => {
        const ctrl = (mods & Gdk.ModifierType.CONTROL_MASK) !== 0;
        switch (keyval) {
        case Gdk.KEY_plus: case Gdk.KEY_equal: setZoom(zoomStep(effectiveScale(), +1)); return true;
        case Gdk.KEY_minus: setZoom(zoomStep(effectiveScale(), -1)); return true;
        case Gdk.KEY_0: state.fitMode = 'fit'; render(); return true;
        case Gdk.KEY_1: setZoom(1); return true;
        case Gdk.KEY_w: if (ctrl) { win.close(); return true; } break;
        case Gdk.KEY_o: if (ctrl) { chooseFile(); return true; } break;
        case Gdk.KEY_r: state.rotation = rotate(state.rotation, 90); render(); return true;
        case Gdk.KEY_Right: case Gdk.KEY_Page_Down: case Gdk.KEY_space: goPage(+1); return true;
        case Gdk.KEY_Left: case Gdk.KEY_Page_Up: goPage(-1); return true;
        case Gdk.KEY_bracketright: goFile(+1); return true;
        case Gdk.KEY_bracketleft: goFile(-1); return true;
        }
        return false;
    });
    win.add_controller(keys);
    // Re-fit on resize, but only in a fit mode: recomputing while the
    // user is at a chosen zoom would fight them.
    win.connect('notify::default-width', () => { if (state.fitMode !== 'free') render(); });
    return win;
}

function chooseFile() {
    const dialog = new Gtk.FileDialog();
    dialog.open(win, null, (src, res) => {
        try {
            const file = src.open_finish(res);
            if (file) loadFile(file.get_path());
        } catch (e) { /* dismissed */ }
    });
}

/// What the agent can read. Read-tier, `file` provenance (lib/mcp-protocol.js).
const handlers = {
    async readDocument() {
        if (!state.path) return {error: 'nothing open'};
        const base = {path: state.path, name: state.path.split('/').pop(), kind: state.kind};
        if (state.kind === 'document' && state.doc) {
            const page = state.doc.get_page(state.pageIndex);
            const text = page.get_text() ?? '';
            const cap = 30000;
            return {
                ...base,
                page: state.pageIndex + 1, pages: state.pageCount,
                text: text.slice(0, cap),
                // A truncation the model cannot see is a page it thinks
                // it has read (the lesson from Surfer's extract.js).
                truncated: text.length > cap,
            };
        }
        const {width, height} = contentSize();
        // No OCR and no vision model here. Saying so beats returning an
        // empty `text` field that reads as "this image contains nothing".
        return {...base, width, height, text: null,
            note: 'image metadata only — Preview does not OCR or caption'};
    },
};

/// The Agent Bus socket lives exactly as long as a window does
/// (mcp-bus defers socket activation, so presence == usability).
///
/// Started HERE and not in `startup`, which is where I put it first: the
/// service bound and accepted connections, and then never answered one
/// — `incoming` fired for nobody. Whatever the cause, `startup` runs
/// before the application has a window and is the wrong moment; Surfer
/// starts its socket from `activate` and works, and matching that is
/// worth more than a theory about why.
function ensureUi() {
    if (!win) buildWindow();
    if (!mcp) {
        mcp = new McpServer(handlers);
        try { mcp.start(); } catch (e) { logError(e, 'lisa-preview mcp'); }
    }
    if (!previewer) {
        // Space in Files lands here (lib/previewer.js).
        previewer = new Previewer({
            onShow: (uri) => {
                const file = Gio.File.new_for_uri(uri);
                const path = file.get_path();
                // A URI with no local path is a remote or virtual file.
                // Refusing by name beats opening an empty window.
                if (!path) { toast(`Preview cannot open ${uri}`); return; }
                if (loadFile(path)) win.present();
            },
            onClose: () => win?.close(),
            isVisible: () => !!win?.get_visible(),
        });
        try { previewer.start(); } catch (e) { logError(e, 'lisa-preview previewer'); }
    }
}

app.connect('open', (_a, files) => {
    ensureUi();
    const path = files[0]?.get_path();
    win.present();
    if (path) loadFile(path);
});
app.connect('activate', () => { ensureUi(); win.present(); render(); });
app.connect('shutdown', () => {
    try { mcp?.stop(); } catch (e) { /* exiting */ }
    try { previewer?.stop(); } catch (e) { /* exiting */ }
});

app.run([imports.system.programInvocationName, ...ARGV]);
