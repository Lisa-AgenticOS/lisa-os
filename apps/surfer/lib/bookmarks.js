// Bookmarks — add, list, open, remove. Pure.
//
// The only rule here that is more than bookkeeping: a bookmark is a
// thing that gets CLICKED, possibly months later, so the set of schemes
// that may be bookmarked is an allowlist rather than the refusal list
// lib/url.js applies at the address bar. A stored `javascript:` row is a
// self-XSS with a nice icon, which is exactly why Firefox and Chrome
// both stopped honouring them in bookmarks.

/// Schemes that may be stored. `file:` is here because a person
/// bookmarking a local document is their business (ADR-0029's second
/// test) — this is a person-facing list, not the agent's, which is
/// narrower still (lib/actions.js, #214).
const BOOKMARKABLE = ['http:', 'https:', 'file:'];

export const BOOKMARK_LIMIT = 2000;

export function bookmarkable(url) {
    const u = String(url ?? '').trim().toLowerCase();
    if (u === '') return false;
    return BOOKMARKABLE.some(s => u.startsWith(s));
}

/// Is this address already bookmarked? Drives the star's state, so it
/// has to agree exactly with what `addBookmark` deduplicates on.
export function isBookmarked(list, url) {
    const u = String(url ?? '').trim();
    if (u === '') return false;
    return (Array.isArray(list) ? list : []).some(e => e && e.url === u);
}

/// Add one, newest first.
///
/// Re-adding an address already bookmarked updates its title and keeps
/// its original `addedAt` and position — a person pressing Ctrl+D twice
/// has not made a second bookmark, and should not see their list
/// reshuffle for it.
export function addBookmark(list, {url, title, at} = {}) {
    const items = Array.isArray(list) ? list : [];
    const u = String(url ?? '').trim();
    if (!bookmarkable(u)) return items;
    const t = String(title ?? '').trim();
    const when = typeof at === 'number' && Number.isFinite(at) ? at : 0;
    const existing = items.find(e => e && e.url === u);
    if (existing) {
        return items.map(e => (e && e.url === u
            ? {...e, title: t || e.title}
            : e));
    }
    return [{url: u, title: t, addedAt: when}, ...items].slice(0, BOOKMARK_LIMIT);
}

/// Remove one — every row for the address, not the first found.
export function removeBookmark(list, url) {
    const u = String(url ?? '').trim();
    if (u === '') return Array.isArray(list) ? list : [];
    return (Array.isArray(list) ? list : []).filter(e => e && e.url !== u);
}

/// Ctrl+D on a page that is already bookmarked removes it, the way the
/// star in every other browser does. One function so the keyboard and
/// the button cannot disagree.
export function toggleBookmark(list, entry) {
    return isBookmarked(list, entry?.url)
        ? removeBookmark(list, entry.url)
        : addBookmark(list, entry);
}

export function searchBookmarks(list, query) {
    const items = Array.isArray(list) ? list : [];
    const q = String(query ?? '').trim().toLowerCase();
    if (q === '') return items;
    return items.filter(e => {
        const title = String(e?.title ?? '').toLowerCase();
        const url = String(e?.url ?? '').toLowerCase();
        return title.includes(q) || url.includes(q);
    });
}

/// What a row shows: the title, or the address when there is none.
export function bookmarkLabel(entry) {
    const title = String(entry?.title ?? '').trim();
    return title !== '' ? title : String(entry?.url ?? '');
}
