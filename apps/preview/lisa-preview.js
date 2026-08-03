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
import {looksBinary, truncateText, cardSubtitle, folderSubtitle, mediaClock} from './lib/peek.js';
import {exportFormats, saveOptions, exportName, rasterScale, pageExportNames} from './lib/export.js';
import {normalizeStrokes, stampSize, serializeSignature, deserializeSignature} from './lib/signature.js';
import {McpServer} from './lib/mcp.js';
import {Previewer} from './lib/previewer.js';

const Cairo = imports.cairo;

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
    // The Space gesture runs as its OWN app id (app.lisaos.PreviewPeek,
    // a NoDisplay .desktop): the shell can then treat a peek as a
    // transient — no dock presence, filtered by the Lisa desktop — and
    // a peek never unifies with a real Preview instance, exactly the
    // macOS Quick-Look-panel vs Preview-app split.
    application_id: serviceLaunch ? 'app.lisaos.PreviewPeek' : 'app.lisaos.Preview',
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
let video = null, audioPage = null, audioControls = null;

// No "is there a media backend" pre-check, deliberately: on Arch the
// GStreamer backend is linked INTO libgtk-4 itself (no module dir
// exists — a file-probe for one wrongly declared media unsupported on
// the reference device). If a platform truly lacks a backend,
// Gtk.MediaFile reports it through notify::error and the toast says
// what it said.
function stopMedia() {
    try { state.media?.set_playing(false); } catch (e) { /* gone */ }
    state.media = null;
}

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
    } else if (state.kind === 'video') {
        const vp = viewportSize();
        video.set_size_request(vp.width, vp.height);
        stack.set_visible_child_name('video');
    } else if (state.kind === 'audio') {
        const vp = viewportSize();
        audioPage.set_size_request(vp.width, vp.height);
        stack.set_visible_child_name('audio');
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
    const isDir = GLib.file_test(path, GLib.FileTest.IS_DIR);
    let kind = isDir ? 'card' : (kindOf(path) ?? 'card');
    state.textContent = null;
    stopMedia();
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
            card.set_title(info.get_display_name());
            if (isDir) {
                // Folder cards count their children, capped so a
                // 100k-entry directory cannot hang a peek (#200).
                let count = 0, bytes = 0, capped = false;
                const en = file.enumerate_children('standard::size',
                    Gio.FileQueryInfoFlags.NONE, null);
                let child;
                while ((child = en.next_file(null)) !== null) {
                    count++;
                    bytes += child.get_size();
                    if (count >= 1000) { capped = true; break; }
                }
                card.set_description(folderSubtitle(count, capped, bytes));
            } else {
                const ctype = info.get_content_type();
                card.set_description(cardSubtitle(
                    ctype ? Gio.content_type_get_description(ctype) : '',
                    info.get_size()));
            }
            const gicon = info.get_icon();
            const display = Gdk.Display.get_default();
            if (gicon && display) {
                card.paintable = Gtk.IconTheme.get_for_display(display)
                    .lookup_by_gicon(gicon, 128, 1, Gtk.TextDirection.NONE, 0);
            } else {
                card.icon_name = isDir ? 'folder-symbolic' : 'text-x-generic-symbolic';
            }
        }
        if (kind === 'audio' || kind === 'video') {
            state.media = Gtk.MediaFile.new_for_filename(path);
            state.media.connect('notify::error', () => {
                const err = state.media?.get_error();
                if (err) toast(`Cannot play ${path.split('/').pop()}: ${err.message}`);
            });
            if (kind === 'video') {
                video.set_media_stream(state.media);
            } else {
                audioPage.set_title(path.split('/').pop());
                audioControls.set_media_stream(state.media);
            }
            // Autoplay IS the Space gesture — Quick Look plays.
            state.media.set_playing(true);
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
            // Exports use the pristine pixels — the checkerboard below
            // is a VIEW aid and must never reach a saved file.
            state.origPixbuf = state.pixbuf;
            // Transparency gets a checkerboard baked under it — a
            // black symbolic icon over the dark theme was invisible on
            // the device. composite_color_simple is pixbuf's own
            // checkerboard renderer; the checks zoom with the image,
            // which is what says "this is transparency", not texture.
            if (state.pixbuf.get_has_alpha()) {
                state.pixbuf = state.pixbuf.composite_color_simple(
                    state.pixbuf.get_width(), state.pixbuf.get_height(),
                    GdkPixbuf.InterpType.NEAREST, 255, 16, 0x3d3d3d, 0x4d4d4d);
            }
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
    // ← → keys, not the file the user actually asked for. Capped (#206)
    // for the same reason the folder card caps its count — a peek at a
    // file INSIDE a 100k-entry directory must not hang on enumeration;
    // past the cap, browsing degrades to the single file (a PARTIAL
    // sibling list would make ← → skip files invisibly, which is worse
    // than not browsing).
    try {
        const dir = Gio.File.new_for_path(path).get_parent();
        const en = dir.enumerate_children('standard::name', Gio.FileQueryInfoFlags.NONE, null);
        const names = [];
        let info;
        let capped = false;
        while ((info = en.next_file(null)) !== null) {
            names.push(info.get_name());
            if (names.length >= 5000) { capped = true; break; }
        }
        if (capped) {
            state.files = [path];
            state.fileIndex = 0;
        } else {
            const sib = siblings(path, names);
            state.files = sib.files;
            state.fileIndex = sib.index;
        }
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

// --- slice 5: export/convert + signatures --------------------------

/// A display page rendered to a pixbuf at the export dpi. Goes through
/// a temp PNG because that is the one lossless path cairo and pixbuf
/// share; the temp is O_EXCL and unlinked immediately.
function pageToPixbuf(page, dpi) {
    const [pw, phh] = page.get_size();
    const scale = rasterScale(dpi);
    const w = Math.ceil(pw * scale), h = Math.ceil(phh * scale);
    const surface = new Cairo.ImageSurface(Cairo.Format.ARGB32, w, h);
    const cr = new Cairo.Context(surface);
    cr.setSourceRGB(1, 1, 1);
    cr.paint();
    cr.scale(scale, scale);
    page.render(cr);
    cr.$dispose?.();
    const [fd, tmp] = GLib.file_open_tmp('lisa-preview-export-XXXXXX.png');
    GLib.close(fd);
    try {
        surface.writeToPNG(tmp);
        return GdkPixbuf.Pixbuf.new_from_file(tmp);
    } finally {
        // Unreachable-on-throw cleanup is a leak (#203).
        GLib.unlink(tmp);
    }
}

function savePixbuf(pb, path, formatKey) {
    const [keys, values] = saveOptions(formatKey);
    return pb.savev(path, formatKey, keys, values);
}

/// Export the current view. Images convert from the PRISTINE pixbuf
/// (never the checkerboarded view copy); documents rasterize the
/// displayed page, or every page into a chosen folder.
/// A dialog dismissal is silence; anything else is an error the user
/// must SEE (#202) — the old catch-all swallowed real save failures as
/// dismissals.
function dialogDismissed(e) {
    try {
        if (e instanceof GLib.Error &&
            e.matches(Gtk.dialog_error_quark(), Gtk.DialogError.DISMISSED))
            return true;
    } catch (err) { /* fall through to the string check */ }
    return /dismiss/i.test(String(e?.message ?? ''));
}

function runExport(formatKey, ext, allPages) {
    if (state.kind === 'image' && state.origPixbuf) {
        const dialog = new Gtk.FileDialog({initial_name: exportName(state.path, ext)});
        dialog.save(win, null, (src, res) => {
            try {
                const file = src.save_finish(res);
                if (!file) return;
                if (!savePixbuf(state.origPixbuf, file.get_path(), formatKey))
                    throw new Error(`the ${formatKey} writer refused the file`);
                toast(`Exported ${file.get_basename()}`);
            } catch (e) {
                if (!dialogDismissed(e)) toast(`Export failed: ${e.message}`);
            }
        });
        return;
    }
    if (state.kind !== 'document' || !state.doc) return;
    if (!allPages) {
        const page = state.pageOrder[state.pageIndex] + 1;
        const dialog = new Gtk.FileDialog({initial_name: exportName(state.path, ext, page)});
        dialog.save(win, null, (src, res) => {
            try {
                const file = src.save_finish(res);
                if (!file) return;
                if (!savePixbuf(pageToPixbuf(docPage(), 150), file.get_path(), formatKey))
                    throw new Error(`the ${formatKey} writer refused the file`);
                toast(`Exported ${file.get_basename()}`);
            } catch (e) {
                if (!dialogDismissed(e)) toast(`Export failed: ${e.message}`);
            }
        });
        return;
    }
    const dialog = new Gtk.FileDialog();
    dialog.select_folder(win, null, (src, res) => {
        let dir;
        try {
            dir = src.select_folder_finish(res);
        } catch (e) {
            if (!dialogDismissed(e)) toast(`Export failed: ${e.message}`);
            return;
        }
        if (!dir) return;
        // One page per main-loop tick (#204): a 90-page export must not
        // freeze the window, and each pixbuf dies before the next is
        // born. Existing files are skipped and counted, never silently
        // overwritten (#202) — the folder was picked, not each name.
        const names = pageExportNames(state.path, ext, state.pageOrder.length);
        const order = state.pageOrder.slice();
        const doc = state.doc;
        let i = 0, done = 0, skipped = 0, failed = 0;
        GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            if (i >= order.length) {
                const parts = [`Exported ${done} page${done === 1 ? '' : 's'}`];
                if (skipped) parts.push(`${skipped} existed — skipped`);
                if (failed) parts.push(`${failed} FAILED`);
                toast(`${parts.join(' · ')} (${dir.get_basename()})`);
                return GLib.SOURCE_REMOVE;
            }
            const target = `${dir.get_path()}/${names[i]}`;
            try {
                if (GLib.file_test(target, GLib.FileTest.EXISTS)) {
                    skipped++;
                } else if (savePixbuf(pageToPixbuf(doc.get_page(order[i]), 150),
                    target, formatKey)) {
                    done++;
                } else {
                    failed++;
                }
            } catch (e) {
                failed++;
                logError(e, `export page ${i + 1}`);
            }
            i++;
            return GLib.SOURCE_CONTINUE;
        });
    });
}

// --- signature: one stored scrawl, stamped where the user clicks ----

function signaturePath() {
    return `${GLib.get_user_data_dir()}/lisa/preview/signature.json`;
}

function loadSignature() {
    if (state.signature !== undefined) return state.signature;
    try {
        const [okRead, bytes] = GLib.file_get_contents(signaturePath());
        state.signature = okRead
            ? deserializeSignature(new TextDecoder().decode(bytes)) : null;
    } catch (e) {
        state.signature = null;
    }
    return state.signature;
}

function storeSignature(sig) {
    GLib.mkdir_with_parents(GLib.path_get_dirname(signaturePath()), 0o700);
    GLib.file_set_contents(signaturePath(), serializeSignature(sig));
    state.signature = sig;
}

/// The stored strokes rendered to an alpha surface at 3× the stamp
/// size, in ink blue-black — crisp through PDF zoom.
function signatureSurface(sig, widthPt, heightPt) {
    const scale = 3;
    const w = Math.ceil(widthPt * scale), h = Math.ceil(heightPt * scale);
    const surface = new Cairo.ImageSurface(Cairo.Format.ARGB32, w, h);
    const cr = new Cairo.Context(surface);
    const sx = w / sig.width, sy = h / sig.height;
    const s = Math.min(sx, sy);
    cr.setSourceRGB(0.08, 0.09, 0.35);
    cr.setLineWidth(2.5 * s);
    cr.setLineCap(Cairo.LineCap.ROUND);
    cr.setLineJoin(Cairo.LineJoin.ROUND);
    for (const stroke of sig.strokes) {
        stroke.forEach((p, i) => {
            if (i === 0) cr.moveTo(p.x * s, p.y * s);
            else cr.lineTo(p.x * s, p.y * s);
        });
        if (stroke.length === 1)
            cr.lineTo(stroke[0].x * s + 0.1, stroke[0].y * s);
        cr.stroke();
    }
    cr.$dispose?.();
    return surface;
}

function placeSignature(vx, vy) {
    const sig = loadSignature();
    const page = docPage();
    if (!sig || !page) return false;
    const [pww, phh] = page.get_size();
    const p = viewToPage({x: vx, y: vy}, effectiveScale());
    const size = stampSize(sig);
    // Clamp into the page (#205): signatures go in bottom-right
    // corners, and a stamp past the MediaBox is half-clipped in every
    // other reader the PDF ever meets.
    const x = Math.max(0, Math.min(p.x, pww - size.width));
    const y = Math.max(0, Math.min(p.y, phh - size.height));
    const r = annotRect(
        {x1: x, y1: y, x2: x + size.width, y2: y + size.height}, phh);
    const annot = Poppler.AnnotStamp.new(state.doc, popplerRect(r));
    annot.set_custom_image(signatureSurface(sig, size.width, size.height));
    page.add_annot(annot);
    state.annots.push({page, annot});
    refreshEditUi?.();
    render();
    return true;
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
    // Promote a peek into the real app, macOS-style: the button hands
    // the file to app.lisaos.Preview (a separate app id, so it gets
    // its own dock presence) and the peek closes behind it.
    const openWith = new Gtk.Button({label: 'Open with Preview', css_classes: ['suggested-action']});
    openWith.connect('clicked', () => {
        try {
            const info = Gio.DesktopAppInfo.new('app.lisaos.Preview.desktop');
            if (info && state.path)
                info.launch([Gio.File.new_for_path(state.path)], null);
        } catch (e) {
            logError(e, 'lisa-preview open-with');
        }
        win.close();
    });
    // --- export (slice 5): formats the MACHINE can write ------------
    const available = exportFormats(
        GdkPixbuf.Pixbuf.get_formats().filter(f => f.is_writable()).map(f => f.get_name()));
    const exportBtn = new Gtk.MenuButton({
        icon_name: 'document-save-as-symbolic', tooltip_text: 'Export (Ctrl+E)',
    });
    const exPop = new Gtk.Popover();
    const exBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL, spacing: 8,
        margin_top: 10, margin_bottom: 10, margin_start: 10, margin_end: 10,
    });
    const formatDrop = new Gtk.DropDown({
        model: Gtk.StringList.new(available.map(f => f.label)),
    });
    const allPagesCheck = new Gtk.CheckButton({label: 'All pages'});
    const exGo = new Gtk.Button({label: 'Export…', css_classes: ['suggested-action']});
    exGo.connect('clicked', () => {
        exPop.popdown();
        const f = available[formatDrop.get_selected()];
        if (f) runExport(f.key, f.ext, allPagesCheck.active);
    });
    exBox.append(formatDrop);
    exBox.append(allPagesCheck);
    exBox.append(exGo);
    exPop.set_child(exBox);
    exportBtn.set_popover(exPop);

    // --- signature (slice 5): draw once, stamp anywhere -------------
    const signBtn = new Gtk.MenuButton({label: 'Sign'});
    const signPop = new Gtk.Popover();
    const signBox = new Gtk.Box({
        orientation: Gtk.Orientation.VERTICAL, spacing: 6,
        margin_top: 10, margin_bottom: 10, margin_start: 10, margin_end: 10,
    });
    const placeBtn = new Gtk.Button({label: 'Place signature', css_classes: ['suggested-action']});
    placeBtn.connect('clicked', () => {
        signPop.popdown();
        if (!loadSignature()) { openSignatureDialog(); return; }
        state.tool = 'sign';
        if (state.rotation) { state.rotation = 0; render(); }
        refreshEditUi();
        toast('Click the page to place your signature');
    });
    const drawBtn = new Gtk.Button({label: 'Draw new signature…'});
    drawBtn.connect('clicked', () => { signPop.popdown(); openSignatureDialog(); });
    signBox.append(placeBtn);
    signBox.append(drawBtn);
    signPop.set_child(signBox);
    signBtn.set_popover(signPop);

    function openSignatureDialog() {
        const dlg = new Adw.Dialog({title: 'Draw your signature', content_width: 540});
        const tb = new Adw.ToolbarView();
        tb.add_top_bar(new Adw.HeaderBar());
        const strokes = [];
        let current = null;
        const pad = new Gtk.DrawingArea({
            content_width: 500, content_height: 200,
            margin_top: 8, margin_bottom: 8, margin_start: 20, margin_end: 20,
        });
        pad.set_draw_func((_a, cr, w, h) => {
            cr.setSourceRGB(1, 1, 1);
            cr.rectangle(0, 0, w, h);
            cr.fill();
            cr.setSourceRGB(0.08, 0.09, 0.35);
            cr.setLineWidth(2.5);
            cr.setLineCap(Cairo.LineCap.ROUND);
            cr.setLineJoin(Cairo.LineJoin.ROUND);
            for (const s of [...strokes, current].filter(Boolean)) {
                s.forEach((p, i) => i === 0 ? cr.moveTo(p.x, p.y) : cr.lineTo(p.x, p.y));
                if (s.length === 1) cr.lineTo(s[0].x + 0.1, s[0].y);
                cr.stroke();
            }
            cr.$dispose?.();
        });
        const drag = new Gtk.GestureDrag({button: 1});
        let start = null;
        drag.connect('drag-begin', (_g, x, y) => {
            start = {x, y};
            current = [{x, y}];
            pad.queue_draw();
        });
        drag.connect('drag-update', (_g, dx, dy) => {
            if (!current) return;
            current.push({x: start.x + dx, y: start.y + dy});
            pad.queue_draw();
        });
        drag.connect('drag-end', () => {
            if (current) strokes.push(current);
            current = null;
            pad.queue_draw();
        });
        pad.add_controller(drag);
        const row = new Gtk.Box({
            spacing: 8, halign: Gtk.Align.END,
            margin_bottom: 12, margin_end: 20,
        });
        const clear = new Gtk.Button({label: 'Clear'});
        clear.connect('clicked', () => { strokes.length = 0; current = null; pad.queue_draw(); });
        const save = new Gtk.Button({label: 'Save', css_classes: ['suggested-action']});
        save.connect('clicked', () => {
            const sig = normalizeStrokes(strokes);
            if (!sig) { toast('Nothing drawn yet'); return; }
            storeSignature(sig);
            dlg.close();
            toast('Signature saved — Sign › Place signature to use it');
        });
        row.append(clear);
        row.append(save);
        const col = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
        col.append(pad);
        col.append(row);
        tb.set_content(col);
        dlg.set_child(tb);
        dlg.present(win);
    }

    header.pack_end(openWith);
    header.pack_end(saveBtn);
    header.pack_end(undoBtn);
    header.pack_end(exportBtn);
    header.pack_end(signBtn);
    header.pack_end(pagesBtn);
    header.pack_end(toolBox);

    refreshEditUi = () => {
        const doc = state.kind === 'document';
        toolBox.visible = doc;
        pagesBtn.visible = doc;
        signBtn.visible = doc;
        exportBtn.visible = doc || state.kind === 'image';
        allPagesCheck.visible = doc;
        saveBtn.visible = isDirty();
        undoBtn.visible = state.annots.length > 0;
        openWith.visible = !!state.quickLook;
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
        if (state.kind !== 'document') return;
        if (state.tool === 'sign') {
            if (placeSignature(x, y)) state.tool = null;
            refreshEditUi();
            return;
        }
        if (state.tool !== 'note') return;
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
    // Media (#200): Gtk.Video for pictures-with-time, a StatusPage with
    // bare controls for audio — both driven by one Gtk.MediaFile.
    video = new Gtk.Video({autoplay: true});
    stack.add_named(video, 'video');
    audioControls = new Gtk.MediaControls({halign: Gtk.Align.CENTER, width_request: 360});
    audioPage = new Adw.StatusPage({icon_name: 'audio-x-generic-symbolic'});
    audioPage.set_child(audioControls);
    stack.add_named(audioPage, 'audio');
    win.connect('close-request', () => { stopMedia(); return false; });

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
        case Gdk.KEY_e: if (ctrl && exportBtn.visible) { exportBtn.popup(); return true; } break;
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
            // keeps Space as page-forward — except media, where Space
            // is play/pause like every player ever.
            if (state.quickLook) { win.close(); return true; }
            if ((state.kind === 'audio' || state.kind === 'video') && state.media) {
                state.media.set_playing(!state.media.get_playing());
                return true;
            }
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
                note: 'unrecognised type or folder — name and location only'};
        }
        if (state.kind === 'audio' || state.kind === 'video') {
            const clock = mediaClock(state.media?.get_duration() ?? 0);
            return {...base, text: null, duration: clock || null,
                note: 'media metadata only — Preview does not transcribe'};
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
                    refreshEditUi?.();
                    // Size the panel to the content, Quick-Look-style:
                    // a portrait PDF gets a portrait window, capped to
                    // most of the monitor.
                    const {width: cw, height: ch} = contentSize();
                    if (cw > 0 && ch > 0) {
                        const mon = Gdk.Display.get_default()
                            ?.get_monitors()?.get_item(0);
                        const geo = mon?.get_geometry() ?? {width: 1600, height: 1000};
                        const s = Math.min((geo.width * 0.9 - 24) / cw,
                            (geo.height * 0.85 - 96) / ch);
                        win.set_default_size(
                            Math.round(cw * s) + 24, Math.round(ch * s) + 96);
                    }
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
