// Honest per-account state (#249) — a diagnostic before it is a
// preference sheet.
//
// A person connects Google in Settings, opens Mail, and sees nothing.
// Every layer is working exactly as designed and not one of them is in a
// position to say so: GOA holds an account, `lisa mail setup` has not
// run so mbsync knows nothing about it, the Maildir is empty because
// nothing fills it, and Mail is correct to show an empty folder. The
// failure is in the gaps, which is precisely where no component is
// looking.
//
// So a row names the layer that is blocking and offers the ONE action
// that unblocks it. "Set up sync" on an account that is already set up
// is how a person loops; "Set up sync" on an account whose owner
// switched mail off in Online Accounts is us overriding their decision.
//
// Pure. Every input is a fact somebody else observed: what GOA reported,
// whether the mbsync config names this account, whether a Maildir root
// exists and how many messages are in it.

/// Thousands separators without pulling in a locale: 8407 → "8,407".
/// A five-digit message count read as a bare number is a wall.
function grouped(n) {
    return String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

/// One account's state.
///
/// `goa` is the Online Accounts record or `null`; `configured` is
/// whether `~/.config/lisa/mbsyncrc` declares a store for it; `root` is
/// its Maildir if one exists; `messages` is how many are in it.
export function accountState({goa = null, configured = false, root = null, messages = 0} = {}) {
    // A tree with no account behind it. On the reference device this is
    // ~/Mail/{INBOX,Sent,Drafts,Spam} — 9,125 messages that no mbsync
    // channel points at, duplicating the live account's mail (#224).
    //
    // Named rather than hidden, and with NO action: it is somebody's
    // mail, and an app that offers a one-click removal of 9,125 messages
    // has made an irreversible decision look like a preference.
    if (!goa) {
        return {
            state: 'orphaned',
            title: root || 'Unknown',
            detail: `${grouped(messages)} message(s) on disk with no account behind them — ` +
                'nothing keeps it up to date. Remove the folder yourself, or connect ' +
                'the account it came from.',
            action: null,
            ok: false,
        };
    }

    const title = goa.identity || goa.imapUser || goa.provider || 'Account';

    // Their decision, in Settings, and not ours to route around. Running
    // mbsync here would fetch mail for an account whose owner told GOA
    // not to.
    if (goa.mailDisabled) {
        return {
            state: 'mail-off',
            title,
            detail: 'Mail is switched off for this account in Online Accounts.',
            action: 'online-accounts',
            ok: false,
        };
    }

    // The silent failure this whole group exists for.
    if (!configured) {
        return {
            state: 'never-set-up',
            title,
            detail: 'Connected, but sync has never been configured. ' +
                'Set up sync to start fetching this account.',
            action: 'setup',
            ok: false,
        };
    }

    if (!root || messages === 0) {
        return {
            state: 'never-synced',
            title,
            detail: 'Set up, but nothing has been fetched yet. Sync now.',
            action: 'sync',
            ok: false,
        };
    }

    return {
        state: 'synced',
        title,
        detail: `${grouped(messages)} message(s) on disk.`,
        action: null,
        ok: true,
    };
}

/// One row per account, in the caller's order.
export function accountStates(facts) {
    if (!Array.isArray(facts)) return [];
    return facts.map((f) => accountState(f));
}
