// The prompt half of the dock bar (ADR-0035 §2).
//
// ADR-0035 §2's load-bearing claim: "One bar: apps on the left, the
// prompt filling the rest… launching a program and asking for an
// outcome are the same gesture, so they get the same surface." This is
// the part of that which can be wrong, kept free of GNOME imports so it
// runs under the unit-test runner on a dev host.
//
// # What this module decides, and what it deliberately does not
//
// It decides **where one submission goes** — launch an app, or hand the
// text to the assistant. It does NOT decide what the assistant answers,
// stream a reply, or raise a confirmation. The dock hands off to
// `dev.lisaos.Overlay1.UI.Summon` and forgets, which is not a
// convenience: ADR-0035 §4 says a dock that owns the prompt must not
// also own the confirmation dialog, and the cheapest way to guarantee
// that is a surface that never sees the answer at all.
//
// # Why an exact match, and not the launcher's ranking
//
// ADR-0035 §2 also folds `shell/launcher` into this bar, "growing
// upward into results as you type". That is not built. Without a
// visible result list there is nothing to disambiguate a prefix
// against, so a prefix match would launch a program the person never
// named — and the wrong window opening is a worse failure than a
// sentence reaching the assistant, which the person can simply read and
// ignore. So: an exact name (or desktop-id stem) launches; everything
// else asks. Ranking arrives with the results list, in the same change.

/// Where an `ask` goes: `dev.lisaos.Overlay1.UI.Summon`, the UI-control
/// surface the overlay extension already exports (see
/// `shell/overlay-extension/lib/iface.js`, which is the source of these
/// strings).
///
/// The dock is a THIRD frontend on one headless backend, not a second
/// implementation of it. It sends a string and hears nothing back: no
/// D-Bus client for `dev.lisaos.Overlay1`, no inference routing, no
/// token stream, and — the part that matters — no confirmation dialog
/// (ADR-0035 §4).
///
/// Copied rather than imported because the two extensions are installed
/// as separate trees and neither can `import` from the other at
/// runtime. A copied bus name is exactly how #244 and #255 happened —
/// a correct service nobody called, a chord bound to nothing — so
/// `tests/prompt.test.js` imports both and fails if they ever differ.
export const ASSISTANT_BUS_NAME = 'dev.lisaos.Overlay1.UI';
export const ASSISTANT_OBJECT_PATH = '/dev/lisaos/Overlay1/UI';
export const ASSISTANT_METHOD = 'Summon';
export const ASSISTANT_SIGNATURE = '(sa{sv})';

/// Keyvals, spelled out rather than imported: this module must run
/// where Clutter does not exist. These are the X11/Clutter constants
/// and they have not moved in thirty years.
const KEY_RETURN = 0xff0d;
const KEY_KP_ENTER = 0xff8d;
const KEY_ISO_ENTER = 0xfe34;
const KEY_ESCAPE = 0xff1b;

/// Strip a desktop id down to the word a person would type:
/// `app.lisaos.Mail.desktop` → `mail`, `firefox.desktop` → `firefox`.
function idStem(id) {
    if (typeof id !== 'string')
        return null;
    const base = id.endsWith('.desktop') ? id.slice(0, -'.desktop'.length) : id;
    const stem = base.includes('.') ? base.slice(base.lastIndexOf('.') + 1) : base;
    return stem === '' ? null : stem.toLowerCase();
}

/// Case- and accent-insensitive, because "Café" is typed both ways and
/// an app that fails to launch over a diacritic reads as broken.
///
/// `normalize` is ES2015 and present in gjs, node and jsc alike; the
/// guard is for a runtime that ships without the ICU data rather than
/// for a runtime without the method.
function fold(text) {
    const s = String(text ?? '').trim().toLowerCase();
    try {
        return s.normalize('NFD').replace(/[\u0300-\u036f]/g, '');
    } catch {
        return s;
    }
}

/// Does this candidate answer to exactly this typed word?
export function namesApp(query, candidate) {
    const q = fold(query);
    if (q === '')
        return false;
    if (fold(candidate?.name) === q)
        return true;
    const stem = idStem(candidate?.id);
    return stem !== null && fold(stem) === q;
}

/// What pressing Enter in the dock's prompt means.
///
/// `candidates` is what the caller's app lookup returned, as
/// `{id, name}` pairs — passing them in rather than reaching for
/// `Shell.AppSystem` here is what makes the decision testable off a
/// live session.
///
/// Returns one of:
///   `{kind: 'none'}`                    — nothing to do
///   `{kind: 'launch', id, prompt}`      — start that app
///   `{kind: 'ask', prompt}`             — hand to the assistant
///
/// Whitespace-only input is `none`, never `ask`: a permanent entry
/// collects stray Enters from a person who clicked it by accident, and
/// summoning an empty assistant layer for each one would make the dock
/// something you learn to avoid clicking.
export function submission(text, candidates = []) {
    const prompt = String(text ?? '').trim();
    if (prompt === '')
        return {kind: 'none'};
    for (const candidate of candidates ?? []) {
        if (namesApp(prompt, candidate))
            return {kind: 'launch', id: candidate.id, prompt};
    }
    return {kind: 'ask', prompt};
}

/// What a key press in the entry does.
///
/// Returns `'submit'`, `'clear'`, `'release'` or `'pass'`.
///
/// Escape is two-stage, the shape Spotlight and every browser address
/// bar use: with text it clears, and only an already-empty entry gives
/// the keyboard back. One key that both erases a half-typed thought and
/// moves focus somewhere invisible is a key people stop pressing.
///
/// Everything else is `'pass'` — the entry is a text field first, and a
/// dock that swallowed keys it had no plan for would break every
/// selection, every accent and every input method in it.
export function keyAction(keyval, state = {}) {
    if (keyval === KEY_RETURN || keyval === KEY_KP_ENTER || keyval === KEY_ISO_ENTER)
        return 'submit';
    if (keyval === KEY_ESCAPE)
        return state?.hasText ? 'clear' : 'release';
    return 'pass';
}
