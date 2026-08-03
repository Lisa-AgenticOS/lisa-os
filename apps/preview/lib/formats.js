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

/// Text-ish files Space can peek at (slice: text/html/card). Extension
/// lists, same reasoning as images; the binary sniff in lib/peek.js is
/// the second gate for files that lie.
const TEXT_EXTENSIONS = [
    'txt', 'md', 'markdown', 'rst', 'log', 'csv', 'tsv',
    'json', 'jsonl', 'xml', 'yaml', 'yml', 'toml', 'ini', 'conf', 'desktop',
    'sh', 'bash', 'zsh', 'py', 'js', 'mjs', 'ts', 'rs', 'c', 'h', 'cpp',
    'hpp', 'css', 'go', 'rb', 'lua', 'sql', 'service', 'patch', 'diff',
];

const HTML_EXTENSIONS = ['html', 'htm', 'xhtml'];

/// Media Space can play (#200). Every container/codec here maps to a
/// GStreamer plugin VERIFIED on the reference image 2026-08-03
/// (isomp4, matroska, ogg, vorbis, opus, flac, mpg123, wavparse,
/// openh264+libav, vpx) — the list is grounded in what the machine
/// decodes, not in what a file manager might send.
const AUDIO_EXTENSIONS = ['flac', 'm4a', 'mp3', 'oga', 'ogg', 'opus', 'wav'];
const VIDEO_EXTENSIONS = ['m4v', 'mkv', 'mov', 'mp4', 'webm'];

/// MIME types for the .desktop file. Generated from the same lists so
/// the two cannot drift — a viewer registered for a type it cannot open
/// is worse than not registering, because the file manager stops
/// offering anything else.
///
/// DELIBERATELY images + pdf only, NOT text or html: the previewer
/// (Space) peeks at more kinds than the .desktop claims. Registering
/// text/plain or text/html would put Preview in every Open With list
/// and risk making a peek tool the double-click default over the
/// editor and the browser.
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

/// What kind of thing a path is, by extension: 'image', 'document',
/// 'text', 'html', or null when Preview does not recognise it.
///
/// Null rather than a guess — but null no longer means "show nothing":
/// the previewer path renders a generic file card for it, because
/// Nautilus sends Space for ANY selected file and silence reads as
/// broken (the lesson this app keeps re-learning).
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
    if (TEXT_EXTENSIONS.includes(ext))
        return 'text';
    if (HTML_EXTENSIONS.includes(ext))
        return 'html';
    if (AUDIO_EXTENSIONS.includes(ext))
        return 'audio';
    if (VIDEO_EXTENSIONS.includes(ext))
        return 'video';
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
