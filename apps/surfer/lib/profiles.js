// Browsing profiles, and which one an agent gets (#259, #181).
//
// No GTK or WebKit imports, so every rule here runs under
// `just shell-test` on any host.
//
// # The agent profile is a security property, not a convenience
//
// Today one `WebKit.NetworkSession` serves everything, so a
// `navigate`/`click`/`fill` driven by the model runs inside the session
// where the person is logged into everything. A prompt-injected page can
// therefore ask the model to act as them. WebKit supports several
// sessions with distinct data directories, and a `WebView` is built
// against one — so an agent can browse as *nobody*, and the worst case
// becomes a logged-out browser.
//
// #181's wording is **"never as the user by default"**, and the reason
// this lives in a tested module is that the DEFAULT is the boundary. A
// default that is only right when a caller remembers to pass an
// argument is not a boundary — it is a convention with a security label
// on it.

/// The person's own session. Keeps the data directory Surfer already
/// uses, so nobody loses a login when profiles arrive.
export const DEFAULT_PROFILE = 'personal';

/// The session agent-driven navigation gets. Reserved: if a person can
/// delete it, an agent that talks them into deleting it gets the
/// person's session back.
export const AGENT_PROFILE = 'agent';

const RESERVED = new Set([DEFAULT_PROFILE, AGENT_PROFILE]);

export function isReserved(name) {
    return RESERVED.has(name);
}

/// May a person create a profile under this name?
///
/// The name becomes a directory component, so anything that could
/// escape the data directory is refused — a profile that escapes is one
/// reading another profile's cookies. Reserved names are refused too,
/// because two things called `agent` is one thing with a confusion.
export function validProfileName(name) {
    if (typeof name !== 'string') return false;
    const n = name.trim();
    if (n === '' || n.length > 64) return false;
    if (RESERVED.has(n)) return false;
    if (n.startsWith('.')) return false;
    // Letters, digits, space, dash, underscore. Deliberately no dot: a
    // dot is legal in a directory name and buys nothing here, while
    // `..` is the thing being kept out.
    return /^[A-Za-z0-9][A-Za-z0-9 _-]*$/.test(n);
}

/// Which profile an agent-driven call runs in.
///
/// **The agent profile, unless the person handed the agent a specific
/// tab** — the one exception #181 allows, because the person
/// deliberately gave that tab over.
///
/// Note what is NOT an escape hatch: a `profile` field on the request.
/// That would be the model choosing its own session, which is the whole
/// thing being prevented. The tab's profile comes from the tab.
///
/// Everything ambiguous fails closed: no request, junk, a non-boolean
/// `handedTab`, or a handed tab with no profile on it all land in the
/// agent profile.
export function profileForAgent(request) {
    const r = request && typeof request === 'object' ? request : {};
    if (r.handedTab !== true)
        return AGENT_PROFILE;
    const tab = r.tabProfile;
    if (typeof tab !== 'string' || tab.trim() === '')
        return AGENT_PROFILE;
    return tab;
}

/// Where one profile's data lives, or `null` if the name is unsafe.
///
/// The default profile keeps the base directory it already had, so
/// existing cookies, logins and site data survive the arrival of
/// profiles untouched. Everything else lives under `profiles/`.
export function dataDirFor(name, base) {
    if (name === DEFAULT_PROFILE) return base;
    if (name === AGENT_PROFILE) return `${base}/profiles/agent`;
    if (!validProfileName(name)) return null;
    return `${base}/profiles/${name.trim()}`;
}

/// The profile list, whatever the config says.
///
/// The two reserved profiles are always present and always first: the
/// person always has their own session, and the agent always has one to
/// be confined to. A config listing neither must not produce a browser
/// with nowhere to browse.
///
/// A duplicate or unsafe entry is dropped rather than rejecting the
/// whole list — one bad line must not cost somebody every profile they
/// made, which is the same trade `Protections::parse` makes in the
/// guard.
export function profileNamesFrom(configured) {
    const out = [DEFAULT_PROFILE, AGENT_PROFILE];
    for (const name of Array.isArray(configured) ? configured : []) {
        if (!validProfileName(name)) continue;
        const n = name.trim();
        if (!out.includes(n)) out.push(n);
    }
    return out;
}
