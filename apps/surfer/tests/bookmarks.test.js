// Bookmarks (#146 follow-up). Mostly bookkeeping, with one rule that is
// not: a bookmark is a navigation somebody triggers later, so the set of
// schemes it may hold is an allowlist. A stored `javascript:` row is a
// self-XSS with a nice icon — which is why Firefox and Chrome both
// stopped honouring them.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    addBookmark, bookmarkLabel, bookmarkable, isBookmarked, removeBookmark,
    searchBookmarks, toggleBookmark,
} from '../lib/bookmarks.js';

test('only addresses that can be safely reopened may be stored', () => {
    assert(bookmarkable('https://example.org/'));
    assert(bookmarkable('http://localhost:3000/'));
    assert(bookmarkable('file:///home/me/notes.html'),
        'a person bookmarking their own document is their business');
    assert(!bookmarkable('javascript:alert(1)'));
    assert(!bookmarkable('JAVASCRIPT:alert(1)'));
    assert(!bookmarkable('data:text/html,x'));
    assert(!bookmarkable('blob:https://example.org/x'));
    assert(!bookmarkable('lisa://start'), 'the new-tab page is not a bookmark');
    assert(!bookmarkable(''));
    assert(!bookmarkable(null));
});

test('a refused scheme cannot get in through addBookmark either', () => {
    // The allowlist has to be applied where the write happens, not only
    // where the button is drawn.
    assertEq(addBookmark([], {url: 'javascript:alert(1)', title: 'free money'}), []);
    assertEq(addBookmark([], {url: 'data:text/html,x', title: 'x'}), []);
});

test('adding, listing and removing', () => {
    let list = [];
    list = addBookmark(list, {url: 'https://a.example/', title: 'A', at: 1});
    list = addBookmark(list, {url: 'https://b.example/', title: 'B', at: 2});
    assertEq(list.map(e => e.url), ['https://b.example/', 'https://a.example/'],
        'newest first');
    assert(isBookmarked(list, 'https://a.example/'));
    assert(!isBookmarked(list, 'https://c.example/'));
    list = removeBookmark(list, 'https://a.example/');
    assertEq(list.map(e => e.url), ['https://b.example/']);
    assert(!isBookmarked(list, 'https://a.example/'),
        'a removed bookmark is gone, not hidden');
});

test('bookmarking the same page twice does not make two, or reshuffle the list', () => {
    let list = addBookmark([], {url: 'https://a.example/', title: 'A', at: 1});
    list = addBookmark(list, {url: 'https://b.example/', title: 'B', at: 2});
    list = addBookmark(list, {url: 'https://a.example/', title: 'A, retitled', at: 3});
    assertEq(list.length, 2);
    assertEq(list.map(e => e.url), ['https://b.example/', 'https://a.example/'],
        'a re-add is an update, not a move');
    const a = list.find(e => e.url === 'https://a.example/');
    assertEq(a.title, 'A, retitled');
    assertEq(a.addedAt, 1, 'when you bookmarked it does not change');
});

test('removing every row for an address, not the first one found', () => {
    const list = [
        {url: 'https://a.example/', title: 'one'},
        {url: 'https://b.example/', title: 'two'},
        {url: 'https://a.example/', title: 'a duplicate from an older file'},
    ];
    const after = removeBookmark(list, 'https://a.example/');
    assertEq(after.map(e => e.title), ['two']);
    assert(!isBookmarked(after, 'https://a.example/'));
});

test('Ctrl+D on a bookmarked page unbookmarks it', () => {
    const entry = {url: 'https://a.example/', title: 'A', at: 1};
    const on = toggleBookmark([], entry);
    assert(isBookmarked(on, entry.url));
    const off = toggleBookmark(on, entry);
    assert(!isBookmarked(off, entry.url), 'the star is a toggle, like everywhere else');
    assertEq(off.length, 0);
});

test('search matches title and address', () => {
    const list = [
        {url: 'https://docs.example/gtk4', title: 'GTK4 reference'},
        {url: 'https://news.example/', title: 'Headlines'},
    ];
    assertEq(searchBookmarks(list, 'gtk').length, 1);
    assertEq(searchBookmarks(list, 'HEADLINES').length, 1);
    assertEq(searchBookmarks(list, '').length, 2);
    assertEq(searchBookmarks(list, 'zzz').length, 0);
});

test('a bookmark without a title still says something', () => {
    assertEq(bookmarkLabel({title: 'T', url: 'https://a/'}), 'T');
    assertEq(bookmarkLabel({url: 'https://a/'}), 'https://a/');
});

finish('surfer/bookmarks');
