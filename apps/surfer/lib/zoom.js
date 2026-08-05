// Page zoom (Ctrl +/-/0). Small, and the only reason it is a module is
// that "one step up from here" is a list lookup with two edges, and
// both edges were wrong when it was a `level * 1.1`: multiplying
// compounds float error until Ctrl+0 no longer reaches exactly 1, and
// nothing stops it at 40 presses.

/// The steps, in the shape every browser uses. `1` is in the list on
/// purpose: zooming out from 1.1 must land ON 100%, not near it.
export const ZOOM_STEPS = Object.freeze(
    [0.3, 0.5, 0.67, 0.8, 0.9, 1, 1.1, 1.2, 1.33, 1.5, 1.7, 2, 2.4, 3]);

export const DEFAULT_ZOOM = 1;

/// The step nearest `level`, by index — the anchor both directions move
/// from, so a zoom set by anything other than these keys (a restored
/// value, a future per-site setting) still steps sanely.
///
/// A value exactly between two steps anchors on the LOWER one (the
/// comparison is strict), so the first press after a stray value moves
/// in the direction asked for instead of appearing to do nothing.
function nearestIndex(level) {
    const l = typeof level === 'number' && Number.isFinite(level) && level > 0
        ? level : DEFAULT_ZOOM;
    let best = 0;
    let bestDelta = Infinity;
    for (let i = 0; i < ZOOM_STEPS.length; i++) {
        const delta = Math.abs(ZOOM_STEPS[i] - l);
        if (delta < bestDelta) { best = i; bestDelta = delta; }
    }
    return best;
}

export function zoomIn(level) {
    const i = nearestIndex(level);
    return ZOOM_STEPS[Math.min(i + 1, ZOOM_STEPS.length - 1)];
}

export function zoomOut(level) {
    const i = nearestIndex(level);
    return ZOOM_STEPS[Math.max(i - 1, 0)];
}

export function zoomReset() {
    return DEFAULT_ZOOM;
}

/// `1.33` → `133%`. Rounded, because 1.33 is 133.00000000000003 of a
/// percent and nobody wants to read that.
export function zoomLabel(level) {
    const l = typeof level === 'number' && Number.isFinite(level) && level > 0
        ? level : DEFAULT_ZOOM;
    return `${Math.round(l * 100)}%`;
}
