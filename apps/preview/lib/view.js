// Zoom, fit and page navigation — the arithmetic, with no GTK in it.
// Pure so `just shell-test` can check the edges that are otherwise only
// reachable by clicking: the last page, the smallest zoom, a rotated
// fit.

/// Zoom steps, in the order the +/- keys walk them. Not a multiplier:
/// a fixed ladder means 100% is always reachable exactly, which
/// repeated *1.25 never is, and "actual size" being 99.998% is the kind
/// of detail that makes an app feel wrong without anyone naming why.
export const ZOOM_STEPS = [
    0.125, 0.25, 0.33, 0.5, 0.67, 1, 1.5, 2, 3, 4, 6, 8,
];

export const MIN_ZOOM = ZOOM_STEPS[0];
export const MAX_ZOOM = ZOOM_STEPS[ZOOM_STEPS.length - 1];

/// The next step up or down from an arbitrary zoom (a fit ratio is
/// rarely on the ladder), clamped at both ends.
export function zoomStep(current, direction) {
    if (direction > 0) {
        const next = ZOOM_STEPS.find(z => z > current + 1e-9);
        return next ?? MAX_ZOOM;
    }
    const prev = [...ZOOM_STEPS].reverse().find(z => z < current - 1e-9);
    return prev ?? MIN_ZOOM;
}

/// The scale that fits `content` inside `viewport`, honouring rotation.
///
/// Never enlarges past 1: a 16×16 favicon blown up to fill a 4K window
/// is not "fit", it is a blur. macOS Preview does the same, and the
/// reason is the same.
export function fitScale(content, viewport, rotation = 0) {
    const quarterTurn = ((rotation % 360) + 360) % 360 % 180 !== 0;
    const w = quarterTurn ? content.height : content.width;
    const h = quarterTurn ? content.width : content.height;
    if (!(w > 0 && h > 0 && viewport.width > 0 && viewport.height > 0))
        return 1;
    return Math.min(viewport.width / w, viewport.height / h, 1);
}

/// The scale that fills the viewport's width — "fit width", the one
/// that matters for reading a PDF. Unlike fitScale this MAY enlarge:
/// the point is legibility, not preserving pixels.
export function fitWidthScale(content, viewport, rotation = 0) {
    const quarterTurn = ((rotation % 360) + 360) % 360 % 180 !== 0;
    const w = quarterTurn ? content.height : content.width;
    if (!(w > 0 && viewport.width > 0))
        return 1;
    return viewport.width / w;
}

/// Move within a bounded sequence. Returns the same index at the ends
/// rather than wrapping — wrapping from the last page to the first
/// reads as "the document reloaded" and loses the reader's place.
export function step(index, count, delta) {
    if (!(count > 0))
        return 0;
    return Math.max(0, Math.min(count - 1, index + delta));
}

/// Rotation normalised to 0/90/180/270, in either direction.
export function rotate(current, delta) {
    return (((current + delta) % 360) + 360) % 360;
}
