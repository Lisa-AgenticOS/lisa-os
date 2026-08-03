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
