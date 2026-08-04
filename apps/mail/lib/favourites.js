// The curated sidebar (#249): which of an account's folders show.
//
// `reloadFolders` renders every directory the Maildir contains, which is
// the disk rather than a decision. At one account that is fine; at the
// owner's eight it is not, because a Maildir accumulates folders nobody
// chose — Spark's own list carries `beanstalk`, `Boomerang-Outbox`,
// `Delegated`, `Snoozed` and `Reminders`. The sidebar should be a
// curated subset.
//
// THE RULE THAT MATTERS IS THE DEFAULT. A curation feature whose unset
// state is "show nothing" turns a working app into an empty one the
// moment it ships, and the person it happens to has no way to know a
// setting did it. So an account nobody has curated is an account where
// everything is a favourite — the feature is opt-out per folder, never
// opt-in per folder.
//
// Config shape, in `lisa/mail.json`:
//
//     "favouriteFolders": { "<maildir root>": ["INBOX", "Sent"] }
//
// Keyed by the Maildir root and not the address, for the reason
// `lib/rail.js` colours by root: it is the identifier that survives
// renaming the account.

/// Has this account been curated at all?
function curated(config, root) {
    const table = config?.favouriteFolders;
    return !!table && Object.hasOwn(table, root) && Array.isArray(table[root]);
}

/// The favourites of one account, as a list.
///
/// `onDisk` is what the account really has; an uncurated account
/// reports all of it, which is what the sidebar is already doing and
/// what the settings checkboxes must therefore render as ticked.
export function favouritesFor(config, root, onDisk = []) {
    if (!curated(config, root))
        return [...onDisk];
    return config.favouriteFolders[root];
}

/// Is one folder a favourite? Ticked by default, per the rule above.
export function isFavourite(config, root, folder) {
    if (!curated(config, root))
        return true;
    return config.favouriteFolders[root].includes(folder);
}

/// What the sidebar shows for one account.
///
/// Order comes from `onDisk` — the caller's ordering, INBOX / Sent /
/// Drafts then the rest alphabetically — never from the order folders
/// were ticked. A sidebar that reorders itself while you configure it
/// is disorienting in a way no one reports as a bug.
///
/// Two floors, because a settings page that can produce an empty
/// sidebar is a trap: INBOX always shows, and if there is no INBOX on
/// disk either then everything comes back. An empty sidebar is never
/// the right answer to "which folders do you want".
export function visibleFolders(onDisk = [], config = null, root = '') {
    if (!curated(config, root))
        return [...onDisk];
    const wanted = new Set(config.favouriteFolders[root]);
    const shown = onDisk.filter((f) => wanted.has(f));
    if (shown.length > 0)
        return shown;
    if (onDisk.includes('INBOX'))
        return ['INBOX'];
    return [...onDisk];
}

/// Turn one folder's favourite state over, returning a NEW config.
///
/// Never mutates its argument: the caller holds the config it read from
/// disk, and a settings page that edits it in place makes "did the save
/// succeed" unanswerable — the in-memory copy already changed.
///
/// The first toggle on an uncurated account starts from *everything*
/// and removes one. Starting from nothing and adding one would collapse
/// the sidebar to a single folder on the first click, which reads as the
/// app breaking rather than as a preference taking effect.
export function toggleFavourite(config, root, folder, onDisk = []) {
    const current = favouritesFor(config, root, onDisk);
    const next = current.includes(folder)
        ? current.filter((f) => f !== folder)
        : [...current, folder];
    return {
        ...config,
        favouriteFolders: {...(config?.favouriteFolders ?? {}), [root]: next},
    };
}
