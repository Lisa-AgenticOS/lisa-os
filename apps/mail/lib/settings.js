// What the settings page is allowed to say, computed from facts.
//
// Pure: every input is a fact somebody else observed — is mbsync on the
// disk, is there a Secret Service on the bus, what did GNOME Online
// Accounts report, what is actually in the Maildir. Nothing here probes
// anything, so the interesting part (what those facts *mean* to a
// person who connected an account and got no mail) is testable without
// a session bus.
//
// # Why this page exists at all
//
// A user connects Google in Settings, opens Mail, and sees nothing.
// Every layer involved is working exactly as designed and not one of
// them is in a position to say so: GOA holds an account it cannot get a
// token for, the Maildir is empty because nothing fills it, and Mail is
// correct to show an empty folder. The failure is in the gaps, which is
// precisely where no component is looking.
//
// So this page names the gap. It is a diagnostic before it is a
// preference sheet, and it is deliberately honest about the parts that
// do not exist yet (issues #154 and #155) rather than showing a Sync
// button that would do nothing.

/// Config lives in a plain JSON file. No GSettings schema: a schema has
/// to be compiled into the session's schema directory to be readable at
/// all, which makes the app undebuggable from a checkout — and this app
/// is meant to be run from one.
export const CONFIG_NAME = 'lisa/mail.json';

export const DEFAULTS = {
    /// `null` means "decide at runtime" — see `resolveMaildir`.
    maildir: null,
};

/// Read config text into an object, never throwing.
///
/// A malformed config returns the defaults. Losing your preferences to
/// a stray comma is annoying; losing your mail client to one is not a
/// trade anybody would accept, and an app that refuses to start is the
/// hardest kind of thing to fix from inside a desktop session.
export function parseConfig(text) {
    let raw;
    try {
        raw = JSON.parse(String(text ?? ''));
    } catch {
        return {...DEFAULTS};
    }
    if (!raw || typeof raw !== 'object' || Array.isArray(raw))
        return {...DEFAULTS};
    const maildir = typeof raw.maildir === 'string' && raw.maildir.trim()
        ? raw.maildir.trim()
        : null;
    return {maildir};
}

export function serializeConfig(config) {
    return `${JSON.stringify({maildir: config?.maildir ?? null}, null, 2)}\n`;
}

/// Where the Maildir actually is, and which of the three answers won.
///
/// `LISA_MAILDIR` beats the stored preference on purpose: it is how the
/// app gets pointed at a test Maildir, and an environment variable that
/// a saved setting can silently override is a debugging trap — you set
/// it, nothing changes, and the reason is invisible.
///
/// Returns the source too, so the page can say *why* the path is what
/// it is instead of showing a path with no explanation.
export function resolveMaildir({env = null, config = null, home = ''} = {}) {
    const fromEnv = typeof env === 'string' && env.trim() ? env.trim() : null;
    if (fromEnv)
        return {path: fromEnv, source: 'env'};
    const fromConfig = typeof config?.maildir === 'string' && config.maildir.trim()
        ? config.maildir.trim()
        : null;
    if (fromConfig)
        return {path: fromConfig, source: 'config'};
    return {path: `${home}/Mail`, source: 'default'};
}

/// Is this a path we are willing to store as the Maildir root?
///
/// Not a check that mail is there — an empty directory you are about to
/// sync into is perfectly valid. This refuses the shapes that are
/// mistakes: a relative path (resolved against whatever the working
/// directory happened to be), and `~`, which is shell syntax that
/// nothing in GIO expands, so storing it would produce a literal `./~`
/// directory the user never asked for.
export function validateMaildir(path) {
    const text = String(path ?? '').trim();
    if (!text)
        return {ok: false, reason: 'Enter a folder path'};
    if (text.startsWith('~'))
        return {ok: false, reason: 'Write the full path — ~ is not expanded here'};
    if (!text.startsWith('/'))
        return {ok: false, reason: 'Use an absolute path, starting with /'};
    if (text.includes('\0'))
        return {ok: false, reason: 'That is not a folder path'};
    return {ok: true, path: text.replace(/\/+$/, '') || '/'};
}

/// One line per connected account, from what GOA reported.
///
/// `mailDisabled` is shown rather than filtered out: an account whose
/// Mail switch is off looks identical to no account at all from inside
/// this app, and "you have it, it is switched off" is a different
/// problem from "you have not added it".
export function accountRows(accounts = []) {
    return (accounts ?? []).filter(Boolean).map((a) => ({
        title: a.provider || 'Account',
        subtitle: a.mailDisabled
            ? `${a.identity || 'unknown'} — Mail is switched off for this account`
            : (a.imapUser || a.identity || 'unknown'),
        usable: !a.mailDisabled,
    }));
}

/// Why there is no mail, in the order the answers block each other.
///
/// The order is the point. Each check is a precondition for the next
/// one being interesting: telling somebody their account is fine while
/// the machine has no syncer sends them to debug the wrong layer, which
/// is the specific failure this whole page exists to prevent.
export function syncStatus({
    mbsync = false,
    secretService = false,
    accounts = [],
    bridged = false,
} = {}) {
    const usable = (accounts ?? []).filter((a) => a && !a.mailDisabled);

    if (!mbsync) {
        return {
            kind: 'blocked',
            title: 'No syncer installed',
            detail: 'Mail reads a Maildir and something else fills it. This system ' +
                'has no mbsync, so nothing can. Newer Lisa OS images ship it.',
        };
    }
    if (!secretService) {
        return {
            kind: 'blocked',
            title: 'Online Accounts cannot store credentials',
            detail: 'There is no keyring on this system, so an account can be added ' +
                'and then never hand out a token. Adding one will appear to work and ' +
                'will not. Issue #154.',
        };
    }
    if (usable.length === 0) {
        return {
            kind: 'action',
            title: 'No account connected',
            detail: (accounts ?? []).length
                ? 'The connected account has Mail switched off.'
                : 'Connect one in Settings, under Online Accounts.',
        };
    }
    if (!bridged) {
        return {
            kind: 'action',
            title: 'Nothing is syncing yet',
            detail: `${usable[0].identity || 'The account'} is connected and mbsync is ` +
                'installed, but nothing yet writes the config that joins them, so the ' +
                'Maildir stays as it is. Issue #155.',
        };
    }
    return {
        kind: 'ok',
        title: 'Syncing',
        detail: 'mbsync is configured against the connected account.',
    };
}

/// What is on disk right now, as one line.
///
/// Shown next to the sync status because the two answer different
/// questions — "is anything arriving" and "is anything here" — and a
/// person debugging an empty window is usually asking both.
export function storeSummary(folders = [], counts = {}) {
    const names = (folders ?? []).filter(Boolean);
    if (names.length === 0)
        return 'No folders — this is not a Maildir, or it is empty';
    const total = names.reduce((n, f) => n + (Number(counts?.[f]) || 0), 0);
    const folderWord = names.length === 1 ? 'folder' : 'folders';
    const messageWord = total === 1 ? 'message' : 'messages';
    return `${names.length} ${folderWord}, ${total} ${messageWord}`;
}
