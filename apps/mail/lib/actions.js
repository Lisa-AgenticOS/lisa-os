// What the buttons do, as arithmetic on filenames.
//
// A Maildir action is a rename. Marking read adds `S` to the flag
// section; pinning adds `F`; archiving moves the file to another
// folder's `cur/`. Nothing here touches the filesystem — these functions
// say what the new path should be, and `lisa-mail.js` performs it. That
// keeps the part that is easy to get wrong (flag order, the `new/`→`cur/`
// promotion, name collisions) testable without a disk.
//
// # What is deliberately not here
//
// **Sending.** Reply, forward and compose need SMTP, which means
// credentials and egress — a different class of thing from renaming a
// file, and one that belongs behind the same scrutiny as any other
// egress in this OS. The buttons for those exist and say why they are
// disabled rather than being quietly absent, because a mail app with no
// visible reply button reads as unfinished, and one whose reply button
// does nothing reads as broken.
//
// **Permanent deletion.** Trash is a move to the Trash folder. Maildir
// has a `T` flag meaning "deleted", but what actually removes a message
// is a separate expunge, and a button labelled with a bin that destroys
// a message immediately is not a button, it is a trap.

/// Flags a Maildir filename carries, in the order the spec requires:
/// ASCII, ascending. Mail clients disagree about a lot; they agree
/// about this, and a client that writes them unsorted produces files
/// other clients mis-sort.
const FLAG_ORDER = ['D', 'F', 'P', 'R', 'S', 'T'];

/// Split `name` into its unique part and its flags.
export function splitName(name) {
    const text = String(name ?? '');
    const i = text.indexOf(':');
    if (i < 0)
        return {unique: text, flags: []};
    const info = text.slice(i + 1);
    const flags = info.startsWith('2,') ? [...info.slice(2)] : [];
    return {unique: text.slice(0, i), flags};
}

/// Rebuild a filename from a unique part and a flag set.
///
/// Always emits the `:2,` section, even when empty: a message that has
/// been acted on lives in `cur/`, and a `cur/` file with no info section
/// is what some clients treat as new all over again.
export function buildName(unique, flags) {
    const seen = new Set(flags);
    const ordered = FLAG_ORDER.filter((f) => seen.has(f));
    return `${unique}:2,${ordered.join('')}`;
}

/// Add or remove one flag.
///
/// Returns `null` when nothing would change, so a caller can skip the
/// rename entirely — renaming a file to its own name is a no-op that
/// still costs an inode update and still races with a syncer.
export function withFlag(name, flag, on) {
    const {unique, flags} = splitName(name);
    const has = flags.includes(flag);
    if (has === on)
        return null;
    const next = on ? [...flags, flag] : flags.filter((f) => f !== flag);
    const built = buildName(unique, next);
    return built === name ? null : built;
}

/// Where a message goes when a flag changes.
///
/// A message in `new/` moves to `cur/` the moment anything is done to
/// it — that is what the two directories mean: `new/` is "delivered and
/// untouched". Clients that skip this leave read mail looking unread to
/// every other client on the same Maildir.
export function flagChange(message, flag, on) {
    const name = withFlag(message.filename, flag, on);
    if (!name && message.dir === 'cur')
        return null;
    return {
        fromDir: message.dir,
        toDir: 'cur',
        fromName: message.filename,
        toName: name ?? buildName(splitName(message.filename).unique,
            splitName(message.filename).flags),
    };
}

/// Where a message goes when it is moved to another folder.
///
/// Lands in `cur/`, not `new/`: it has been read or acted on by
/// definition — you moved it — and dropping it into another folder's
/// `new/` would make it unread again and, on a synced Maildir, generate
/// a notification for a message you just filed.
export function moveTo(message, folder) {
    if (!folder || folder === message.folder)
        return null;
    const {unique, flags} = splitName(message.filename);
    return {
        toFolder: folder,
        fromDir: message.dir,
        toDir: 'cur',
        fromName: message.filename,
        // Same name in the new folder. Maildir uniqueness is global by
        // construction (time + pid + host), so a collision here means
        // the message is already there.
        toName: buildName(unique, flags),
    };
}

/// The actions a message can be given, with their current state.
///
/// Returned as data so the toolbar is a loop rather than eight
/// hand-written buttons that drift apart — and so what each button does
/// is testable without a window.
export function actionsFor(message, folders = [], canSend = false) {
    const inTrash = message.folder === 'Trash';
    return [
        {
            id: 'pin',
            label: message.flagged ? 'Unpin' : 'Pin',
            icon: 'view-pin-symbolic',
            active: !!message.flagged,
            kind: 'flag',
            flag: 'F',
            on: !message.flagged,
        },
        {
            id: 'read',
            label: message.seen ? 'Mark as unread' : 'Mark as read',
            icon: message.seen ? 'mail-unread-symbolic' : 'mail-read-symbolic',
            active: false,
            kind: 'flag',
            flag: 'S',
            on: !message.seen,
        },
        {
            id: 'archive',
            label: 'Archive',
            // NOT `box-symbolic`, and not `mail-archive-symbolic`
            // either: neither is in Adwaita. Mail apps that show an
            // archive box ship their own copy of it, which is why the
            // name looks like it should exist. A folder with a down
            // arrow does exist and means the same thing.
            icon: 'folder-download-symbolic',
            kind: 'move',
            folder: 'Archive',
            enabled: folders.includes('Archive') && message.folder !== 'Archive',
        },
        {
            id: 'trash',
            // "Trash", never "Delete": this moves the message. What
            // actually destroys mail is an expunge, and a bin icon that
            // destroys immediately is a trap.
            label: inTrash ? 'Already in Trash' : 'Move to Trash',
            icon: 'user-trash-symbolic',
            kind: 'move',
            folder: 'Trash',
            enabled: folders.includes('Trash') && !inTrash,
        },
        {
            id: 'reply',
            label: 'Reply',
            icon: 'mail-reply-sender-symbolic',
            kind: 'send',
            // Enabled when there is an outgoing account (#168). Still
            // SHOWN when there is not, with the reason on the tooltip:
            // a mail app with no reply button reads as unfinished; one
            // whose reply button does nothing reads as broken. Saying
            // why is neither.
            enabled: !!canSend,
            why: canSend ? '' : 'Sending needs an outgoing account — run `lisa mail setup`',
        },
        {
            id: 'forward',
            label: 'Forward',
            icon: 'mail-forward-symbolic',
            kind: 'send',
            enabled: !!canSend,
            why: canSend ? '' : 'Sending needs an outgoing account — run `lisa mail setup`',
        },
    ];
}
