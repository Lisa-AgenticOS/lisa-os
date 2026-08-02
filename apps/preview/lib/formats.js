// What Preview can open, and how it decides. Pure — no gi:// imports —
// so it runs under node in `just shell-test`.
//
// The list is not aspirational. Every image format here was verified
// present on the reference device on 2026-08-02 by asking GdkPixbuf for
// its own format list rather than by trusting a package name:
//
//     ani avif bmp dds gif heic icns ico jpeg jxl openexr png pnm qoi
//     qtif raw svg tga tiff webp xbm xpm
//
// That is a wider set than the browser's — WebKit on the same machine
// refuses AVIF while GdkPixbuf decodes it — which is why Preview asks
// the pixbuf loaders rather than reusing anything from Surfer.

/// Extensions we claim, grouped by how the app has to render them.
///
/// Kept as extensions rather than MIME types because that is what a
/// file manager hands over on the command line, and because
/// `Gio.content_type_guess` on a file with no read permission returns a
/// confident wrong answer from the name alone.
const IMAGE_EXTENSIONS = [
    'avif', 'bmp', 'dds', 'gif', 'heic', 'heif', 'ico', 'jfif', 'jpe',
    'jpeg', 'jpg', 'jxl', 'png', 'pnm', 'qoi', 'svg', 'tga', 'tif',
    'tiff', 'webp', 'xbm', 'xpm',
];

const DOCUMENT_EXTENSIONS = ['pdf'];

/// MIME types for the .desktop file. Generated from the same lists so
/// the two cannot drift — a viewer registered for a type it cannot open
/// is worse than not registering, because the file manager stops
/// offering anything else.
/// De-duplicated, because several extensions share one type (.jpg/.jpeg/
/// .jpe/.jfif are all image/jpeg). A .desktop that lists image/jpeg four
/// times is not fatal, but it is the kind of sloppiness that makes a
/// reader distrust the rest of the file — and `update-desktop-database`
/// has no reason to tolerate it.
export const MIME_TYPES = [...new Set([
    ...IMAGE_EXTENSIONS.map(e => `image/${({
        jpg: 'jpeg', jpe: 'jpeg', jfif: 'jpeg', tif: 'tiff',
        svg: 'svg+xml', ico: 'vnd.microsoft.icon', heic: 'heif',
    })[e] ?? e}`),
    'application/pdf',
])];

/// What kind of thing a path is, by extension: 'image', 'document', or
/// null when Preview should not claim it.
///
/// Null rather than a guess. A viewer that opens an unknown file and
/// shows a grey rectangle has told the user their file is corrupt, when
/// what happened is that we did not recognise it.
export function kindOf(path) {
    if (typeof path !== 'string')
        return null;
    const base = path.split('/').pop() ?? '';
    const dot = base.lastIndexOf('.');
    // A leading dot is a hidden file, not an extension: ".webp" is a
    // file named .webp, and treating it as one would open the wrong
    // thing on a directory of dotfiles.
    if (dot <= 0)
        return null;
    const ext = base.slice(dot + 1).toLowerCase();
    if (IMAGE_EXTENSIONS.includes(ext))
        return 'image';
    if (DOCUMENT_EXTENSIONS.includes(ext))
        return 'document';
    return null;
}

/// Every sibling of `path` Preview can open, sorted, with the index of
/// `path` itself — the model behind ← / → browsing a folder.
///
/// Takes the directory listing as an argument rather than reading it, so
/// the ordering rule is testable without a filesystem.
export function siblings(path, entries) {
    const dir = path.slice(0, path.lastIndexOf('/') + 1);
    const openable = entries
        .filter(e => kindOf(e) !== null)
        .sort((a, b) => a.localeCompare(b, undefined, {numeric: true, sensitivity: 'base'}));
    const name = path.slice(dir.length);
    return {files: openable.map(e => dir + e), index: openable.indexOf(name)};
}
