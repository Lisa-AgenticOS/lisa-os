// Publishing the unread count to the dock (#190), from the emitter's
// side.
//
// The wire spelling is the whole contract. A hyphen turned into a camel
// hump, an `i` where the convention says `x`, a uri missing its
// `.desktop` — every one of those is a signal that is sent, accepted by
// the bus, and silently ignored by every consumer on the machine. There
// is no error to notice, which is why these are assertions and not a
// comment saying "keep these in sync".
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    LAUNCHER_IFACE, LAUNCHER_PATH, LAUNCHER_SIGNAL, LAUNCHER_SIGNATURE,
    PROP_TYPES, inboxUnread, launcherUpdate, launcherUri,
} from '../lib/launcher.js';

test('the address is the convention\'s, character for character', () => {
    // Not ours to choose. A path we spelled our own way is a signal
    // nothing on the machine is listening for.
    assertEq(LAUNCHER_PATH, '/com/canonical/Unity/LauncherEntry');
    assertEq(LAUNCHER_IFACE, 'com.canonical.Unity.LauncherEntry');
    assertEq(LAUNCHER_SIGNAL, 'Update');
    assertEq(LAUNCHER_SIGNATURE, '(sa{sv})');
});

test('the property names are hyphenated, and the count is an int64', () => {
    // `countVisible` would be accepted by the bus and understood by
    // nobody; `i` instead of `x` is dropped by strict consumers.
    assertEq(PROP_TYPES['count'], 'x');
    assertEq(PROP_TYPES['count-visible'], 'b');
    assertEq(PROP_TYPES['progress'], 'd');
    assertEq(PROP_TYPES['progress-visible'], 'b');
    assertEq(PROP_TYPES['urgent'], 'b');
});

test('the uri names the app, with or without the suffix', () => {
    assertEq(launcherUri('app.lisaos.Mail'), 'application://app.lisaos.Mail.desktop');
    assertEq(launcherUri('app.lisaos.Mail.desktop'), 'application://app.lisaos.Mail.desktop');
    assertEq(launcherUri(''), null);
    assertEq(launcherUri(null), null);
});

test('a count of zero is published, not withheld', () => {
    // The most common way this convention is implemented wrongly: emit
    // when the number goes up, go quiet when it goes to zero, and leave
    // yesterday's number on the icon forever. Zero with
    // `count-visible: false` is the convention's own "clear it".
    const update = launcherUpdate('app.lisaos.Mail', {count: 0});
    assertEq(update.props['count'], 0);
    assertEq(update.props['count-visible'], false);
});

test('a real count is visible', () => {
    const update = launcherUpdate('app.lisaos.Mail', {count: 7});
    assertEq(update.uri, 'application://app.lisaos.Mail.desktop');
    assertEq(update.props['count'], 7);
    assertEq(update.props['count-visible'], true);
});

test('an uncomputable count says nothing, which is not the same as zero', () => {
    // Zero asserts "nothing is waiting". A count we failed to compute
    // asserts nothing at all, and must not clear a badge that may still
    // be true.
    assertEq(launcherUpdate('app.lisaos.Mail', {}).props, {});
    assertEq(launcherUpdate('app.lisaos.Mail', {count: null}).props, {});
    assertEq(launcherUpdate('app.lisaos.Mail', {count: NaN}).props, {});
    assertEq(launcherUpdate('app.lisaos.Mail', {count: -4}).props, {},
        'a negative count is a bug upstream, not a badge');
});

test('progress is a fraction, and is only "in progress" between the ends', () => {
    assertEq(launcherUpdate('x', {progress: 0.4}).props['progress'], 0.4);
    assertEq(launcherUpdate('x', {progress: 0.4}).props['progress-visible'], true);
    assertEq(launcherUpdate('x', {progress: 1}).props['progress-visible'], false,
        'finished is not in progress');
    assertEq(launcherUpdate('x', {progress: 0}).props['progress-visible'], false);
    assertEq(launcherUpdate('x', {progress: 3}).props['progress'], 1, 'clamped');
    assert(!('progress' in launcherUpdate('x', {}).props));
});

test('an app with no id publishes nothing rather than a broken uri', () => {
    assertEq(launcherUpdate('', {count: 3}), null);
    assertEq(launcherUpdate(null, {count: 3}), null);
});

// ---- what Mail actually counts ---------------------------------------

const counts = (table) => (root, folder) => table[`${root}/${folder}`];

test('the badge is unread INBOX, summed across every account', () => {
    // One Mail icon in the dock, so one number. A per-account count on
    // a single icon would be a number about something the icon does not
    // represent.
    const accounts = [{root: '/m/a'}, {root: '/m/b'}];
    const table = {
        '/m/a/INBOX': {total: 40, unread: 3},
        '/m/b/INBOX': {total: 12, unread: 4},
    };
    assertEq(inboxUnread(accounts, counts(table)), 7);
});

test('Sent, Drafts, Archive and Spam are not waiting for anybody', () => {
    // lib/rail.js excludes them from its badge for the same reason: a
    // permanent four-figure Spam count teaches people to ignore the
    // badge, which costs more than the badge was worth.
    const accounts = [{root: '/m/a'}];
    const table = {
        '/m/a/INBOX': {unread: 2},
        '/m/a/Sent': {unread: 500},
        '/m/a/Drafts': {unread: 9},
        '/m/a/Archive': {unread: 4000},
        '/m/a/Spam': {unread: 912},
        '/m/a/Trash': {unread: 88},
    };
    assertEq(inboxUnread(accounts, counts(table)), 2);
});

test('one unreadable account does not cost the others their badge', () => {
    // An unmounted maildir is a normal Tuesday. A counter that throws
    // must not take the whole number down with it.
    const accounts = [{root: '/m/gone'}, {root: '/m/b'}];
    const countsFor = (root, folder) => {
        if (root === '/m/gone')
            throw new Error('no such directory');
        return {unread: 5};
    };
    assertEq(inboxUnread(accounts, countsFor), 5);
});

test('no accounts, no counts, no nonsense is zero — a clearable badge', () => {
    assertEq(inboxUnread([], () => ({unread: 3})), 0);
    assertEq(inboxUnread(null, () => ({unread: 3})), 0);
    assertEq(inboxUnread([{root: '/m/a'}], () => undefined), 0);
    assertEq(inboxUnread([{root: '/m/a'}], () => ({unread: -2})), 0);
    assertEq(inboxUnread([{root: '/m/a'}], () => ({unread: '4'})), 0,
        'a string is not a count');
});

finish('mail/launcher');
