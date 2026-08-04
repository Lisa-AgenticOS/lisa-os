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
    /// Load images and other remote content in messages without asking.
    ///
    /// DEFAULT ON, by the project owner's decision (2026-08-02), and
    /// the trade is written down here rather than left for someone to
    /// rediscover: a remote image is how a tracking pixel works. With
    /// this on, opening a message tells the sender that you opened it,
    /// when, and roughly from where. Most mail clients default it off
    /// for exactly that reason; the counter-argument is that a mail
    /// client which renders every newsletter as broken boxes is one
    /// people stop using, and a banner on every message trains them to
    /// click banners.
    ///
    /// It is a setting, so it is the user's to change either way, and
    /// the settings page says what it costs.
    showRemoteImages: true,
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
    // Only an explicit `false` turns it off. A missing key is an older
    // config, not a preference — and silently reading "absent" as "off"
    // would change behaviour on upgrade with nothing to explain it.
    const showRemoteImages = raw.showRemoteImages === false ? false : DEFAULTS.showRemoteImages;
    return {maildir, showRemoteImages};
}

export function serializeConfig(config) {
    return `${JSON.stringify({
        maildir: config?.maildir ?? null,
        showRemoteImages: config?.showRemoteImages !== false,
    }, null, 2)}\n`;
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


/// Why there is no mail, in the order the answers block each other.
///
/// The order is the point. Each check is a precondition for the next
/// one being interesting: telling somebody their account is fine while
/// the machine has no syncer sends them to debug the wrong layer, which
/// is the specific failure this whole page exists to prevent.
///
/// # One decision, two surfaces (#265)
///
/// This is the ONLY thing allowed to decide what "blocked" means. The
/// settings page renders it, the main window renders it as a banner,
/// and #249 will render it per account. Adding a case here changes all
/// of them; a second opinion anywhere else is how two surfaces start
/// disagreeing about the same machine.
///
/// # `action`
///
/// Each answer says whether there is anything to *press*, as
/// `{id, label}` or `null`. That is deliberately part of the return
/// rather than a lookup table beside it: a banner that draws a button
/// for a state nothing can fix teaches people that the buttons do
/// nothing, and "there is nothing to offer here" is a decision about
/// the state, so it belongs with the state. The ids are handled by the
/// window (`unlock-keyring`, `online-accounts`).
export function syncStatus({
    mbsync = false,
    secretService = false,
    keyringLocked = false,
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
            // Nothing this app can press installs a package, and a
            // button that would need `sudo` is one we may not offer
            // (ADR-0034, `escalate.privilege` is an unoverridable Deny).
            action: null,
        };
    }
    if (!secretService) {
        return {
            kind: 'blocked',
            title: 'Online Accounts cannot store credentials',
            detail: 'There is no keyring on this system, so an account can be added ' +
                'and then never hand out a token. Adding one will appear to work and ' +
                'will not. Issue #154.',
            action: null,
        };
    }
    if (usable.length === 0) {
        return {
            kind: 'action',
            title: 'No account connected',
            detail: (accounts ?? []).length
                ? 'The connected account has Mail switched off.'
                : 'Connect one in Settings, under Online Accounts.',
            action: {id: 'online-accounts', label: 'Open Online Accounts'},
        };
    }
    if (!bridged) {
        return {
            kind: 'action',
            title: 'Nothing is syncing yet',
            detail: `${usable[0].identity || 'The account'} is connected and mbsync is ` +
                'installed, but nothing yet writes the config that joins them, so the ' +
                'Maildir stays as it is. Issue #155.',
            // `lisa mail setup` is a terminal command that asks
            // questions. Launching it from a banner would either run it
            // blind or open a terminal the person did not ask for, so
            // the banner says the command and stops.
            action: null,
        };
    }
    // Everything is wired and nothing arrives: this is the state the
    // reference device sat in for a day (#265). mbsync asks `lisa mail
    // token` for a credential once per run, `lisa` refuses while the
    // login collection is locked, and lisa-mail-sync.service fails
    // every five minutes with nobody watching the journal.
    //
    // The words are the CLI's own (cli/lisa/src/mail.rs `token`),
    // because they are already right: they name the layer that is
    // stuck, say it is expected after a reboot rather than a fault,
    // and give the fix.
    if (keyringLocked) {
        return {
            kind: 'blocked',
            title: 'The login keyring is locked',
            detail: 'The login keyring is locked, so no account can hand over a token. ' +
                'This happens after every reboot: autologin starts the session but ' +
                'never unlocks the keyring. Unlock it once at the machine and mail ' +
                'resumes on its own.',
            action: {id: 'unlock-keyring', label: 'Unlock'},
        };
    }
    return {
        kind: 'ok',
        title: 'Syncing',
        detail: 'mbsync is configured against the connected account.',
        action: null,
    };
}

/// One line for a banner, out of a status that has two.
///
/// The settings page has a title row and a subtitle under it; an
/// `Adw.Banner` has a single string. Dropping either half loses
/// something real — the title alone rarely says what to do, and some
/// details do not say what is wrong — so they are joined.
///
/// The exception is a detail that already opens with its own title,
/// which is the locked-keyring case: the CLI's sentence begins "The
/// login keyring is locked, so…", and prefixing it with "The login
/// keyring is locked — " is a sentence stuttering at the reader.
export function bannerText(status) {
    const title = String(status?.title ?? '').trim();
    const detail = String(status?.detail ?? '').trim();
    if (!detail)
        return title;
    if (!title || detail.toLowerCase().startsWith(title.toLowerCase()))
        return detail;
    return `${title} — ${detail}`;
}

/// How long ago mail last arrived from the server, as one quiet line.
///
/// Shown because stale mail that looks current is a different and worse
/// experience from stale mail with a timestamp on it (#265): the window
/// is correct either way, and only one of them lets a person notice
/// that nothing has come in since breakfast.
///
/// Seconds in, seconds in, so `now` is an argument and the function is
/// testable. `Never synced` for anything that is not a real timestamp —
/// a missing file, a zero, a string — because "just now" is the one
/// answer that must never be invented. A time in the future is a clock
/// that moved, not a sync that has not happened yet, so it reads as
/// just now rather than as a negative age.
export function lastSynced(seconds, now) {
    const at = Number(seconds);
    if (!Number.isFinite(at) || at <= 0)
        return 'Never synced';
    const age = Math.max(0, Math.floor(Number(now) - at));
    if (age < 60)
        return 'Synced just now';
    const [count, unit] = age < 3600
        ? [Math.floor(age / 60), 'minute']
        : age < 86400
            ? [Math.floor(age / 3600), 'hour']
            : [Math.floor(age / 86400), 'day'];
    return `Synced ${count} ${unit}${count === 1 ? '' : 's'} ago`;
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
