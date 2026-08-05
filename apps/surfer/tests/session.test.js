// Session restore (#146 follow-up).
//
// The switch is the part worth testing. Reopening tabs is a convenience
// for most people and a hazard for some — the tab you had open is
// visible to whoever starts the browser next — so "off" has to mean the
// snapshot is not read AND, at the window layer, that the file goes.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    SESSION_LIMIT, restorable, restoreEnabled, selectedIndex, sessionSnapshot,
    tabsToRestore,
} from '../lib/session.js';
import {AGENT_PROFILE, DEFAULT_PROFILE} from '../lib/profiles.js';
import {START_URI} from '../lib/startpage.js';

const tab = (url, extra = {}) => ({url, title: url, ...extra});

test('restore is on unless somebody turned it off', () => {
    assert(restoreEnabled(undefined), 'a browser with no settings file still restores');
    assert(restoreEnabled({}));
    assert(restoreEnabled({restoreSession: true}));
    assert(!restoreEnabled({restoreSession: false}));
    // Only an exact false counts: a half-written settings file must not
    // quietly disable a feature the person chose.
    assert(restoreEnabled({restoreSession: 'false'}));
    assert(restoreEnabled({restoreSession: 0}));
});

test('a restored tab is a load nobody pressed a key for, so the schemes are an allowlist', () => {
    assert(restorable('https://example.org/'));
    assert(restorable('http://localhost:3000/'));
    assert(restorable('file:///home/me/notes.html'));
    assert(!restorable('javascript:alert(1)'));
    assert(!restorable('data:text/html,x'));
    assert(!restorable(START_URI), 'a fresh tab lands there anyway');
    assert(!restorable(''));
});

test('the snapshot holds the tabs that are worth reopening', () => {
    const snap = sessionSnapshot([
        tab('https://a.example/'),
        tab(START_URI),
        tab('https://b.example/', {selected: true}),
        tab('javascript:alert(1)'),
    ], {at: 42, profile: DEFAULT_PROFILE});
    assertEq(snap.tabs.map(t => t.url), ['https://a.example/', 'https://b.example/']);
    assertEq(snap.selected, 1, 'the selected index follows the tab, not its old position');
    assertEq(snap.savedAt, 42);
});

test('nothing worth saving means delete the file, not keep yesterday\'s', () => {
    assertEq(sessionSnapshot([], {profile: DEFAULT_PROFILE}), null);
    assertEq(sessionSnapshot([tab(START_URI)], {profile: DEFAULT_PROFILE}), null);
    assertEq(sessionSnapshot(null, {profile: DEFAULT_PROFILE}), null);
});

test('the agent profile neither saves nor restores a session', () => {
    assertEq(sessionSnapshot([tab('https://a.example/')], {profile: AGENT_PROFILE}), null);
    assertEq(tabsToRestore({tabs: [{url: 'https://a.example/'}]}, {profile: AGENT_PROFILE}), []);
});

test('turning restore off restores nothing', () => {
    const snap = {tabs: [{url: 'https://a.example/', title: 'A'}], selected: 0};
    assertEq(tabsToRestore(snap, {settings: {restoreSession: false}, profile: DEFAULT_PROFILE}),
        []);
    assertEq(tabsToRestore(snap, {settings: {}, profile: DEFAULT_PROFILE}).map(t => t.url),
        ['https://a.example/']);
});

test('a corrupt or hostile snapshot opens nothing rather than something', () => {
    assertEq(tabsToRestore(null, {profile: DEFAULT_PROFILE}), []);
    assertEq(tabsToRestore({}, {profile: DEFAULT_PROFILE}), []);
    assertEq(tabsToRestore({tabs: 'nope'}, {profile: DEFAULT_PROFILE}), []);
    assertEq(tabsToRestore({tabs: [{url: 'javascript:alert(1)'}]}, {profile: DEFAULT_PROFILE}),
        [], 'a snapshot file is not a way to get a refused scheme loaded');
});

test('a session with three hundred tabs does not open three hundred tabs', () => {
    const many = Array.from({length: 300}, (_, i) => tab(`https://x${i}.example/`));
    assertEq(sessionSnapshot(many, {profile: DEFAULT_PROFILE}).tabs.length, SESSION_LIMIT);
    assertEq(tabsToRestore({tabs: many}, {profile: DEFAULT_PROFILE}).length, SESSION_LIMIT);
});

test('the selected index is clamped into what actually opened', () => {
    assertEq(selectedIndex({selected: 0}, 3), 0);
    assertEq(selectedIndex({selected: 2}, 3), 2);
    assertEq(selectedIndex({selected: 9}, 3), 2, 'an index past the end selects nothing');
    assertEq(selectedIndex({selected: -1}, 3), 0);
    assertEq(selectedIndex({}, 3), 0);
    assertEq(selectedIndex({selected: 0}, 0), -1);
});

finish('surfer/session');
