// One dark/light path (ADR-0056, #282).
//
// # The bug this exists to make hard
//
// #282's acceptance asks for "one dark/light path". Measured on
// 2026-08-06, there was not one — there was ONE APP with a path and
// five without. Only Surfer consulted `Adw.StyleManager`; Mail and
// Preview had dark-related comments and no handling; the Assistant, the
// Ledger app and Notes had nothing. Everything except Surfer simply
// inherited libadwaita's automatic theming, which is why Surfer read as
// the odd one out.
//
// And Surfer's own handling had the shape this module is built against.
// Its window CSS reloaded on `notify::dark`, correctly. Its per-tab
// WebKit backdrop did not:
//
//     rgba.parse(Adw.StyleManager.get_default().dark ? … : …);
//     view.set_background_color(rgba);
//
// A read at construction, never revisited. Change the system theme with
// tabs open and every one of them kept the backdrop of the scheme it
// was born in. The bug is not the branch — the branch is right — it is
// that reading the scheme and following the scheme look identical at
// the call site, and only one of them is correct.
//
// So this module does not offer a getter as its main verb. `onScheme`
// calls you IMMEDIATELY and again on every change, which makes the
// correct thing the shorter thing to write.

import Adw from 'gi://Adw';

/// Is the session dark right now?
///
/// Deliberately secondary. Use it for a one-off decision that genuinely
/// cannot be re-run — and if you find yourself storing the answer, you
/// wanted `onScheme`.
export function isDark() {
    return Adw.StyleManager.get_default().dark;
}

/// Run `fn(isDark)` now, and again whenever the scheme changes.
///
/// Returns a function that stops listening; call it when the widget the
/// callback paints is gone, or the callback outlives its target and
/// paints something that no longer exists.
export function onScheme(fn) {
    if (typeof fn !== 'function')
        throw new Error('lisa.sdk/theme: onScheme needs a function');
    const mgr = Adw.StyleManager.get_default();
    const id = mgr.connect('notify::dark', () => fn(mgr.dark));
    // Immediately, not on the next change: a caller that only painted
    // on change would start in whatever colours the widget defaulted
    // to, and look correct only after the person toggled the theme.
    fn(mgr.dark);
    return () => mgr.disconnect(id);
}

/// The two ground tones, from branding/tokens.json rather than typed
/// in at the call site.
///
/// Surfer had the dark ground and `'#FFFFFF'` written out in two places
/// with `/* token: … */` comments beside them, because
/// `branding/out/tokens.js` was not reachable from an app. It is now
/// (ui/tokens.js), so the comment can go back to being a fact.
import {TOKENS} from './tokens.js';

export function groundColor(dark) {
    return dark ? TOKENS['base'] : TOKENS['surface'];
}
