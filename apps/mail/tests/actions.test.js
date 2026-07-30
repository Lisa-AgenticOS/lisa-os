// Buttons, as arithmetic on filenames.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {actionsFor, buildName, flagChange, moveTo, splitName, withFlag} from '../lib/actions.js';

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

finish('mail/actions');
