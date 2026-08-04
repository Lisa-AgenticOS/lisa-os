// The parts of a message that are not prose (#211, #169).
//
// Pure: no GNOME imports, so the whole attachment model — what is
// listed, how big it is, what its bytes are, and what it is safe to
// call the file — is unit-tested without a Maildir or a window.
//
// # Why this is a separate module
//
// `rfc822.js` parses; this decides. The split matters because the
// decisions here are security decisions and they read better where a
// reviewer can find them: an attachment's filename, size and type are
// written by whoever sent the message, and every one of them ends up
// somewhere consequential — a label, a path, a viewer's idea of what
// the file is. Nothing here trusts a single one of them.
//
// # What it deliberately does not do
//
// It does not decode a part into text, and it does not hand anything to
// a model. `read_message` gives an agent `body`, which is prose;
// attachment BYTES are a file the person asked for, and a summariser
// that could be handed a PDF is a summariser that can be handed
// anything a sender likes.

import {
    MAX_PART_BYTES, decodeTransfer, decodedSize, headerParam, leafParts,
} from './rfc822.js';

/// Characters that never belong in a filename or a label: the C0 and C1
/// controls (NUL included) and the bidi overrides.
const HOSTILE_CHARS =
    /[\u0000-\u001F\u007F-\u009F\u200E\u200F\u202A-\u202E\u2066-\u2069]/g;

/// Everything attached to a message: filename, type, size, content id,
/// disposition, and the part path that addresses the bytes.
///
/// The body is NOT an attachment. A `text/plain` or `text/html` leaf
/// with no filename and no `attachment` disposition is what the reading
/// pane is already showing; listing it again would put a "part 1.txt"
/// under every message anyone has ever sent.
///
/// Everything else is kept, including inline images — see
/// `listedAttachments` for which of them a window should show. A PDF
/// arriving with `Content-Disposition: inline` (which real invoicing
/// systems send) is an attachment by any useful definition, and
/// dropping it is the bug this module exists to fix.
export function attachments(raw) {
    const out = [];
    for (const leaf of leafParts(raw)) {
        const ctype = leaf.headers.get('content-type');
        const mimeType = normaliseType(ctype);
        // A multipart leaf only happens at the depth bound — a
        // container, not a file.
        if (mimeType.startsWith('multipart/'))
            continue;

        const disposition = leaf.headers.get('content-disposition');
        const declared = /^\s*attachment\b/i.test(disposition) ? 'attachment'
            : /^\s*inline\b/i.test(disposition) ? 'inline' : '';
        // `filename` on the disposition is the right place; `name` on
        // the Content-Type is the older spelling, and plenty of mailers
        // still send only that one.
        const name = headerParam(disposition, 'filename') || headerParam(ctype, 'name');

        const isBodyText = mimeType === 'text/plain' || mimeType === 'text/html';
        if (isBodyText && !name && declared !== 'attachment')
            continue;

        const encoding = leaf.headers.get('content-transfer-encoding').trim().toLowerCase();
        out.push({
            path: leaf.path,
            filename: displayName(name),
            mimeType,
            size: decodedSize(leaf.body, encoding),
            encoding,
            contentId: leaf.headers.get('content-id').trim().replace(/^<|>$/g, ''),
            // Nothing declared: a part with a filename is an attachment
            // and one without is inline. Guessing the other way round
            // would hide the PDF this issue is about.
            disposition: declared || (name ? 'attachment' : 'inline'),
        });
    }
    return out;
}

/// The bytes of one attachment, addressed by its part path.
///
/// `null` when the path addresses no part — the caller may be holding a
/// path from a message that has since been re-synced — and `null` again
/// when the part is LARGER THAN THE BOUND. That second one is #238:
/// `decodedSize` (the number in the row) is uncapped, `decodeTransfer`
/// stops at `maxBytes`, and nothing compared them, so a 68 MB
/// attachment showed 71,303,169 bytes in the list and Save As wrote
/// 67,108,864 of them with no error anywhere. Three quarters of
/// somebody's file, written under the name of the whole thing, is the
/// worst kind of failure this app could have: it opens far enough to
/// look like their problem.
///
/// So the whole part or nothing. The caller decides what a bound it
/// cannot meet means — the window raises it for a save the person
/// explicitly asked for, and says so when even that is not enough.
export function attachmentBytes(raw, path, options = {}) {
    if (!Array.isArray(path))
        return null;
    const want = path.join('.');
    for (const leaf of leafParts(raw)) {
        if (leaf.path.join('.') !== want)
            continue;
        const encoding = leaf.headers.get('content-transfer-encoding');
        const max = Math.max(0, options.maxBytes ?? MAX_PART_BYTES);
        if (decodedSize(leaf.body, encoding) > max)
            return null;
        return decodeTransfer(leaf.body, encoding, options);
    }
    return null;
}

/// The attachments a window should put on screen.
///
/// An inline image with a content id is body imagery — the logo in a
/// newsletter's footer, the tracking pixel under it — referenced by
/// `cid:` from the HTML. Listing those turns a marketing email into
/// twenty attachments and buries the one file that matters.
///
/// Everything else stays, including an inline PDF: `inline` on a
/// document means "show it if you can", not "this is decoration".
export function listedAttachments(list) {
    return (list ?? []).filter(
        (a) => !(a.disposition === 'inline' && a.contentId && a.mimeType.startsWith('image/')));
}

/// A filename it is safe to build a path out of.
///
/// The name came from the sender, so this is the discipline
/// `messagePath` in maildir.js already applies to a message id:
/// separators, traversal, NUL and control characters are refused rather
/// than escaped, and what comes back is a plain name with no path in
/// it. It is still only ever joined onto a directory this app made or a
/// person chose in a file dialog — a sanitiser is the second line of
/// defence, not the first.
///
/// Three things beyond the obvious:
///
///   - **A leading dot is dropped.** `.bashrc` is not traversal, but an
///     attachment that saves itself invisibly into a directory is a
///     trick, not a filename.
///   - **Bidi overrides go.** A right-to-left override renders
///     `invoice\u202Egnp.exe` as `invoiceexe.png` in every file manager on earth.
///     That is the oldest attachment trick there is, and it survives
///     being saved.
///   - **Length is capped, extension kept.** A 400-character name is a
///     path-limit error at save time, and the extension is the part
///     that decides which viewer opens it.
///
/// Spaces, parentheses, commas and non-ASCII letters are left alone:
/// `Fatura F-2026-0007 (mars).pdf` is what the sender called it and
/// what the person is looking for.
export function safeFilename(name, mimeType = '') {
    let base = String(name ?? '').split(/[/\\]/).pop() ?? '';
    base = base.replace(HOSTILE_CHARS, '').trim();
    // A name that is only dots is traversal wearing a filename.
    if (/^\.+$/.test(base))
        base = '';
    base = base.replace(/^\.+/, '').trim();
    if (base.length > 120) {
        const dot = base.lastIndexOf('.');
        const ext = dot > 0 && base.length - dot <= 11 ? base.slice(dot) : '';
        base = base.slice(0, 120 - ext.length) + ext;
    }
    return base || `attachment.${extensionFor(mimeType)}`;
}

/// A type good enough to show in a label, from a claim in a header.
///
/// A MIME type is a token, a slash and a token — so anything outside
/// that is not a type, and dropping it means a `Content-Type` of
/// `application/pdf<b>` cannot smuggle markup into a widget that
/// renders Pango. Bounded, lowercased, and `text/plain` when there is
/// nothing usable, which is what RFC 2045 says a part with no
/// Content-Type is.
function normaliseType(value) {
    const declared = String(value ?? '').split(';')[0].trim().toLowerCase();
    const clean = declared.replace(/[^a-z0-9!#$&^_.+/-]/g, '').slice(0, 80);
    return clean || 'text/plain';
}

/// A filename as it should appear on screen.
///
/// Control characters and bidi overrides are stripped here too, because
/// the label is what a person reads before deciding to open the file: a
/// name that lies in a label has already done its work, whatever the
/// saved file ends up being called.
function displayName(name) {
    return String(name ?? '')
        .replace(HOSTILE_CHARS, '')
        .replace(/\s+/g, ' ')
        .trim()
        .slice(0, 160);
}

/// An extension for a part that gave no usable filename.
///
/// From the declared type, which is a claim — but the claim only
/// decides what OUR invented name ends in, and Preview decides what it
/// can open by extension. A wrong guess shows a file card; a guess that
/// trusted the sender's own name would not be a guess.
function extensionFor(mimeType) {
    const [type, rawSub = ''] = String(mimeType ?? '').toLowerCase().split('/');
    const sub = rawSub.split(';')[0].split('+')[0].trim().replace(/[^a-z0-9-]/g, '');
    const known = {
        jpeg: 'jpg', plain: 'txt', markdown: 'md', 'octet-stream': 'bin', 'x-png': 'png',
    };
    if (known[sub])
        return known[sub];
    if (type === 'text' && sub)
        return sub.length <= 5 ? sub : 'txt';
    const usable = ['pdf', 'png', 'gif', 'svg', 'webp', 'zip', 'csv', 'ics', 'json', 'xml', 'heic'];
    return usable.includes(sub) ? sub : 'bin';
}
