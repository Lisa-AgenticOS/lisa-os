// Dock badges from the Unity LauncherEntry convention (#190).
//
// `com.canonical.Unity.LauncherEntry.Update` is a session-bus signal
// carrying an app uri and a dictionary: `count`, `count-visible`,
// `progress`, `progress-visible`, `urgent`. Every toolkit and Electron
// app already emits it, so adopting it badges third-party apps with no
// Lisa-specific code and makes our dock just another consumer. Inventing
// a protocol would have been the same feature with a smaller reach.
//
// Stock GNOME Shell renders no dock badges. Lisa owns its dock, so this
// is a thing we can simply define.
//
// # This is a parser for hostile input
//
// A signal is something ANY session peer can emit. Nothing here trusts
// the payload: a count is believed only if it is a real non-negative
// integer, an app uri only if it is a plausible desktop id, and every
// unparseable field degrades to "no badge" rather than to a stale one.
// The worst a hostile emitter should manage is badging its OWN icon.

/// The desktop id the dock keys on, or `null`.
///
/// Refused rather than guessed at when it is not a desktop id: the dock
/// matches this against real items, so a guess that happens to collide
/// with an installed app would let any peer badge somebody else's icon —
/// a small lie told with our own UI.
export function desktopIdFromUri(uri) {
    if (typeof uri !== 'string') return null;
    let id = uri.trim();
    if (id.startsWith('application://')) id = id.slice('application://'.length);
    if (id.endsWith('/')) id = id.slice(0, -1);
    // A desktop id, and nothing that could be a path or a shell word.
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]*\.desktop$/.test(id)) return null;
    if (id.includes('..')) return null;
    return id;
}

/// The most a badge label can say before it stops being readable at
/// 20px. The count itself stays exact for tooltips and accessibility.
const LABEL_CAP = 99;

function wholeCount(value) {
    if (typeof value !== 'number' || !Number.isFinite(value)) return null;
    // Truncated rather than rounded: 2.7 unread is 2 that certainly
    // exist, and rounding up invents one.
    const n = Math.trunc(value);
    if (n <= 0) return null;
    return n;
}

function fraction(value) {
    if (typeof value !== 'number' || !Number.isFinite(value)) return null;
    return Math.min(1, Math.max(0, value));
}

/// What to draw for one LauncherEntry payload.
///
/// `count: null` means no badge — which is what `count-visible: false`
/// means in the convention, and the only way an emitter can clear one.
/// Rendering a count while the emitter says it is not visible would make
/// a badge impossible to dismiss.
export function badgeFor(props) {
    const p = props && typeof props === 'object' ? props : {};
    const count = p['count-visible'] === true ? wholeCount(p.count) : null;
    const progress = p['progress-visible'] === true ? fraction(p.progress) : null;
    return {
        count,
        // A badge reading 0 is worse than none: it draws the eye to say
        // nothing is there. `wholeCount` already refuses it.
        label: count === null ? null : (count > LABEL_CAP ? `${LABEL_CAP}+` : String(count)),
        progress,
        // Carried, not rendered — the dock can decide later. Deliberately
        // NOT turned into a count: an urgent app with nothing unread must
        // not sprout a number.
        urgent: p.urgent === true,
    };
}

/// Does this badge draw anything at all?
export function isEmpty(badge) {
    return (badge?.label ?? null) === null && (badge?.progress ?? null) === null;
}

/// The most distinct apps one session will remember a badge for.
///
/// A bound, because the emitter list is not ours to bound: any session
/// peer may emit for any uri it likes, and an unbounded map is a memory
/// leak with a public trigger. 64 is far past a plausible dock and far
/// short of a problem.
const MAX_TRACKED = 64;

/// The last thing each app published about itself.
///
/// # Why a store exists at all
///
/// Badges are drawn onto the Dash's icon actors, and **the Dash throws
/// those actors away** — it rebuilds its whole box whenever favourites
/// change, an app starts, or the icon size settles. Drawing on receipt
/// and nothing else meant a badge survived until the next unrelated app
/// launched, then silently vanished until the app happened to emit
/// again. Mail emits on sync, so "unread mail, no badge" would have been
/// the normal state of the dock within minutes.
///
/// The earlier comment against keeping a map — that it "would go stale
/// silently" — was about the wrong thing. Actor references do go stale.
/// *What an app said about itself* does not: it is the app's own truth
/// and stays true until the app says otherwise. So this stores payloads,
/// never actors, and the dock re-applies it after every rebuild.
export class BadgeState {
    constructor(max = MAX_TRACKED) {
        this._max = max;
        this._byId = new Map();
    }

    /// Record what `desktopId` published. An empty badge is a *removal*
    /// rather than a stored blank: "nothing to report" is the resting
    /// state of every app on the machine, and storing it for each one
    /// would fill the map with apps that have nothing to say.
    ///
    /// Returns the badge, so a caller can record and draw in one line.
    set(desktopId, badge) {
        if (typeof desktopId !== 'string' || desktopId === '')
            return badge;
        if (isEmpty(badge)) {
            this._byId.delete(desktopId);
            return badge;
        }
        // Re-insert to keep Map's insertion order meaningful: the
        // oldest key is the one evicted, so a peer spraying new uris
        // cannot push out an app that keeps publishing.
        this._byId.delete(desktopId);
        this._byId.set(desktopId, badge);
        while (this._byId.size > this._max)
            this._byId.delete(this._byId.keys().next().value);
        return badge;
    }

    /// What to draw for one app, or `null`.
    get(desktopId) {
        return this._byId.get(desktopId) ?? null;
    }

    /// Every badge to re-apply after the Dash rebuilds, as
    /// `[desktopId, badge]` pairs.
    entries() {
        return [...this._byId.entries()];
    }

    get size() {
        return this._byId.size;
    }

    clear() {
        this._byId.clear();
    }
}
