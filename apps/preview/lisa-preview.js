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
import {COLORS, normalizeRect, isClick, viewToPage, annotRect, savePathFor, unsavedLabel}
    from './lib/annotate.js';
import {movePage, removePage, orderChanged, qpdfPageSpec} from './lib/reorder.js';
import {looksBinary, truncateText, cardSubtitle} from './lib/peek.js';
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

/// WebKit is optional exactly like Poppler: the html peek degrades to
/// showing the SOURCE as text on a host without webkitgtk-6.0, rather
/// than the app failing to start. Same synchronous accessor, same
/// reason (the top-level-await footgun documented above).
let WebKit = null;
try {
    imports.gi.versions.WebKit = '6.0';
    WebKit = imports.gi.WebKit;
} catch (e) {
    WebKit = null;
}

// Launched by dbus-daemon to answer Nautilus's startup ping (see
// org.gnome.NautilusPreviewer.service): register the previewer name and
// wait. The first activate is GApplication's own from run() — presenting
// there would flash an empty window at every login. Later activates are
// real launches and present normally.
const serviceLaunch = ARGV.includes('--previewer-service');
const argv = ARGV.filter(a => a !== '--previewer-service');
let suppressPresent = serviceLaunch;

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
    // Slice 2 (annotation + page order). `annots` holds what has been
    // applied to the in-memory document, for undo and the dirty count;
    // `pageOrder` is display order over ORIGINAL page indices; `drag`
    // is the live marquee in widget coordinates while one is in flight.
    annots: [], tool: null, pageOrder: [], drag: null,
};

let picture = null, drawing = null, stack = null, pageLabel = null, zoomLabel = null, titleLabel = null;
let refreshEditUi = null, rebuildThumbs = null;
let textView = null, webView = null, card = null;

/// The poppler page at the CURRENT DISPLAY position — after reordering,
/// display position i shows original page pageOrder[i].
function docPage() {
    if (!state.doc || !state.pageOrder.length) return null;
    return state.doc.get_page(state.pageOrder[state.pageIndex]);
}

function contentSize() {
    if (state.kind === 'image' && state.pixbuf)
        return {width: state.pixbuf.get_width(), height: state.pixbuf.get_height()};
    if (state.kind === 'document' && state.doc) {
        const [w, h] = docPage().get_size();
        return {width: w, height: h};
    }
    return {width: 0, height: 0};
}

function viewportSize() {
    if (!win) return {width: 800, height: 600};
    let w = win.get_width(), h = win.get_height();
    // Before the first allocation both are 0 — and the previewer-service
    // path loads a file BEFORE present(), so fitting against a 1×1
    // viewport showed a 16×16 icon at 6% on the device. The default
    // size is the truth until the window has one of its own.
    if (w === 0 || h === 0) { w = win.default_width; h = win.default_height; }
    // Subtract the header bar; an image "fitted" to the whole window is
    // always a scrollbar taller than the space it has.
    return {width: Math.max(1, w - 24), height: Math.max(1, h - 96)};
}

function effectiveScale() {
    if (state.fitMode === 'fit') return fitScale(contentSize(), viewportSize(), state.rotation);
    // 'fill' is the EXPLICIT fit — the button, the 0 key — and may
    // enlarge; 'fit' is the on-open default and never does.
    if (state.fitMode === 'fill') return fitScale(contentSize(), viewportSize(), state.rotation, true);
    if (state.fitMode === 'width') return fitWidthScale(contentSize(), viewportSize(), state.rotation);
    return state.zoom;
}

function render() {
    const scale = effectiveScale();
    if (zoomLabel) {
        zoomLabel.label = `${Math.round(scale * 100)}%`;
        // A zoom percentage over a text peek or a file card is a
        // number about nothing.
        zoomLabel.visible = state.kind === 'image' || state.kind === 'document';
    }
    if (pageLabel) {
        pageLabel.label = state.kind === 'document'
            ? `${state.pageIndex + 1} / ${state.pageOrder.length}` : '';
        pageLabel.visible = state.kind === 'document';
    }
    if (titleLabel && state.path) {
        titleLabel.set_title(state.path.split('/').pop());
        titleLabel.set_subtitle(unsavedLabel(state.annots.length,
            state.kind === 'document' && orderChanged(state.pageOrder, state.pageCount)));
    }

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
    } else if (state.kind === 'text') {
        stack.set_visible_child_name('text');
    } else if (state.kind === 'html' && webView) {
        // WebKit scrolls internally; give it the viewport, not its
        // (zero) natural size, or it collapses inside the scroller.
        const vp = viewportSize();
        webView.set_size_request(vp.width, vp.height);
        stack.set_visible_child_name('html');
    } else if (state.kind === 'card') {
        const vp = viewportSize();
        card.set_size_request(vp.width, vp.height);
        stack.set_visible_child_name('card');
    }
}

function drawPage(area, cr, _w, _h) {
    if (!state.doc) return;
    const page = docPage();
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
    // Live marquee while a highlight/box drag is in flight — drawn in
    // widget space so it tracks the pointer exactly.
    if (state.drag) {
        const d = state.drag;
        const c = state.tool === 'highlight'
            ? [1, 0.85, 0.31, 0.35] : [0.43, 0.27, 0.79, 0.9];
        cr.setSourceRGBA(...c);
        if (state.tool === 'highlight') {
            cr.rectangle(d.x1, d.y1, d.x2 - d.x1, d.y2 - d.y1);
            cr.fill();
        } else {
            cr.setLineWidth(2);
            cr.rectangle(d.x1, d.y1, d.x2 - d.x1, d.y2 - d.y1);
            cr.stroke();
        }
    }
    cr.$dispose?.();
}

function loadFile(path) {
    // Null means "unrecognised", and unrecognised gets the generic
    // card — Nautilus sends Space for ANY selected file, and a peek
    // tool that shows nothing has told the user their key is broken.
    let kind = kindOf(path) ?? 'card';
    state.textContent = null;
    try {
        if (kind === 'text' || kind === 'html') {
            const [okRead, bytes] = GLib.file_get_contents(path);
            if (!okRead) throw new Error('unreadable');
            if (looksBinary(bytes)) {
                // A .log that is actually gzip lands on the card, not
                // in a text view full of mojibake.
                kind = 'card';
            } else if (kind === 'html' && WebKit) {
                state.htmlUri = Gio.File.new_for_path(path).get_uri();
                webView.load_uri(state.htmlUri);
                state.textContent = new TextDecoder().decode(bytes);
            } else {
                // html without WebKit shows its source — degraded and
                // labelled, never silent.
                if (kind === 'html') toast('WebKit is not installed — showing the HTML source');
                kind = 'text';
                const {text, truncated} = truncateText(new TextDecoder().decode(bytes));
                state.textContent = text;
                textView.buffer.set_text(text, -1);
                if (truncated) toast('Large file — showing the first part only');
            }
        }
        if (kind === 'card') {
            const file = Gio.File.new_for_path(path);
            const info = file.query_info(
                'standard::display-name,standard::size,standard::content-type,standard::icon',
                Gio.FileQueryInfoFlags.NONE, null);
            const ctype = info.get_content_type();
            card.set_title(info.get_display_name());
            card.set_description(cardSubtitle(
                ctype ? Gio.content_type_get_description(ctype) : '',
                info.get_size()));
            const gicon = info.get_icon();
            const display = Gdk.Display.get_default();
            if (gicon && display) {
                card.paintable = Gtk.IconTheme.get_for_display(display)
                    .lookup_by_gicon(gicon, 128, 1, Gtk.TextDirection.NONE, 0);
            } else {
                card.icon_name = 'text-x-generic-symbolic';
            }
        }
        if (kind === 'image') {
            // An SVG has no natural pixel size — rasterizing at its
            // declared viewBox showed a 16×16 icon as a dot. 2048 keeps
            // vectors crisp through a full-window fit; raster formats
            // keep their true size (upscaling THEM is the blur fitScale
            // refuses by default).
            state.pixbuf = path.toLowerCase().endsWith('.svg')
                ? GdkPixbuf.Pixbuf.new_from_file_at_size(path, 2048, 2048)
                : GdkPixbuf.Pixbuf.new_from_file(path);
            state.doc = null;
            state.pageCount = 1;
        } else if (kind === 'document') {
            if (!Poppler) {
                toast('PDF support needs poppler-glib, which is not installed');
                return false;
            }
            state.doc = Poppler.Document.new_from_gfile(Gio.File.new_for_path(path), null, null);
            state.pixbuf = null;
            state.pageCount = state.doc.get_n_pages();
        } else {
            state.doc = null;
            state.pixbuf = null;
            state.pageCount = 1;
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
    state.annots = [];
    state.tool = null;
    state.drag = null;
    // Every load starts in app manners; the previewer's onShow flips
    // this AFTER loadFile, so only the Space gesture gets the toggle.
    state.quickLook = false;
    state.pageOrder = kind === 'document'
        ? Array.from({length: state.pageCount}, (_, i) => i) : [];
    refreshEditUi?.();
    // Rebuild (or clear) the pages sidebar: stale rows pin the OLD
    // document's PopplerPage refs in their draw funcs, and a click on
    // one indexes past the new order (#196).
    rebuildThumbs?.();

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
    // Navigation is bounded by the DISPLAY order, which shrinks when
    // pages are removed — not by the document's own page count.
    const next = step(state.pageIndex, state.pageOrder.length, delta);
    if (next !== state.pageIndex) { state.pageIndex = next; render(); }
}

function setZoom(z) { state.fitMode = 'free'; state.zoom = z; render(); }

// --- slice 2: annotation + page order ------------------------------

function isDirty() {
    return state.annots.length > 0 ||
        (state.kind === 'document' && orderChanged(state.pageOrder, state.pageCount));
}

function popplerRect(r) {
    const rect = new Poppler.Rectangle();
    rect.x1 = r.x1; rect.y1 = r.y1; rect.x2 = r.x2; rect.y2 = r.y2;
    return rect;
}

function popplerColor(c) {
    const color = new Poppler.Color();
    color.red = c.red; color.green = c.green; color.blue = c.blue;
    return color;
}

/// A sticky note on a page, at TOP-DOWN page points — the shared core
/// under both the click gesture and the agent tool.
function noteOnPage(page, x, y, text) {
    if (!page) return false;
    const [, ph] = page.get_size();
    const r = annotRect({x1: x, y1: y, x2: x + 22, y2: y + 22}, ph);
    const annot = Poppler.AnnotText.new(state.doc, popplerRect(r));
    annot.set_contents(text);
    annot.set_color(popplerColor(COLORS.note));
    page.add_annot(annot);
    state.annots.push({page, annot});
    refreshEditUi?.();
    render();
    return true;
}

/// A highlight or box over a TOP-DOWN page-points rect. Highlight is a
/// real PDF text-markup annotation (quadrilaterals in PDF coords);
/// building the quad array can fail at the GJS boxed-struct boundary,
/// and the fallback — a yellow square outline — marks the same area
/// honestly instead of dropping the gesture.
function rectOnPage(page, td, tool) {
    if (!page) return false;
    const [, ph] = page.get_size();
    const r = annotRect(td, ph);
    let annot = null;
    if (tool === 'highlight') {
        try {
            const mk = (x, y) => {
                const pt = new Poppler.Point();
                pt.x = x; pt.y = y;
                return pt;
            };
            const quad = new Poppler.Quadrilateral();
            quad.p1 = mk(r.x1, r.y2); quad.p2 = mk(r.x2, r.y2);
            quad.p3 = mk(r.x1, r.y1); quad.p4 = mk(r.x2, r.y1);
            annot = Poppler.AnnotTextMarkup.new_highlight(
                state.doc, popplerRect(r), [quad]);
            annot.set_color(popplerColor(COLORS.highlight));
        } catch (e) {
            logError(e, 'lisa-preview: text-markup highlight unavailable, using box');
            annot = null;
        }
    }
    if (!annot) {
        annot = Poppler.AnnotSquare.new(state.doc, popplerRect(r));
        annot.set_color(popplerColor(
            tool === 'highlight' ? COLORS.highlight : COLORS.box));
    }
    page.add_annot(annot);
    state.annots.push({page, annot});
    refreshEditUi?.();
    render();
    return true;
}

function addNoteAt(vx, vy, text) {
    const p = viewToPage({x: vx, y: vy}, effectiveScale());
    return noteOnPage(docPage(), p.x, p.y, text);
}

function addRectAnnot(viewRect, tool) {
    const scale = effectiveScale();
    return rectOnPage(docPage(), {
        x1: viewRect.x1 / scale, y1: viewRect.y1 / scale,
        x2: viewRect.x2 / scale, y2: viewRect.y2 / scale,
    }, tool);
}

function undoAnnot() {
    const last = state.annots.pop();
    if (!last) return;
    try { last.page.remove_annot(last.annot); } catch (e) {
        logError(e, 'lisa-preview undo');
    }
    refreshEditUi?.();
    render();
}

function applyOrder(next) {
    if (!next) { toast('A document needs at least one page'); return; }
    state.pageOrder = next;
    state.pageIndex = Math.min(state.pageIndex, next.length - 1);
    rebuildThumbs?.();
    refreshEditUi?.();
    render();
}

/// Save an "(edited)" copy next to the original — never over it.
/// Annotations are already applied to the in-memory document, so
/// poppler saves them; a changed page order needs qpdf on top (poppler
/// cannot reorder or delete pages), staged through a temp file.
let saveInFlight = false;

function saveEdited() {
    if (saveInFlight) { toast('Already saving'); return; }
    if (!isDirty()) { toast('Nothing to save'); return; }
    let names = [];
    try {
        const dir = Gio.File.new_for_path(state.path).get_parent();
        const en = dir.enumerate_children('standard::name', Gio.FileQueryInfoFlags.NONE, null);
        let info;
        while ((info = en.next_file(null)) !== null) names.push(info.get_name());
    } catch (e) { /* best effort — savePathFor handles [] */ }
    const target = savePathFor(state.path, names);
    const reordered = orderChanged(state.pageOrder, state.pageCount);
    if (reordered && !GLib.find_program_in_path('qpdf')) {
        toast('Page reordering needs qpdf, which is not installed');
        return;
    }
    const annotsAtSave = state.annots.length;
    try {
        if (!reordered) {
            state.doc.save(Gio.File.new_for_path(target).get_uri());
            afterSave(target, annotsAtSave);
            return;
        }
        // O_EXCL temp file (#197): a predictable /tmp name is a symlink
        // target someone else can plant. file_open_tmp creates it
        // atomically; poppler then overwrites the real file we own.
        const [fd, tmp] = GLib.file_open_tmp('lisa-preview-XXXXXX.pdf');
        GLib.close(fd);
        state.doc.save(Gio.File.new_for_path(tmp).get_uri());
        saveInFlight = true;
        const proc = Gio.Subprocess.new(
            ['qpdf', tmp, '--pages', '.', qpdfPageSpec(state.pageOrder), '--', target],
            Gio.SubprocessFlags.STDERR_PIPE);
        proc.communicate_utf8_async(null, null, (p, res) => {
            saveInFlight = false;
            let stderr = '';
            try { [, , stderr] = p.communicate_utf8_finish(res); } catch (e) { /* below */ }
            GLib.unlink(tmp);
            if (p.get_successful()) afterSave(target, annotsAtSave);
            else toast(`qpdf failed: ${(stderr || 'unknown error').trim().slice(0, 120)}`);
        });
    } catch (e) {
        saveInFlight = false;
        toast(`Could not save: ${e.message}`);
    }
}

function afterSave(target, annotsAtSave) {
    toast(`Saved ${target.split('/').pop()}`);
    // Annotations added WHILE qpdf ran are only in this window —
    // reloading would silently discard them (#197). Keep working state
    // in that case; otherwise open the copy that was just written: its
    // state is clean, the result is on screen, and the untouched
    // original stays behind on disk.
    if (state.annots.length !== annotsAtSave) return;
    loadFile(target);
}

function buildWindow() {
    win = new Adw.ApplicationWindow({application: app, default_width: 1000, default_height: 720});
    const toasts = new Adw.ToastOverlay();
    win.__toasts = toasts;

    const header = new Adw.HeaderBar();
    titleLabel = new Adw.WindowTitle({title: 'Preview'});
    header.set_title_widget(titleLabel);

    const open = new Gtk.Button({icon_name: 'document-open-symbolic', tooltip_text: 'Open (Ctrl+O)'});
    open.connect('clicked', chooseFile);
    header.pack_start(open);

    const zoomOut = new Gtk.Button({icon_name: 'zoom-out-symbolic', tooltip_text: 'Zoom out (-)'});
    zoomOut.connect('clicked', () => setZoom(zoomStep(effectiveScale(), -1)));
    const zoomIn = new Gtk.Button({icon_name: 'zoom-in-symbolic', tooltip_text: 'Zoom in (+)'});
    zoomIn.connect('clicked', () => setZoom(zoomStep(effectiveScale(), +1)));
    const fit = new Gtk.Button({icon_name: 'zoom-fit-best-symbolic', tooltip_text: 'Fit (0)'});
    fit.connect('clicked', () => { state.fitMode = 'fill'; render(); });
    const rot = new Gtk.Button({icon_name: 'object-rotate-right-symbolic', tooltip_text: 'Rotate (R)'});
    rot.connect('clicked', () => { state.rotation = rotate(state.rotation, 90); render(); });
    zoomLabel = new Gtk.Label({label: '100%', width_chars: 5});
    pageLabel = new Gtk.Label({label: '', width_chars: 8});
    [zoomOut, zoomIn, fit, rot].forEach(b => header.pack_start(b));
    header.pack_end(pageLabel);
    header.pack_end(zoomLabel);

    // --- annotation tools (documents only) --------------------------
    const noteBtn = new Gtk.ToggleButton({label: 'Note', tooltip_text: 'Add a note (N)'});
    const hiBtn = new Gtk.ToggleButton({label: 'Highlight', tooltip_text: 'Highlight an area (H)'});
    const boxBtn = new Gtk.ToggleButton({label: 'Box', tooltip_text: 'Draw a box (B)'});
    const tools = [[noteBtn, 'note'], [hiBtn, 'highlight'], [boxBtn, 'box']];
    for (const [btn, name] of tools) {
        btn.connect('toggled', () => {
            if (btn.active) {
                for (const [other] of tools) if (other !== btn) other.active = false;
                state.tool = name;
                // Annotating a rotated view would need a third
                // coordinate space; resetting is honest and visible.
                if (state.rotation) { state.rotation = 0; render(); }
            } else if (state.tool === name) {
                state.tool = null;
            }
        });
    }
    const toolBox = new Gtk.Box({spacing: 4, css_classes: ['linked']});
    [noteBtn, hiBtn, boxBtn].forEach(b => toolBox.append(b));

    const pagesBtn = new Gtk.ToggleButton({icon_name: 'view-grid-symbolic', tooltip_text: 'Pages (P)'});
    const saveBtn = new Gtk.Button({icon_name: 'document-save-symbolic', tooltip_text: 'Save an edited copy (Ctrl+S)'});
    saveBtn.connect('clicked', saveEdited);
    const undoBtn = new Gtk.Button({icon_name: 'edit-undo-symbolic', tooltip_text: 'Undo annotation (Ctrl+Z)'});
    undoBtn.connect('clicked', undoAnnot);
    header.pack_end(saveBtn);
    header.pack_end(undoBtn);
    header.pack_end(pagesBtn);
    header.pack_end(toolBox);

    refreshEditUi = () => {
        const doc = state.kind === 'document';
        toolBox.visible = doc;
        pagesBtn.visible = doc;
        saveBtn.visible = isDirty();
        undoBtn.visible = state.annots.length > 0;
        if (!doc) { pagesBtn.active = false; state.tool = null; }
        for (const [btn, name] of tools) btn.active = doc && state.tool === name;
    };

    // can_shrink MUST be true: false means "never render smaller than
    // the content", so for content larger than the window every zoom
    // and fit written via set_size_request was silently ignored
    // downward — seen on the device as a 2048px-rasterized SVG pinned
    // at 100% while the label claimed 33%. With shrink allowed, the
    // size request is the single authority and CONTAIN scales into it.
    picture = new Gtk.Picture({
        can_shrink: true,
        content_fit: Gtk.ContentFit.CONTAIN,
        halign: Gtk.Align.CENTER, valign: Gtk.Align.CENTER,
    });
    drawing = new Gtk.DrawingArea({halign: Gtk.Align.CENTER, valign: Gtk.Align.CENTER});
    drawing.set_draw_func(drawPage);

    // --- gestures: click places a note, drag draws highlight/box ----
    const click = new Gtk.GestureClick({button: 1});
    click.connect('released', (_g, _n, x, y) => {
        if (state.tool !== 'note' || state.kind !== 'document') return;
        const pop = new Gtk.Popover();
        const row = new Gtk.Box({spacing: 6, margin_top: 6, margin_bottom: 6, margin_start: 6, margin_end: 6});
        const entry = new Gtk.Entry({placeholder_text: 'Note…', width_chars: 28});
        const add = new Gtk.Button({label: 'Add', css_classes: ['suggested-action']});
        row.append(entry); row.append(add);
        pop.set_child(row);
        pop.set_parent(drawing);
        const at = new Gdk.Rectangle({x: Math.round(x), y: Math.round(y), width: 1, height: 1});
        pop.set_pointing_to(at);
        const commit = () => {
            const text = entry.get_text().trim();
            pop.popdown();
            if (text) addNoteAt(x, y, text);
        };
        entry.connect('activate', commit);
        add.connect('clicked', commit);
        pop.connect('closed', () => GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            pop.unparent(); return GLib.SOURCE_REMOVE;
        }));
        pop.popup();
        entry.grab_focus();
    });
    drawing.add_controller(click);

    const drag = new Gtk.GestureDrag({button: 1});
    let dragStart = null;
    drag.connect('drag-begin', (_g, x, y) => {
        if (state.tool !== 'highlight' && state.tool !== 'box') { dragStart = null; return; }
        dragStart = {x, y};
    });
    drag.connect('drag-update', (_g, dx, dy) => {
        if (!dragStart) return;
        state.drag = normalizeRect(dragStart, {x: dragStart.x + dx, y: dragStart.y + dy});
        drawing.queue_draw();
    });
    drag.connect('drag-end', (_g, dx, dy) => {
        if (!dragStart) return;
        const end = {x: dragStart.x + dx, y: dragStart.y + dy};
        const rect = normalizeRect(dragStart, end);
        const wasClick = isClick(dragStart, end);
        state.drag = null;
        dragStart = null;
        if (!wasClick) addRectAnnot(rect, state.tool);
        else drawing.queue_draw();
    });
    drawing.add_controller(drag);

    stack = new Gtk.Stack();
    stack.add_named(picture, 'image');
    stack.add_named(drawing, 'document');

    // --- peek surfaces: text, html, and the generic card ------------
    textView = new Gtk.TextView({
        editable: false, monospace: true, cursor_visible: false,
        wrap_mode: Gtk.WrapMode.WORD_CHAR,
        left_margin: 16, right_margin: 16, top_margin: 12, bottom_margin: 12,
    });
    stack.add_named(textView, 'text');
    if (WebKit) {
        webView = new WebKit.WebView();
        // A peek, not a browser: no scripts, and the only navigation
        // ever allowed is the load we asked for — a link click in the
        // preview does nothing rather than quietly fetching the web.
        webView.get_settings().set_enable_javascript(false);
        webView.connect('decide-policy', (_v, decision, decisionType) => {
            if (decisionType === WebKit.PolicyDecisionType.NAVIGATION_ACTION ||
                decisionType === WebKit.PolicyDecisionType.NEW_WINDOW_ACTION) {
                const uri = decision.get_navigation_action().get_request().get_uri();
                if (decisionType === WebKit.PolicyDecisionType.NAVIGATION_ACTION &&
                    uri === state.htmlUri) {
                    decision.use();
                } else {
                    decision.ignore();
                }
                return true;
            }
            return false;
        });
        stack.add_named(webView, 'html');
    }
    card = new Adw.StatusPage();
    stack.add_named(card, 'card');

    // --- pages sidebar: thumbnails, reorder, remove -----------------
    const thumbList = new Gtk.ListBox({css_classes: ['navigation-sidebar']});
    thumbList.connect('row-activated', (_l, row) => {
        const i = row.get_index();
        if (i < 0 || i >= state.pageOrder.length) return;
        state.pageIndex = i;
        render();
    });
    rebuildThumbs = () => {
        let child;
        while ((child = thumbList.get_first_child()) !== null) thumbList.remove(child);
        // Rows are built only while the sidebar is shown — clearing is
        // what releases stale page refs; building 400 thumbnails behind
        // a closed sidebar is waste.
        if (state.kind !== 'document' || !pagesBtn.active) return;
        state.pageOrder.forEach((orig, i) => {
            const page = state.doc.get_page(orig);
            const [pw, ph] = page.get_size();
            const tScale = 110 / pw;
            const thumb = new Gtk.DrawingArea({
                content_width: 110, content_height: Math.round(ph * tScale),
                halign: Gtk.Align.CENTER,
            });
            thumb.set_draw_func((_a, cr, w, h) => {
                cr.setSourceRGB(1, 1, 1);
                cr.rectangle(0, 0, w, h);
                cr.fill();
                cr.scale(tScale, tScale);
                page.render(cr);
                cr.$dispose?.();
            });
            const up = new Gtk.Button({icon_name: 'go-up-symbolic', css_classes: ['flat'], tooltip_text: 'Move up'});
            const down = new Gtk.Button({icon_name: 'go-down-symbolic', css_classes: ['flat'], tooltip_text: 'Move down'});
            const del = new Gtk.Button({icon_name: 'user-trash-symbolic', css_classes: ['flat'], tooltip_text: 'Remove page'});
            up.connect('clicked', () => applyOrder(movePage(state.pageOrder, i, i - 1)));
            down.connect('clicked', () => applyOrder(movePage(state.pageOrder, i, i + 1)));
            del.connect('clicked', () => applyOrder(removePage(state.pageOrder, i)));
            up.sensitive = i > 0;
            down.sensitive = i < state.pageOrder.length - 1;
            const btnRow = new Gtk.Box({halign: Gtk.Align.CENTER});
            [up, down, del].forEach(b => btnRow.append(b));
            const cell = new Gtk.Box({
                orientation: Gtk.Orientation.VERTICAL, spacing: 2,
                margin_top: 6, margin_bottom: 6, margin_start: 6, margin_end: 6,
            });
            cell.append(thumb);
            cell.append(new Gtk.Label({label: `${i + 1}`, css_classes: ['dim-label', 'caption']}));
            cell.append(btnRow);
            thumbList.append(cell);
        });
    };
    const thumbScroller = new Gtk.ScrolledWindow({
        child: thumbList, width_request: 170,
        hscrollbar_policy: Gtk.PolicyType.NEVER,
    });
    const sidebar = new Gtk.Revealer({
        child: thumbScroller, reveal_child: false,
        transition_type: Gtk.RevealerTransitionType.SLIDE_RIGHT,
    });
    pagesBtn.connect('toggled', () => {
        if (pagesBtn.active) rebuildThumbs();
        sidebar.reveal_child = pagesBtn.active;
    });

    const scroller = new Gtk.ScrolledWindow({hexpand: true, vexpand: true, child: stack});
    const split = new Gtk.Box({orientation: Gtk.Orientation.HORIZONTAL});
    split.append(sidebar);
    split.append(scroller);
    const view = new Adw.ToolbarView({content: split});
    view.add_top_bar(header);
    toasts.set_child(view);
    win.set_content(toasts);
    refreshEditUi();

    const keys = new Gtk.EventControllerKey();
    keys.connect('key-pressed', (_c, keyval, _code, mods) => {
        const ctrl = (mods & Gdk.ModifierType.CONTROL_MASK) !== 0;
        switch (keyval) {
        case Gdk.KEY_plus: case Gdk.KEY_equal: setZoom(zoomStep(effectiveScale(), +1)); return true;
        case Gdk.KEY_minus: setZoom(zoomStep(effectiveScale(), -1)); return true;
        case Gdk.KEY_0: state.fitMode = 'fill'; render(); return true;
        case Gdk.KEY_1: setZoom(1); return true;
        case Gdk.KEY_w: if (ctrl) { win.close(); return true; } break;
        case Gdk.KEY_o: if (ctrl) { chooseFile(); return true; } break;
        case Gdk.KEY_s: if (ctrl) { saveEdited(); return true; } break;
        case Gdk.KEY_z: if (ctrl) { undoAnnot(); return true; } break;
        case Gdk.KEY_n: if (state.kind === 'document') { noteBtn.active = !noteBtn.active; return true; } break;
        case Gdk.KEY_h: if (state.kind === 'document') { hiBtn.active = !hiBtn.active; return true; } break;
        case Gdk.KEY_b: if (state.kind === 'document') { boxBtn.active = !boxBtn.active; return true; } break;
        case Gdk.KEY_p: if (state.kind === 'document') { pagesBtn.active = !pagesBtn.active; return true; } break;
        case Gdk.KEY_r: state.rotation = rotate(state.rotation, 90); render(); return true;
        case Gdk.KEY_space:
            // Quick Look manners, but only for windows Space opened:
            // Space toggles the peek closed again (Nautilus only gets
            // to send its close toggle while Files keeps focus, and it
            // does not — this window takes it). A file opened normally
            // keeps Space as page-forward, like any reader.
            if (state.quickLook) { win.close(); return true; }
            goPage(+1); return true;
        case Gdk.KEY_Escape:
            if (state.quickLook) { win.close(); return true; }
            break;
        case Gdk.KEY_Right: case Gdk.KEY_Page_Down: goPage(+1); return true;
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
    // ...and once the window first maps: the default-size fit above is
    // an estimate, and the real allocation lands only after present().
    win.connect('map', () => {
        GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            if (state.fitMode !== 'free') render();
            return GLib.SOURCE_REMOVE;
        });
    });
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
            // The DISPLAYED page — after a reorder, state.pageIndex is a
            // display position, and reading the same-numbered original
            // page silently hands the agent a different page than the
            // one on screen (#195).
            const page = docPage();
            const text = page.get_text() ?? '';
            const cap = 30000;
            return {
                ...base,
                page: state.pageIndex + 1, pages: state.pageOrder.length,
                text: text.slice(0, cap),
                // A truncation the model cannot see is a page it thinks
                // it has read (the lesson from Surfer's extract.js).
                truncated: text.length > cap,
            };
        }
        if (state.kind === 'text' || state.kind === 'html') {
            const cap = 30000;
            const text = state.textContent ?? '';
            return {
                ...base,
                text: text.slice(0, cap),
                truncated: text.length > cap,
                note: state.kind === 'html' ? 'html source, not rendered text' : undefined,
            };
        }
        if (state.kind === 'card') {
            return {...base, text: null,
                note: 'unrecognised type — name and location only'};
        }
        const {width, height} = contentSize();
        // No OCR and no vision model here. Saying so beats returning an
        // empty `text` field that reads as "this image contains nothing".
        return {...base, width, height, text: null,
            note: 'image metadata only — Preview does not OCR or caption'};
    },

    /// Write-tier: annotate the open document. Coordinates are TOP-DOWN
    /// page points, `page` is the 1-based DISPLAY page. Deliberately no
    /// save tool: annotations land in the window and the human decides
    /// with Ctrl+S whether they reach disk.
    async addNote({page, x, y, text}) {
        const idx = displayPageIndex(page);
        if (idx.error) return idx;
        if (typeof text !== 'string' || !text.trim())
            return {error: 'text is required'};
        const done = noteOnPage(state.doc.get_page(state.pageOrder[idx.value]),
            finiteOr(x, 0), finiteOr(y, 0), text.trim());
        return done ? {ok: true, page: idx.value + 1, unsaved: state.annots.length}
            : {error: 'annotation was not added'};
    },

    async highlight({page, x1, y1, x2, y2}) {
        const idx = displayPageIndex(page);
        if (idx.error) return idx;
        const rect = normalizeRect({x: finiteOr(x1, 0), y: finiteOr(y1, 0)},
            {x: finiteOr(x2, 0), y: finiteOr(y2, 0)});
        const done = rectOnPage(state.doc.get_page(state.pageOrder[idx.value]), rect, 'highlight');
        return done ? {ok: true, page: idx.value + 1, unsaved: state.annots.length}
            : {error: 'annotation was not added'};
    },
};

/// `page` from the wire -> a validated 0-based display index, or the
/// error to return. `1.5` and `"abc"` must land HERE, not inside
/// poppler as get_page(undefined) surfacing a marshalling error (#198).
function displayPageIndex(page) {
    if (state.kind !== 'document' || !state.doc)
        return {error: 'no document open'};
    const p = page === undefined ? 1 : page;
    if (!Number.isInteger(p) || p < 1 || p > state.pageOrder.length)
        return {error: `page must be an integer 1..${state.pageOrder.length}`};
    return {value: p - 1};
}

function finiteOr(n, fallback) {
    return Number.isFinite(n) ? n : fallback;
}

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
                if (loadFile(path)) {
                    // Space opened this window; Space closes it again.
                    state.quickLook = true;
                    win.present();
                }
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
app.connect('activate', () => {
    ensureUi();
    if (suppressPresent) { suppressPresent = false; return; }
    win.present();
    render();
});
app.connect('shutdown', () => {
    try { mcp?.stop(); } catch (e) { /* exiting */ }
    try { previewer?.stop(); } catch (e) { /* exiting */ }
});

app.run([imports.system.programInvocationName, ...argv]);
