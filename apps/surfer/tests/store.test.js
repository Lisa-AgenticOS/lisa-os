// Per-profile storage paths (#146 follow-up).
//
// The property under test is one sentence: **a profile's file never
// resolves inside another profile's directory.** History, bookmarks,
// downloads and the session snapshot are all written through this, so a
// leak here is every one of them leaking at once — and the agent
// profile's browsing landing in the person's history is the case that
// matters most (lib/profiles.js, #181).
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    STORE_FILES, decodeSettings, decodeStore, encodeSettings, encodeStore,
    profileStorePath,
} from '../lib/store.js';
import {AGENT_PROFILE, DEFAULT_PROFILE} from '../lib/profiles.js';

const BASE = '/home/me/.local/share/lisa-surfer';

test('each profile writes into its own directory, and never into another', () => {
    const kinds = Object.keys(STORE_FILES);
    const seen = new Set();
    for (const profile of [DEFAULT_PROFILE, AGENT_PROFILE, 'work', 'Client A']) {
        for (const kind of kinds) {
            const path = profileStorePath(profile, BASE, kind);
            assert(typeof path === 'string' && path !== '',
                `${profile}/${kind} has no path`);
            assert(!seen.has(path),
                `${profile}/${kind} resolved to a path another profile already owns: ${path}`);
            seen.add(path);
        }
    }
});

test('the agent profile cannot reach the person\'s history file', () => {
    // The one that would be a real leak: the agent browses in its own
    // session, so its history must be its own file too.
    const mine = profileStorePath(DEFAULT_PROFILE, BASE, 'history');
    const theirs = profileStorePath(AGENT_PROFILE, BASE, 'history');
    assertEq(mine, `${BASE}/history.json`);
    assertEq(theirs, `${BASE}/profiles/agent/history.json`);
    assert(mine !== theirs);
});

test('a profile name that could escape has no path at all', () => {
    // dataDirFor already refuses these; asserting it HERE is the point,
    // because this is the function every store call goes through and a
    // future refactor is what the test is for.
    for (const bad of ['../../etc', 'a/b', '', null, '.hidden', 'x'.repeat(200)]) {
        assertEq(profileStorePath(bad, BASE, 'history'), null,
            `${JSON.stringify(bad)} produced a path`);
    }
});

test('only the files we know about get a path', () => {
    assertEq(profileStorePath(DEFAULT_PROFILE, BASE, 'passwords'), null,
        'an unknown kind is a bug, not a new file');
    assertEq(profileStorePath(DEFAULT_PROFILE, BASE, '../../.bashrc'), null);
    assertEq(profileStorePath(DEFAULT_PROFILE, BASE, 'toString'), null,
        'a prototype member is not one of our files');
    assertEq(profileStorePath(DEFAULT_PROFILE, '', 'history'), null);
});

test('a corrupt file opens the browser anyway', () => {
    // A truncated write from a power cut must cost a history, not a
    // launch.
    assertEq(decodeStore('', 'history'), []);
    assertEq(decodeStore('{"surfer_store":1,"history":[', 'history'), []);
    assertEq(decodeStore('null', 'history'), []);
    assertEq(decodeStore('[1,2,3]', 'history'), []);
    assertEq(decodeStore('{"surfer_store":1,"history":"nope"}', 'history'), []);
    assertEq(decodeStore('{"surfer_store":1,"history":[{"url":"x"}]}', 'history'),
        [{url: 'x'}]);
});

test('what is written comes back', () => {
    const items = [{url: 'https://example.org/', title: 'E'}];
    assertEq(decodeStore(encodeStore('bookmarks', items), 'bookmarks'), items);
    assertEq(decodeStore(encodeStore('bookmarks', null), 'bookmarks'), []);
    // The version marker is there so a future shape change is
    // distinguishable from a corrupt file.
    assert(encodeStore('bookmarks', items).includes('"surfer_store":1'));
});

test('settings tolerate the same damage, and default rather than inherit', () => {
    assertEq(decodeSettings(''), {});
    assertEq(decodeSettings('not json'), {});
    assertEq(decodeSettings('[1]'), {});
    assertEq(decodeSettings('{"restoreSession":false}'), {restoreSession: false});
    assertEq(decodeSettings(encodeSettings({restoreSession: false})),
        {surfer_store: 1, restoreSession: false});
});

finish('surfer/store');
