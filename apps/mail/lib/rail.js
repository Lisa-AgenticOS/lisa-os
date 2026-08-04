// The account rail's decisions (#248). No GTK imports, so every rule
// here runs under `just shell-test` on any host.
//
// WHY A RAIL RATHER THAN A DEEPER TREE. At the owner's real scale —
// eight accounts — both tree shapes collapse: folder-first expands INBOX
// to eight rows and Sent to eight more; account-first is forty rows.
// Taking the account axis out of the tree makes the sidebar five rows
// again. Spark is folder-first and its sidebar is scrolled past
// "Recents" before you reach the folders you use, so inverting the
// nesting was never the fix.
//
// It also aligns the UI with the store. Sending, Drafts and signatures
// are per-account; folder-first leaves "which account am I acting in"
// ambiguous, which is how Sent and Drafts were being written into the
// orphan tree. The sidebar was the one place that inverted the store's
// shape, and that impedance mismatch is what drew two real accounts as
// empty folders (#222).

/// The account accent palette — `color.account` in
/// `branding/tokens.json`, in that order.
///
/// Identity, never status. An account tinted error-red would read as a
/// failure every time you looked at the rail, which is why these are a
/// separate token group rather than reused `color.semantic` values.
///
/// WRITTEN OUT RATHER THAN IMPORTED, and the reason is not laziness:
/// `branding/out/tokens.js` is not staged into the apps payload
/// (`os/repo-tools/build-apps-payload.sh` copies app trees, not the
/// branding directory), so importing it would resolve on a dev host and
/// throw on a device — a working test suite over an app that cannot
/// start, which is this repo's most-repeated defect. Pulling the sheet
/// into the payload is ADR-0047 §6's shared-library work and should be
/// done there, once, for every app.
///
/// So this list is a copy, and `os/repo-tools/check-tokens.py` asserts
/// it is EXACTLY `color.account`, in order — not merely that each hex
/// appears somewhere in the token file, which is all the membership
/// check would have caught.
export const ACCENTS = [
    '#6D45C9',
    '#2F8C8C',
    '#C97F2F',
    '#C04C7A',
    '#5C8A3C',
    '#556B8D',
    '#8B4A8B',
    '#A05A45',
];

/// Folders whose unread count is not a call to action.
///
/// A permanent 912 on the rail teaches people to ignore every badge on
/// it, and junk is the one folder guaranteed to carry a number like
/// that. Archive is deliberately NOT here: unread mail you filed is
/// still unread mail you have not read.
const NOT_A_CALL_TO_ACTION = new Set(['Spam', 'Junk', 'Trash']);

/// One rail entry per account, in the order the store gave them.
///
/// `countsFor(root, folder)` is `Store.counts` narrowed to what the rail
/// reads — passed in rather than imported so this stays testable without
/// a Maildir on disk.
export function railEntries(accounts, folders, countsFor, config = null) {
    if (!Array.isArray(accounts)) return [];
    return accounts.map((a) => {
        let unread = 0;
        for (const folder of folders) {
            if (NOT_A_CALL_TO_ACTION.has(folder)) continue;
            unread += countsFor(a.root, folder)?.unread ?? 0;
        }
        return {
            root: a.root,
            // The address is what the tooltip and the compose From line
            // need; the label is what fits in a rail 200px narrower.
            address: a.name,
            label: localPart(a.name),
            initial: initialOf(a.name),
            accent: colourFor(a.root, config),
            unread,
        };
    });
}

/// `flakerimi@basecode.al` → `flakerimi`. A discovered flat tree has no
/// address at all (it is named `Mail`), and must not render as an empty
/// rail button — that is #222's shape in a new place.
function localPart(name) {
    const s = String(name ?? '').trim();
    if (!s) return 'Mail';
    const at = s.indexOf('@');
    return at > 0 ? s.slice(0, at) : s;
}

/// One uppercase character, and never empty.
///
/// The first CHARACTER, not the first byte: `日本@example.test` must give
/// `日`, and many scripts have no uppercase form at all, which
/// `toUpperCase` handles by returning the character unchanged. A rail
/// button with no glyph looks unclickable, so an unusable name falls
/// back to `?` rather than to nothing.
export function initialOf(name) {
    const s = String(name ?? '').trim();
    if (!s) return '?';
    return [...s][0].toUpperCase();
}

/// The colour to draw an account in: chosen if there is one, else hashed.
///
/// #249 asked for a swatch, because the hash is stable and UNCHOSEN —
/// it never collides with what the account looks like in your head. One
/// value drives the rail, the settings row, and a future unified-list
/// stripe.
///
/// A value outside the palette is IGNORED rather than drawn.
/// `check-tokens.py` polices hex literals in source and cannot reach one
/// that arrives from a config file at runtime, and a hand-edited
/// `mail.json` is a real thing. Falling back to the hash keeps an
/// off-brief colour off the screen without refusing to draw the account.
export function colourFor(root, config = null) {
    const chosen = config?.accountColours?.[root];
    if (typeof chosen === 'string' && ACCENTS.includes(chosen))
        return chosen;
    return accentFor(root);
}

/// An account's colour, derived from its Maildir root.
///
/// From the root and not the index on purpose: assigning by position
/// means adding an account in the middle recolours every account below
/// it, and this rail is navigated by colour before it is read. The root
/// is the one identifier that survives a rename of the address and a
/// reordering of the list.
export function accentFor(root) {
    const s = String(root ?? '');
    // FNV-1a, 32-bit. Any stable hash would do; this one is four lines
    // and has no dependency.
    let h = 0x811c9dc5;
    for (let i = 0; i < s.length; i++) {
        h ^= s.charCodeAt(i);
        h = Math.imul(h, 0x01000193) >>> 0;
    }
    return ACCENTS[h % ACCENTS.length];
}

/// Should a rail toggle change the account?
///
/// GTK's grouped ToggleButtons fire `toggled` TWICE per click — once for
/// the button turning off, once for the one turning on. Acting on both
/// switches the account twice per press, and the first of the two names
/// the account you just left. So: only the press that turns one ON
/// counts, and re-pressing the account already shown is a no-op rather
/// than a rebuild that scrolls the folder list back to the top.
export function shouldSwitch(isActive, entryRoot, currentRoot) {
    if (!isActive) return false;
    if (!entryRoot) return false;
    return entryRoot !== currentRoot;
}

/// Is the rail worth showing?
///
/// One account is furniture: a chooser between one thing costs a column
/// and answers nothing. Zero is not "hide" but "there is nothing here at
/// all", which the caller handles before reaching the rail — an empty
/// rail beside an empty sidebar says the same nothing twice.
export function railIsVisible(entries) {
    return Array.isArray(entries) && entries.length > 1;
}
