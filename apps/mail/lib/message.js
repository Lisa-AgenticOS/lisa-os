// One message, as everything downstream needs it. Pure.
//
// # Why this exists rather than living in the window
//
// The reading pane, the Agent Bus tools, the toolbar and Reply all want
// the same object, and it used to be built inline in `lisa-mail.js` —
// where nothing can test it, because that file imports `gi://`. So the
// suites tested `replyFields` with a hand-written literal that supplied
// `messageId` and `references` *no producer ever set*, and every reply
// this app composed went out with no In-Reply-To and no References
// (#223). The fixture was the bug's hiding place: it asserted the
// consumer against fields the producer did not exist to provide.
//
// One producer, here, over the BYTES of a message file (`messageText`
// in rfc822.js), so a test can start where the app starts.

import {
    decodeWords, parseAddress, parseHeaders, readableBody, renderableBody, splitMessage,
} from './rfc822.js';
import {attachments, listedAttachments} from './attachments.js';

/// Everything one message file yields: who it is from, what it says,
/// what is attached, and what thread it belongs to.
///
/// `meta` is whatever the caller already knows from the filename —
/// folder, unique part, dir, flags — and is spread FIRST so a parsed
/// header can never overwrite the identity the disk gave it.
///
/// `body` is prose or it is empty. It is never the bytes of an
/// attachment (#221): a message whose only content is a document has an
/// empty body and a listed attachment, which is the truth, rather than
/// a reading pane full of `%PDF-1.4 %Ǭ` and a model handed three
/// megabytes of a ZIP.
export function messageView(raw, meta = {}) {
    const {headerText} = splitMessage(raw);
    const headers = parseHeaders(headerText);
    return {
        ...meta,
        from: parseAddress(headers.get('from')),
        to: headers.get('to'),
        subject: decodeSubject(headers.get('subject')),
        date: headers.get('date'),
        ...threadFields(headers),
        body: readableBody(raw),
        // The HTML as sent, for the window. Tools never see this: a
        // model is handed `body`, which is prose.
        html: renderableBody(raw).html,
        // What is attached: metadata only, and only the parts worth
        // showing (lib/attachments.js). The BYTES are fetched on demand.
        attachments: listedAttachments(attachments(raw)),
    };
}

/// Where a message sits in a thread.
///
/// `messageId` is this message's own identity and becomes the reply's
/// `In-Reply-To`; `references` is the chain it belongs to and the reply
/// extends it (`referencesFor` in compose.js). Threading in the
/// RECIPIENT's client depends on both, and this app set neither.
///
/// A message with no `References` but with an `In-Reply-To` still has a
/// chain — its parent — and RFC 5322 §3.6.4 says to use it. Plenty of
/// mailers send one and not the other.
export function threadFields(headers) {
    const get = (name) => String(headers.get(name) ?? '').trim();
    return {
        messageId: get('message-id'),
        references: get('references') || get('in-reply-to'),
    };
}

/// A subject, decoded and never empty.
///
/// Written first as `parseAddress(value).address`, which produced the
/// right string for the wrong reason — `parseAddress` decodes on the
/// way in and returns the whole text when there is no `<…>` — and would
/// have mangled any subject containing angle brackets. A subject is not
/// an address.
export function decodeSubject(value) {
    return decodeWords(value).trim() || '(no subject)';
}
