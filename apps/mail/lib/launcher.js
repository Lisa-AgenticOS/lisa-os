// Publishing app state to the dock (#190) — the Unity LauncherEntry
// convention, from the emitter's side.
//
// The dock's half of this lives in `shell/desktop/lib/badges.js` and is
// a parser for hostile input. This is the other half, and it has the
// opposite job: emit a payload that every consumer of the convention
// already understands, so Mail badges on Lisa's dock, on Unity, on
// Plasma and on any Electron-era shell that reads the same signal.
//
// # This is a contract, not a Mail feature
//
// Nothing here knows what mail is. `launcherUpdate(appId, {count})` is
// the whole surface, and the next app to want a badge — Surfer with a
// download, the Ledger with an unreviewed action — calls the same three
// lines with its own id. The dock has no Mail-specific code and must
// never grow any: an app it has never heard of, emitting this signal,
// badges exactly the same.
//
// It lives under `apps/mail/lib/` because a shared GJS library does not
// exist yet (ADR-0047 §6 asks for one; CLAUDE.md's component map says
// plainly that it is UNBUILT). When it does, this file moves there
// unchanged — it has no Mail imports, deliberately, so that move is a
// `git mv`.
//
// # Why not a Lisa protocol
//
// Rule 8's spirit. A protocol we invented would be the same feature
// with a smaller reach: no third-party app would emit it, and our dock
// would be the only thing on earth that could read it.

/// The bus address of the convention. Named here so an emitter is not
/// retyping the path — a signal sent to the wrong object path fails
/// silently and forever, which is the failure mode this whole file is
/// written to avoid.
export const LAUNCHER_PATH = '/com/canonical/Unity/LauncherEntry';
export const LAUNCHER_IFACE = 'com.canonical.Unity.LauncherEntry';
export const LAUNCHER_SIGNAL = 'Update';
export const LAUNCHER_SIGNATURE = '(sa{sv})';

/// The D-Bus type of each property the convention defines.
///
/// Published as data so the emitter builds its variants from the same
/// table the payload comes from. `count` is `x` (int64) because that is
/// what the convention specifies and what other consumers unpack; an
/// emitter that sends `i` or `u` is dropped by strict readers.
export const PROP_TYPES = Object.freeze({
    'count': 'x',
    'count-visible': 'b',
    'progress': 'd',
    'progress-visible': 'b',
    'urgent': 'b',
});

/// `app.lisaos.Mail` or `app.lisaos.Mail.desktop` → the uri consumers
/// key on. Both spellings, because an app's own id constant is written
/// both ways across this repo and a mismatched uri is a badge that
/// simply never appears.
export function launcherUri(appId) {
    const id = String(appId ?? '').trim();
    if (id === '')
        return null;
    return `application://${id.endsWith('.desktop') ? id : `${id}.desktop`}`;
}

function wholeCount(value) {
    if (typeof value !== 'number' || !Number.isFinite(value))
        return null;
    const n = Math.trunc(value);
    return n < 0 ? null : n;
}

/// The payload for one update: `{uri, props}`, ready to be wrapped in
/// variants with `PROP_TYPES` and emitted.
///
/// `count: 0` is a real, deliberate payload — count 0 with
/// `count-visible: false` is how the convention says **clear it**, and
/// omitting the update instead would leave yesterday's number on the
/// icon forever. That asymmetry is the single most common way this
/// convention is implemented wrongly: apps emit when the number goes up
/// and go quiet when it goes to zero.
///
/// A count this app cannot compute is `null` and emits no count fields
/// at all — different from zero, which asserts "nothing is waiting".
export function launcherUpdate(appId, state = {}) {
    const uri = launcherUri(appId);
    if (uri === null)
        return null;
    const props = {};
    const count = wholeCount(state?.count);
    if (count !== null) {
        props['count'] = count;
        props['count-visible'] = count > 0;
    }
    if (typeof state?.progress === 'number' && Number.isFinite(state.progress)) {
        const p = Math.min(1, Math.max(0, state.progress));
        props['progress'] = p;
        props['progress-visible'] = p > 0 && p < 1;
    }
    if (state?.urgent === true)
        props['urgent'] = true;
    return {uri, props};
}

/// Folders whose unread count is what a dock badge means.
///
/// INBOX only, and the argument is `lib/rail.js`'s: a badge says "mail
/// is waiting". Sent, Drafts and Archive are not waiting for anybody,
/// and Spam is a permanent four-figure number that teaches people to
/// ignore the badge — which costs more than the badge was worth.
export const BADGED_FOLDERS = Object.freeze(['INBOX']);

/// The number Mail publishes: unread INBOX across every account.
///
/// Across accounts because the dock shows ONE Mail icon. A per-account
/// number on a single icon would be a number about a thing the icon
/// does not represent.
///
/// `countsFor(root, folder)` is the store's own counter, passed in so
/// this is testable without a Maildir on disk.
export function inboxUnread(accounts, countsFor) {
    let total = 0;
    for (const account of accounts ?? []) {
        for (const folder of BADGED_FOLDERS) {
            let counts;
            try {
                counts = countsFor(account?.root, folder);
            } catch {
                // One unreadable account must not cost the badge for
                // the others: an unmounted maildir is a normal Tuesday,
                // and a thrown counter would publish nothing at all.
                continue;
            }
            const unread = counts?.unread;
            if (typeof unread === 'number' && Number.isFinite(unread) && unread > 0)
                total += Math.trunc(unread);
        }
    }
    return total;
}
