// The account rail's decisions (#248), with the owner's eight accounts.
//
// The rail exists because both tree shapes collapse at real scale: eight
// accounts folder-first is INBOX expanding to eight rows, Pins to eight
// more; account-first is forty rows. Taking the account axis out of the
// tree makes the sidebar five rows again.
//
// Every decision the rail makes is here rather than in the GTK code, for
// the reason #222 exists: the sidebar was the one place that inverted
// the store's shape, and the impedance mismatch drew two real accounts
// as empty folders. A rail that disagrees with the store is that bug
// again with a nicer picture.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    railEntries, initialOf, accentFor, railIsVisible, shouldSwitch, ACCENTS,
} from '../lib/rail.js';

// The device's real account set, names as `discoverAccounts` returns
// them (`_at_` already turned back into `@`).
const EIGHT = [
    {name: 'flakerimi@basecode.al', root: '/home/lisa/Mail/flakerimi_at_basecode.al'},
    {name: 'apple@example.test', root: '/home/lisa/Mail/apple_at_example.test'},
    {name: 'linkedin@example.test', root: '/home/lisa/Mail/linkedin_at_example.test'},
    {name: 'google@example.test', root: '/home/lisa/Mail/google_at_example.test'},
    {name: 'ipko@example.test', root: '/home/lisa/Mail/ipko_at_example.test'},
    {name: 'canva@example.test', root: '/home/lisa/Mail/canva_at_example.test'},
    {name: 'zzz@example.test', root: '/home/lisa/Mail/zzz_at_example.test'},
    {name: 'Mail', root: '/home/lisa/Mail'},
];

/// Unread per (root, folder), defaulting to 0 — the shape `Store.counts`
/// exposes, reduced to what the rail actually reads.
function counts(table) {
    return (root, folder) => ({unread: table[`${root}|${folder}`] ?? 0});
}

const FOLDERS = ['INBOX', 'Sent', 'Drafts', 'Archive', 'Spam', 'Trash'];

test('one entry per account, in the order the store gave them', () => {
    const rows = railEntries(EIGHT, FOLDERS, counts({}));
    assertEq(rows.length, 8);
    assertEq(rows.map((r) => r.root).join('\n'), EIGHT.map((a) => a.root).join('\n'));
});

test('the badge is that account unread total, not the folder total', () => {
    // The bug this pins: summing across accounts is what the folder-first
    // expander did, and it is the wrong number on a per-account rail.
    const rows = railEntries(EIGHT, FOLDERS, counts({
        '/home/lisa/Mail/flakerimi_at_basecode.al|INBOX': 18,
        '/home/lisa/Mail/flakerimi_at_basecode.al|Archive': 4,
        '/home/lisa/Mail/apple_at_example.test|INBOX': 3,
    }));
    assertEq(rows[0].unread, 22, 'both of this account folders, and nobody else\'s');
    assertEq(rows[1].unread, 3);
    assertEq(rows[2].unread, 0);
});

test('Spam and Trash do not raise a badge', () => {
    // A badge is a call to action. Junk is not one, and a permanent 900
    // on the rail teaches people to ignore every badge on it.
    const rows = railEntries(EIGHT, FOLDERS, counts({
        '/home/lisa/Mail/apple_at_example.test|Spam': 912,
        '/home/lisa/Mail/apple_at_example.test|Trash': 40,
        '/home/lisa/Mail/apple_at_example.test|INBOX': 2,
    }));
    assertEq(rows[1].unread, 2);
});

test('the label is the local part, and the address is kept for the tooltip', () => {
    const rows = railEntries(EIGHT, FOLDERS, counts({}));
    assertEq(rows[0].label, 'flakerimi');
    assertEq(rows[0].address, 'flakerimi@basecode.al');
    // A discovered flat tree has no address at all; it must not render
    // as an empty rail button (#222's shape).
    assertEq(rows[7].label, 'Mail');
    assertEq(rows[7].address, 'Mail');
});

test('the initial is one uppercase character, and never empty', () => {
    assertEq(initialOf('flakerimi@basecode.al'), 'F');
    assertEq(initialOf('Mail'), 'M');
    assertEq(initialOf('  spaced@x.test'), 'S');
    // Non-Latin scripts have no uppercase form for many letters; taking
    // the first CHARACTER rather than the first byte is what matters.
    assertEq(initialOf('日本@example.test'), '日');
    assertEq(initialOf('émile@x.test'), 'É');
    // Never blank: a rail button with no glyph is unclickable-looking.
    assertEq(initialOf(''), '?');
    assertEq(initialOf('   '), '?');
    assertEq(initialOf(null), '?');
});

test('an account keeps its colour across restarts and across reordering', () => {
    // Derived from the root path, not the index: assigning by position
    // means adding an account in the middle recolours every account
    // below it, and people navigate this rail by colour.
    const first = railEntries(EIGHT, FOLDERS, counts({}));
    const reordered = railEntries([...EIGHT].reverse(), FOLDERS, counts({}));
    for (const row of first) {
        const same = reordered.find((r) => r.root === row.root);
        assertEq(same.accent, row.accent, `${row.root} kept its colour`);
    }
});

test('every accent comes from the token sheet', () => {
    for (const a of EIGHT)
        assert(ACCENTS.includes(accentFor(a.root)), `${a.root} -> a real token`);
});

test('no accounts means no rail rows, not a crash', () => {
    assertEq(railEntries([], FOLDERS, counts({})).length, 0);
    assertEq(railEntries(null, FOLDERS, counts({})).length, 0);
});

test('a grouped toggle only switches on the press that turns one ON', () => {
    // GTK fires `toggled` twice per click — once for the button going
    // off, once for the one coming on. Acting on both switches twice per
    // press, and the first of the two names the account you just LEFT.
    const A = '/home/lisa/Mail/a', B = '/home/lisa/Mail/b';
    assert(shouldSwitch(true, B, A), 'the button coming on switches');
    // The button going OFF is the one for the account being LEFT, so its
    // root differs from `currentRoot` — which means only the isActive
    // guard can reject it. Writing this with A and A instead passed even
    // with that guard deleted, because the roots-differ check caught it:
    // a test that two rules both satisfy proves neither.
    assert(!shouldSwitch(false, A, B), 'the button going off does not');
    // Re-pressing the account already shown must not rebuild: a rebuild
    // scrolls the folder list back to the top under the reader.
    assert(!shouldSwitch(true, A, A), 'the account already shown is a no-op');
    // First selection, nothing current yet.
    assert(shouldSwitch(true, A, null), 'the first press selects');
    assert(!shouldSwitch(true, null, A), 'an entry with no root selects nothing');
});

test('the rail is furniture with one account', () => {
    const one = [{root: '/a'}];
    assert(!railIsVisible(one), 'a chooser between one thing answers nothing');
    assert(railIsVisible([{root: '/a'}, {root: '/b'}]));
    assert(!railIsVisible([]));
    assert(!railIsVisible(null));
});

finish('mail/rail');
