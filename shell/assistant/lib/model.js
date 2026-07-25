// Lisa Assistant — pure view-model logic (this session's ADR; PLAN §5.7.1).
//
// No GNOME imports: runs under gjs (the app) and jsc (unit tests on any dev
// host), like the overlay's lib/. The window owns the live widgets; this
// module owns the transforms — model-list assembly, the send payload, and
// the egress marker — so they can be tested without a display.

/**
 * `remote:<provider>:<model>` ids route through the egress broker: the turn
 * leaves the machine and is ledgered `remote.*`.
 * @param {string} id
 * @returns {boolean}
 */
export function isRemote(id) {
    return typeof id === 'string' && id.startsWith('remote:');
}

/**
 * Local model entries from lisa-inferenced `GET /v1/models`.
 * @param {object} modelsJson  the parsed `{object:"list", data:[{id}]}`
 * @returns {{id: string, label: string, kind: string}[]}
 */
export function parseLocalModels(modelsJson) {
    const data = Array.isArray(modelsJson?.data) ? modelsJson.data : [];
    return data
        .map(m => m?.id)
        .filter(id => typeof id === 'string' && id.length > 0)
        .map(id => ({id, label: id, kind: 'local'}));
}

/**
 * Providers from `dev.lisaos.Remote1.State()` that can actually serve a chat —
 * signed in (OAuth) or holding an API key.
 * @param {object} stateJson  parsed State() document
 * @returns {{id: string, display_name: string}[]}
 */
export function usableProviders(stateJson) {
    const providers = Array.isArray(stateJson?.providers)
        ? stateJson.providers : [];
    return providers
        .filter(p => p && (p.connected || p.has_key))
        .map(p => ({
            id: String(p.id),
            display_name: String(p.display_name ?? p.id),
        }));
}

/**
 * Cloud model entries for one provider, from its `ListModels`.
 * @param {string} provider
 * @param {string} displayName
 * @param {string[]} modelIds
 * @returns {{id: string, label: string, kind: string, provider: string}[]}
 */
export function cloudEntries(provider, displayName, modelIds) {
    return (modelIds ?? [])
        .filter(m => typeof m === 'string' && m.length > 0)
        .map(m => ({
            id: `remote:${provider}:${m}`,
            label: `${displayName} · ${m}`,
            kind: 'cloud',
            provider,
        }));
}

/**
 * Merge local + cloud entries into the picker order (local first). Deduped
 * by id, preserving first occurrence.
 * @param {object[]} localEntries
 * @param {object[]} cloudEntries
 * @returns {object[]}
 */
export function mergeModelList(localEntries, cloudEntries) {
    const seen = new Set();
    const out = [];
    for (const e of [...(localEntries ?? []), ...(cloudEntries ?? [])]) {
        if (!e || seen.has(e.id))
            continue;
        seen.add(e.id);
        out.push(e);
    }
    return out;
}

/**
 * The `history_json` payload for a chat Ask: the completed prior turns as
 * OpenAI messages. Call this BEFORE appending the new user turn to the UI.
 * @param {{role: string, text: string}[]} turns
 * @returns {{role: string, content: string}[]}
 */
export function historyPayload(turns) {
    return (turns ?? [])
        .filter(t => t && (t.role === 'user' || t.role === 'assistant') &&
            typeof t.text === 'string' && t.text !== '')
        .map(t => ({role: t.role, content: t.text}));
}

/**
 * The conversation as portable Markdown (issue #8): `**You:**` /
 * `**Lisa (label):**` per completed turn; a cloud turn carries the
 * egress note, because an export must not launder that the text left
 * the machine. `models` resolves ids to picker labels; an unknown id
 * falls back to itself.
 * @param {{role: string, text: string, model?: ?string}[]} turns
 * @param {{id: string, label: string}[]} models
 * @returns {string}  '' for an empty conversation
 */
export function conversationMarkdown(turns, models) {
    const label = id => (models ?? []).find(m => m.id === id)?.label ?? id;
    const blocks = [];
    for (const t of turns ?? []) {
        if (!t || (t.role !== 'user' && t.role !== 'assistant') ||
            typeof t.text !== 'string' || t.text === '')
            continue;
        if (t.role === 'user') {
            blocks.push(`**You:**\n\n${t.text}`);
            continue;
        }
        const heading = t.model
            ? `**Lisa (${label(t.model)}):**` : '**Lisa:**';
        let block = `${heading}\n\n${t.text}`;
        if (isRemote(t.model))
            block += `\n\n_↗ left this machine via ${label(t.model)}_`;
        blocks.push(block);
    }
    return blocks.length > 0 ? `${blocks.join('\n\n')}\n` : '';
}

/**
 * Completed turns as the stored payload: plain {role, text, model} —
 * no widgets, no in-flight text. This is the wire shape harness-core's
 * SessionStore reads and writes byte for byte (`SessionTurn` in
 * libs/harness-core/src/session.rs), so it is validated the same way
 * coming from a widget list or from stored JSON.
 * @param {{role: string, text: string, model?: ?string}[]} turns
 * @returns {{role: string, text: string, model: ?string}[]}
 */
export function normalizeTurns(turns) {
    return (turns ?? [])
        .filter(t => t && (t.role === 'user' || t.role === 'assistant') &&
            typeof t.text === 'string' && t.text !== '')
        .map(t => ({
            role: t.role,
            text: t.text,
            model: typeof t.model === 'string' ? t.model : null,
        }));
}

/**
 * Parse a stored conversation array back into renderable turns — the
 * pre-sessions `conversation` value on upgrade (lib/sessions.js), and a
 * session record's `turns`. Shape-validated per turn; anything
 * malformed — bad JSON, non-array, junk entries — is dropped rather
 * than trusted, so a corrupt memory value can never break startup.
 * @param {string} json
 * @returns {{role: string, text: string, model: ?string}[]}
 */
export function deserializeConversation(json) {
    let parsed;
    try {
        parsed = JSON.parse(json);
    } catch {
        return [];
    }
    return Array.isArray(parsed) ? normalizeTurns(parsed) : [];
}
