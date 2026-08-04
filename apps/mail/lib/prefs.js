// The preferences #249 asks for that are not folders or colours, and the
// defaults they fall back to.
//
// Every one of these adjusts a feature that already ships — compose,
// Save As, the archive and trash buttons. A settings page may only carry
// settings for things that exist (CLAUDE.md rule 10 at the UI layer), so
// there is nothing here for scheduling, snoozing or templates.
//
// The interesting part of each is the DEFAULT, because a default is what
// almost everybody gets and the only one most people ever see.

/// Which account a new message is from.
///
/// The default is CONTEXTUAL: the account whose folder you are reading.
/// Replying to Apple from the basecode address is the mistake this
/// prevents, and at eight accounts it is the mistake you make
/// constantly.
///
/// A pinned default overrides that, because somebody who sends
/// everything from one address said so explicitly and honouring the
/// sidebar instead would ignore the preference they set for exactly
/// this case.
///
/// A pin naming an account that no longer exists falls through rather
/// than failing: accounts get removed, and a dangling pin must not leave
/// the composer with no From and no explanation.
export function composeFrom(config, accounts = [], reading = null) {
    const list = Array.isArray(accounts) ? accounts : [];
    const pinned = config?.composeFrom;
    if (pinned) {
        const match = list.find((a) => a.root === pinned);
        if (match)
            return match;
    }

    if (reading && list.some((a) => a.root === reading.root))
        return reading;
    return list[0] ?? null;
}

/// What the reading pane does after a message is archived or trashed.
///
/// `next` by default, because that is what the app ALREADY DOES and the
/// reason is written where it happens: "Clearing the pane instead would
/// punish the user for acting: archive three messages and you would be
/// staring at a blank pane three times."
///
/// I had this as `list` first, on the argument that `next` puts an
/// unread message on screen — marking it read — as a side effect of
/// filing a different one. That argument is real, and it is not enough:
/// adding a preference is not a licence to change what everybody
/// already has. The trade belongs in the setting's description, where
/// the person choosing can weigh it, not in a silent default flip.
///
/// An unrecognised value is the default rather than an error: a
/// hand-edited config must not be able to leave the reading pane in a
/// state no branch handles.
const AFTER_ACTIONS = new Set(['list', 'next', 'stay']);

export function afterAction(config) {
    const want = config?.afterAction;
    return AFTER_ACTIONS.has(want) ? want : 'next';
}

/// Where Save As starts.
///
/// XDG Downloads is what the rest of the desktop answers, so it is the
/// default rather than an invented `~/Mail-attachments`. With neither a
/// preference nor an XDG directory the answer is `null` — let the file
/// dialog choose rather than pointing it at a path that may not exist.
export function saveFolder(config, xdgDownloads = null) {
    const chosen = config?.attachmentDir;
    if (typeof chosen === 'string' && chosen.trim() !== '')
        return chosen;
    return xdgDownloads || null;
}

/// Smart or Classic (#250).
///
/// **Smart is the default because it is what already ships.** The
/// grouped list with section headers has been the only list this app
/// has ever drawn, so the toggle adds Classic rather than adding Smart —
/// the same reason `afterAction` defaults to `next`.
///
/// Classic is a plain reverse-chronological list: the same messages, no
/// headers. It exists because grouping is a judgement, and a judgement
/// you cannot switch off is one you have to trust.
const VIEWS = new Set(['smart', 'classic']);

export function listView(config) {
    const want = config?.listView;
    return VIEWS.has(want) ? want : 'smart';
}

/// The list, arranged for the chosen view.
///
/// One function, so the window has no second opinion about what a
/// message is. #250 asks for exactly this: "Smart mode must not become
/// a second source of truth" — Classic is not a different classifier,
/// it is the same messages with the grouping step skipped.
///
/// `groupOf` is `smart.grouped`, injected so this stays testable without
/// importing the classifier here.
export function sections(messages, view, groupOf) {
    if (view === 'classic')
        return [{name: null, items: messages ?? []}];
    return groupOf(messages ?? []);
}
