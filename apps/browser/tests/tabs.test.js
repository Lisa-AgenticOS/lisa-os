// Tab state (ADR-0037, issue #146).
import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    newTabs, open, close, activate, move, update, activeTab, __resetIds,
} from '../lib/tabs.js';

const ids = (s) => s.tabs.map(t => t.id);

test('opening focuses the new tab and appends it', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'https://a'});
    assertEq(ids(s), [1]);
    assertEq(s.active, 1);
    s = open(s, {url: 'https://b'});
    assertEq(ids(s), [1, 2]);
    assertEq(s.active, 2);
});

test('a link opened from a tab lands beside it, not at the end', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'https://a'});
    s = open(s, {url: 'https://b'});
    s = open(s, {url: 'https://c'});
    // Open from tab 1: belongs at index 1, before b.
    s = open(s, {url: 'https://from-a', after: 1});
    assertEq(ids(s), [1, 4, 2, 3]);
});

test('open with focus:false leaves the active tab alone', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'https://a'});
    s = open(s, {url: 'https://background', focus: false});
    assertEq(s.active, 1);
    assertEq(ids(s), [1, 2]);
});

test('closing the active tab focuses its right neighbour', () => {
    __resetIds();
    let s = newTabs();
    for (const u of ['a', 'b', 'c', 'd'])
        s = open(s, {url: u});
    s = activate(s, 2);
    s = close(s, 2);
    // b closed → c takes focus, not a.
    assertEq(ids(s), [1, 3, 4]);
    assertEq(s.active, 3);
});

test('closing the LAST tab falls back to the left', () => {
    __resetIds();
    let s = newTabs();
    for (const u of ['a', 'b', 'c'])
        s = open(s, {url: u});
    s = activate(s, 3);
    s = close(s, 3);
    assertEq(ids(s), [1, 2]);
    assertEq(s.active, 2);
});

test('closing an inactive tab does not move focus', () => {
    __resetIds();
    let s = newTabs();
    for (const u of ['a', 'b', 'c'])
        s = open(s, {url: u});
    s = activate(s, 1);
    s = close(s, 3);
    assertEq(s.active, 1);
});

test('closing the only tab leaves no active tab', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'a'});
    s = close(s, 1);
    assertEq(s.tabs.length, 0);
    assertEq(s.active, null);
    assertEq(activeTab(s), null);
});

test('closing an unknown id changes nothing', () => {
    __resetIds();
    const s = open(newTabs(), {url: 'a'});
    assertEq(close(s, 999), s);
});

test('ids are never reused, so a stale reference cannot hit a new tab', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'a'});
    s = close(s, 1);
    s = open(s, {url: 'b'});
    assertEq(ids(s), [2]);
});

test('moving reorders and keeps the active tab active', () => {
    __resetIds();
    let s = newTabs();
    for (const u of ['a', 'b', 'c'])
        s = open(s, {url: u});
    s = activate(s, 1);
    s = move(s, 1, 2);
    assertEq(ids(s), [2, 3, 1]);
    assertEq(s.active, 1, 'the moved tab is still the active one');
});

test('a drag past either end is clamped, not refused', () => {
    __resetIds();
    let s = newTabs();
    for (const u of ['a', 'b', 'c'])
        s = open(s, {url: u});
    assertEq(ids(move(s, 1, 99)), [2, 3, 1]);
    assertEq(ids(move(s, 3, -5)), [3, 1, 2]);
});

test('update touches one tab and nothing else', () => {
    __resetIds();
    let s = newTabs();
    s = open(s, {url: 'a'});
    s = open(s, {url: 'b'});
    s = update(s, 1, {title: 'Alpha', loading: true});
    assertEq(s.tabs[0].title, 'Alpha');
    assertEq(s.tabs[0].loading, true);
    assertEq(s.tabs[1].title, '');
});

test('activeTab returns the focused tab', () => {
    __resetIds();
    let s = open(newTabs(), {url: 'https://a'});
    s = open(s, {url: 'https://b'});
    assertEq(activeTab(s).url, 'https://b');
    s = activate(s, 1);
    assertEq(activeTab(s).url, 'https://a');
});

finish('browser/tabs');
