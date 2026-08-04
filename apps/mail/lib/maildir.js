// Maildir, the format every Linux mail syncer already writes.
//
// Pure: paths and filenames in, structure out. The listing half takes an
// injected reader so it is testable without a filesystem.
//
// # Why Maildir and not IMAP
//
// Lisa does not reimplement mail sync. `mbsync`/`isync`, `offlineimap`
// and `notmuch` are mature, and an OS whose defining constraint is
// egress control should not grow its own network client to fetch mail
// when a battle-tested one can put the same messages on disk. So Mail
// reads a Maildir and says so; the sync layer is somebody else's job and
// a better one at it.
//
// The practical consequence is honest: Mail shows what has been synced.
// If nothing syncs, it is empty, and the app says that rather than
// pretending to be offline.

/// Is this directory a Maildir folder?
///
/// It holds `cur/` or `new/`. Nothing else is a folder — not a
/// directory that merely sits where a folder would, which is what the
/// sidebar used to assume and how two ACCOUNTS ended up drawn as two
/// empty folders (#222).
///
/// `listDir(path) -> [{name, isDir}]` is injected, so the whole layout
/// question is answerable without a filesystem.
export function isMaildirFolder(path, listDir) {
    return listDir(path).some((e) => e.isDir && (e.name === 'cur' || e.name === 'new'));
}

/// Folders, in the order a person thinks of them rather than
/// alphabetically: the inbox, then what you sent, then the rest.
export const FOLDER_ORDER = ['INBOX', 'Sent', 'Drafts', 'Archive', 'Spam', 'Trash'];

/// The folders of one account root.
///
/// Every subdirectory used to qualify. On the reference device that put
/// `flakerimi_at_basecode.al` and `flakerimi_at_gmail.com` in the
/// sidebar as folders containing nothing, next to an INBOX that was not
/// theirs.
export function foldersIn(root, listDir) {
    const found = listDir(root)
        .filter((e) => e.isDir && !e.name.startsWith('.'))
        .filter((e) => isMaildirFolder(`${root}/${e.name}`, listDir))
        .map((e) => e.name);
    const known = FOLDER_ORDER.filter((f) => found.includes(f));
    const rest = found.filter((f) => !FOLDER_ORDER.includes(f)).sort();
    return [...known, ...rest];
}

/// The accounts a Maildir root holds, and the subtree each one owns.
///
/// Two layouts exist: `lisa mail setup` writes a flat tree for one
/// account (`~/Mail/INBOX`) and a per-account tree for several
/// (`~/Mail/you_at_example.com/INBOX`). Detection is by structure, not
/// by config — a directory holding `cur/` is a folder, and a directory
/// holding folders is an account.
///
/// **BOTH AT ONCE IS THE REAL DEVICE STATE, not a corner case** (#222).
/// This returned on the first flat folder it saw, so a root containing
/// an old flat tree *and* two account subtrees answered
/// `[{name:'Mail', root:'~/Mail'}]`: both real accounts were invisible,
/// 24,456 messages were unreachable, and the 8,407 on screen were a
/// leftover tree whose Message-IDs are duplicates of the live one.
/// `search_mail` answered out of it and Sent/Drafts were written into
/// it.
///
/// Order is deliberate: **named accounts first**, because the store
/// opens on the first one and live mail is what a person means. The
/// loose folders at the root come last, as an account named plainly,
/// and NOTHING IS MOVED OR DELETED — an app that decides your mail is
/// stale and tidies it away is not a mail client. Deciding what an
/// orphan tree is for is the owner's call; showing it honestly is ours.
export function discoverAccounts(root, listDir, {label = null} = {}) {
    const dirs = listDir(root).filter((e) => e.isDir && !e.name.startsWith('.'));
    const nested = [];
    let flat = false;
    for (const d of dirs) {
        const path = `${root}/${d.name}`;
        // A folder belongs to the root itself…
        if (isMaildirFolder(path, listDir)) {
            flat = true;
            continue;
        }
        // …and a directory of folders is somebody's account.
        if (listDir(path).some((e) => e.isDir && isMaildirFolder(`${path}/${e.name}`, listDir)))
            nested.push({name: d.name.replace('_at_', '@'), root: path});
    }
    if (!flat)
        return nested;
    // The sync config names the account when the flat tree is all there
    // is. When it is not, that name belongs to one of the subtrees, and
    // two accounts with one name is worse than the bug being fixed:
    // `use(name)` takes the first match, so the orphan would keep being
    // served under the live account's name.
    let name = nested.length === 0 ? (label || 'Mail') : 'Mail';
    while (nested.some((a) => a.name === name))
        name = `${name} (root)`;
    return [...nested, {name, root}];
}

/// Flags live in the filename after `:2,` — `S`een, `R`eplied,
/// `F`lagged, `T`rashed, `D`raft, `P`assed. A message in `new/` has no
/// flag section at all and is unread by definition.
export function parseFilename(name, dir) {
    const text = String(name ?? '');
    const [unique, info = ''] = text.split(':');
    const flags = info.startsWith('2,') ? info.slice(2) : '';
    return {
        unique,
        filename: text,
        // `new/` is the arrival directory; nothing there has been read.
        seen: dir === 'new' ? false : flags.includes('S'),
        flagged: flags.includes('F'),
        replied: flags.includes('R'),
        draft: flags.includes('D'),
        trashed: flags.includes('T'),
    };
}

/// A stable id for a message, safe to put in a URL, a tool argument or a
/// filename.
///
/// The Maildir unique part can contain `,` `:` and `/`-hostile
/// characters (it embeds the hostname), and it is the thing an agent
/// will hand back to us in `read_message`. So it is normalised, and
/// `messagePath` refuses anything that did not come from here.
export function messageId(folder, unique) {
    return `${sanitise(folder)}/${sanitise(unique)}`;
}

function sanitise(s) {
    return String(s ?? '').replace(/[^A-Za-z0-9._-]/g, '_');
}

/// Does this on-disk unique part correspond to that id fragment?
///
/// THE INVERSE OF `messageId`, AND IT WAS MISSING. `messageId` maps
/// `1785529483.3297_1.lisa,U=8407` to `1785529483.3297_1.lisa_U_8407`,
/// and the lookup then compared that sanitised string against the raw
/// filename with `startsWith` — which matches only when the unique part
/// happened to contain nothing outside [A-Za-z0-9._-].
///
/// mbsync appends `,U=<uid>` to every message it syncs, so on a real
/// synced Maildir NOTHING matched: `search_mail` handed out ids that
/// `read_message` rejected with "no message". Measured on the reference
/// device 2026-08-02. The unit tests missed it because their fixtures
/// use synthetic alphanumeric names — the sanitiser is a no-op on those,
/// so both sides agreed by accident.
///
/// Sanitising BOTH sides is the fix. The mapping is lossy (two files
/// could sanitise to one id), so the caller takes the first match and
/// that is the same message `search_mail` would have listed under that
/// id — a collision means two files whose names differ only in
/// punctuation, which Maildir uniqueness already rules out in practice.
///
/// BOTH SPELLINGS OF THE WANTED SIDE MATCH, and that is the whole of
/// #210. There are two callers, not one: the tools pass the sanitised
/// id fragment, and the *window* passes `unique` straight off the disk,
/// because a list row is built by `listFolder` from `parseFilename`
/// output. Comparing sanitised-disk against a raw wanted made every GUI
/// lookup miss — `showMessage` then fell back to the list row, which
/// carries no body, so every message on the reference device opened to
/// an empty reading pane. Sanitising the wanted side too is idempotent
/// (the output alphabet is already [A-Za-z0-9._-]), so an id that was
/// sanitised once still compares equal, and the raw form now does too.
export function uniqueMatchesId(diskUnique, idUnique) {
    const wanted = String(idUnique ?? '');
    if (!wanted)
        return false;
    return sanitise(diskUnique) === sanitise(wanted);
}

/// Turn `folder`, `dir` and a filename into a path under `root`, or
/// `null`.
///
/// Every component is checked (issue #90's shape, in a different app):
/// an id arrives from a model, and a model that has read a hostile
/// message can be talked into asking for `../../.ssh/id_rsa`. Nothing
/// here interpolates a caller-supplied string into a path without first
/// refusing separators and traversal.
export function messagePath(root, folder, dir, filename) {
    for (const part of [folder, dir, filename]) {
        const s = String(part ?? '');
        if (!s || s === '.' || s === '..' || s.includes('/') || s.includes('\\') || s.includes('\0'))
            return null;
    }
    if (dir !== 'cur' && dir !== 'new')
        return null;
    return `${root}/${folder}/${dir}/${filename}`;
}

/// Build the message list for one folder.
///
/// `entries` is `[{dir, name}]` — supplied by the caller so this stays
/// pure. Sorted newest first by the Maildir timestamp prefix, which is
/// the delivery time and is what a mail list is ordered by.
export function listFolder(folder, entries) {
    return (entries ?? [])
        .filter((e) => e && (e.dir === 'cur' || e.dir === 'new'))
        .map((e) => {
            const meta = parseFilename(e.name, e.dir);
            return {
                ...meta,
                folder,
                dir: e.dir,
                id: messageId(folder, meta.unique),
                // Maildir names start with the delivery time in seconds.
                delivered: Number.parseInt(String(e.name).split('.')[0], 10) || 0,
            };
        })
        .filter((m) => !m.trashed)
        .sort((a, b) => b.delivered - a.delivered);
}

/// A one-line preview, from the readable body.
///
/// Quoted reply text and signature blocks are dropped: in a list, the
/// useful line is what THIS message says, not what it is replying to.
export function previewOf(body, max = 140) {
    const lines = String(body ?? '').split(/\r?\n/);
    // Cut at the signature delimiter BEFORE joining: `-- ` is a whole
    // line, so it has to be recognised while lines still exist. Doing
    // this after the join looked right and silently never matched.
    const end = lines.findIndex((l) => l.trimEnd() === '--');
    const kept = (end >= 0 ? lines.slice(0, end) : lines)
        .filter((l) => !l.startsWith('>'));
    const text = kept.join(' ').replace(/\s+/g, ' ').trim();
    return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
