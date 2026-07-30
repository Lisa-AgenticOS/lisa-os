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
        const any = decoded.find((d) => d.text?.trim());
        return any ? any.text : '';
    }

    let text = body;
    const enc = headers.get('content-transfer-encoding').toLowerCase();
    if (enc === 'base64')
        text = decodeBytes(base64Bytes(body), charsetOf(headers));
    else if (enc === 'quoted-printable')
        text = decodeBytes(
            quotedPrintableBytes(body.replace(/=\r?\n/g, '')), charsetOf(headers));

    if (headers.get('content-type').toLowerCase().startsWith('text/html'))
        return htmlToText(text);
    return text;
}

function charsetOf(headers) {
    return headers.get('content-type').match(/charset\s*=\s*"?([^";]+)"?/i)?.[1] ?? 'utf-8';
}

function escapeRe(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
