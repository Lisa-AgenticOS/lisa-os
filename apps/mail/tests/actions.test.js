// Buttons, as arithmetic on filenames.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    actionsFor, buildName, flagChange, movePaths, moveTo, splitName, withFlag,
} from '../lib/actions.js';
import {listFolder} from '../lib/maildir.js';

const msg = (over = {}) => ({
    folder: 'INBOX', dir: 'cur', filename: '1753900000.a.host:2,S',
    seen: true, flagged: false, ...over,
});

test('flags are written in ASCII order, whatever order they arrive in', () => {
    // Clients disagree about a lot; they agree about this, and unsorted
    // flags make other clients mis-sort the same maildir.
    assertEq(buildName('1.a.host', ['S', 'F', 'D']), '1.a.host:2,DFS');
    assertEq(buildName('1.a.host', []), '1.a.host:2,');
    assertEq(splitName('1.a.host:2,FS'), {unique: '1.a.host', flags: ['F', 'S']});
    // A name with no info section at all.
    assertEq(splitName('1.a.host'), {unique: '1.a.host', flags: []});
});

test('setting a flag that is already set changes nothing', () => {
    // Renaming a file to its own name still costs an inode update and
    // still races with whatever is syncing the maildir.
    assertEq(withFlag('1.a.host:2,S', 'S', true), null);
    assertEq(withFlag('1.a.host:2,', 'S', false), null);
    assertEq(withFlag('1.a.host:2,S', 'S', false), '1.a.host:2,');
    assertEq(withFlag('1.a.host:2,S', 'F', true), '1.a.host:2,FS');
});

test('acting on a message in new/ promotes it to cur/', () => {
    // That is what the two directories mean. Clients that skip this
    // leave read mail looking unread to everything else on the maildir.
    const change = flagChange(msg({dir: 'new', filename: '1753900000.a.host', seen: false}), 'S', true);
    assertEq(change.fromDir, 'new');
    assertEq(change.toDir, 'cur');
    assertEq(change.toName, '1753900000.a.host:2,S');
});

test('a no-op flag change on a cur/ message is skipped entirely', () => {
    assertEq(flagChange(msg(), 'S', true), null);
});

test('a move lands in cur/, never new/', () => {
    // Dropping a filed message into another folder's new/ makes it
    // unread again and, on a synced maildir, notifies about a message
    // you just filed.
    const mv = moveTo(msg(), 'Archive');
    assertEq(mv.toFolder, 'Archive');
    assertEq(mv.toDir, 'cur');
    assertEq(mv.toName, '1753900000.a.host:2,S');
    // Moving somewhere it already is does nothing.
    assertEq(moveTo(msg({folder: 'Archive'}), 'Archive'), null);
    assertEq(moveTo(msg(), ''), null);
});

test('trash is a move, and it says so', () => {
    const trash = actionsFor(msg(), ['INBOX', 'Archive', 'Trash']).find((a) => a.id === 'trash');
    assertEq(trash.kind, 'move');
    assertEq(trash.folder, 'Trash');
    // Never labelled "Delete": what destroys a message is an expunge,
    // and a bin that destroys immediately is a trap.
    assert(!trash.label.toLowerCase().includes('delete'), trash.label);
    // Already there: offered, and disabled.
    assertEq(actionsFor(msg({folder: 'Trash'}), ['Trash']).find((a) => a.id === 'trash').enabled, false);
});

test('an action for a folder that does not exist is disabled, not hidden', () => {
    const noArchive = actionsFor(msg(), ['INBOX']).find((a) => a.id === 'archive');
    assertEq(noArchive.enabled, false);
    assertEq(actionsFor(msg(), ['INBOX', 'Archive']).find((a) => a.id === 'archive').enabled, true);
});

test('reply and forward are shown, disabled, and explain themselves', () => {
    // A mail app with no reply button reads as unfinished; one whose
    // reply button does nothing reads as broken. Saying why is neither.
    for (const id of ['reply', 'forward']) {
        const a = actionsFor(msg(), []).find((x) => x.id === id);
        assertEq(a.enabled, false, id);
        assert(a.why && a.why.length > 10, `${id} does not say why`);
    }
});

test('the labels track the current state, so the button says what it will do', () => {
    assertEq(actionsFor(msg({seen: true}), []).find((a) => a.id === 'read').label, 'Mark as unread');
    assertEq(actionsFor(msg({seen: false}), []).find((a) => a.id === 'read').label, 'Mark as read');
    assertEq(actionsFor(msg({flagged: true}), []).find((a) => a.id === 'pin').label, 'Unpin');
    // …and the flag it would set is the opposite of the current state.
    assertEq(actionsFor(msg({seen: true}), []).find((a) => a.id === 'read').on, false);
    assertEq(actionsFor(msg({flagged: false}), []).find((a) => a.id === 'pin').on, true);
});

test('every action can be drawn without its icon', () => {
    // The icon theme is somebody else's, and it changes: `box-symbolic`
    // was never in Adwaita, so the Archive button rendered as a gap on
    // the first device that saw it. The window falls back to the label
    // when a name does not resolve — which only works if there is
    // always a label worth reading, so that is what is checked here.
    for (const a of actionsFor(msg(), ['INBOX', 'Archive', 'Trash'])) {
        assert(a.label && a.label.length > 2, `${a.id} has no usable label`);
        assert(a.icon && /^[a-z0-9-]+-symbolic$/.test(a.icon), `${a.id} icon: ${a.icon}`);
    }
});


test('reply and forward follow whether an account exists (#168)', () => {
    const msg = {folder: 'INBOX', dir: 'cur', filename: 'a:2,S', seen: true, flagged: false};
    const off = actionsFor(msg, ['INBOX'], false).find((a) => a.id === 'reply');
    assertEq(off.enabled, false);
    assert(off.why.includes('lisa mail setup'), 'and says how to fix it');
    const on = actionsFor(msg, ['INBOX'], true).find((a) => a.id === 'reply');
    assertEq(on.enabled, true);
    assertEq(on.why, '', 'no excuse when it works');
    // Shown either way: a missing reply button reads as unfinished.
    assert(actionsFor(msg, ['INBOX'], false).some((a) => a.id === 'forward'));
});

// --- which account an action lands in (#264) --------------------------
//
// THE TEST THAT WOULD HAVE CAUGHT IT. Every action used to resolve
// against `store.root`, one mutable module-level string that
// `Store.use(account)` overwrote — so a second window, or one click in
// the sidebar, silently repointed the trash/archive/move of a message
// that was listed from somewhere else.

const WORK = '/home/x/Mail/you_at_work.test';
const HOME = '/home/x/Mail/you_at_home.test';

test('a listed message remembers which account it was listed from (#264)', () => {
    const [row] = listFolder('INBOX', [{dir: 'cur', name: '1785529483.a.host:2,S'}], {root: WORK});
    assertEq(row.root, WORK);
    // No root supplied is `null`, never a guess. A row with no account
    // is a row nothing may act on — see the refusal below.
    assertEq(listFolder('INBOX', [{dir: 'cur', name: '1.a.host:2,S'}])[0].root, null);
});

test('a move resolves against the listing account, not the current one (#264)', () => {
    const [row] = listFolder('INBOX', [{dir: 'cur', name: '1785529483.a.host:2,S'}], {root: WORK});
    const paths = movePaths(row, moveTo(row, 'Trash'), 'Trash');
    assertEq(paths.from, `${WORK}/INBOX/cur/1785529483.a.host:2,S`);
    assertEq(paths.to, `${WORK}/Trash/cur/1785529483.a.host:2,S`);
    assertEq(paths.dir, `${WORK}/Trash/cur`);
    // The point of the whole issue: whatever else the window is showing
    // now, THIS message's file is under the account it came from.
    assert(!paths.from.startsWith(HOME), `${paths.from} escaped into the other account`);
    assert(!paths.to.startsWith(HOME), `${paths.to} escaped into the other account`);
});

test('two accounts, two answers, from the same folder name (#264)', () => {
    // Same folder, same filename, different account — the aliasing was
    // invisible with one account because both answers were identical.
    const name = '1785529483.a.host:2,S';
    const [work] = listFolder('INBOX', [{dir: 'cur', name}], {root: WORK});
    const [home] = listFolder('INBOX', [{dir: 'cur', name}], {root: HOME});
    assert(movePaths(work, moveTo(work, 'Archive'), 'Archive').to !==
           movePaths(home, moveTo(home, 'Archive'), 'Archive').to,
    'two accounts must not resolve to one path');
});

test('a message with no account is refused, not guessed at (#264)', () => {
    const [row] = listFolder('INBOX', [{dir: 'cur', name: '1.a.host:2,S'}]);
    assertEq(movePaths(row, moveTo(row, 'Trash'), 'Trash'), null);
    // …and the path checks messagePath already makes are still made
    // here, because this is the only builder the window calls now.
    const [ok] = listFolder('INBOX', [{dir: 'cur', name: '1.a.host:2,S'}], {root: WORK});
    assertEq(movePaths(ok, moveTo(ok, 'Trash'), '../../etc'), null);
    assertEq(movePaths(ok, {fromDir: 'cur', toDir: 'nowhere', fromName: 'a', toName: 'a'},
        'Trash'), null);
    assertEq(movePaths(null, moveTo(ok, 'Trash'), 'Trash'), null);
    assertEq(movePaths(ok, null, 'Trash'), null);
});

finish('mail/actions');
