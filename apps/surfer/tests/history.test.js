// History (#146 follow-up).
//
// Two of these are not bookkeeping. `recordable` keeps the agent's
// browsing out of the person's history, and the three forget functions
// are the difference between a history and a surveillance log — a
// browser whose "delete" only hides a row is worse than one with no
// history at all, because the person believes it went.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    HISTORY_LIMIT, addVisit, clearHistory, forgetSince, forgetUrl,
    historyLabel, recordable, retitle, searchHistory,
} from '../lib/history.js';
import {AGENT_PROFILE, DEFAULT_PROFILE} from '../lib/profiles.js';

test('the agent\'s browsing never enters the person\'s history', () => {
    assert(!recordable('https://example.org/', AGENT_PROFILE),
        'the agent browses in its own session; its history is not the person\'s');
    assert(recordable('https://example.org/', DEFAULT_PROFILE));
    assert(recordable('https://example.org/', 'work'));
    // No profile at all fails closed rather than defaulting to the
    // person's — a default here would be the boundary living in
    // whichever caller remembered the argument.
    assert(!recordable('https://example.org/'));
    assert(!recordable('https://example.org/', ''));
    assert(!recordable('https://example.org/', null));
});

test('Surfer\'s own furniture is not a place you went', () => {
    assert(!recordable('lisa://start', DEFAULT_PROFILE));
    assert(!recordable('lisa-go:submit?q=cats', DEFAULT_PROFILE));
    assert(!recordable('about:blank', DEFAULT_PROFILE));
    assert(!recordable('', DEFAULT_PROFILE));
    assert(!recordable('   ', DEFAULT_PROFILE));
});

test('a history row is a thing that gets clicked later, so it holds no script', () => {
    // url.js refuses these at the address bar. They are refused again
    // here because a stored row is a navigation nobody typed.
    assert(!recordable('javascript:alert(1)', DEFAULT_PROFILE));
    assert(!recordable('JavaScript:alert(1)', DEFAULT_PROFILE));
    assert(!recordable('data:text/html,<script>x</script>', DEFAULT_PROFILE));
    assert(!recordable('blob:https://example.org/abc', DEFAULT_PROFILE));
    assert(!recordable('vbscript:msgbox', DEFAULT_PROFILE));
    // A person's own file is their business.
    assert(recordable('file:///home/me/notes.html', DEFAULT_PROFILE));
});

test('one row per address, newest first, with a count', () => {
    let list = [];
    list = addVisit(list, {url: 'https://a.example/', title: 'A', at: 1});
    list = addVisit(list, {url: 'https://b.example/', title: 'B', at: 2});
    assertEq(list.map(e => e.url), ['https://b.example/', 'https://a.example/']);
    list = addVisit(list, {url: 'https://a.example/', title: 'A', at: 3});
    assertEq(list.map(e => e.url), ['https://a.example/', 'https://b.example/'],
        'revisiting moves the row to the front rather than adding a second');
    assertEq(list[0].visits, 2);
    assertEq(list[0].firstVisit, 1, 'the first visit is still the first visit');
    assertEq(list[0].lastVisit, 3);
});

test('a page that has not titled itself yet does not erase a good title', () => {
    let list = addVisit([], {url: 'https://a.example/', title: 'Real Title', at: 1});
    list = addVisit(list, {url: 'https://a.example/', title: '', at: 2});
    assertEq(list[0].title, 'Real Title');
});

test('a late title correction is not a second visit', () => {
    // A page sets document.title after the load finishes. Routing that
    // back through addVisit would count a visit per title change, and a
    // page whose title is a clock would climb on its own.
    let list = addVisit([], {url: 'https://a.example/', title: '', at: 1});
    assertEq(list[0].visits, 1);
    list = retitle(list, 'https://a.example/', 'The Real Title');
    assertEq(list[0].title, 'The Real Title');
    assertEq(list[0].visits, 1, 'retitling counted a visit');
    // Nothing to say, nothing changes — and an unknown URL adds no row.
    assertEq(retitle(list, 'https://a.example/', '')[0].title, 'The Real Title');
    assertEq(retitle(list, 'https://b.example/', 'X').length, 1);
});

test('the list is bounded', () => {
    let list = [];
    for (let i = 0; i < 12; i++)
        list = addVisit(list, {url: `https://x${i}.example/`, at: i}, {limit: 10});
    assertEq(list.length, 10);
    assertEq(list[0].url, 'https://x11.example/');
    assertEq(HISTORY_LIMIT, 5000);
});

test('search finds a page by title or by address', () => {
    const list = [
        {url: 'https://news.example/politics', title: 'Election night', visits: 1, lastVisit: 3},
        {url: 'https://docs.example/gtk', title: 'GTK4 reference', visits: 1, lastVisit: 2},
    ];
    assertEq(searchHistory(list, 'gtk').map(e => e.url), ['https://docs.example/gtk']);
    assertEq(searchHistory(list, 'ELECTION').map(e => e.url), ['https://news.example/politics'],
        'case does not matter to somebody looking for a page they saw');
    assertEq(searchHistory(list, 'zzz'), []);
    assertEq(searchHistory(list, '').length, 2, 'an empty box shows everything');
});

test('forgetting one address forgets it, not just the first row for it', () => {
    const list = [
        {url: 'https://a.example/', title: 'A', lastVisit: 3},
        {url: 'https://b.example/', title: 'B', lastVisit: 2},
        {url: 'https://a.example/', title: 'A again', lastVisit: 1},
    ];
    const after = forgetUrl(list, 'https://a.example/');
    assertEq(after.map(e => e.url), ['https://b.example/']);
    assert(!after.some(e => e.url === 'https://a.example/'),
        'a delete that leaves a copy behind is not a delete');
    assertEq(forgetUrl(list, '').length, 3, 'an empty address deletes nothing');
});

test('clearing the last hour clears the last hour', () => {
    const now = 10_000;
    const hour = 3_600;
    const list = [
        {url: 'https://recent.example/', lastVisit: now - 10},
        {url: 'https://edge.example/', lastVisit: now - hour},
        {url: 'https://old.example/', lastVisit: now - hour - 1},
        {url: 'https://undated.example/'},
    ];
    const after = forgetSince(list, now - hour);
    assertEq(after.map(e => e.url), ['https://old.example/', 'https://undated.example/']);
    // A row stamped exactly on the boundary is inside the window.
    assert(!after.some(e => e.url === 'https://edge.example/'));
    // …and a row with no timestamp is kept: deleting on a missing field
    // is the wrong way round for a delete.
    assert(after.some(e => e.url === 'https://undated.example/'));
    assertEq(forgetSince(list, null).length, 4);
});

test('clear all clears all', () => {
    assertEq(clearHistory(), []);
});

test('a row without a title still says something', () => {
    assertEq(historyLabel({title: 'T', url: 'https://a/'}), 'T');
    assertEq(historyLabel({title: '  ', url: 'https://a/'}), 'https://a/');
    assertEq(historyLabel({url: 'https://a/'}), 'https://a/');
});

finish('surfer/history');
