// The settings page's job is to say why there is no mail. Getting that
// wrong sends a person to debug the wrong layer, which is worse than
// saying nothing.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    DEFAULTS, accountRows, bannerText, lastSynced, parseConfig, resolveMaildir,
    serializeConfig, storeSummary, syncStatus, validateMaildir,
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

// --- the failure a person can finally see (#265) ----------------------
//
// On the reference device sync had been failing every five minutes
// since boot and Mail said nothing. `syncStatus` is the ONE place that
// decides what "blocked" means — the settings page renders it, the main
// window's banner renders it, and #249 will render it per account.

const running = {...working, bridged: true};

test('a locked keyring is a blocked state, in the CLI\'s own words (#265)', () => {
    const status = syncStatus({...running, keyringLocked: true});
    assertEq(status.kind, 'blocked');
    assert(status.title.toLowerCase().includes('locked'), status.title);
    // The CLI's message is good because it says three things: what is
    // stuck, that it is expected after a reboot, and what to do. A
    // paraphrase that drops any of them sends the person nowhere.
    assert(status.detail.includes('reboot'), status.detail);
    assert(status.detail.includes('token'), status.detail);
    assert(/unlock/i.test(status.detail), status.detail);
    // …and it is not invented when the keyring is open.
    assertEq(syncStatus(running).kind, 'ok');
});

test('the keyring is only interesting once there is something to unlock for (#265)', () => {
    // Each check is a precondition for the next being worth reading.
    // Telling somebody to unlock a keyring they have no account in
    // sends them to debug the wrong layer, which is what this whole
    // function exists to prevent.
    assertEq(syncStatus({...running, keyringLocked: true, mbsync: false}).title,
        'No syncer installed');
    assertEq(syncStatus({...running, keyringLocked: true, accounts: []}).title,
        'No account connected');
    assertEq(syncStatus({...working, keyringLocked: true}).title, 'Nothing is syncing yet');
});

test('every status says whether there is anything to press (#265)', () => {
    // A banner with a dead button is worse than a banner with a
    // sentence: it teaches people that the buttons do nothing.
    assertEq(syncStatus({...running, mbsync: false}).action, null,
        'nothing this app can do installs mbsync');
    assertEq(syncStatus({...running, secretService: false}).action, null);
    assertEq(syncStatus({...running, accounts: []}).action.id, 'online-accounts');
    assertEq(syncStatus({...running, keyringLocked: true}).action.id, 'unlock-keyring');
    assertEq(syncStatus(running).action, null, 'a working sync has nothing to fix');
    // Every offered action carries a label, or the banner draws a
    // button with nothing in it.
    for (const status of [
        syncStatus({...running, accounts: []}),
        syncStatus({...running, keyringLocked: true}),
    ])
        assert(status.action.label && status.action.label.length > 2, status.title);
});

test('the banner carries the whole answer, without stuttering (#265)', () => {
    // A banner has one line where the settings page has two, and both
    // halves matter: the title alone does not say what to do, and some
    // details do not say what is wrong. So they are joined — except
    // where the detail already opens with its own title, which is a
    // sentence repeating itself.
    const locked = syncStatus({...running, keyringLocked: true});
    assertEq(bannerText(locked), locked.detail);
    const none = syncStatus({...running, accounts: []});
    assert(bannerText(none).startsWith('No account connected'), bannerText(none));
    assert(bannerText(none).endsWith(none.detail), bannerText(none));
    // Nothing renders as nothing, rather than as ' — '.
    assertEq(bannerText({title: 'Only a title', detail: ''}), 'Only a title');
    assertEq(bannerText({}), '');
    assertEq(bannerText(null), '');
});

test('stale mail says how stale, and never guesses (#265)', () => {
    // Mail that looks current and is six hours old is a different
    // experience from mail that says it is six hours old.
    const now = 1_800_000_000;
    assertEq(lastSynced(0, now), 'Never synced');
    assertEq(lastSynced(null, now), 'Never synced');
    assertEq(lastSynced('nonsense', now), 'Never synced');
    assertEq(lastSynced(now, now), 'Synced just now');
    assertEq(lastSynced(now - 30, now), 'Synced just now');
    assertEq(lastSynced(now - 60, now), 'Synced 1 minute ago');
    assertEq(lastSynced(now - 5 * 60, now), 'Synced 5 minutes ago');
    assertEq(lastSynced(now - 3600, now), 'Synced 1 hour ago');
    assertEq(lastSynced(now - 6 * 3600, now), 'Synced 6 hours ago');
    assertEq(lastSynced(now - 26 * 3600, now), 'Synced 1 day ago');
    assertEq(lastSynced(now - 5 * 86400, now), 'Synced 5 days ago');
    // A clock that went backwards is not a sync from the future.
    assertEq(lastSynced(now + 600, now), 'Synced just now');
});

finish('mail/settings');
