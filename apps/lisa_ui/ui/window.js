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
export function lisaSplitWindow({
    app, title, width = 900, height = 640,
    sidebarTitle = title, sidebarWidth = 300,
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
    const sidebarPage = new Adw.NavigationPage({
        title: sidebarTitle, child: sidebarView,
    });

    const contentHeader = new Adw.HeaderBar();
    const contentView = new Adw.ToolbarView();
    contentView.add_top_bar(contentHeader);
    const contentPage = new Adw.NavigationPage({
        title, child: contentView,
    });

    // The Lisa sheet, installed by the primitive rather than by each
    // app: an app should not have to opt in to looking like the system
    // it is part of. Idempotent, so several windows cost one provider.
    installStyle(window.get_display());
    sidebarView.add_css_class('lisa-glass');
    sidebarView.add_css_class('lisa-glass-edge-end');

    const split = new Adw.NavigationSplitView({
        sidebar: sidebarPage,
        content: contentPage,
        min_sidebar_width: 220,
        sidebar_width_fraction: 0.34,
        max_sidebar_width: sidebarWidth,
    });
    window.content = split;

    return {
        window, split, sidebarHeader, contentHeader,
        setSidebar: (w) => { sidebarView.content = w; },
        setContent: (w) => { contentView.content = w; },
        /// On a collapsed (narrow) window, bring the content pane
        /// forward — what selecting a row should do on a phone-width
        /// window and a no-op on a wide one.
        showContent: () => { split.set_show_content(true); },
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
