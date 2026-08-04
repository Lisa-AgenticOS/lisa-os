// The remaining preferences (#249), and the defaults they fall back to.
//
// Each of these is a preference for a feature that already exists — a
// settings page may only contain settings for things that are built
// (CLAUDE.md rule 10, applied at the UI layer). Compose, Save As and
// the archive/trash buttons are all real; none of them was adjustable.
//
// The interesting part of every one of them is the DEFAULT, because a
// default is what almost everybody gets.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {composeFrom, afterAction, saveFolder, listView, sections} from '../lib/prefs.js';

const A = {name: 'flakerimi@basecode.al', root: '/home/lisa/Mail/flakerimi_at_basecode.al'};
const B = {name: 'apple@example.test', root: '/home/lisa/Mail/apple_at_example.test'};
const ACCOUNTS = [A, B];

test('a new message comes from the account you are reading', () => {
    // The default is CONTEXTUAL, not fixed. Replying to Apple from the
    // basecode address is the mistake this prevents, and at eight
    // accounts it is the mistake you make constantly.
    assertEq(composeFrom(null, ACCOUNTS, B).name, B.name);
    assertEq(composeFrom(null, ACCOUNTS, A).name, A.name);
});

test('a pinned default overrides the account you are reading', () => {
    // Somebody who sends everything from one address said so explicitly;
    // honouring the sidebar over that would ignore the preference they
    // set for exactly this.
    const config = {composeFrom: A.root};
    assertEq(composeFrom(config, ACCOUNTS, B).name, A.name);
});

test('a pinned account that no longer exists falls back rather than failing', () => {
    // Accounts get removed. A dangling pin must not leave compose with
    // no From at all — an unsendable composer with no explanation.
    const config = {composeFrom: '/home/lisa/Mail/gone'};
    assertEq(composeFrom(config, ACCOUNTS, B).name, B.name);
});

test('with nothing to read from, the first account is the From', () => {
    assertEq(composeFrom(null, ACCOUNTS, null).name, A.name);
    assertEq(composeFrom(null, [], null), null, 'no accounts is null, not a crash');
});

test('archiving opens the next message by default — what the app already did', () => {
    // The default preserves existing behaviour, and that is the point.
    // `next` puts an unread message on screen, marking it read, as a
    // side effect of filing a different one — a real argument for
    // `list`, and not enough: adding a preference is not a licence to
    // change what everybody already has. The trade goes in the
    // setting's description, not into a silent default flip.
    assertEq(afterAction(null), 'next');
    assertEq(afterAction({}), 'next');
    assertEq(afterAction({afterAction: 'list'}), 'list');
    assertEq(afterAction({afterAction: 'stay'}), 'stay');
});

test('an unrecognised afterAction is the default, not an error', () => {
    // A hand-edited config must not be able to put the reading pane in
    // a state no branch handles.
    assertEq(afterAction({afterAction: 'explode'}), 'next');
    assertEq(afterAction({afterAction: 42}), 'next');
});

test('attachments save where the system says, until told otherwise', () => {
    // XDG Downloads is the answer the rest of the desktop gives, so it
    // is the default rather than an invented ~/Mail-attachments.
    assertEq(saveFolder(null, '/home/lisa/Downloads'), '/home/lisa/Downloads');
    assertEq(saveFolder({attachmentDir: '/home/lisa/Papers'}, '/home/lisa/Downloads'),
        '/home/lisa/Papers');
    // No XDG dir and no preference: null, so the caller lets the dialog
    // choose rather than inventing a path that may not exist.
    assertEq(saveFolder(null, null), null);
    assertEq(saveFolder({attachmentDir: ''}, '/home/lisa/Downloads'), '/home/lisa/Downloads');
});

test('Smart is the default, because Smart is what already ships', () => {
    // The grouped list with section headers is the only list this app
    // has ever drawn. So the toggle ADDS Classic; it does not add Smart.
    // Same reason afterAction defaults to `next`.
    assertEq(listView(null), 'smart');
    assertEq(listView({}), 'smart');
    assertEq(listView({listView: 'classic'}), 'classic');
    assertEq(listView({listView: 'nonsense'}), 'smart');
});

test('Classic is the same messages with the grouping skipped', () => {
    // #250: "Smart mode must not become a second source of truth about
    // what a message is." Classic is not a second classifier — it is one
    // section with everything in it, in the order it arrived.
    const messages = [{id: 1, group: 'People'}, {id: 2, group: 'Seen'}, {id: 3, group: 'People'}];
    const groupOf = () => { throw new Error('Classic must not call the classifier'); };
    const out = sections(messages, 'classic', groupOf);
    assertEq(out.length, 1);
    assertEq(out[0].name, null, 'no heading, so the renderer draws none');
    assertEq(out[0].items.map((m) => m.id).join(','), '1,2,3', 'order untouched');
});

test('Smart defers entirely to the classifier', () => {
    const messages = [{id: 1, group: 'People'}];
    let asked = null;
    const groupOf = (m) => { asked = m; return [{name: 'People', items: m}]; };
    const out = sections(messages, 'smart', groupOf);
    assertEq(asked, messages, 'the classifier decides, not this function');
    assertEq(out[0].name, 'People');
});

test('an empty list is an empty list in both views', () => {
    assertEq(sections([], 'classic', () => []).length, 1);
    assertEq(sections(null, 'classic', () => []) [0].items.length, 0);
    assertEq(sections(null, 'smart', () => []).length, 0);
});

finish('mail/prefs');
