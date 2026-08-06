// `LisaWindow` — one window shape for every Lisa surface
// (ADR-0056 step 3, #282).
//
// # The bug this is built to make impossible
//
// #282, in the owner's words: *"close button is on the app title bar
// same place always, like if you compare main and Surfer it's strangely
// different, dark/light acts strange."* Confirmed in a screenshot on
// 2026-08-06: Surfer had a bare `×` floating over its content with no
// header bar, while Settings behind it had an ordinary one.
//
// That is not eight bugs, it is one missing abstraction. Every surface
// built its own `Adw.ApplicationWindow` + `Adw.HeaderBar` +
// `Adw.ToolbarView` by hand, so "where do the window controls go" was a
// convention eight files each re-implemented, and a convention is
// something you can only half-follow.
//
// # What it does NOT do
//
// It does not theme, restyle or wrap widgets. Content is whatever the
// app puts in it. libadwaita stays the toolkit (rule 11) — this is a
// dialect, not a replacement, so a window built here is an ordinary
// `Adw.ApplicationWindow` an app can reach and use directly.

import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import {installStyle} from './style.js';

/// Build an application window with Lisa's standard chrome.
///
///   app          — the Adw.Application
///   title        — window title
///   width/height — defaults, in logical pixels
///   content      — the widget below the header bar
///
/// Returns `{window, header, view}`: the header is there so an app can
/// `pack_start`/`pack_end` its own controls, which is the ONLY part of
/// the chrome an app should be deciding.
export function lisaWindow({app, title, width = 900, height = 640, content = null}) {
    if (!app)
        throw new Error('lisa_ui/window: an Adw.Application is required');
    if (typeof title !== 'string' || !title)
        throw new Error('lisa_ui/window: a window needs a title');

    const window = new Adw.ApplicationWindow({
        application: app,
        title,
        default_width: width,
        default_height: height,
    });

    // The header bar is ALWAYS present and always real. Surfer's bare
    // `×` came from drawing a close affordance over content instead of
    // having a header at all; with one here, "the close button moved"
    // stops being reachable — Adwaita places the controls, and it puts
    // them in the same place in every window on the system, including
    // GNOME's own.
    const header = new Adw.HeaderBar();

    // ToolbarView rather than set_titlebar: it is what handles a header
    // over content correctly under Adwaita 1.4+, including the rounded
    // corners and the shadow that make a window look like it belongs to
    // this desktop rather than to 2014.
    installStyle(window.get_display());
    const view = new Adw.ToolbarView();
    view.add_top_bar(header);
    if (content)
        view.content = content;
    window.content = view;

    return {window, header, view};
}

/// A two-pane window: a list on the left, the selected thing on the
/// right. Mail, Notes and a Files browser are all this shape.
///
/// Extracted when a second caller needed it, not invented up front —
/// ADR-0056 step 4. Notes is that caller: a list-and-editor app cannot
/// use `lisaWindow` because a split view has TWO header bars, one per
/// pane, and a single-header primitive would have forced Notes to build
/// its own chrome. That is exactly how #282 happened the first time.
///
/// Adwaita collapses this to one pane on a narrow window by itself, so
/// the app gets small-screen behaviour without asking.
///
/// Returns the two headers as well as the pages, because what an app
/// puts IN a header is the only part of the chrome it should decide.
/// Glass by DEFAULT. A Lisa app with a sidebar gets a see-through
/// pane flush to the window's left edge, and an app opts OUT with
/// `overlay: false` rather than opting in.
///
/// That direction is the point of a UI library. When it was opt-in,
/// looking like the rest of the system was a thing each app had to
/// remember, and #282 is the record of what happens then: eight
/// surfaces, three answers to where the window controls go, two answers
/// to what a sidebar is.
///
/// What the glass is made of, in two halves:
///
///   TRANSPARENCY comes from here — the window paints nothing where the
///   pane is, so the desktop shows through. Works on any GNOME.
///   FROST comes from the compositor — shell/glass clones the wallpaper,
///   blurs it, and slips it under the window. Without that extension an
///   app is transparent but sharp, which is a fair degradation and not
///   a broken window. Apps cannot do this half themselves: mutter#3023.
export function lisaSplitWindow({
    app, title, width = 900, height = 640,
    sidebarTitle = title, sidebarWidth = 300, overlay = true,
}) {
    if (!app)
        throw new Error('lisa_ui/window: an Adw.Application is required');
    if (typeof title !== 'string' || !title)
        throw new Error('lisa_ui/window: a window needs a title');

    const window = new Adw.ApplicationWindow({
        application: app,
        title,
        default_width: width,
        default_height: height,
    });

    const sidebarHeader = new Adw.HeaderBar();
    const sidebarView = new Adw.ToolbarView();
    sidebarView.add_top_bar(sidebarHeader);

    const contentHeader = new Adw.HeaderBar();
    const contentView = new Adw.ToolbarView();
    contentView.add_top_bar(contentHeader);

    // The NavigationPages are built ONLY in the split branch. Wrapping
    // the views in pages and then also adding those views to a
    // Gtk.Overlay gives each one two parents, which GTK refuses — the
    // first version of overlay mode died at startup with no message at
    // all, because a widget with two parents is not a runtime warning,
    // it is the end of the process.

    // The Lisa sheet, installed by the primitive rather than by each
    // app: an app should not have to opt in to looking like the system
    // it is part of. Idempotent, so several windows cost one provider.
    installStyle(window.get_display());
    sidebarView.add_css_class('lisa-glass');
    sidebarView.add_css_class('lisa-glass-edge-end');

    let split;
    if (overlay) {
        // Content fills the window; the sidebar floats over its left
        // edge, so the blur has something to work on.
        split = new Gtk.Overlay();

        // The content is INSET past the pane. Without this the editor
        // sits underneath the sidebar and the app looks like two
        // windows stacked — which is exactly what the first attempt
        // looked like.
        // Flush to the frame, not floating inside it. A pane inset from
        // the window edge leaves a strip of wallpaper down the left of
        // the app, which reads as a gap rather than as a layer — and on
        // a rounded window the toolkit already clips the corners, so
        // the pane follows the frame for free.
        const gap = 0;
        // Content is INSET past the pane and keeps its own opaque
        // ground. That leaves the region behind the pane painting
        // nothing — which, with a see-through window, is the DESKTOP.
        //
        // The trade is explicit and worth stating: content beside the
        // pane means there is nothing of the app to blur through it, so
        // this buys transparency and gives up frost. Frost over the
        // desktop is Mutter#3023 and is not ours to fix.
        contentView.set_margin_start(sidebarWidth + gap);
        contentView.add_css_class('lisa-ground');
        split.set_child(contentView);

        // ONE set of window controls. Both header bars drew their own,
        // so a floating sidebar produced two close buttons in one
        // window — the precise complaint #282 opened with, reintroduced
        // by the fix for it.
        sidebarHeader.set_show_start_title_buttons(false);
        sidebarHeader.set_show_end_title_buttons(false);
        // ...and on a buttons-at-start layout the content header would
        // draw them at x=sidebarWidth, mid-window, because the floating
        // pane is invisible to Adwaita's placement (#320). No change on
        // the default layout, where the start side is empty.
        contentHeader.set_show_start_title_buttons(false);

        // A floating panel, not a full-height slab: margins on every
        // side and a radius, so it reads as a layer above the content
        // rather than a region of the window.
        sidebarView.set_halign(Gtk.Align.START);
        sidebarView.set_size_request(sidebarWidth, -1);
        sidebarView.add_css_class('lisa-glass-edge-end');
        // The window itself is see-through where nothing paints. Mutter
        // cannot BLUR what is behind a window (#3023, open), but plain
        // alpha compositing has always worked — so the desktop shows
        // through the pane even though it is not frosted.
        window.add_css_class('lisa-see-through');
        split.add_overlay(sidebarView);
        window.content = split;
    } else {
        split = new Adw.NavigationSplitView({
            sidebar: new Adw.NavigationPage({title: sidebarTitle, child: sidebarView}),
            content: new Adw.NavigationPage({title, child: contentView}),
            min_sidebar_width: 220,
            sidebar_width_fraction: 0.34,
            max_sidebar_width: sidebarWidth,
        });
        window.content = split;
    }

    return {
        window, split, sidebarHeader, contentHeader,
        setSidebar: (w) => { sidebarView.content = w; },
        setContent: (w) => { contentView.content = w; },
        /// On a collapsed (narrow) window, bring the content pane
        /// forward — what selecting a row should do on a phone-width
        /// window and a no-op on a wide one.
        showContent: () => { if (split.set_show_content) split.set_show_content(true); },
    };
}

/// A three-pane window: a narrow sidebar, a list, and the selected
/// thing. Mail is this shape — rail+folders | messages | reader — and
/// Mail is where this was EXTRACTED from, per ADR-0056 step 4: its
/// measurements below are Mail's shipped ones, not invented defaults.
///
/// The glass rules are lisaSplitWindow's, applied to the leftmost pane
/// only: the sidebar is see-through and flush to the frame, the list
/// and content keep their own opaque ground. One window shape, one
/// place for the window controls: the content header carries them.
/// Adwaita only suppresses the list header's END buttons (as the inner
/// split's sidebar) — its START side, and both sides of the floating
/// glass pane, are suppressed explicitly here, because a floating
/// Gtk.Overlay child is invisible to Adwaita's placement and a
/// buttons-at-start layout would otherwise draw window controls in the
/// middle of the window (#320, the exact shape of #282).
export function lisaTripleWindow({
    app, title, width = 1280, height = 820,
    sidebarWidth = 280, overlay = true,
}) {
    if (!app)
        throw new Error('lisa_ui/window: an Adw.Application is required');
    if (typeof title !== 'string' || !title)
        throw new Error('lisa_ui/window: a window needs a title');

    const window = new Adw.ApplicationWindow({
        application: app,
        title,
        default_width: width,
        default_height: height,
    });

    const sidebarHeader = new Adw.HeaderBar();
    const sidebarView = new Adw.ToolbarView();
    sidebarView.add_top_bar(sidebarHeader);

    const listHeader = new Adw.HeaderBar();
    const listView = new Adw.ToolbarView();
    listView.add_top_bar(listHeader);

    const contentHeader = new Adw.HeaderBar();
    const contentView = new Adw.ToolbarView();
    contentView.add_top_bar(contentHeader);

    installStyle(window.get_display());
    sidebarView.add_css_class('lisa-glass');
    sidebarView.add_css_class('lisa-glass-edge-end');

    // The inner split is the same in both modes: list | content, and
    // Adwaita manages which of their headers shows window controls.
    const inner = new Adw.OverlaySplitView({
        sidebar: listView, content: contentView,
        min_sidebar_width: 320, max_sidebar_width: 460,
        sidebar_width_fraction: 0.34,
    });

    let outer;
    if (overlay) {
        // Same construction as lisaSplitWindow's overlay branch: the
        // inner split fills the window inset past the pane, and the
        // glass sidebar floats flush to the frame's left edge.
        outer = new Gtk.Overlay();
        inner.set_margin_start(sidebarWidth);
        listView.add_css_class('lisa-ground');
        contentView.add_css_class('lisa-ground');
        outer.set_child(inner);

        sidebarHeader.set_show_start_title_buttons(false);
        sidebarHeader.set_show_end_title_buttons(false);
        // The list header is the leftmost pane Adwaita can SEE, so on a
        // buttons-at-start layout it would grow window controls at
        // x=280 while the true left edge (the floating pane) has none.
        // Pixel-identical on the default layout, where start is empty.
        listHeader.set_show_start_title_buttons(false);
        sidebarView.set_halign(Gtk.Align.START);
        sidebarView.set_size_request(sidebarWidth, -1);
        window.add_css_class('lisa-see-through');
        outer.add_overlay(sidebarView);
        window.content = outer;
    } else {
        outer = new Adw.OverlaySplitView({
            sidebar: sidebarView, content: inner,
            min_sidebar_width: 200, max_sidebar_width: sidebarWidth,
            sidebar_width_fraction: 0.18,
        });
        window.content = outer;
    }

    // The panes are returned as the ordinary Adw.ToolbarViews they are:
    // Mail pins a sync banner above its list and a "last synced" line
    // below it, and add_top_bar/add_bottom_bar on the pane IS the
    // toolkit's API for that (rule 11 — a dialect, not a wrapper).
    return {
        window, outer, inner, sidebarHeader, listHeader, contentHeader,
        sidebarPane: sidebarView, listPane: listView, contentPane: contentView,
        setSidebar: (w) => { sidebarView.content = w; },
        setList: (w) => { listView.content = w; },
        setContent: (w) => { contentView.content = w; },
    };
}

/// A window control an app adds itself, with the tooltip that makes it
/// legible. Sugar, but the kind that stops six apps spelling the same
/// three lines six ways.
export function headerButton({icon, tooltip, onClick}) {
    const b = Gtk.Button.new_from_icon_name(icon);
    if (tooltip)
        b.tooltip_text = tooltip;
    if (onClick)
        b.connect('clicked', onClick);
    return b;
}
