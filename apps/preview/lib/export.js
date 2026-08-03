// Export decisions — pure. The I/O half (rendering pages, pixbuf
// saving) lives in lisa-preview.js; what can be got wrong quietly is
// here and tested: which formats we OFFER (grounded in what the
// machine's pixbuf can actually write, injected — never assumed), what
// the exported file is called, and the rasterization arithmetic.

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

/// All-pages export names: the page number is part of every file.
export function pageExportNames(sourcePath, ext, pageCount) {
    return Array.from({length: pageCount},
        (_, i) => exportName(sourcePath, ext, i + 1));
}
