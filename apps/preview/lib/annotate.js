// Annotation decisions — pure. The I/O half (creating PopplerAnnots,
// saving the document) lives in lisa-preview.js; everything that can be
// got wrong quietly is here and tested:
//
//   - RECT ORIENTATION. Poppler speaks two coordinate spaces and mixing
//     them is invisible until a highlight lands mirrored at the bottom
//     of the page: rendering and text-selection APIs count y DOWN from
//     the top-left (the space our clicks arrive in, divided by scale),
//     while annotation rects count y UP from the bottom-left (PDF
//     native). `annotRect` is the only crossing point.
//   - DRAG SEMANTICS. A drag in any direction must normalize, and a
//     twitch must not become a 0.3pt highlight — `isClick` draws that
//     line once.
//   - SAVE NAMING. Saving never overwrites the original: macOS Preview
//     autosaves in place with version history; we have no version
//     store, so "(edited)" copies are the honest equivalent.

/// Functional colors, 16-bit channels as PopplerColor wants them.
/// Highlighter yellow is not in branding/tokens.json for the same
/// reason error-red is not: it is semantic, not brand — every PDF
/// reader on earth highlights in yellow, and violet ink over violet
/// highlight would be unreadable. Notes and boxes ARE brand: violet-500
/// #6D45C9.
export const COLORS = {
    highlight: {red: 0xffff, green: 0xd9d9, blue: 0x4f4f},
    note: {red: 0x6d6d, green: 0x4545, blue: 0xc9c9},
    box: {red: 0x6d6d, green: 0x4545, blue: 0xc9c9},
};

/// Two drag endpoints (any direction) -> a rect with x1<x2, y1<y2.
export function normalizeRect(a, b) {
    return {
        x1: Math.min(a.x, b.x), y1: Math.min(a.y, b.y),
        x2: Math.max(a.x, b.x), y2: Math.max(a.y, b.y),
    };
}

/// A drag smaller than `threshold` page-points in both directions is a
/// click, not a marquee — highlighting nothing beats highlighting a
/// speck the user cannot find to undo.
export function isClick(a, b, threshold = 3) {
    return Math.abs(a.x - b.x) < threshold && Math.abs(a.y - b.y) < threshold;
}

/// Widget coordinates -> top-down page points. The caller passes the
/// scale the page was RENDERED at, so this is exact, not approximate.
export function viewToPage(point, scale) {
    return {x: point.x / scale, y: point.y / scale};
}

/// Top-down rect (render space) -> bottom-up rect (annotation space).
/// The flip swaps which corner is which: top-down y1 (nearer the top)
/// becomes the annotation's y2 (farther from the bottom).
export function annotRect(rect, pageHeight) {
    return {
        x1: rect.x1, y1: pageHeight - rect.y2,
        x2: rect.x2, y2: pageHeight - rect.y1,
    };
}

/// Where an annotated copy is saved. Never the original; never a name
/// that already exists — "(edited)", then "(edited 2)", counting up.
/// `existing` is the sibling file names, so the decision needs no I/O.
export function savePathFor(path, existing) {
    const dir = path.slice(0, path.lastIndexOf('/') + 1);
    const name = path.slice(dir.length);
    const dot = name.lastIndexOf('.');
    const stem = dot > 0 ? name.slice(0, dot) : name;
    const ext = dot > 0 ? name.slice(dot) : '';
    const have = new Set(existing);
    let candidate = `${stem} (edited)${ext}`;
    for (let n = 2; have.has(candidate); n++)
        candidate = `${stem} (edited ${n})${ext}`;
    return dir + candidate;
}

/// The title's unsaved marker. One annotation is "1 note"; the count is
/// whatever has not reached disk yet, across annotation types, page
/// reordering and page rotation.
///
/// Rotation is named SEPARATELY from reordering rather than folded into
/// one "pages edited": they are undone by different gestures and land
/// through different qpdf flags, and a subtitle that says "reordered"
/// when only a rotation is pending sends the user looking for a move
/// they never made.
export function unsavedLabel(annotCount, orderChanged, rotated = false) {
    const parts = [];
    if (annotCount > 0)
        parts.push(`${annotCount} annotation${annotCount === 1 ? '' : 's'}`);
    if (orderChanged) parts.push('pages reordered');
    if (rotated) parts.push('pages rotated');
    return parts.length ? `${parts.join(', ')} — unsaved` : '';
}
