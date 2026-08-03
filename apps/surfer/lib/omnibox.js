// Address-bar suggestions (#182 v2). Pure: what the dropdown OFFERS is
// a set of rules worth testing; the popover just renders them.
//
// v1 sources, deliberately bounded: the OPEN TABS (switch, don't
// duplicate) and a search row. History-backed completion needs a
// history feature first (storage + retention are their own decision),
// and provider suggest-as-you-type is egress per keystroke — its own
// toggle, off by default, when it exists at all.

import {resolveInput} from './url.js';

/// What the dropdown offers for `text`, given the open tabs
/// [{title, uri}]. Order: go-to-URL (when the text IS one), matching
/// tabs, search — so Enter's meaning (navigate/search, exactly what
/// the bar did before) is always the first row.
export function suggestionsFor(text, tabs, max = 6) {
    const t = String(text ?? '').trim();
    if (t === '')
        return [];
    const out = [];

    const r = resolveInput(t);
    // A refused scheme offers NOTHING navigable — not even indirectly.
    // The search row still appears: searching FOR hostile text is
    // harmless; navigating to it is not.
    if (r.kind === 'load' && r.url)
        out.push({kind: 'url', url: r.url});

    const needle = t.toLowerCase();
    for (const [index, tab] of (tabs ?? []).entries()) {
        if (out.length >= max - 1)
            break;
        const title = String(tab?.title ?? '');
        const uri = String(tab?.uri ?? '');
        if (title.toLowerCase().includes(needle) || uri.toLowerCase().includes(needle))
            out.push({kind: 'tab', index, title, uri});
    }

    out.push({kind: 'search', query: t});
    return out.slice(0, max);
}
