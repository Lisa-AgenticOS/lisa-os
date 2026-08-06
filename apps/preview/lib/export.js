// Export decisions — pure. The I/O half (rendering pages, pixbuf
// saving) lives in lisa-preview.js; what can be got wrong quietly is
// here and tested: which formats we OFFER (grounded in what the
// machine's pixbuf can actually write, injected — never assumed), what
// the exported file is called, the rasterization arithmetic, and (since
// #299) WHAT an export of a page actually renders.

import {rotationOf} from './reorder.js';

/// The formats worth offering, in menu order. `key` is the pixbuf
/// writer name; the list the UI shows is the intersection with the
/// machine's own writable set (asked at startup via
/// GdkPixbuf.Pixbuf.get_formats, the #146 lesson).
export const EXPORT_FORMATS = [
    {key: 'png', label: 'PNG', ext: 'png'},
    {key: 'jpeg', label: 'JPEG', ext: 'jpg'},
    {key: 'webp', label: 'WebP', ext: 'webp'},
    {key: 'avif', label: 'AVIF', ext: 'avif'},
    {key: 'tiff', label: 'TIFF', ext: 'tif'},
];

export function exportFormats(writableNames) {
    const have = new Set(writableNames);
    return EXPORT_FORMATS.filter(f => have.has(f.key));
}

/// Save options per format. jpeg/webp/avif take a quality; png/tiff
/// take nothing. Returned as the two parallel arrays savev wants.
export function saveOptions(key) {
    if (key === 'jpeg' || key === 'webp' || key === 'avif')
        return [['quality'], ['92']];
    return [[], []];
}

/// What the exported file is called. Same stem, new extension; a page
/// number when a specific PDF page is exported; and never the same
/// name as the source — converting photo.png to png must not invite
/// an overwrite of the original.
export function exportName(sourcePath, ext, page = null) {
    const base = sourcePath.split('/').pop() ?? 'export';
    const dot = base.lastIndexOf('.');
    const stem = dot > 0 ? base.slice(0, dot) : base;
    const suffix = page !== null ? ` — page ${page}` : '';
    let name = `${stem}${suffix}.${ext}`;
    if (name === base) name = `${stem} (exported).${ext}`;
    return name;
}

/// PDF points -> pixels at the target dpi. 150 dpi is the export
/// default: crisp enough to read, not a 100 MB surprise.
export function rasterScale(dpi = 150) {
    return dpi / 72;
}

/// The extent a page occupies once a pending rotation is applied, in
/// PDF points, plus the normalized angle.
///
/// One function for the view and for the export, because #299 was
/// exactly the two disagreeing: the canvas drew the rotation and the
/// exporter did not, so an agent that rotated a page and exported it got
/// an unrotated PNG and an `ok`. Quarter turns swap width and height;
/// half turns do not. The caller rounds — the view rounds to the nearest
/// pixel, the exporter rounds up so nothing is clipped off the edge.
export function rotatedExtent(width, height, angle = 0) {
    const deg = ((Math.round(angle) % 360) + 360) % 360;
    return deg % 180 !== 0
        ? {width: height, height: width, angle: deg}
        : {width, height, angle: deg};
}

/// Everything an export of ONE display page renders: which ORIGINAL
/// page, at which pending rotation, at what dpi.
///
/// The page and the angle travel together on purpose (#299). They used
/// to be decided in different places — the exporter picked
/// `order[displayIndex]` and nothing picked an angle at all — so a page
/// the person had rotated exported unrotated, with an `ok`, while the
/// view, the thumbnails and the window subtitle all showed the turn. A
/// caller can no longer choose a page without also choosing its
/// rotation, because there is one value and it carries both.
///
/// `rotations` is the pending DOCUMENT rotation map — the edit qpdf
/// would write. The R key's view rotation is not in it and must not be:
/// that one is a way of looking at the page, not a change to it.
export function pageExportRender(order, rotations, displayIndex, dpi = 150) {
    const page = order[displayIndex];
    return {page, angle: rotationOf(rotations, page), dpi};
}

/// All-pages export names: the page number is part of every file.
export function pageExportNames(sourcePath, ext, pageCount) {
    return Array.from({length: pageCount},
        (_, i) => exportName(sourcePath, ext, i + 1));
}
