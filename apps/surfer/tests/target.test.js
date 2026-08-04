// Which tab a write-tier action acts on (#213).
//
// The defect: every action resolved `currentView()` at execution time,
// so the tab could change between the confirmation and the doing. The
// agent's own click can open a popup that becomes the selected tab, and
// the next `fill` — approved as "#q" on the page the human was reading —
// lands in the popup. A page can do the same to itself with a timed
// location.href and no gesture at all.
//
// So a write says which page it means, the human sees that URL in the
// consent dialog, and this module refuses when the tab it describes is
// not there any more.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {pinTarget} from '../lib/target.js';

const refusal = (tabs, args) => {
    try {
        pinTarget(tabs, args);
        return null;
    } catch (e) {
        return e.message;
    }
};

test('an action that does not say where it acts is refused', () => {
    const tabs = [{url: 'https://example.org/', selected: true}];
    for (const args of [{}, {url: ''}, {url: null}, {url: '   '}]) {
        assert(refusal(tabs, args) !== null,
            `no url must refuse: ${JSON.stringify(args)}`);
    }
});

test('a blank-URL tab is not a match for a blank url', () => {
    // A tab that has not loaded anything yet reports '' for its URI. If
    // the empty argument were treated as an address, an action naming no
    // page at all would MATCH that tab — refusing for the wrong reason
    // is a refusal that stops the day someone opens a new tab.
    const tabs = [
        {url: '', selected: true},
        {url: 'https://example.org/', selected: false},
    ];
    for (const args of [{}, {url: ''}, {url: '  '}])
        assert(refusal(tabs, args) !== null, JSON.stringify(args));
});

test('the named tab is pinned even when another tab is selected', () => {
    // steal2.html: the agent's click opened a popup, the popup became
    // the selected tab, and the approved fill would have gone there.
    const tabs = [
        {url: 'https://example.org/form', selected: false},
        {url: 'about:blank', selected: true},   // the popup
    ];
    assertEq(pinTarget(tabs, {url: 'https://example.org/form'}), 0);
});

test('a tab that is gone is refused, not silently substituted', () => {
    const tabs = [{url: 'about:blank', selected: true}];
    const why = refusal(tabs, {url: 'https://example.org/form'});
    assert(why !== null, 'a closed tab must refuse');
    assert(why.includes('https://example.org/form'),
        'the refusal must name the page that is missing');
});

test('a page that navigated itself away is refused', () => {
    // slow.html: location.href on a 5s timer, no gesture needed. The
    // fill was approved for Page One; by the time it runs the tab is at
    // the attacker's page.
    const tabs = [{url: 'file:///tmp/surfrev/attacker.html', selected: true}];
    assert(refusal(tabs, {url: 'file:///tmp/surfrev/slow.html'}) !== null);
});

test('the selected tab wins when two tabs are at the same URL', () => {
    const tabs = [
        {url: 'https://example.org/a', selected: false},
        {url: 'https://example.org/a', selected: true},
    ];
    assertEq(pinTarget(tabs, {url: 'https://example.org/a'}), 1);
});

test('two unselected tabs at the same URL are ambiguous, so refused', () => {
    const tabs = [
        {url: 'https://example.org/a', selected: false},
        {url: 'https://example.org/a', selected: false},
        {url: 'about:blank', selected: true},
    ];
    assert(refusal(tabs, {url: 'https://example.org/a'}) !== null,
        'guessing between two tabs is not consent');
});

test('a trailing slash is not a different page', () => {
    // The engine reports https://example.org/ for what the model may
    // echo back as https://example.org. Dropping one trailing slash
    // cannot merge two different origins or paths.
    assertEq(pinTarget([{url: 'https://example.org/', selected: true}],
        {url: 'https://example.org'}), 0);
    assertEq(pinTarget([{url: 'https://example.org/a', selected: true}],
        {url: 'https://example.org/a/'}), 0);
    assert(refusal([{url: 'https://example.org/a', selected: true}],
        {url: 'https://example.org/b'}) !== null,
        'different paths are still different pages');
});

test('no open tabs at all is a refusal', () => {
    assert(refusal([], {url: 'https://example.org/'}) !== null);
});

await finish('surfer/target');
