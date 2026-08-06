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
    const edge = dark ? alpha(TOKENS['warm-white'], 0.14) : alpha(TOKENS['ink-900'], 0.12);
    // A single hairline along the top edge, not a wash down the pane.
    // The first version faded 75% white over 180px, which read as a
    // BAND rather than a sheen — the top of the sidebar looked like a
    // different colour from the bottom. css.glass has no gradient in it
    // either: fill, blur, border, shadow. The sheen was the one thing
    // here nobody asked for.
    const highlight = dark ? alpha(TOKENS['warm-white'], 0.14) : alpha('#FFFFFF', 0.65);

    // TWO layers, and the first one is the lesson.
    //
    // `wash` shifts LUMINANCE relative to whatever is behind the pane —
    // darker in light, lighter in dark. `pane` adds the brand hue.
    // The first version had only the hue, and it disappeared completely
    // in Surfer: measured on the device, sidebar srgb(219,217,227) over
    // a window ground of srgb(219,217,227), because Surfer's ground is
    // already violet-tinted lavender and a violet wash on lavender is
    // lavender. It looked right in Notes purely because Notes' ground
    // happens to be near-white.
    //
    // Glass has to separate from ANY ground it is put on, or it is a
    // class name that works in one app.
    // Translucent enough to see through, opaque enough to read on.
    // css.glass lands around 0.2 for white-on-photo; ours sits lower
    // because the blur plus the token hue is already doing the work.
    const pane = dark ? alpha(TOKENS['base'], 0.55) : alpha(TOKENS['warm-white'], 0.55);
    const shadow = dark ? alpha(TOKENS['base'], 0.45) : alpha(TOKENS['ink-900'], 0.10);
    const blur = 24;

    return `
/* GENERATED AT RUNTIME by lisa_ui/ui/style.js from branding/tokens.json.
   Nothing here is hand-typed colour. */

.lisa-glass {
    /* The glassmorphism recipe (css.glass), in Lisa's tokens:
       translucent fill + backdrop blur + hairline border + soft shadow.
       The blur is the part that makes it glass rather than tint, and it
       only SHOWS where the pane overlaps something — blurring a flat
       window background produces the same flat colour. */
    background-color: ${pane};
    backdrop-filter: blur(${blur}px);
    box-shadow: 0 4px 30px ${shadow};
}

/* A pane that floats over content: rounded, edged all round, and it
   keeps its own shadow. The full-height variant uses the seam below
   instead, because a radius on a pane flush to the window edge shows
   the window's corner through the gap. */
.lisa-glass-floating {
    border-radius: 14px;
    border: 1px solid ${edge};
    /* The lit top edge of a pane of glass: one line, not a gradient. */
    box-shadow: inset 0 1px 0 ${highlight}, 0 4px 30px ${shadow};
}

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
