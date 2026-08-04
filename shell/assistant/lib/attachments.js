// Lisa Assistant — attachments (issue #209's last mile). Pure logic, no
// GNOME imports, like the rest of lib/: runs under gjs (the app) and
// node/jsc (unit tests on any dev host).
//
// The window owns the file dialog, the clipboard and the thumbnails;
// this module owns the two things worth testing without a display —
// what an attached file becomes on the wire, and the guard that stops a
// local text-only model being handed a picture.

import {isRemote} from './model.js';

/**
 * The staged attachments that belong to conversation `sessionId`.
 *
 * An attachment belongs to the conversation it was staged in, not to
 * the composer (#235). It used to belong to the composer: attach an
 * image, switch conversations, type — and the picture went with the new
 * message, to THAT conversation's provider. A picture staged in a chat
 * with a local model and sent from a chat with a cloud one leaves the
 * machine, which makes it a disclosure rather than a stray widget.
 *
 * The window clears the strip on every switch, which is the fix; this
 * is the second mechanism, so a switch path nobody remembered to clear
 * still cannot put one conversation's bytes on another's wire. An
 * untagged attachment belongs to nobody rather than to everybody —
 * fail closed, because the failure this guards is disclosure.
 * @param {?object[]} items
 * @param {?string} sessionId
 * @returns {object[]}
 */
export function stagedForSession(items, sessionId) {
    if (typeof sessionId !== 'string' || sessionId === '')
        return [];
    return (items ?? []).filter(a => a?.session === sessionId);
}

/**
 * Image types the wire already carries end to end: the Assistant builds
 * the same `image_url` part `lisa ask --attach` does, inferenced passes
 * it through as `Content::Parts`, and lisa-remoted rewrites the data URI
 * into Anthropic's `{"type":"image","source":…}` shape.
 *
 * Images only. `lisa ask --attach` also takes wav/mp3, but nothing in
 * this window records or picks audio, and listing a type the composer
 * cannot produce would be documenting intent as behaviour.
 */
export const IMAGE_MIME_BY_EXT = {
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    webp: 'image/webp',
    gif: 'image/gif',
};

/**
 * The image mime for a file name or path, or null when it is not one of
 * ours. Null rather than a guess: labelling a .pdf `image/*` produces a
 * provider-side error that reads like our bug.
 * @param {string} name  file name or full path
 * @returns {?string}
 */
export function imageMimeForName(name) {
    if (typeof name !== 'string')
        return null;
    const dot = name.lastIndexOf('.');
    if (dot < 0)
        return null;
    return IMAGE_MIME_BY_EXT[name.slice(dot + 1).toLowerCase()] ?? null;
}

/**
 * One OpenAI content part: a data: URI, so the bytes travel inside the
 * request and no temporary upload exists to leak.
 * @param {string} mime
 * @param {string} b64  base64 of the file's bytes
 * @returns {object}
 */
export function imagePart(mime, b64) {
    return {type: 'image_url', image_url: {url: `data:${mime};base64,${b64}`}};
}

/**
 * The `attachments` option's payload: the parts array, in the order the
 * person attached them. The daemon puts the message TEXT in front of
 * these — this array is the attachments alone.
 *
 * Items that never got bytes or a mime are dropped here rather than
 * sent half-formed; the composer only ever adds complete ones, so this
 * is the belt to the window's braces.
 * @param {{name: string, mime: string, b64: string}[]} items
 * @returns {object[]}
 */
export function attachmentsPayload(items) {
    return (items ?? [])
        .filter(a => a && typeof a.mime === 'string' && a.mime !== '' &&
            typeof a.b64 === 'string' && a.b64 !== '')
        .map(a => imagePart(a.mime, a.b64));
}

/**
 * How big one attached image may be, in bytes on disk.
 *
 * Issue #226: nothing bounded this anywhere, so the first ceiling a
 * person met was axum's undeclared 2 MiB request default — reached at a
 * ~1.5 MB file, because base64 is 4/3 of what it carries — and it
 * arrived as a bare `413` after the round trip.
 *
 * 8 MiB is what a full-screen PNG from a large display costs with room
 * to spare, and it is the number the layers behind it were sized
 * against: 16 MiB per send → 21.4 MiB of base64 → under harnessd's
 * 24 MiB `attachments` cap → inside inferenced's 32 MiB request limit.
 */
export const MAX_ATTACHMENT_BYTES = 8 * 1024 * 1024;

/** How much all staged attachments may come to for one send. */
export const MAX_ATTACHMENTS_TOTAL_BYTES = 16 * 1024 * 1024;

/** Bytes → a number a person reads, at one decimal under 10 MB. */
function mb(bytes) {
    const n = bytes / (1024 * 1024);
    return `${n < 10 ? n.toFixed(1).replace(/\.0$/, '') : Math.round(n)} MB`;
}

/**
 * Why this image cannot be attached, or null if it can.
 *
 * Checked at ATTACH time, not at send time: the person still has the
 * file dialog in mind and can pick a smaller picture. Learning it after
 * a round trip, as a `413`, is the whole of #226.
 *
 * This is a courtesy, not the bound — harnessd applies the real one, and
 * a check the caller can skip is not a guard (ADR-0029).
 *
 * @param {string} name    the file's name, for the message
 * @param {number} bytes   its size on disk
 * @param {{bytes: number}[]} staged  what the composer already holds
 * @returns {?string}  a sentence for the transcript, or null
 */
export function attachmentSizeRefusal(name, bytes, staged) {
    if (bytes > MAX_ATTACHMENT_BYTES) {
        return `Cannot attach ${name} — it is ${mb(bytes)}, and one image ` +
            `may be up to ${mb(MAX_ATTACHMENT_BYTES)}. Scale it down and ` +
            'try again.';
    }
    const already = (staged ?? []).reduce((n, a) => n + (a?.bytes ?? 0), 0);
    if (already + bytes > MAX_ATTACHMENTS_TOTAL_BYTES) {
        return `Cannot attach ${name} — together with what is already ` +
            `attached that comes to ${mb(already + bytes)}, and one message ` +
            `may carry up to ${mb(MAX_ATTACHMENTS_TOTAL_BYTES)}. Send some ` +
            'of them separately.';
    }
    return null;
}

/**
 * Why this send cannot happen, or null if it can.
 *
 * A local engine reads text only and says so — lisa-inferenced's llama
 * backend refuses content parts outright rather than flattening an
 * image into `[image_url]` and answering about a picture nobody saw.
 * That refusal is correct and it is also five layers away: by the time
 * it surfaces the person has watched a spinner and gets a daemon error.
 * So the same rule is applied here, where they can still act on it.
 *
 * This is a UI courtesy, not a guardrail — the daemon's refusal is the
 * mechanism, and it stays (ADR-0029: a check the caller can skip is not
 * a guard). An unknown model fails closed: we cannot know it is
 * multimodal, and guessing yes ends in that same confident answer.
 *
 * That sentence was FALSE AS WRITTEN until #236. The refusal existed on
 * inferenced's typed lane only, and the lane this window actually uses
 * is the tools lane, which handed the body to llama-server verbatim —
 * so skipping this check got you a raw 500 with an mmproj hint, not
 * Lisa's sentence. Both lanes refuse now
 * (`daemons/inferenced/src/llama.rs`), which is what makes the claim
 * above something a reader can rely on rather than something we meant.
 * @param {?{id: string, label: string}} model  the picked model entry
 * @param {object[]} items  attachments the composer is holding
 * @returns {?string}  a sentence for the transcript, or null
 */
export function attachmentRefusal(model, items) {
    if (!items || items.length === 0)
        return null;
    if (!model || typeof model.id !== 'string')
        return 'Pick a model before attaching an image — a cloud model, ' +
            'since the local ones read text only.';
    if (isRemote(model.id))
        return null;
    const name = model.label ?? model.id;
    return `${name} runs on this machine and reads text only — it cannot ` +
        'see an image. Pick a cloud model in the picker, or remove the ' +
        'attachment.';
}
