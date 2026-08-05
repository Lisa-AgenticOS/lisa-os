// History — what gets remembered, and what forgetting means.
//
// # Two rules that are not conveniences
//
// **A history a person cannot clear is surveillance.** `forgetUrl`,
// `forgetSince` and `clearHistory` are as load-bearing as `addVisit`,
// and `tests/history.test.js` mutates each of them to prove the delete
// actually deletes rather than filtering the view.
//
// **The agent's browsing is not the person's history.** `recordable`
// refuses the agent profile outright. The agent already browses in its
// own `NetworkSession` (lib/profiles.js, #181/#259); writing what it
// visited into the person's file would hand the loop a way to put text
// of its choosing into a surface the person reads, and would make a
// history entry a thing a page can cause.
//
// No gi:// import. The window supplies `now` and does the file I/O
// (lib/store.js decides the path); everything here is a reducer.

import {AGENT_PROFILE} from './profiles.js';

/// How many rows to keep. Old enough that nobody hits it in normal use,
/// small enough that the file stays a file.
export const HISTORY_LIMIT = 5000;

/// Schemes that never enter history.
///
/// `lisa:` and `lisa-go:` are Surfer's own furniture — the start page is
/// not a place you went. `about:` is the same. The executing schemes
/// (`javascript:`, `data:`, `blob:`, `vbscript:`) are refused by
/// lib/url.js before they can be navigated at all; they are listed again
/// here because a history row is a thing that gets CLICKED later, and a
/// list that could hold one is a list that could re-run it.
const NEVER_RECORDED = [
    'lisa:', 'lisa-go:', 'about:', 'javascript:', 'data:', 'blob:', 'vbscript:',
];

/// May this visit be written down?
///
/// The profile argument is not optional and not defaulted: a default
/// here would be the security boundary living in whichever caller
/// remembered to pass it, which is the mistake profiles.js's own header
/// warns about.
export function recordable(url, profile) {
    if (profile === AGENT_PROFILE) return false;
    if (typeof profile !== 'string' || profile.trim() === '') return false;
    const u = String(url ?? '').trim();
    if (u === '') return false;
    const lower = u.toLowerCase();
    return !NEVER_RECORDED.some(s => lower.startsWith(s));
}

/// Record a visit. One row per URL, newest first, with a visit count —
/// a hundred rows for one page you keep reloading is a history nobody
/// can read.
///
/// The title is updated only when the new one says something: a page
/// whose title has not loaded yet reports its URL, and letting that
/// overwrite a good title is how every row ends up looking like a link.
export function addVisit(list, {url, title, at} = {}, {limit = HISTORY_LIMIT} = {}) {
    const items = Array.isArray(list) ? list : [];
    const u = String(url ?? '').trim();
    if (u === '') return items;
    const t = String(title ?? '').trim();
    const when = typeof at === 'number' && Number.isFinite(at) ? at : 0;
    const previous = items.find(e => e && e.url === u);
    const rest = items.filter(e => e && e.url !== u);
    const row = previous
        ? {...previous, title: t || previous.title, visits: (previous.visits || 0) + 1, lastVisit: when}
        : {url: u, title: t, visits: 1, firstVisit: when, lastVisit: when};
    return [row, ...rest].slice(0, limit);
}

/// Correct the title of a row that is already there, WITHOUT counting a
/// second visit.
///
/// A page sets `document.title` after the load finishes, so the title at
/// the moment a visit is recorded is often the URL or nothing. Routing
/// that correction back through `addVisit` would count one visit per
/// title change — which, on a page that updates its title as a counter
/// or a clock, is a visit count that climbs on its own.
export function retitle(list, url, title) {
    const items = Array.isArray(list) ? list : [];
    const u = String(url ?? '').trim();
    const t = String(title ?? '').trim();
    if (u === '' || t === '') return items;
    return items.map(e => (e && e.url === u ? {...e, title: t} : e));
}

/// Substring match over title and URL, case-insensitive. An empty query
/// is the whole list — the history window opens showing everything.
export function searchHistory(list, query, {limit = 500} = {}) {
    const items = Array.isArray(list) ? list : [];
    const q = String(query ?? '').trim().toLowerCase();
    if (q === '') return items.slice(0, limit);
    return items.filter(e => {
        const title = String(e?.title ?? '').toLowerCase();
        const url = String(e?.url ?? '').toLowerCase();
        return title.includes(q) || url.includes(q);
    }).slice(0, limit);
}

/// Forget one address — every row for it, not the first one found.
export function forgetUrl(list, url) {
    const u = String(url ?? '').trim();
    if (u === '') return Array.isArray(list) ? list : [];
    return (Array.isArray(list) ? list : []).filter(e => e && e.url !== u);
}

/// Forget everything visited at or after `since` — "clear the last
/// hour", the thing people actually want.
///
/// `>=` rather than `>`: a row stamped exactly at the boundary is inside
/// the window being cleared. A row with no usable timestamp is KEPT,
/// because deleting data on the strength of a missing field is the
/// wrong way round for a delete.
export function forgetSince(list, since) {
    const items = Array.isArray(list) ? list : [];
    if (typeof since !== 'number' || !Number.isFinite(since)) return items;
    return items.filter(e => {
        const t = e?.lastVisit;
        if (typeof t !== 'number' || !Number.isFinite(t)) return true;
        return t < since;
    });
}

/// Forget all of it. Returns a new empty list rather than mutating —
/// the caller writes it, and a clear that only emptied the in-memory
/// copy would come back on the next start.
export function clearHistory() {
    return [];
}

/// What a row shows. A page with no title is its own address; an entry
/// that renders as an empty string is a row nobody can click on
/// purpose.
export function historyLabel(entry) {
    const title = String(entry?.title ?? '').trim();
    return title !== '' ? title : String(entry?.url ?? '');
}
