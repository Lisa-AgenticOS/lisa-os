// Building a message to send (issue #168). Pure — no GNOME imports —
// so every rule here is testable without a display or an SMTP server.
//
// The counterpart to lib/rfc822.js, which reads. That module is total
// because it parses hostile input; this one is strict because it
// *produces* the bytes another mail system will parse, and a message
// that is subtly malformed is one that renders wrongly in a stranger's
// client with no way for us to know.
//
// # What is deliberately here rather than left to msmtp
//
// msmtp is an SMTP client, not a message composer: give it bytes and a
// recipient and it delivers them. Everything about *what a reply is* —
// the Re: prefix that must not stack, the References chain that keeps
// threading working in the recipient's client, the quoted body — is
// ours, and none of it involves the network, so none of it needs a
// network to test.

/// Header value → RFC 2047 encoded word when it needs one.
///
/// ASCII passes through unchanged: encoding "Hello" as
/// `=?UTF-8?B?SGVsbG8=?=` is legal, ugly, and makes every subject line
/// unreadable in a plain-text dump of the mailbox.
export function encodeHeaderValue(value) {
    const text = String(value ?? '');
    // eslint-disable-next-line no-control-regex
    if (!/[^\x20-\x7e]/.test(text))
        return text;
    return `=?UTF-8?B?${base64(utf8Bytes(text))}?=`;
}

/// `Re:` exactly once, whatever the sender's client did.
///
/// "Re: Re: Re: invoice" is what happens when every client prepends
/// blindly. Also matches localised forms mailers actually emit, because
/// a subject that already says "AW:" does not need "Re: AW:" on top.
export function replySubject(subject) {
    const text = String(subject ?? '').trim();
    if (/^(re|aw|sv|vs|res)\s*:/i.test(text))
        return text;
    return `Re: ${text}`;
}

export function forwardSubject(subject) {
    const text = String(subject ?? '').trim();
    if (/^(fwd?|wg|tr)\s*:/i.test(text))
        return text;
    return `Fwd: ${text}`;
}

/// The References chain for a reply.
///
/// Threading in the RECIPIENT's client depends on this and nothing
/// else: In-Reply-To alone is not enough for clients that thread on
/// References, and dropping it turns a conversation into a pile of
/// unrelated messages in everyone's inbox but ours.
///
/// Capped at 20, keeping the first and the most recent — the shape
/// RFC 5322 §3.6.4 suggests when a chain grows unbounded.
export function referencesFor(inReplyTo, existingReferences) {
    const prior = String(existingReferences ?? '')
        .split(/\s+/)
        .filter((s) => s.startsWith('<') && s.endsWith('>'));
    const id = String(inReplyTo ?? '').trim();
    const all = id && !prior.includes(id) ? [...prior, id] : prior;
    if (all.length <= 20)
        return all.join(' ');
    return [all[0], ...all.slice(-19)].join(' ');
}

/// Quote a body the way every mail client does, so the reply reads as a
/// reply rather than as a wall of text.
export function quoteBody(body, attribution) {
    const quoted = String(body ?? '')
        .split(/\r?\n/)
        .map((line) => (line.startsWith('>') ? `>${line}` : `> ${line}`))
        .join('\n');
    return attribution ? `${attribution}\n${quoted}` : quoted;
}

/// A Message-ID that is unique and does not leak the machine.
///
/// The local hostname is conventional here and is also a fingerprint
/// sent to every recipient. The domain of the sending address is both
/// unique enough and already known to them.
export function messageIdFor(fromAddress, random, now) {
    const domain = String(fromAddress ?? '').split('@')[1] || 'localhost';
    return `<${now}.${random}@${domain}>`;
}

/// The complete message, ready for msmtp.
///
/// CRLF line endings, because SMTP is a CRLF protocol and a bare LF is
/// the kind of thing that works against one server and is rejected by
/// the next.
///
/// The body is base64 with `Content-Transfer-Encoding: base64` rather
/// than 8bit: SMTP limits a line to 998 octets, a pasted URL or a long
/// paragraph with no newline exceeds that, and the failure is a server
/// rejection or a silently mangled message rather than anything we
/// could catch.
export function buildMessage(fields) {
    const {
        from, to, cc = '', subject = '', body = '',
        inReplyTo = '', references = '', date, messageId,
    } = fields ?? {};
    if (!String(from ?? '').trim())
        throw new Error('a message needs a From address');
    if (!String(to ?? '').trim() && !String(cc ?? '').trim())
        throw new Error('a message needs at least one recipient');

    const headers = [
        ['From', from],
        ['To', to],
    ];
    if (String(cc).trim())
        headers.push(['Cc', cc]);
    headers.push(['Subject', encodeHeaderValue(subject)]);
    headers.push(['Date', date]);
    headers.push(['Message-ID', messageId]);
    if (String(inReplyTo).trim()) {
        headers.push(['In-Reply-To', inReplyTo]);
        const chain = referencesFor(inReplyTo, references);
        if (chain)
            headers.push(['References', chain]);
    }
    headers.push(['MIME-Version', '1.0']);
    headers.push(['Content-Type', 'text/plain; charset=utf-8']);
    headers.push(['Content-Transfer-Encoding', 'base64']);

    const head = headers
        .filter(([, v]) => String(v ?? '').trim() !== '')
        .map(([k, v]) => `${k}: ${v}`)
        .join('\r\n');
    // 76-char lines, as base64 in mail is required to be wrapped.
    const encoded = base64(utf8Bytes(String(body))).replace(/(.{76})/g, '$1\r\n');
    return `${head}\r\n\r\n${encoded}\r\n`;
}

/// Everything a Reply window should open with.
export function replyFields(message, me) {
    const from = message?.from ?? {};
    const attribution = `On ${message?.date ?? 'an earlier date'}, ` +
        `${from.name || from.address || 'someone'} wrote:`;
    return {
        to: from.address ?? '',
        subject: replySubject(message?.subject),
        body: `\n\n${quoteBody(message?.body, attribution)}`,
        inReplyTo: message?.messageId ?? '',
        references: message?.references ?? '',
        from: me ?? '',
    };
}

/// Everything a Forward window should open with. No recipient: the
/// whole point of forwarding is choosing one, and pre-filling the
/// original sender is how mail gets sent back to the person who sent it.
export function forwardFields(message, me) {
    const from = message?.from ?? {};
    const header = [
        '---------- Forwarded message ----------',
        `From: ${from.name ? `${from.name} <${from.address}>` : from.address ?? ''}`,
        `Date: ${message?.date ?? ''}`,
        `Subject: ${message?.subject ?? ''}`,
        `To: ${message?.to ?? ''}`,
    ].join('\n');
    return {
        to: '',
        subject: forwardSubject(message?.subject),
        body: `\n\n${header}\n\n${message?.body ?? ''}`,
        inReplyTo: '',
        references: '',
        from: me ?? '',
    };
}

function utf8Bytes(text) {
    const out = [];
    for (const ch of String(text)) {
        const cp = ch.codePointAt(0);
        if (cp < 0x80) {
            out.push(cp);
        } else if (cp < 0x800) {
            out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
        } else if (cp < 0x10000) {
            out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
        } else {
            out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f),
                0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
        }
    }
    return out;
}

function base64(bytes) {
    const table = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    let out = '';
    for (let i = 0; i < bytes.length; i += 3) {
        const b0 = bytes[i];
        const b1 = bytes[i + 1];
        const b2 = bytes[i + 2];
        out += table[b0 >> 2];
        out += table[((b0 & 3) << 4) | ((b1 ?? 0) >> 4)];
        out += b1 === undefined ? '=' : table[((b1 & 15) << 2) | ((b2 ?? 0) >> 6)];
        out += b2 === undefined ? '=' : table[b2 & 63];
    }
    return out;
}
