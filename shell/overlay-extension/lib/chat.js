// Chat-lane helpers for the assistant backend (PLAN §5.7.1; the persistent
// chat window, this session's ADR). Pure logic, no GNOME imports — runs
// under gjs (the backend) and the unit-test runner on any dev host.
//
// The chat lane differs from the overlay's one-shot inference lane: it is
// multi-turn (carries prior turns as OpenAI-style messages) and talks to
// lisa-inferenced's OpenAI-compat endpoint so the model's chat template is
// applied and cloud providers route through the broker
// (model = `remote:<provider>:<model>`). Streaming is Server-Sent Events.

/**
 * Assemble the OpenAI `messages` array from prior turns + the new prompt.
 * Only well-formed user/assistant turns are kept; the new prompt is always
 * appended as the final user turn.
 *
 * @param {{role: string, content: string}[]} history
 * @param {string} prompt
 * @returns {{role: string, content: string}[]}
 */
export function buildMessages(history, prompt) {
    const msgs = [];
    for (const turn of history ?? []) {
        if (turn && (turn.role === 'user' || turn.role === 'assistant') &&
            typeof turn.content === 'string' && turn.content !== '')
            msgs.push({role: turn.role, content: turn.content});
    }
    msgs.push({role: 'user', content: String(prompt ?? '')});
    return msgs;
}

/**
 * The POST body for `/v1/chat/completions`. `model` may be a local id or
 * `remote:<provider>:<model>`; omitted → the daemon's default.
 *
 * @param {string|undefined} model
 * @param {{role: string, content: string}[]} messages
 * @returns {object}
 */
export function chatRequestBody(model, messages) {
    const body = {messages, stream: true};
    if (model)
        body.model = model;
    return body;
}

/**
 * Parse one SSE line from the streaming completion.
 *   `data: {…delta…}` → {delta: string}
 *   `data: {"error":…}` → {error: string}
 *   `data: [DONE]`      → {done: true}
 *   anything else       → null (comment / blank / non-content chunk)
 *
 * @param {string} line
 * @returns {{delta?: string, error?: string, done?: boolean}|null}
 */
export function parseSseLine(line) {
    const s = (line ?? '').trim();
    if (!s.startsWith('data:'))
        return null;
    const payload = s.slice(5).trim();
    if (payload === '[DONE]')
        return {done: true};
    let obj;
    try {
        obj = JSON.parse(payload);
    } catch {
        return null;
    }
    if (obj.error)
        return {error: obj.error.message ?? String(obj.error)};
    const delta = obj.choices?.[0]?.delta?.content;
    if (typeof delta === 'string' && delta.length > 0)
        return {delta};
    return null;
}

/**
 * A `remote:<provider>:<model>` id routes through the egress broker — i.e.
 * this turn leaves the machine and is ledgered `remote.*`.
 *
 * @param {string} model
 * @returns {boolean}
 */
export function isRemoteModel(model) {
    return typeof model === 'string' && model.startsWith('remote:');
}

/**
 * Length of the longest prefix of `bytes` that ends on a complete UTF-8
 * sequence. GJS's TextDecoder lacks {stream:true} (field iMac), so chunked
 * decoding must trim a chunk-split multibyte char instead of half-decoding
 * it into a replacement character.
 *
 * @param {Uint8Array} bytes
 * @returns {number}
 */
export function utf8Complete(bytes) {
    const n = bytes.length;
    if (n === 0)
        return 0;
    // Find the last lead byte within the final 4 bytes.
    let i = n - 1;
    const limit = Math.max(0, n - 4);
    while (i > limit && (bytes[i] & 0xC0) === 0x80)
        i--;
    const b = bytes[i];
    let need = 1;
    if ((b & 0xE0) === 0xC0)
        need = 2;
    else if ((b & 0xF0) === 0xE0)
        need = 3;
    else if ((b & 0xF8) === 0xF0)
        need = 4;
    else if ((b & 0x80) !== 0)
        return n; // continuation/invalid at lead position — let decode handle it
    return (n - i >= need) ? n : i;
}
