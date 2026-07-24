// Lisa Settings — AI/Intelligence view-model (PLAN §5.3 Settings panel,
// §5.11, §8; ADR-0008).
//
// Pure logic (no GTK imports; unit-tests under gjs/node/jsc) over two
// data sources:
//   • the broker's org.lisa.Remote1 `State` JSON — {providers:[{id,
//     display_name, base_url, auth, dialect, notes, builtin,
//     has_credential, oauth_available}], may_offload:{scope:bool}};
//   • `lisa models catalog --json` — {profile:{total_ram_gb, tier, …},
//     models:[{id, task, license, engine, min_ram_gb, fit, fit_label,
//     note, installed, available}]} — local models annotated by what
//     THIS machine can run (§8 hardware-aware fit).

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

/**
 * Consent switch rows, in stable scope order. `prompt` is the primary
 * scope — inferenced always sends it, so remote inference is refused
 * while it is off; its row carries that explainer and a `primary` flag
 * the UI uses to set it apart.
 */
export function consentRows(mayOffload) {
    return SCOPES.map(s => ({
        id: s.id,
        label: s.label,
        description: s.id === 'prompt'
            ? `${s.description} — required for any remote request`
            : s.description,
        active: mayOffload?.[s.id] === true,
        primary: s.id === 'prompt',
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
 * Can remote models actually be used? Both halves of the consent trap:
 * some provider must have a stored credential AND the `prompt` scope
 * must be on — inferenced always sends scope `prompt`, and the broker
 * refuses a remote request unless every scope it carries is consented
 * (default: off). A user can therefore add a provider and a key and
 * still have every request refused; `reason` names what is missing.
 *
 * @param {{providers: object[], mayOffload: Object<string, boolean>}} state
 * @returns {{usable: boolean, hasKeyedProvider: boolean,
 *   promptAllowed: boolean, reason: 'no-key'|'prompt-off'|'ready'}}
 */
export function remoteReadiness(state) {
    const hasKeyedProvider = (state?.providers ?? [])
        .some(p => p.has_credential === true);
    const promptAllowed = state?.mayOffload?.prompt === true;
    const reason = !hasKeyedProvider ? 'no-key'
        : !promptAllowed ? 'prompt-off'
            : 'ready';
    return {usable: reason === 'ready', hasKeyedProvider, promptAllowed, reason};
}

/**
 * Why provider/consent edits are disabled, or null when they can be
 * saved. Offline (broker unreachable) the page shows defaults and every
 * write would just throw against a missing D-Bus name.
 *
 * @param {{offline: boolean}} state @returns {string|null}
 */
export function providersDisabledReason(state) {
    return state?.offline === true
        ? 'The lisa-remoted broker is not running yet — provider and ' +
            'consent changes cannot be saved.'
        : null;
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

// ---------------------------------------------------------------------
// Local models (§8 hardware-aware fit) — over `lisa models catalog
// --json`. Local inference never leaves the machine, so nothing here is
// egress-marked; the fit tells you what runs here vs what needs a
// provider.
// ---------------------------------------------------------------------

/**
 * Parse `lisa models catalog --json`. Defensive: bad/missing input
 * renders as no profile and no models (the app then shows "modeld
 * unavailable" rather than lying about capacity).
 *
 * @param {string|object} raw
 * @returns {{profile: object|null, models: object[]}}
 */
export function parseCatalog(raw) {
    let d = raw;
    if (typeof raw === 'string') {
        try {
            d = JSON.parse(raw);
        } catch {
            d = {};
        }
    }
    return {
        profile: d?.profile ?? null,
        models: Array.isArray(d?.models) ? d.models : [],
    };
}

/** A model's local-fit badge: what runs here, in plain words. */
export function fitBadge(model) {
    if (model?.installed === true)
        return {label: 'installed', kind: 'installed'};
    switch (model?.fit) {
        case 'runs':
            return {label: 'runs on this Mac', kind: 'runs'};
        case 'tight':
            return {label: 'tight fit', kind: 'tight'};
        case 'toobig':
            return {label: 'too big — use a provider', kind: 'toobig'};
        default:
            return {label: 'unknown fit', kind: 'unknown'};
    }
}

/** Human subtitle: task · license · size. */
export function localModelSubtitle(model) {
    const parts = [];
    if (model?.task)
        parts.push(model.task);
    if (model?.license)
        parts.push(model.license);
    if (typeof model?.min_ram_gb === 'number')
        parts.push(`needs ~${model.min_ram_gb} GiB`);
    return parts.join(' · ');
}

/**
 * Rows for the local-model list, ordered by usefulness on THIS machine:
 * installed first, then models that run, then tight, then remote-only;
 * ties broken by id. A model is gettable when it has a pinned artifact,
 * isn't installed, and actually fits locally.
 *
 * @param {object[]} models @returns {object[]}
 */
export function localModelRows(models) {
    const rank = m =>
        m.installed === true ? 0
            : m.fit === 'runs' ? 1
                : m.fit === 'tight' ? 2
                    : 3;
    return [...(Array.isArray(models) ? models : [])]
        .sort((a, b) => rank(a) - rank(b) || (a.id < b.id ? -1 : 1))
        .map(m => ({
            id: m.id,
            title: m.id,
            subtitle: localModelSubtitle(m),
            note: m.note ?? '',
            installed: m.installed === true,
            canGet: m.available === true && m.installed !== true && m.fit !== 'toobig',
            // Available but too big to run here: honest about why "Get"
            // is absent (it would only run through a provider).
            remoteOnly: m.fit === 'toobig',
            badge: fitBadge(m),
        }));
}

/** One-line hardware summary for the header/overview (or a fallback). */
export function profileSummary(profile) {
    if (!profile || typeof profile.total_ram_gb !== 'number')
        return 'Hardware profile unavailable (is lisa-modeld installed?).';
    const bits = [`${profile.total_ram_gb} GiB RAM`, `tier ${profile.tier}`];
    if (profile.gpu_nodes > 0)
        bits.push(`${profile.gpu_nodes} GPU`);
    if (profile.npu_nodes > 0)
        bits.push(`${profile.npu_nodes} NPU`);
    if (profile.unified_memory)
        bits.push('unified memory');
    return `This machine: ${bits.join(' · ')}.`;
}

/**
 * Per-provider model-routing help. The broker doesn't enumerate a
 * provider's models (that would need egress + a key), and rule 8 forbids
 * inventing a model list — so we surface the real routing format and the
 * provider's own registry `notes` (which carry model-id guidance, e.g.
 * HuggingFace's `openai/gpt-oss-120b:cheapest`). `null` for local-only.
 *
 * @param {object} provider @returns {{route: string, hint: string}}
 */
export function providerModelHelp(provider) {
    const id = provider?.id ?? '';
    return {
        route: `remote:${id}:<model-id>`,
        hint: provider?.notes?.trim()
            ? provider.notes.trim()
            : `Reference any model this provider serves as remote:${id}:<model-id>.`,
    };
}
