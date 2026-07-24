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
