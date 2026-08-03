// Page-order decisions — pure. The order is an array of ORIGINAL
// 0-based page indices in display order; the I/O half applies it with
// qpdf at save time (poppler-glib can add annotations but cannot
// reorder or delete pages — checked against poppler 25.x, there is no
// API for it, and faking it by re-rendering pages into a new PDF would
// rasterize text).

/// Move the page at display position `from` to display position `to`.
/// Returns a new array; out-of-range positions return the input
/// unchanged rather than throwing mid-drag.
export function movePage(order, from, to) {
    if (from < 0 || from >= order.length || to < 0 || to >= order.length ||
        from === to)
        return order;
    const next = order.slice();
    const [page] = next.splice(from, 1);
    next.splice(to, 0, page);
    return next;
}

/// Remove the page at display position `index`. A document with zero
/// pages is not a document: removing the last page returns null and the
/// caller says why, rather than qpdf failing at save with its own
/// vocabulary.
export function removePage(order, index) {
    if (order.length <= 1 || index < 0 || index >= order.length) return null;
    const next = order.slice();
    next.splice(index, 1);
    return next;
}

export function isIdentity(order) {
    return order.every((page, i) => page === i);
}

/// qpdf's --pages selection, 1-based, consecutive runs compressed to
/// ranges: [2,0,1] -> "3,1-2". Compression is not cosmetic — a
/// 400-page document with one page moved is two ranges, not 400 comma
/// terms handed to a command line.
export function qpdfPageSpec(order) {
    if (order.length === 0) return '';
    const parts = [];
    let start = order[0], prev = order[0];
    for (const page of order.slice(1)) {
        if (page === prev + 1) { prev = page; continue; }
        parts.push(start === prev ? `${start + 1}` : `${start + 1}-${prev + 1}`);
        start = prev = page;
    }
    parts.push(start === prev ? `${start + 1}` : `${start + 1}-${prev + 1}`);
    return parts.join(',');
}
