#!/usr/bin/env -S gjs -m
// Browser — the web as an agent surface (ADR-0037, issue #146).
//
// GJS + GTK4 + libadwaita + WebKit-6.0, the same stack as
// shell/assistant. The engine is the webkitgtk-6.0 the image already
// ships; this file is chrome around it.
//
// Structure: Adw.TabView owns the per-tab WebViews (drag reordering,
// the overview, close buttons — GNOME's, not reimplemented). The pure
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

const HOME = 'https://duckduckgo.com';

const app = new Adw.Application({application_id: 'app.lisaos.Browser'});
let win = null;
let tabView = null;
let urlBar = null;
let mcp = null;

function currentView() {
    const page = tabView.get_selected_page();
    return page ? page.get_child() : null;
}

function newTab(url = HOME, focus = true) {
    const view = new WebKit.WebView();
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
    // Middle-click / target=_blank land beside their opener, focused per
    // GNOME convention.
    view.connect('create', () => {
        const v = newTab('about:blank', true);
        return v;
    });
    if (url) view.load_uri(url);
    if (focus) tabView.set_selected_page(page);
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
        title: 'Browser',
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
    urlBar.connect('activate', () => navigate(urlBar.get_text()));

    const newBtn = Gtk.Button.new_from_icon_name('tab-new-symbolic');
    newBtn.connect('clicked', () => newTab());

    const header = new Adw.HeaderBar({title_widget: urlBar});
    header.pack_start(back);
    header.pack_start(fwd);
    header.pack_start(reload);
    header.pack_end(newBtn);

    tabView = new Adw.TabView();
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
    const tabBar = new Adw.TabBar({view: tabView, autohide: true});

    const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL});
    const toolbar = new Adw.ToolbarView();
    toolbar.add_top_bar(header);
    toolbar.add_top_bar(tabBar);
    toolbar.set_content(tabView);
    box.append(toolbar);
    win.set_content(box);

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

    win.present();
}

app.connect('activate', () => {
    if (win) { win.present(); return; }
    buildWindow();
    newTab(ARGV[0] && resolveInput(ARGV[0]).kind === 'load' ? resolveInput(ARGV[0]).url : HOME);
    // The Agent Bus socket lives exactly as long as a window does
    // (mcp-bus defers socket activation, so presence == usability).
    mcp = new McpServer({readCurrentPage, readSelection, screenshotCurrent});
    mcp.start();
});
app.connect('shutdown', () => mcp?.stop());
app.run([]);
