// The settings page's job is to say why there is no mail. Getting that
// wrong sends a person to debug the wrong layer, which is worse than
// saying nothing.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    DEFAULTS, accountRows, parseConfig, resolveMaildir, serializeConfig,
    storeSummary, syncStatus, validateMaildir,
} from '../lib/settings.js';

const account = (over = {}) => ({
    provider: 'Google', identity: 'you@example.test',
    imapUser: 'you@example.test', mailDisabled: false, ...over,
});
const working = {mbsync: true, secretService: true, accounts: [account()]};

test('a broken config gives up the preferences, not the app', () => {
    // Losing a preference to a stray comma is annoying. Losing the mail
    // client to one is not a trade anybody would accept.
    for (const bad of ['', '{', 'null', '[]', '"a string"', 'undefined'])
        assertEq(parseConfig(bad).maildir, DEFAULTS.maildir, JSON.stringify(bad));
    assertEq(parseConfig('{"maildir": "/home/x/Mail"}').maildir, '/home/x/Mail');
    // A key of the wrong type is not a path.
    assertEq(parseConfig('{"maildir": 42}').maildir, null);
    assertEq(parseConfig('{"maildir": "   "}').maildir, null);
    // Unknown keys are dropped rather than carried forward.
    assertEq(JSON.parse(serializeConfig(parseConfig('{"maildir":"/m","evil":1}'))).evil, undefined);
});

test('the environment beats the stored preference, and the page can say so', () => {
    // An env var a saved setting can silently override is a debugging
    // trap: you set it, nothing changes, and the reason is invisible.
    assertEq(resolveMaildir({env: '/tmp/test', config: {maildir: '/home/x/Mail'}, home: '/home/x'}),
        {path: '/tmp/test', source: 'env'});
    assertEq(resolveMaildir({config: {maildir: '/home/x/Other'}, home: '/home/x'}),
        {path: '/home/x/Other', source: 'config'});
    assertEq(resolveMaildir({home: '/home/x'}), {path: '/home/x/Mail', source: 'default'});
    // An empty env var is not a setting.
    assertEq(resolveMaildir({env: '  ', home: '/home/x'}).source, 'default');
});

test('a path that only looks right is refused before it is stored', () => {
    // `~` is shell syntax and nothing in GIO expands it, so storing it
    // makes a literal ./~ directory nobody asked for.
    assert(!validateMaildir('~/Mail').ok);
    assert(validateMaildir('~/Mail').reason.includes('~'));
    assert(!validateMaildir('Mail').ok, 'a relative path resolves against who knows what');
    assert(!validateMaildir('').ok);
    assert(!validateMaildir('/m\0/x').ok);
    assertEq(validateMaildir('  /home/x/Mail//  ').path, '/home/x/Mail');
    // An empty directory is valid: you are about to sync into it.
    assert(validateMaildir('/home/x/Mail').ok);
});

test('the blocking answer wins, in the order the layers block each other', () => {
    // Telling somebody their account is fine while the machine has no
    // syncer sends them to debug the wrong layer.
    assertEq(syncStatus({...working, mbsync: false}).title, 'No syncer installed');
    // …and a missing syncer outranks a missing keyring, which outranks
    // a missing account, whatever else is also wrong.
    assertEq(syncStatus({mbsync: false, secretService: false, accounts: []}).title,
        'No syncer installed');
    assertEq(syncStatus({...working, secretService: false}).title,
        'Online Accounts cannot store credentials');
    assertEq(syncStatus({...working, accounts: []}).title, 'No account connected');
});

test('an account with mail switched off is not a connected account', () => {
    // It looks identical to no account from inside this app, and it is
    // a different problem with a different fix.
    const off = syncStatus({...working, accounts: [account({mailDisabled: true})]});
    assertEq(off.title, 'No account connected');
    assert(off.detail.includes('switched off'), off.detail);
    // …which is a different sentence from having added nothing at all.
    assert(syncStatus({...working, accounts: []}).detail.includes('Online Accounts'));
});

test('everything present but unbridged says so, and names the issue', () => {
    const status = syncStatus(working);
    assertEq(status.kind, 'action');
    assertEq(status.title, 'Nothing is syncing yet');
    // Naming the issue is the difference between "broken" and "not
    // built yet", and the user can go read which.
    assert(status.detail.includes('#155'), status.detail);
    assert(status.detail.includes('you@example.test'), status.detail);
    // Only the bridged case is allowed to claim it is syncing.
    assertEq(syncStatus({...working, bridged: true}).kind, 'ok');
});

test('the account list shows what is there rather than filtering it', () => {
    const rows = accountRows([account(), account({provider: 'Fastmail', mailDisabled: true})]);
    assertEq(rows.length, 2);
    assertEq(rows[0].usable, true);
    assertEq(rows[1].usable, false);
    assert(rows[1].subtitle.includes('switched off'), rows[1].subtitle);
    assertEq(accountRows([]), []);
    assertEq(accountRows(), []);
});

test('the on-disk summary counts what is there and pluralises like a person', () => {
    assertEq(storeSummary(['INBOX', 'Sent'], {INBOX: 3, Sent: 1}), '2 folders, 4 messages');
    assertEq(storeSummary(['INBOX'], {INBOX: 1}), '1 folder, 1 message');
    assertEq(storeSummary(['INBOX'], {}), '1 folder, 0 messages');
    // No folders at all is a different statement from an empty inbox.
    assert(storeSummary([], {}).includes('not a Maildir'));
});

// --- remote images (2026-08-02) -------------------------------------

test('remote images default on, and an older config is not read as off', () => {
    assertEq(parseConfig('{}').showRemoteImages, true, 'empty config');
    assertEq(parseConfig('{"showRemoteImages": false}').showRemoteImages, false, 'explicit false');
    // A config written before this setting existed must not flip
    // behaviour on upgrade: absent is not a preference.
    assertEq(parseConfig('{"maildir":"/x"}').showRemoteImages, true, 'older config');
    assertEq(parseConfig('{"showRemoteImages": "no"}').showRemoteImages, true, 'non-boolean');
});

test('the setting survives a restart', () => {
    // Toggling it in the UI is worth nothing if it does not round-trip.
    for (const value of [true, false]) {
        const round = parseConfig(serializeConfig({maildir: null, showRemoteImages: value}));
        assertEq(round.showRemoteImages, value, `round-trip ${value}`);
    }
});

finish('mail/settings');
