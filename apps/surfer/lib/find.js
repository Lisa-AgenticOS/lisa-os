// Find in page (Ctrl+F) — the arithmetic, not the widget.
//
// `WebKit.FindController` does the searching; what is decided here is
// which option bits it is handed and what the counter says. Both are
// small and both were wrong in the obvious way when this was first
// wired: a search with no WRAP_AROUND stops dead at the bottom of the
// page and looks broken, and a counter that shows "0" while the count is
// still being computed reads as "not found" for the first frame of every
// search.
//
// No gi:// import, so the option values are PINNED here rather than
// read from the enum. That is a deliberate duplication with a test that
// names the source: WebKit-6.0.gir on the reference device
// (WebKitGTK 2.48), bitfield WebKitFindOptions.

/// WebKitFindOptions, verified against
/// /usr/share/gir-1.0/WebKit-6.0.gir on the reference iMac, 2026-08-05.
export const FIND_NONE = 0;
export const FIND_CASE_INSENSITIVE = 1;
export const FIND_AT_WORD_STARTS = 2;
export const FIND_TREAT_MEDIAL_CAPITAL_AS_WORD_START = 4;
export const FIND_BACKWARDS = 8;
export const FIND_WRAP_AROUND = 16;

/// The most matches we ask WebKit to count. Counting is work the engine
/// does on the whole document; a page with a million occurrences of "e"
/// should not cost a second of main-loop time to tell you so.
export const MAX_MATCH_COUNT = 1000;

/// The option bits for a search.
///
/// Case-INsensitive by default, because that is what every browser's
/// Ctrl+F does and a case-sensitive default silently finds less than the
/// person expects. Wrap on by default for the same reason.
///
/// `backwards` is here because it is part of the bitfield and pinning
/// the whole bitfield is the point of this module — but the window does
/// NOT use it. Stepping backwards through matches is
/// `search_previous()`, not a second `search()` with the direction bit
/// flipped: re-issuing `search()` restarts from the top, so the arrows
/// would find the first match forever.
export function findOptions({matchCase = false, backwards = false, wrap = true} = {}) {
    let opts = FIND_NONE;
    if (!matchCase) opts |= FIND_CASE_INSENSITIVE;
    if (backwards) opts |= FIND_BACKWARDS;
    if (wrap) opts |= FIND_WRAP_AROUND;
    return opts;
}

/// Is this search worth sending to the engine at all?
///
/// An empty box is not "no results", it is no search: the previous
/// highlight should be cleared and the counter should say nothing.
export function searchable(query) {
    return typeof query === 'string' && query !== '';
}

/// What the little label next to the box says.
///
/// `count === null` means "the engine has not answered yet", which is a
/// real state on a long page and must not render as zero.
export function matchLabel(query, count) {
    if (!searchable(query)) return '';
    if (count === null || count === undefined) return 'Searching…';
    if (typeof count !== 'number' || !Number.isFinite(count) || count <= 0)
        return 'No results';
    if (count >= MAX_MATCH_COUNT) return `${MAX_MATCH_COUNT}+ matches`;
    return count === 1 ? '1 match' : `${count} matches`;
}
