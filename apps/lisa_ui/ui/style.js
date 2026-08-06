// The Lisa stylesheet: what makes an app look like it belongs here
// (ADR-0056 step 2, ADR-0038).
//
// # Why this is a stylesheet and not a widget set
//
// ADR-0056's promise: the theme layer is LIVE. Tokens and CSS reach a
// running app through a file, so "ship glass and every app is glass"
// is true without rebuilding anything — while the widget layer is
// next-launch, because GJS caches ES modules. This file is the live
// half. Every colour in it comes from branding/tokens.json via
// ui/tokens.js, so the palette has exactly one source.
//
// # What "glass" is here, honestly
//
// It is a translucent LAYER OVER THE WINDOW'S OWN GROUND, with a
// hairline edge and a top highlight — the things that make a surface
// read as glass rather than as faded paint.
//
// It is NOT backdrop blur of the desktop behind the window, and no
// amount of client CSS can make it so: GTK4 apps cannot blur what is
// behind them, because Mutter exposes no backdrop-blur protocol to
// clients. Real vibrancy is compositor work, and we do fork the Shell
// (ADR-0038), so it is possible later — but it is not this, and calling
// this vibrancy would be the kind of overclaim rule 10 exists to stop.

import Gtk from 'gi://Gtk';
import Gdk from 'gi://Gdk';
import {TOKENS} from './tokens.js';
import {onScheme} from './theme.js';

/// `#RRGGBB` → `rgba(r, g, b, a)`, so a token can carry alpha without
/// a second token for every opacity we happen to want.
function alpha(hex, a) {
    const h = String(hex).replace('#', '');
    const n = parseInt(h.length === 3 ? h.replace(/(.)/g, '$1$1') : h, 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

/// The sheet, for one scheme.
///
/// Written against Adwaita's own selectors rather than a parallel
/// widget vocabulary: this is a dialect on top of the toolkit, never a
/// replacement for it (rule 11).
function sheet(dark) {
    const ground = dark ? TOKENS['base'] : TOKENS['warm-white'];
    const tint = dark ? TOKENS['violet-300'] : TOKENS['violet-700'];
    const edge = dark ? alpha(TOKENS['warm-white'], 0.10) : alpha(TOKENS['ink-900'], 0.09);
    const highlight = dark ? alpha(TOKENS['warm-white'], 0.09) : alpha('#FFFFFF', 0.75);
    // The pane itself: a wash of the brand tint over the ground, kept
    // low so text contrast is Adwaita's problem and not ours.
    const pane = dark ? alpha(tint, 0.16) : alpha(tint, 0.10);

    return `
/* GENERATED AT RUNTIME by lisa_ui/ui/style.js from branding/tokens.json.
   Nothing here is hand-typed colour. */

.lisa-glass {
    background-image: linear-gradient(to bottom, ${highlight}, transparent 180px);
    background-color: ${pane};
}

/* Adwaita's own sidebar styling paints an opaque background, which sat
   ON TOP of the glass and made the first attempt almost invisible in a
   screenshot. The pane below has to show through the widgets inside it,
   or there is no pane — only a class name. */
.lisa-glass .navigation-sidebar,
.lisa-glass scrolledwindow,
.lisa-glass viewport,
.lisa-glass list,
.lisa-glass headerbar { background: transparent; background-image: none; }

/* The header over a glass pane keeps the seam and loses its own fill,
   so the glass runs the full height of the pane rather than starting
   below the toolbar. */
.lisa-glass headerbar { box-shadow: none; }

/* The seam between a glass pane and the content beside it. A hairline,
   not a border: a 1px line at full contrast is what makes a translucent
   panel look like a table cell. */
.lisa-glass-edge-end { border-right: 1px solid ${edge}; }

/* The window ground the glass sits on. Named so an app can opt a single
   window out without redefining the palette. */
.lisa-ground { background-color: ${ground}; }
`;
}

let provider = null;

/// Install the Lisa sheet on a display, and keep it in step with the
/// scheme.
///
/// Idempotent: one provider per process, reloaded rather than stacked.
/// Adding a second provider at the same priority is how an app ends up
/// with two sheets fighting and a colour that changes depending on
/// which loaded last.
export function installStyle(display) {
    const d = display ?? Gdk.Display.get_default();
    if (!d)
        return null;
    if (!provider) {
        provider = new Gtk.CssProvider();
        Gtk.StyleContext.add_provider_for_display(
            d, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    // Loads now AND on every scheme change — the whole reason theme.js
    // leads with onScheme rather than a getter.
    onScheme((dark) => provider.load_from_string(sheet(dark)));
    return provider;
}
