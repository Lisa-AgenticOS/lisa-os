// Agent Bus client logic for the overlay (PLAN §5.4/§5.7.1, ADR-0013).
//
// Pure logic, no GNOME imports: runs under gjs (the backend), GNOME
// Shell (the frontend renders consentView output), node, and jsc
// (unit tests on any dev host). The backend turns an actionable prompt
// into an dev.lisaos.Agent1 RequestCall; the model is deliberately NOT
// in this lane — tool picking reuses Agent1.Discover plus the same
// deterministic token-overlap scoring as agentd's registry
// (daemons/agentd/src/registry.rs — keep scoreTool/tokenize in sync),
// and arg filling is a small schema-driven heuristic. Prompts that
// don't route stay on the inference streaming path.

// Routing threshold: agentd scores name-token hits 3, description and
// app-id hits 1. Requiring ≥ 3 means at least one tool-name token must
// appear in the prompt; weaker (description-only) matches stay chat.
export const NAME_HIT_SCORE = 3;

// Glue words stripped from the front of an extracted argument.
const GLUE_WORDS = new Set([
    'a', 'an', 'the', 'my', 'me', 'to', 'for', 'of', 'on', 'in', 'at',
    'about', 'called', 'titled', 'named', 'that', 'saying', 'please', 'with',
]);

/** JSON.parse that never throws. */
export function safeParse(text) {
    try {
        return JSON.parse(text);
    } catch {
        return null;
    }
}

/**
 * Mirror of registry.rs tokens(): lowercase, split on non-ASCII-
 * alphanumeric runs, drop empties.
 *
 * @param {string} text
 * @returns {string[]}
 */
export function tokenize(text) {
    return (text ?? '').toLowerCase().split(/[^a-z0-9]+/).filter(t => t !== '');
}

/**
 * Mirror of registry.rs discover() scoring: per query token, a
 * tool-name token hit weighs 3, a description or app-id hit weighs 1.
 *
 * @param {string[]} queryTokens tokenize()d prompt
 * @param {{name?: string, description?: string, app_id?: string}} tool
 * @returns {number}
 */
export function scoreTool(queryTokens, tool) {
    const name = new Set(tokenize(tool?.name));
    const desc = new Set(tokenize(tool?.description));
    const app = new Set(tokenize(tool?.app_id));
    let score = 0;
    for (const q of queryTokens) {
        if (name.has(q))
            score += 3;
        else if (desc.has(q) || app.has(q))
            score += 1;
    }
    return score;
}

/**
 * Decide whether a prompt is an actionable request against the tools
 * Agent1.Discover returned. Discover already ranks and drops zero-hit
 * tools; walking its order and keeping the first maximum preserves its
 * tie-break (score, then app_id, then name).
 *
 * @param {string} prompt the user's turn
 * @param {object[]} tools ToolRef JSON from Agent1.Discover
 * @returns {?{tool: object, args: object, score: number}} null = stay
 * on the inference path (no confident hit, or args we can't fill
 * without the model).
 */
export function decideAction(prompt, tools) {
    const queryTokens = tokenize(prompt);
    if (queryTokens.length === 0 || !Array.isArray(tools) || tools.length === 0)
        return null;
    let best = null;
    let bestScore = 0;
    for (const tool of tools) {
        const score = scoreTool(queryTokens, tool);
        if (score > bestScore) {
            best = tool;
            bestScore = score;
        }
    }
    if (!best || bestScore < NAME_HIT_SCORE)
        return null;
    const args = fillArgs(prompt, best);
    if (args === null)
        return null;
    return {tool: best, args, score: bestScore};
}

/** Quoted spans ("…" or '…') in prompt order, trimmed, empties dropped. */
export function quotedSpans(prompt) {
    const out = [];
    const re = /"([^"]+)"|'([^']+)'/g;
    let m;
    while ((m = re.exec(prompt ?? '')) !== null) {
        const span = (m[1] ?? m[2]).trim();
        if (span !== '')
            out.push(span);
    }
    return out;
}

/**
 * The prompt remainder used as a single string argument: text after a
 * colon wins ("note: buy milk"); otherwise everything after the last
 * tool-name token in the prompt, with leading glue words stripped.
 *
 * @param {string} prompt
 * @param {{name?: string}} tool the tool the prompt routed to
 * @returns {string} '' when nothing usable remains
 */
export function residualText(prompt, tool) {
    const text = (prompt ?? '').trim();
    if (text === '')
        return '';
    const colon = text.indexOf(':');
    if (colon >= 0) {
        const after = text.slice(colon + 1).trim();
        if (after !== '')
            return after;
    }
    const nameTokens = new Set(tokenize(tool?.name));
    const words = text.split(/\s+/);
    let cut = -1;
    words.forEach((word, i) => {
        if (nameTokens.has(word.toLowerCase().replace(/[^a-z0-9]/g, '')))
            cut = i;
    });
    // No name token in the prompt (defensive; decideAction scored one):
    // drop just the leading imperative verb.
    const rest = cut >= 0 ? words.slice(cut + 1) : words.slice(1);
    while (rest.length > 0 &&
            GLUE_WORDS.has(rest[0].toLowerCase().replace(/[^a-z0-9]/g, '')))
        rest.shift();
    return rest.join(' ').trim().replace(/[.!?]+$/, '');
}

/**
 * Fill a tool's required arguments from the prompt against its
 * input_schema. No model in the loop: quoted spans feed string props
 * in order; a lone required string prop takes the residual prompt
 * text; integer/number props take the first number in the prompt.
 *
 * @param {string} prompt
 * @param {{input_schema?: object}} tool
 * @returns {?object} null when a required argument can't be filled —
 * the caller stays on the inference path instead of guessing.
 */
export function fillArgs(prompt, tool) {
    const schema = tool?.input_schema ?? {};
    const required = Array.isArray(schema.required) ? schema.required : [];
    if (required.length === 0)
        return {};
    const props = schema.properties ?? {};
    const quotes = quotedSpans(prompt);
    const args = {};
    let qi = 0;
    for (const key of required) {
        const type = props[key]?.type ?? 'string';
        if (type === 'string') {
            if (qi < quotes.length) {
                args[key] = quotes[qi++];
            } else if (required.length === 1) {
                const residual = residualText(prompt, tool);
                if (residual === '')
                    return null;
                args[key] = residual;
            } else {
                return null; // splitting one utterance across several
                // string props is the intent-router model's job.
            }
        } else if (type === 'integer') {
            const m = (prompt ?? '').match(/-?\d+/);
            if (!m)
                return null;
            args[key] = parseInt(m[0], 10);
        } else if (type === 'number') {
            const m = (prompt ?? '').match(/-?\d+(\.\d+)?/);
            if (!m)
                return null;
            args[key] = parseFloat(m[0]);
        } else {
            return null; // booleans/arrays/objects need the model
        }
    }
    return args;
}

function scalarText(value) {
    if (value === null || value === undefined)
        return '';
    if (typeof value === 'object')
        return objectLine(value);
    return String(value);
}

function objectLine(obj) {
    if (obj.title !== undefined)
        return `${obj.id !== undefined ? `#${obj.id} ` : ''}${obj.title}`;
    const parts = [];
    for (const [key, value] of Object.entries(obj)) {
        if (value === null || typeof value === 'object')
            continue;
        parts.push(`${key}: ${value}`);
        if (parts.length >= 6)
            break;
    }
    return parts.join(', ');
}

/**
 * Compact rendering of an MCP tool result for the response area:
 * strings pass through, arrays become one "· item" line per entry
 * (objects prefer `title`, else scalar "key: value" pairs), a
 * single-key object wrapping an array unwraps it ({notes: [...]}).
 *
 * @param {*} value the "result" field of an executed detail_json
 * @returns {string}
 */
export function formatValue(value) {
    if (typeof value === 'string')
        return value;
    if (value === null || value === undefined)
        return '';
    if (typeof value !== 'object')
        return String(value);
    if (Array.isArray(value)) {
        if (value.length === 0)
            return '(none)';
        return value
            .map(item => `· ${scalarText(item)}`)
            .join('\n');
    }
    const keys = Object.keys(value);
    if (keys.length === 1 && Array.isArray(value[keys[0]]))
        return formatValue(value[keys[0]]);
    return objectLine(value) || JSON.stringify(value);
}

/**
 * Parse an Agent1 "executed" detail_json ({result, ledger_ref}).
 *
 * @param {string} detailJson
 * @returns {{text: string, ledgerRef: ?number}}
 */
export function formatExecuted(detailJson) {
    const detail = safeParse(detailJson);
    if (detail === null || typeof detail !== 'object')
        return {text: String(detailJson ?? ''), ledgerRef: null};
    const ledgerRef = typeof detail.ledger_ref === 'number' ? detail.ledger_ref : null;
    return {text: formatValue(detail.result), ledgerRef};
}

/**
 * The human-readable reason out of a "denied" ({reason}) or "failed"
 * ({error}) detail_json; unparseable input passes through as-is.
 *
 * @param {string} detailJson
 * @returns {string}
 */
export function reasonText(detailJson) {
    const detail = safeParse(detailJson);
    if (detail === null || typeof detail !== 'object')
        return String(detailJson ?? '');
    return detail.reason ?? detail.error ?? JSON.stringify(detail);
}

/** "app.lisaos.notes" → "notes" — short app label for consent UI. */
export function appShort(appId) {
    const parts = String(appId ?? '').split('.').filter(p => p !== '');
    return parts.length > 0 ? parts[parts.length - 1] : String(appId ?? '?');
}

/**
 * Map an Agent1 confirmation spec (the detail_json of a confirm-chip /
 * confirm-modal disposition, or a ConfirmationRequested spec_json) to
 * what a consent surface renders. Pure data; the frontend picks styles
 * from `modal` and lists `warnings`.
 *
 * @param {string} specJson
 * @returns {{modal: boolean, title: string, description: string,
 *   argsText: string, warnings: string[]}}
 */
export function consentView(specJson) {
    const spec = safeParse(specJson) ?? {};
    const tier = spec.effective_tier ?? spec.tier;
    const warnings = [];
    if (spec.escalated)
        warnings.push('requested via untrusted context');
    if (tier === 'destructive')
        warnings.push('destructive — this can delete data');
    else if (spec.undoable === false)
        warnings.push('not undoable');
    const args = spec.args;
    const argsText = args && typeof args === 'object' && Object.keys(args).length > 0
        ? Object.entries(args).map(([k, v]) => `${k}: ${formatValue(v)}`).join('\n')
        : '';
    return {
        modal: spec.confirmation === 'modal',
        title: `${appShort(spec.app_id)} wants to ${spec.tool ?? 'act'}`,
        description: spec.description ?? '',
        argsText,
        warnings,
    };
}
