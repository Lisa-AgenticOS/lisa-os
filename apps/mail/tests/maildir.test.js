// The id↔filename round trip, with a REAL synced filename.
//
// This suite exists because the previous fixtures were synthetic and
// alphanumeric, which made the sanitiser a no-op and hid the fact that
// `search_mail` handed out ids `read_message` rejected on every real
// maildir (#167).
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    discoverAccounts, foldersIn, isMaildirFolder, messageId, uniqueMatchesId,
    messagePath, parseFilename,
} from '../lib/maildir.js';

// Exactly what mbsync wrote on the reference device, 2026-08-02:
//   ~/Mail/INBOX/cur/1785529483.3297_1.lisa,U=8407:2,PS
const REAL = '1785529483.3297_1.lisa,U=8407:2,PS';
const REAL_UNIQUE = '1785529483.3297_1.lisa,U=8407';

test('a real mbsync filename parses into unique + flags', () => {
    const meta = parseFilename(REAL, 'cur');
    assertEq(meta.unique, REAL_UNIQUE);
    assert(meta.seen, 'S is set');
});

test('the id an agent receives round-trips back to the file it came from', () => {
    // The bug: messageId sanitises `,` and `=` to `_`, and the lookup
    // compared that against the raw name. Nothing matched.
    const id = messageId('INBOX', REAL_UNIQUE);
    assertEq(id, 'INBOX/1785529483.3297_1.lisa_U_8407');
    const idUnique = id.slice(id.indexOf('/') + 1);
    assert(uniqueMatchesId(REAL_UNIQUE, idUnique),
        'the on-disk unique part must match the id it produced');
});

test('a plain alphanumeric name still matches — the case that hid the bug', () => {
    assert(uniqueMatchesId('123.abc', '123.abc'));
});

test('the RAW unique part matches too — the window passes that one (#210)', () => {
    // The other caller, and the one nobody wrote a test for. The window's
    // list rows carry `unique` straight off the disk (listFolder keeps
    // `parseFilename`'s output verbatim), so `showMessage` asks for
    // `...lisa,U=8407` while the tools ask for `...lisa_U_8407`.
    //
    // Sanitising both sides fixed the tool path (#167) and broke this
    // one: every GUI lookup returned null, `showMessage` fell back to
    // the list row, and the row has no body — so EVERY message opened
    // to an empty reading pane on the reference device. Both spellings
    // name the same file and both must match.
    assert(uniqueMatchesId(REAL_UNIQUE, REAL_UNIQUE),
        'the window hands back the raw unique part it read off the disk');
});

test('a different message does not match', () => {
    assert(!uniqueMatchesId('1785529483.3297_1.lisa,U=9999', '1785529483.3297_1.lisa_U_8407'));
    assert(!uniqueMatchesId('', 'anything'));
    assert(!uniqueMatchesId('something', ''));
});

test('messagePath still refuses traversal after all this', () => {
    // The sanitised id is what a model hands back; the path builder is
    // the thing standing between it and the disk.
    assertEq(messagePath('/root', '..', 'cur', 'x'), null);
    assertEq(messagePath('/root', 'INBOX', 'cur', '../../etc/passwd'), null);
    assertEq(messagePath('/root', 'INBOX', 'tmp', 'x'), null, 'only cur and new');
    assert(messagePath('/root', 'INBOX', 'cur', REAL), 'a real filename is allowed');
});

// ---------------------------------------------------------------------
// #222 — both real accounts were invisible, and the mail being read was
// a stale orphan.
//
// The tree below is the reference device's `~/Mail`, exactly: a flat
// set of folders left behind by an earlier sync AND two per-account
// subtrees, at the same time. `discoverAccounts` short-circuited to
// "flat" the moment ANY root child looked like a folder, so it answered
// `[{name:'Mail', root:'~/Mail'}]`; `folders()` then listed every
// subdirectory without checking, so the two accounts appeared in the
// sidebar as EMPTY FOLDERS. 24,456 live messages were unreachable, the
// 8,407 on screen were the orphan, and `search_mail` answered out of it.
// ---------------------------------------------------------------------

/// The device tree as `[{name, isDir}]` per path, so the reader can be
/// injected and the test needs no filesystem.
const DEVICE = {
    '/Mail': [
        {name: 'Drafts', isDir: true}, {name: 'INBOX', isDir: true},
        {name: 'Sent', isDir: true}, {name: 'Spam', isDir: true},
        {name: 'Trash', isDir: true},
        {name: 'flakerimi_at_basecode.al', isDir: true},
        {name: 'flakerimi_at_gmail.com', isDir: true},
        {name: '.mbsyncstate', isDir: false},
    ],
    '/Mail/flakerimi_at_basecode.al': [
        {name: 'Drafts', isDir: true}, {name: 'INBOX', isDir: true},
        {name: 'Sent', isDir: true}, {name: 'Spam', isDir: true},
        {name: 'Trash', isDir: true},
    ],
    '/Mail/flakerimi_at_gmail.com': [
        {name: 'INBOX', isDir: true}, {name: 'Sent', isDir: true},
    ],
};
const MAILDIR = [{name: 'cur', isDir: true}, {name: 'new', isDir: true},
    {name: 'tmp', isDir: true}];

function deviceReader(path) {
    if (DEVICE[path])
        return DEVICE[path];
    // Every folder in the tree above is a real Maildir folder.
    if (/\/(Drafts|INBOX|Sent|Spam|Trash)$/.test(path))
        return MAILDIR;
    return [];
}

test('a directory is a folder only when it actually holds cur/ or new/ (#222)', () => {
    assert(isMaildirFolder('/Mail/INBOX', deviceReader));
    assert(!isMaildirFolder('/Mail/flakerimi_at_gmail.com', deviceReader),
        'an account subtree is not a folder, however much it looks like one in a sidebar');
    assert(!isMaildirFolder('/Mail/nothing-here', deviceReader));
});

test('a root holding BOTH layouts yields every account, not the first one (#222)', () => {
    const accounts = discoverAccounts('/Mail', deviceReader);
    assertEq(accounts.map((a) => a.name),
        ['flakerimi@basecode.al', 'flakerimi@gmail.com', 'Mail']);
    assertEq(accounts.map((a) => a.root), [
        '/Mail/flakerimi_at_basecode.al',
        '/Mail/flakerimi_at_gmail.com',
        '/Mail',
    ]);
    // The named accounts come FIRST, because the store opens on the
    // first one and the live mail is the one a person means. The loose
    // folders at the root are last, named plainly, and nothing about
    // them is moved or deleted — deciding they are stale is the user's
    // call, not this app's.
    assertEq(accounts[0].root, '/Mail/flakerimi_at_basecode.al');
});

test('an account subtree is not listed as a folder of the root (#222)', () => {
    // What the sidebar showed: `flakerimi_at_basecode.al` as an empty
    // folder, next to the orphan INBOX it was reading instead.
    const folders = foldersIn('/Mail', deviceReader);
    assertEq(folders, ['INBOX', 'Sent', 'Drafts', 'Spam', 'Trash']);
    assert(!folders.some((f) => f.includes('_at_')), JSON.stringify(folders));
    // …and each account's own folders, in the order a person thinks of
    // them rather than alphabetically.
    assertEq(foldersIn('/Mail/flakerimi_at_gmail.com', deviceReader), ['INBOX', 'Sent']);
});

test('the single-account tree `lisa mail setup` writes is unchanged', () => {
    // The common case must not have moved: one flat Maildir, one
    // account, named from the sync config when there is one.
    const flat = (path) => (path === '/M'
        ? [{name: 'INBOX', isDir: true}, {name: 'Sent', isDir: true}]
        : MAILDIR);
    assertEq(discoverAccounts('/M', flat, {label: 'me@example.test'}),
        [{name: 'me@example.test', root: '/M'}]);
    // And a per-account tree alone still names its accounts.
    const nested = (path) => {
        if (path === '/N') return [{name: 'a_at_b.test', isDir: true}];
        if (path === '/N/a_at_b.test') return [{name: 'INBOX', isDir: true}];
        return MAILDIR;
    };
    assertEq(discoverAccounts('/N', nested, {label: 'ignored@example.test'}),
        [{name: 'a@b.test', root: '/N/a_at_b.test'}]);
});

test('a Maildir with nothing in it has no accounts and no folders', () => {
    const empty = () => [];
    assertEq(discoverAccounts('/M', empty), []);
    assertEq(foldersIn('/M', empty), []);
});

test('the root tree gets a name of its own when a real account shares it', () => {
    // Two accounts called the same thing would be worse than the bug:
    // `store.use(name)` takes the first match, so the orphan would keep
    // being served under the live account's name.
    const reader = (path) => {
        if (path === '/M')
            return [{name: 'INBOX', isDir: true}, {name: 'Mail', isDir: true}];
        if (path === '/M/Mail')
            return [{name: 'INBOX', isDir: true}];
        return MAILDIR;
    };
    const names = discoverAccounts('/M', reader, {label: 'me@example.test'}).map((a) => a.name);
    assertEq(new Set(names).size, names.length, `duplicate account names: ${names}`);
});

finish('mail/maildir');
