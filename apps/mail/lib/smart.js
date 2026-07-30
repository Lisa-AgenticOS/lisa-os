// Smart grouping: the inbox sorted into the piles people already sort
// it into.
//
// The layout convention — a sidebar, a grouped message list, a reading
// pane — is the one every mail client has converged on, and Spark's
// "Smart" tab is the version worth following: newsletters and automated
// notifications are separated from mail a person wrote, because those
// are three different reading modes and mixing them is why inboxes feel
// like work.
//
// Pure classification over headers. No I/O, no GNOME, fully tested — a
// grouping rule that is wrong is not a crash, it is a message the user
// never sees, which is worse.

/// The groups, in display order. `Pinned` first because it is the
/// user's own decision and outranks any classification of ours.
export const GROUPS = ['Pinned', 'People', 'Newsletters', 'Notifications', 'Seen'];

/// Headers that mean "this was sent by a program to a list".
///
/// `List-Unsubscribe` is the strongest signal there is: it is what the
/// unsubscribe rules require of bulk senders, so it is present on
/// essentially every newsletter and absent from mail a person typed.
const BULK_HEADERS = [
    'list-unsubscribe',
    'list-id',
    'list-post',
];

/// `Precedence: bulk|list|junk` and `Auto-Submitted` are the older
/// conventions, still used by mailing lists and by anything that
/// generates mail automatically.
function isBulk(headers) {
    if (BULK_HEADERS.some((h) => headers.get(h)))
        return true;
    const precedence = headers.get('precedence').toLowerCase();
    if (['bulk', 'list', 'junk'].includes(precedence))
        return true;
    return false;
}

/// A notification is machine-sent mail you are not expected to answer:
/// alerts, receipts, password resets, CI results.
///
/// Recognised by the sender being a no-reply address or the message
/// declaring itself auto-generated — not by keyword matching on the
/// subject, which is how a real message from a person about their
/// "security alert" ends up filed away unread.
function isNotification(headers, from) {
    const auto = headers.get('auto-submitted').toLowerCase();
    if (auto && auto !== 'no')
        return true;
    if (headers.get('x-auto-response-suppress'))
        return true;
    const local = String(from.address ?? '').split('@')[0].toLowerCase();
    return /^(no-?reply|do-?not-?reply|notifications?|alerts?|mailer-daemon|postmaster)$/
        .test(local);
}

/// Which pile a message belongs in.
///
/// Order matters and is deliberate:
///
/// 1. **Pinned** — the user said so. Nothing we infer overrides that.
/// 2. **Seen** — already read, so it drops out of the working set
///    whatever it is. Spark's "Seen" section is the one that makes the
///    top of the list mean "things I have not dealt with".
/// 3. **Newsletters** before **Notifications**: a newsletter from a
///    `noreply@` address is still a newsletter, and it is the
///    unsubscribe header that says so.
/// 4. **People** — everything left. The default is the human pile,
///    because a misfiled message from a person is the expensive error
///    and a misfiled newsletter is not.
export function classify(message, headers) {
    if (message.flagged)
        return 'Pinned';
    if (message.seen)
        return 'Seen';
    const from = message.from ?? {address: headers.get('from')};
    if (isBulk(headers))
        return 'Newsletters';
    if (isNotification(headers, from))
        return 'Notifications';
    return 'People';
}

/// Group a classified list for display, dropping empty groups.
///
/// Empty sections are removed rather than shown empty: a heading with
/// nothing under it is a row of noise in a list whose whole purpose is
/// to be scannable.
export function grouped(messages) {
    const out = [];
    for (const name of GROUPS) {
        const items = (messages ?? []).filter((m) => m.group === name);
        if (items.length > 0)
            out.push({name, items});
    }
    return out;
}

/// The unread count a folder shows in the sidebar.
///
/// Counts what a person would call unread — not drafts, not trash.
export function unreadCount(messages) {
    return (messages ?? []).filter((m) => !m.seen && !m.draft && !m.trashed).length;
}
