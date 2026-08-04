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
import {composeFrom, afterAction, saveFolder} from '../lib/prefs.js';

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

finish('mail/prefs');
