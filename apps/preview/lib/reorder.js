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

/// Whether the order differs from the document. NOT just isIdentity:
/// removing the LAST page leaves [0,1] from a 3-page document — an
/// identity permutation of the wrong length, and treating it as
/// unchanged would silently drop the deletion at save.
export function orderChanged(order, pageCount) {
    return order.length !== pageCount || !isIdentity(order);
}

/// Rotate the ORIGINAL page `orig` by `delta` degrees, returning a new
/// rotation map.
///
/// Keyed by ORIGINAL page index, not by display position, so a rotation
/// travels with its page when the page is moved. Keying by position
/// would silently rotate whatever page later landed there — the same
/// class of bug as #195, where a display index was used to fetch an
/// original page.
///
/// A rotation that comes back to 0 is DELETED rather than stored as 0,
/// so `rotationsChanged` is a plain emptiness test and four taps on
/// Rotate leave a document that is genuinely unmodified — not one that
/// still asks to be saved.
export function rotatePageBy(rotations, orig, delta) {
    const next = {...rotations};
    const deg = (((next[orig] ?? 0) + delta) % 360 + 360) % 360;
    if (deg === 0) delete next[orig];
    else next[orig] = deg;
    return next;
}

/// The rotation of an original page, always 0/90/180/270.
export function rotationOf(rotations, orig) {
    return ((rotations?.[orig] ?? 0) % 360 + 360) % 360;
}

/// Does any page carry a pending rotation? Rotations that survive a
/// REMOVE are not a change: a rotated page that is no longer in the
/// order cannot rotate anything, and treating its leftover entry as
/// dirty would keep offering to save a document nobody edited.
export function rotationsChanged(rotations, order) {
    return order.some(orig => rotationOf(rotations, orig) !== 0);
}

/// qpdf's `--rotate` arguments for a saved copy, one per distinct angle.
///
/// **The page numbers are OUTPUT positions, not input pages** — verified
/// on the reference device 2026-08-05 rather than assumed, because the
/// two readings differ exactly when both flags are used at once, which
/// is our only case:
///
///     qpdf in.pdf --pages . 3,1-2 -- --rotate=+90:1 out.pdf
///     -> out page 1 is input page 3, AND it is the rotated one
///        (600x400 where the input was 400x600)
///
/// Had it meant input pages, every rotation would land on the wrong
/// page the moment a page was also moved, and nothing would say so.
///
/// Ranges are compressed by the same `qpdfPageSpec` the page selection
/// uses, for the same reason: a 400-page document must not become 400
/// comma terms on a command line.
export function qpdfRotateArgs(order, rotations) {
    const byAngle = new Map();
    order.forEach((orig, position) => {
        const deg = rotationOf(rotations, orig);
        if (deg === 0) return;
        if (!byAngle.has(deg)) byAngle.set(deg, []);
        byAngle.get(deg).push(position);
    });
    return [...byAngle.keys()]
        .sort((a, b) => a - b)
        .map(deg => `--rotate=+${deg}:${qpdfPageSpec(byAngle.get(deg))}`);
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
