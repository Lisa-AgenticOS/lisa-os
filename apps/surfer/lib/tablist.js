// Sidebar tab-row labels (#182). Pure, because what a row SAYS is a
// rule worth testing: a page with a title shows it, a page without one
// shows its host rather than a raw URL, and nothing overflows the
// sidebar into an ellipsis-free mess.

/// What the sidebar row displays for a page.
export function rowLabel(title, uri, max = 40) {
    const t = String(title ?? '').trim();
    if (t)
        return truncate(t, max);
    const host = hostOf(uri);
    if (host)
        return truncate(host, max);
    return 'New Tab';
}

function hostOf(uri) {
    const m = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]+)/i.exec(String(uri ?? ''));
    return m ? m[1] : '';
}

function truncate(text, max) {
    return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}
