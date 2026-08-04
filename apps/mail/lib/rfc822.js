// Enough RFC 822/2045/2047 to read a message, and no more.
//
// Pure: no GNOME imports, so it runs under gjs, node and jsc alike and
// is unit-tested on any dev host. Everything here is parsing UNTRUSTED
// input — a message is written by whoever sent it — so every function
// is total: malformed input yields something empty or literal, never an
// exception, because a mail client that throws on one bad message shows
// you an empty inbox.
//
// Deliberately not a MIME library. A full one belongs in a crate, not in
// an app; what a reading pane and an agent tool need is the headers, one
// readable body, and the flags — which is small enough to get right and
// to test.
//
// # The input is BYTES, one character per byte
//
// Everything here takes a message as `messageText` produces it: a
// string in which every character is one byte of the file. That is what
// a message file is, and it is the only form in which `charset` can
// still be honoured — a UTF-8 decode applied before the message has
// said what its charset is destroys every part that was not UTF-8
// (#232). Hand these functions an already-decoded string and the
// non-ASCII parts of it are, at best, left alone (see `decodeText`).

/// A message file's bytes as the string this module parses: ONE
/// CHARACTER PER BYTE.
///
/// The byte→character step, in the one place, exported so a test can
/// exercise it. Everything below assumes its input is a byte string,
/// because a message file is a byte stream: the structure (boundaries,
/// header names, base64) is ASCII, and the part bodies stay intact
/// until `charset` says what they are.
///
/// The app used to do this with `new TextDecoder('utf-8')`, which is
/// LOSSY — every byte sequence that is not valid UTF-8 becomes U+FFFD
/// and the original is gone. That made the Latin-1 branch of
/// `decodeBytes` unreachable and turned `charset=ISO-8859-1; 8bit`
/// mail into `P<FFFD>rsh<FFFD>ndetje` (#232; 198 of 25,207 messages on
/// the reference device). Latin-1 is the only decoding that is a
/// bijection on bytes, so it is the only one that may be applied
/// before the message has said what its charset is.
export function messageText(bytes) {
    const src = bytes ?? [];
    let out = '';
    // In chunks: `String.fromCharCode.apply` on a multi-megabyte array
    // blows the argument limit on every engine.
    for (let i = 0; i < src.length; i += 8192) {
        const chunk = typeof src.subarray === 'function'
            ? src.subarray(i, i + 8192)
            : src.slice(i, i + 8192);
        out += String.fromCharCode.apply(null, chunk);
    }
    return out;
}

/// Split a raw message into headers and body at the first blank line.
///
/// Handles CRLF and LF: a Maildir written by one tool and read by
/// another routinely mixes them.
export function splitMessage(raw) {
    const text = String(raw ?? '');
    const m = text.match(/\r?\n\r?\n/);
    if (!m)
        return {headerText: text, body: ''};
    return {
        headerText: text.slice(0, m.index),
        body: text.slice(m.index + m[0].length),
    };
}

/// Parse a header block into a lowercase-keyed map.
///
/// Folded continuation lines (a line starting with space or tab belongs
/// to the header above) are joined — a long Subject arrives folded and
/// reading only the first line truncates it mid-word.
///
/// Repeated headers keep the FIRST occurrence for single-value lookups
/// but every value is retained in `all`, because `Received` matters in
/// order and a duplicated `From` is a thing worth being able to see.
export function parseHeaders(headerText) {
    const lines = String(headerText ?? '').split(/\r?\n/);
    const unfolded = [];
    for (const line of lines) {
        if (/^[ \t]/.test(line) && unfolded.length > 0)
            unfolded[unfolded.length - 1] += ' ' + line.trim();
        else
            unfolded.push(line);
    }
    const map = {};
    const all = {};
    for (const line of unfolded) {
        const i = line.indexOf(':');
        if (i <= 0)
            continue;
        const key = line.slice(0, i).trim().toLowerCase();
        const value = line.slice(i + 1).trim();
        if (!(key in map))
            map[key] = value;
        (all[key] ??= []).push(value);
    }
    return {get: (k) => map[String(k).toLowerCase()] ?? '', map, all};
}

/// Decode RFC 2047 encoded words: `=?UTF-8?B?…?=` and `=?…?Q?…?=`.
///
/// Real subjects are full of these — every non-ASCII subject line from
/// every mailer — and showing the raw form is how a mail client looks
/// broken on its first screenful.
///
/// Adjacent encoded words are joined without the whitespace between
/// them, per the spec: a subject split across two words must not gain a
/// space in the middle.
export function decodeWords(input) {
    const text = String(input ?? '');
    if (!text.includes('=?'))
        return text;
    const pattern = /=\?([^?]+)\?([BbQq])\?([^?]*)\?=/g;
    let out = '';
    let last = 0;
    let prevEnd = -1;
    for (const m of text.matchAll(pattern)) {
        const between = text.slice(last, m.index);
        // Whitespace BETWEEN two encoded words is separator, not content.
        if (!(prevEnd === last && /^[ \t]*$/.test(between)))
            out += between;
        out += decodeOneWord(m[1], m[2].toUpperCase(), m[3]);
        last = m.index + m[0].length;
        prevEnd = last;
    }
    return out + text.slice(last);
}

function decodeOneWord(charset, encoding, payload) {
    try {
        const bytes = encoding === 'B'
            ? base64Bytes(payload)
            : quotedPrintableBytes(payload.replace(/_/g, ' '));
        return decodeBytes(bytes, charset);
    } catch {
        // Undecodable is shown as written. A mangled subject is a
        // nuisance; a thrown exception is an empty inbox.
        return payload;
    }
}

function base64Bytes(s) {
    const clean = s.replace(/[^A-Za-z0-9+/=]/g, '');
    const table = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const out = [];
    let bits = 0;
    let acc = 0;
    for (const ch of clean) {
        if (ch === '=')
            break;
        const v = table.indexOf(ch);
        if (v < 0)
            continue;
        acc = (acc << 6) | v;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push((acc >> bits) & 0xff);
        }
    }
    return out;
}

function quotedPrintableBytes(s) {
    const out = [];
    for (let i = 0; i < s.length; i++) {
        if (s[i] === '=' && i + 2 < s.length && /^[0-9A-Fa-f]{2}$/.test(s.slice(i + 1, i + 3))) {
            out.push(parseInt(s.slice(i + 1, i + 3), 16));
            i += 2;
        } else {
            out.push(s.charCodeAt(i) & 0xff);
        }
    }
    return out;
}

/// Bytes → string. UTF-8 decoded properly; anything else read as
/// Latin-1, which is wrong for some charsets and readable for most —
/// and readable-but-imperfect beats a row of replacement characters.
function decodeBytes(bytes, charset) {
    const cs = String(charset ?? '').toLowerCase();
    if (cs.startsWith('utf-8') || cs.startsWith('utf8'))
        return utf8(bytes);
    return bytes.map((b) => String.fromCharCode(b)).join('');
}

function utf8(bytes) {
    let out = '';
    for (let i = 0; i < bytes.length;) {
        const b = bytes[i];
        if (b < 0x80) {
            out += String.fromCharCode(b);
            i += 1;
        } else if (b >= 0xc0 && b < 0xe0 && i + 1 < bytes.length) {
            out += String.fromCharCode(((b & 0x1f) << 6) | (bytes[i + 1] & 0x3f));
            i += 2;
        } else if (b >= 0xe0 && b < 0xf0 && i + 2 < bytes.length) {
            out += String.fromCharCode(
                ((b & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f));
            i += 3;
        } else if (b >= 0xf0 && i + 3 < bytes.length) {
            const cp = ((b & 0x07) << 18) | ((bytes[i + 1] & 0x3f) << 12) |
                       ((bytes[i + 2] & 0x3f) << 6) | (bytes[i + 3] & 0x3f);
            const off = cp - 0x10000;
            out += String.fromCharCode(0xd800 + (off >> 10), 0xdc00 + (off & 0x3ff));
            i += 4;
        } else {
            out += '�';
            i += 1;
        }
    }
    return out;
}

/// `"Jane Doe" <jane@example.com>` → `{name, address}`.
///
/// The display name is the part people recognise in a list, and it is
/// also attacker-controlled: `"security@yourbank.com" <evil@x.test>` is
/// a real phishing shape. So both parts are kept and the UI shows the
/// address too — never the name alone.
export function parseAddress(raw) {
    const text = decodeWords(String(raw ?? '')).trim();
    const angle = text.match(/^(.*)<([^>]*)>\s*$/);
    if (angle) {
        return {
            name: angle[1].trim().replace(/^"(.*)"$/, '$1').trim(),
            address: angle[2].trim(),
        };
    }
    return {name: '', address: text};
}

/// Strip HTML to something readable, for a message with no text/plain
/// part.
///
/// Not a renderer: script and style contents are dropped entirely
/// (their text is code, not prose, and an agent summarising a message
/// should never be handed a script body), tags become nothing, block
/// elements become line breaks, and entities are decoded.
export function htmlToText(html) {
    return String(html ?? '')
        .replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1>/gi, ' ')
        .replace(/<!--[\s\S]*?-->/g, ' ')
        .replace(/<\/?(p|div|tr|br|li|h[1-6]|table)\b[^>]*>/gi, '\n')
        .replace(/<[^>]+>/g, '')
        .replace(/&nbsp;/gi, ' ')
        .replace(/&amp;/gi, '&')
        .replace(/&lt;/gi, '<')
        .replace(/&gt;/gi, '>')
        .replace(/&quot;/gi, '"')
        .replace(/&#(\d+);/g, (_, d) => String.fromCharCode(Number(d)))
        .replace(/[ \t]+\n/g, '\n')
        .replace(/\n{3,}/g, '\n\n')
        .trim();
}

/// The readable body of a message: the text/plain part if there is one,
/// otherwise text/html flattened, otherwise the raw body.
///
/// Walks multipart bodies one level deep by boundary, which covers
/// `multipart/alternative` and `multipart/mixed` — the two shapes almost
/// every real message uses. Nested deeper than that, it falls back to
/// the whole body rather than returning nothing.
export function readableBody(raw) {
    const {headerText, body} = splitMessage(raw);
    return bodyOfPart(parseHeaders(headerText), body);
}

/// Both halves of a message body: the HTML as sent, and readable text.
///
/// `readableBody` flattens HTML to text, which is right for a model and
/// wrong for a person — a real newsletter arrives as a table of images
/// and reads, flattened, like a ransom note. So the window gets `html`
/// to render and `text` to fall back on, and the Agent Bus tools keep
/// using the flattened form: a model should be handed prose, not markup
/// it has to parse and which can carry instructions in an attribute.
///
/// `html` is `null` when the message has no HTML part. It is returned
/// **undoctored** — sanitising by string surgery here would be a false
/// promise, because the thing that makes rendering safe is the renderer
/// being told not to run scripts and not to fetch remote resources, not
/// a regex that thinks it can outwit an HTML parser.
export function renderableBody(raw) {
    const {headerText, body} = splitMessage(raw);
    const headers = parseHeaders(headerText);
    const parts = collectParts(headers, body);
    const plain = parts.find((p) => p.type.startsWith('text/plain') && p.text.trim());
    const html = parts.find((p) => p.type.startsWith('text/html') && p.text.trim());
    // PROSE ONLY (#221). This was `parts.find((p) => p.text?.trim())`,
    // which is the same defect `bodyOfPart` carried: a message whose
    // text parts exist and are empty resolved to whatever file came
    // next.
    const fallback = parts.find((p) => isProse(p.type) && p.text?.trim());
    return {
        html: html ? html.text : null,
        text: plain
            ? plain.text
            : html
                ? htmlToText(html.text)
                : (fallback ? fallback.text : ''),
    };
}

/// Flatten a message into its leaf parts, decoded.
///
/// Same walk as `bodyOfPart`, but it keeps every leaf instead of
/// choosing one, because the caller wants plain AND html rather than
/// whichever ranks higher.
function collectParts(headers, body) {
    const out = [];
    walkParts(headers, body, [], 0, out);
    return out.map((leaf) => {
        const type = leaf.headers.get('content-type').toLowerCase();
        // A file is not decoded into text at all. Cheaper — a 20 MB PDF
        // is not turned into a 20 MB string to be discarded — and it
        // means the binary is not sitting in a list somebody might
        // later `find` through (#221).
        return {type, text: isProse(type) ? decodePart(leaf.headers, leaf.body) : ''};
    });
}

/// Which decoded parts may stand in as a message's readable body.
///
/// **text/\***, and a part that declares no Content-Type at all — RFC
/// 2045 says that is `text/plain`. A nested **multipart/\*** counts
/// too, because the recursion has already applied this same rule one
/// level down: that is how a `multipart/alternative` inside a
/// `multipart/mixed` still supplies the body of an invoice mail, and
/// dropping it would have been the regression a naive "text/* only"
/// filter introduced.
///
/// Everything else is a FILE, and the whole of #221 is that the
/// fallback did not ask. `decoded.find((d) => d.text?.trim())` has no
/// type filter, so ordinary "here is the document" mail — where the
/// text parts exist and are EMPTY — resolved to the PDF, the JPEG or
/// the .docx. Measured on the reference device: 117 of 25,207 messages,
/// 90 MB of decoded binary, a `read_message` body of 3,145,615
/// characters starting `PK\x03\x04`, and `%PDF-1.4 %Ǭ… /FlateDecode`
/// in the message list's preview column. The bytes of a file a stranger
/// sent are not prose and this module must not hand them out as any.
function isProse(type) {
    const t = String(type ?? '').trim();
    return t === '' || t.startsWith('text/') || t.startsWith('multipart/');
}

/// How deep the part tree is walked, and how many leaves are kept.
///
/// Both are bounds, not limits anyone will meet: real mail nests two or
/// three deep with a handful of parts. A message is written by whoever
/// sent it, and forty nested multiparts or ten thousand empty parts is
/// a cheap thing to send and an expensive thing to walk.
const MAX_DEPTH = 6;
const MAX_PARTS = 200;

/// Every leaf of the MIME tree, UNDECODED, each with the index path that
/// addresses it.
///
/// The path is how a caller comes back later for the bytes without the
/// message having to be kept in memory: `attachments()` lists what is
/// there, `attachmentBytes()` extracts one. Both walk with this
/// function, so the indices cannot disagree — a second walker with its
/// own idea of which chunks count would hand out addresses that resolve
/// to a different part, which is the kind of bug that shows up as one
/// user's invoice arriving as another attachment's bytes.
///
/// Indices are positions in the raw boundary split, INCLUDING the
/// preamble ("This is a multi-part message in MIME format."). They are
/// opaque addresses, not part numbers, and stable for a given message.
export function leafParts(raw) {
    const {headerText, body} = splitMessage(raw);
    const out = [];
    walkParts(parseHeaders(headerText), body, [], 0, out);
    return out;
}

function walkParts(headers, body, path, depth, out) {
    if (out.length >= MAX_PARTS)
        return;
    const ctype = headers.get('content-type');
    const boundary = ctype.match(/boundary\s*=\s*"?([^";]+)"?/i)?.[1];
    if (boundary && depth < MAX_DEPTH) {
        const chunks = body.split(new RegExp(`--${escapeRe(boundary)}(?:--)?\\r?\\n?`));
        for (let i = 0; i < chunks.length; i++) {
            if (!chunks[i].trim())
                continue;
            const {headerText: ph, body: pb} = splitMessage(chunks[i]);
            // Prose with no headers is the preamble or the epilogue,
            // which belong to nobody.
            if (!ph.includes(':'))
                continue;
            walkParts(parseHeaders(ph), pb, [...path, i], depth + 1, out);
            if (out.length >= MAX_PARTS)
                return;
        }
        return;
    }
    out.push({path, headers, body});
}

/// One parameter of a structured header, decoded — `filename` out of a
/// Content-Disposition, `name` out of a Content-Type.
///
/// Four spellings, all of them real mail:
///
///   - `filename="fatura.pdf"` — the plain one;
///   - `filename*=UTF-8''Fatura%20%28mars%29.pdf` — RFC 2231, the only
///     spelling that carries a charset;
///   - `filename*0*=…; filename*1*=…` — RFC 2231 continuations, which
///     must be joined BEFORE they are percent-decoded, because a
///     multi-byte character can be split across two segments;
///   - `filename="=?UTF-8?Q?…?="` — an encoded word where the spec says
///     one may not appear, sent by half the world anyway.
///
/// Returns '' for anything it cannot find. Never throws: this parses a
/// header written by the sender.
export function headerParam(value, key) {
    const params = parseParams(value);
    const want = String(key ?? '').toLowerCase();
    if (!want)
        return '';

    // Continuations first — `filename*0*` beats a bare `filename`.
    const segments = [];
    let extended = false;
    for (let i = 0; ; i++) {
        const ext = params.get(`${want}*${i}*`);
        const plain = params.get(`${want}*${i}`);
        if (ext !== undefined) {
            segments.push(ext);
            extended = true;
        } else if (plain !== undefined) {
            segments.push(plain);
        } else {
            break;
        }
    }
    if (segments.length > 0) {
        const joined = segments.join('');
        return extended ? decodeExtended(joined) : decodeWords(joined);
    }

    const ext = params.get(`${want}*`);
    if (ext !== undefined)
        return decodeExtended(ext);
    const plain = params.get(want);
    if (plain !== undefined)
        return decodeWords(plain);
    return '';
}

/// `UTF-8''Fatura%20%28mars%29.pdf` → `Fatura (mars).pdf`.
function decodeExtended(value) {
    let text = String(value ?? '');
    let charset = 'utf-8';
    const prefixed = text.match(/^([^']*)'([^']*)'([\s\S]*)$/);
    if (prefixed) {
        charset = prefixed[1] || 'utf-8';
        text = prefixed[3];
    }
    const bytes = [];
    for (let i = 0; i < text.length; i++) {
        if (text[i] === '%' && /^[0-9A-Fa-f]{2}$/.test(text.slice(i + 1, i + 3))) {
            bytes.push(parseInt(text.slice(i + 1, i + 3), 16));
            i += 2;
        } else {
            bytes.push(text.charCodeAt(i) & 0xff);
        }
    }
    return decodeBytes(bytes, charset);
}

/// `type/subtype; a=b; c="d;e"` → a map of the parameters, lowercased
/// keys, quotes and backslash escapes removed.
///
/// Hand-written rather than a `split(';')`, because a quoted parameter
/// value may contain a semicolon — `filename="invoice; final.pdf"` is
/// legal and splitting on `;` turns it into two broken parameters.
function parseParams(value) {
    const s = String(value ?? '');
    const out = new Map();
    let i = s.indexOf(';');
    while (i >= 0 && i < s.length) {
        i += 1;
        let j = i;
        while (j < s.length && s[j] !== '=' && s[j] !== ';')
            j += 1;
        const name = s.slice(i, j).trim().toLowerCase();
        if (s[j] !== '=') {
            // A parameter with no value: skip to the next separator.
            i = s.indexOf(';', j);
            continue;
        }
        j += 1;
        let val = '';
        if (s[j] === '"') {
            j += 1;
            while (j < s.length && s[j] !== '"') {
                if (s[j] === '\\' && j + 1 < s.length)
                    j += 1;
                val += s[j];
                j += 1;
            }
            j += 1;
        } else {
            const start = j;
            while (j < s.length && s[j] !== ';')
                j += 1;
            val = s.slice(start, j).trim();
        }
        if (name)
            out.set(name, val);
        i = s.indexOf(';', j);
    }
    return out;
}

/// The most bytes one part is ever decoded into.
///
/// A bound, not a policy: a reading pane that decodes whatever a sender
/// says to is a reading pane a sender can stop.
export const MAX_PART_BYTES = 64 * 1024 * 1024;

const B64VALUE = (() => {
    const table = new Int8Array(256).fill(-1);
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    for (let i = 0; i < alphabet.length; i++)
        table[alphabet.charCodeAt(i)] = i;
    return table;
})();

/// Transfer-decode one part into BYTES.
///
/// Separate from the charset decoding on purpose: `decodePart` turns a
/// part into a string, which is right for prose and destroys a PDF —
/// every byte above 0x7f becomes U+FFFD and the file no longer opens.
/// An attachment is a byte stream and stays one until somebody saves it.
///
/// Total, and bounded by `maxBytes`: malformed base64 decodes to
/// whatever the valid characters said, and a part larger than the bound
/// stops at the bound rather than being decoded whole.
export function decodeTransfer(text, encoding, {maxBytes = MAX_PART_BYTES} = {}) {
    const s = String(text ?? '');
    const enc = String(encoding ?? '').trim().toLowerCase();
    const cap = Math.min(capacityFor(s.length, enc), Math.max(0, maxBytes));
    const out = new Uint8Array(cap);
    let n = 0;

    if (enc === 'base64') {
        let acc = 0;
        let bits = 0;
        for (let i = 0; i < s.length && n < cap; i++) {
            const c = s.charCodeAt(i);
            // Padding ends the data; anything outside the alphabet
            // (newlines, spaces, the sender's stray punctuation) is
            // skipped rather than treated as an error.
            if (c === 0x3d)
                break;
            const v = c < 256 ? B64VALUE[c] : -1;
            if (v < 0)
                continue;
            acc = ((acc << 6) | v) & 0xffffff;
            bits += 6;
            if (bits >= 8) {
                bits -= 8;
                out[n++] = (acc >> bits) & 0xff;
            }
        }
    } else if (enc === 'quoted-printable') {
        for (let i = 0; i < s.length && n < cap; i++) {
            if (s[i] !== '=') {
                out[n++] = s.charCodeAt(i) & 0xff;
                continue;
            }
            const hex = s.slice(i + 1, i + 3);
            if (/^[0-9A-Fa-f]{2}$/.test(hex)) {
                out[n++] = parseInt(hex, 16);
                i += 2;
            } else if (s[i + 1] === '\r' && s[i + 2] === '\n') {
                i += 2;               // soft line break: nothing at all
            } else if (s[i + 1] === '\n') {
                i += 1;               // …and the LF-only spelling of it
            } else if (i + 1 < s.length) {
                out[n++] = 0x3d;      // a stray `=` is literal
            }
        }
    } else {
        // 7bit, 8bit, binary, or nothing declared: the bytes as they
        // stand. The caller read the message as one byte per character.
        for (let i = 0; i < s.length && n < cap; i++)
            out[n++] = s.charCodeAt(i) & 0xff;
    }
    return n === cap ? out : out.subarray(0, n);
}

function capacityFor(length, enc) {
    // base64 is 3 bytes per 4 characters; the +3 covers a partial group.
    return enc === 'base64' ? Math.ceil((length * 3) / 4) + 3 : length;
}

/// How many bytes a part decodes to, for the size shown next to it.
///
/// base64 is arithmetic — counting characters is far cheaper than
/// decoding a 20 MB attachment to find out how big it is — and
/// everything else defers to the one decoder, so the number shown and
/// the bytes saved cannot disagree.
export function decodedSize(text, encoding) {
    const s = String(text ?? '');
    const enc = String(encoding ?? '').trim().toLowerCase();
    if (enc === 'base64') {
        const end = s.indexOf('=');
        const data = end >= 0 ? s.slice(0, end) : s;
        return Math.floor((data.replace(/[^A-Za-z0-9+/]/g, '').length * 3) / 4);
    }
    if (enc === 'quoted-printable')
        return decodeTransfer(s, enc).length;
    return s.length;
}

/// One leaf part, transfer-decoded and charset-decoded. No flattening.
function decodePart(headers, body) {
    const enc = headers.get('content-transfer-encoding').toLowerCase();
    if (enc === 'base64')
        return decodeBytes(base64Bytes(body), charsetOf(headers));
    if (enc === 'quoted-printable')
        return decodeBytes(
            quotedPrintableBytes(body.replace(/=\r?\n/g, '')), charsetOf(headers));
    return decodeText(body, charsetOf(headers));
}

/// Charset-decode a part that was sent as-is — `7bit`, `8bit`, `binary`
/// or nothing declared.
///
/// This branch used to return the body untouched, which was only ever
/// right because the app had already run the whole file through a lossy
/// UTF-8 decode (#232). Now the file arrives as bytes (`messageText`),
/// so this is where an 8bit part learns what its bytes mean.
///
/// Two shapes pass through unchanged, both deliberate:
///
///   - **pure ASCII**, which every charset here agrees on, and which is
///     the overwhelming majority of parts — skipping the decode keeps a
///     megabyte body a slice rather than a million concatenations;
///   - **a string containing a character above U+00FF**, which cannot
///     have come out of a byte. That means the caller handed us
///     characters rather than bytes, and decoding them a second time is
///     exactly the corruption this function exists to prevent.
function decodeText(body, charset) {
    const text = String(body ?? '');
    // eslint-disable-next-line no-control-regex
    if (!/[\u0080-\u00ff]/.test(text) || /[^\u0000-\u00ff]/.test(text))
        return text;
    return decodeBytes(bytesOf(text), charset);
}

/// A byte string back to the bytes it stands for.
function bytesOf(text) {
    const out = new Array(text.length);
    for (let i = 0; i < text.length; i++)
        out[i] = text.charCodeAt(i) & 0xff;
    return out;
}

function bodyOfPart(headers, body, depth = 0) {
    const ctype = headers.get('content-type');
    const boundary = ctype.match(/boundary\s*=\s*"?([^";]+)"?/i)?.[1];
    if (boundary && depth < 4) {
        const parts = body.split(new RegExp(`--${escapeRe(boundary)}(?:--)?\\r?\\n?`));
        const decoded = [];
        for (const part of parts) {
            if (!part.trim())
                continue;
            const {headerText: ph, body: pb} = splitMessage(part);
            const sub = parseHeaders(ph);
            const t = sub.get('content-type').toLowerCase();
            if (!t && !ph.includes(':'))
                continue;
            decoded.push({type: t, text: bodyOfPart(sub, pb, depth + 1)});
        }
        const plain = decoded.find((d) => d.type.startsWith('text/plain'));
        if (plain?.text?.trim())
            return plain.text;
        const html = decoded.find((d) => d.type.startsWith('text/html'));
        if (html?.text?.trim())
            return htmlToText(html.text);
        // PROSE ONLY (#221) — see `isProse`. Without the filter this
        // line handed back the first non-empty leaf whatever it was,
        // and a message whose text parts are present and empty is
        // ordinary "here is the file" mail.
        const any = decoded.find((d) => isProse(d.type) && d.text?.trim());
        return any ? any.text : '';
    }

    // A LEAF. The same question, and it was not asked here either: a
    // single-part message that IS a document — no multipart wrapper,
    // just `Content-Type: application/pdf` — decoded straight into the
    // reading pane, the preview column and `read_message`.
    const type = headers.get('content-type').toLowerCase();
    if (!isProse(type))
        return '';

    const enc = headers.get('content-transfer-encoding').toLowerCase();
    let text;
    if (enc === 'base64')
        text = decodeBytes(base64Bytes(body), charsetOf(headers));
    else if (enc === 'quoted-printable')
        text = decodeBytes(
            quotedPrintableBytes(body.replace(/=\r?\n/g, '')), charsetOf(headers));
    else
        text = decodeText(body, charsetOf(headers));

    if (type.startsWith('text/html'))
        return htmlToText(text);
    return text;
}

function charsetOf(headers) {
    return headers.get('content-type').match(/charset\s*=\s*"?([^";]+)"?/i)?.[1] ?? 'utf-8';
}

function escapeRe(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
