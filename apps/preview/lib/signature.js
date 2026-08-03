// Signature decisions — pure. Strokes are arrays of {x, y} points in
// capture coordinates; everything that turns a scrawl into a stamp
// that lands where the user clicked, at a sane size, is here and
// tested. Rendering (cairo) and the PopplerAnnotStamp live in
// lisa-preview.js.

/// The bounding box of all strokes, or null when there is no ink —
/// a signature dialog Save with an empty canvas must be a no-op, not
/// a 0×0 stamp.
export function strokeBounds(strokes) {
    let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
    for (const stroke of strokes) {
        for (const p of stroke) {
            if (p.x < x1) x1 = p.x;
            if (p.y < y1) y1 = p.y;
            if (p.x > x2) x2 = p.x;
            if (p.y > y2) y2 = p.y;
        }
    }
    if (!(x1 <= x2 && y1 <= y2)) return null;
    return {x1, y1, x2, y2};
}

/// Translate strokes to the origin with a little breathing room, so
/// the stored signature is position-independent. Returns null for an
/// empty canvas.
export function normalizeStrokes(strokes, pad = 6) {
    const b = strokeBounds(strokes);
    if (!b) return null;
    return {
        width: b.x2 - b.x1 + pad * 2,
        height: b.y2 - b.y1 + pad * 2,
        strokes: strokes
            .filter(s => s.length > 0)
            .map(s => s.map(p => ({x: p.x - b.x1 + pad, y: p.y - b.y1 + pad}))),
    };
}

/// The placed stamp's size in PDF points: a fixed target width,
/// height following the signature's own aspect. A one-dot signature
/// must not become a 150×150000 rect — height is clamped.
export function stampSize(sig, targetWidthPt = 150) {
    const aspect = sig.height / Math.max(1, sig.width);
    const h = Math.min(targetWidthPt * aspect, targetWidthPt);
    return {width: targetWidthPt, height: Math.max(8, h)};
}

/// Serialization for ~/.local/share/lisa/preview/signature.json.
/// Versioned, because a format with no version number is a format
/// that can never change.
export function serializeSignature(sig) {
    return JSON.stringify({version: 1, ...sig});
}

export function deserializeSignature(text) {
    try {
        const d = JSON.parse(text);
        if (d?.version !== 1 || !Array.isArray(d.strokes) ||
            !(d.width > 0) || !(d.height > 0))
            return null;
        return {width: d.width, height: d.height, strokes: d.strokes};
    } catch (e) {
        return null;
    }
}
