// Lisa Settings — Providers page view-model (PLAN §5.11, ADR-0008).
//
// Pure logic over the broker's org.lisa.Remote1 `State` JSON:
// {providers: [{id, display_name, base_url, auth, dialect, notes,
// builtin, has_credential, oauth_available}], may_offload: {scope:
// bool}}. No GTK imports — unit-tests under gjs/node/jsc.

/** The distinct "leaves your hardware" color (§5.11, ADR-0008 §5). */
export const EGRESS_COLOR = '#E66100';
export const EGRESS_CSS_CLASS = 'leaves-hardware';

/** Offloadable scopes, mirroring the broker's consent table. */
export const SCOPES = [
    {id: 'prompt', label: 'Prompts', description: 'The text you type into assistant requests'},
    {id: 'files', label: 'Files', description: 'Document chunks retrieved from your files'},
    {id: 'mail', label: 'Mail', description: 'Mail content retrieved as context'},
    {id: 'calendar', label: 'Calendar', description: 'Calendar and contact context'},
    {id: 'screen', label: 'Screen', description: 'Screen captures you attach to a request'},
    {id: 'memory', label: 'App memory', description: 'Per-app durable memory contents'},
];

/**
 * Parse the broker's State JSON defensively. Anything missing renders
 * as the safe default: no providers, nothing may offload.
 *
 * @param {string|object} raw
 * @returns {{providers: object[], mayOffload: Object<string, boolean>}}
 */
export function parseState(raw) {
    let state = raw;
    if (typeof raw === 'string') {
        try {
            state = JSON.parse(raw);
        } catch {
            state = {};
        }
    }
    const mayOffload = {};
    for (const s of SCOPES)
        mayOffload[s.id] = state?.may_offload?.[s.id] === true;
    return {
        providers: Array.isArray(state?.providers) ? state.providers : [],
        mayOffload,
    };
}

/** One-line provider subtitle: endpoint + credential + caveats. */
export function describeProvider(p) {
    const parts = [];
    parts.push(p.base_url ?? 'endpoint not configured');
    parts.push(p.has_credential ? 'key set' : 'no key');
    if (!p.builtin)
        parts.push('custom');
    return parts.join(' · ');
}

/**
 * Rows for the provider list: built-ins first (registry order), then
 * custom rows sorted by id.
 *
 * @param {object[]} providers @returns {object[]}
 */
export function providerRows(providers) {
    const builtin = providers.filter(p => p.builtin);
    const custom = providers
        .filter(p => !p.builtin)
        .sort((a, b) => (a.id < b.id ? -1 : 1));
    return [...builtin, ...custom].map(p => ({
        id: p.id,
        title: p.display_name || p.id,
        subtitle: describeProvider(p),
        hasCredential: p.has_credential === true,
        builtin: p.builtin === true,
        removable: p.builtin !== true,
        showsSignIn: p.id === 'anthropic',
        signIn: claudeSignInState(p),
    }));
}

/**
 * Sign in with Claude button state. Disabled-with-reason until the
 * broker reports configured OAuth endpoints (ADR-0008 §4: Anthropic
 * publishes no registerable third-party client today — rule 8 forbids
 * guessing the URLs, so the button says why instead of lying).
 */
export function claudeSignInState(provider) {
    if (provider?.id !== 'anthropic')
        return {enabled: false, reason: 'Only available for Anthropic'};
    if (provider.oauth_available === true)
        return {enabled: true, reason: ''};
    return {
        enabled: false,
        reason: 'Not yet available: Anthropic has not published a ' +
            'sign-in client for third parties. Use an API key instead.',
    };
}

/** Consent switch rows, in stable scope order. */
export function consentRows(mayOffload) {
    return SCOPES.map(s => ({
        id: s.id,
        label: s.label,
        description: s.description,
        active: mayOffload?.[s.id] === true,
    }));
}

/** True when any scope may leave the device. */
export function anythingLeaves(mayOffload) {
    return SCOPES.some(s => mayOffload?.[s.id] === true);
}

/** Banner text for the page: measured state, in plain words. */
export function offloadSummary(mayOffload) {
    const on = SCOPES.filter(s => mayOffload?.[s.id] === true).map(s => s.label);
    if (on.length === 0)
        return 'Nothing leaves this machine.';
    return `May leave your hardware: ${on.join(', ')}.`;
}

/**
 * Validate the add-custom-provider form. Returns a list of
 * human-readable errors; empty list = valid.
 */
export function validateCustomProvider({id, displayName, baseUrl}, existingIds = []) {
    const errors = [];
    if (!id || !/^[a-z0-9][a-z0-9_-]*$/.test(id))
        errors.push('Id must be lowercase letters, digits, "-" or "_".');
    else if (existingIds.includes(id))
        errors.push(`Id "${id}" is already taken.`);
    if (!displayName || displayName.trim() === '')
        errors.push('Name must not be empty.');
    if (!baseUrl || !(baseUrl.startsWith('https://') || baseUrl.startsWith('http://')))
        errors.push('Base URL must start with https:// (or http:// for local endpoints).');
    return errors;
}
