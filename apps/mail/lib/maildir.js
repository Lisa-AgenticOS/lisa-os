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
    const safe = String(unique ?? '').replace(/[^A-Za-z0-9._-]/g, '_');
    const folderSafe = String(folder ?? '').replace(/[^A-Za-z0-9._-]/g, '_');
    return `${folderSafe}/${safe}`;
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
