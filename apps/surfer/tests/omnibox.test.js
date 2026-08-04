import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {suggestionsFor} from '../lib/omnibox.js';

const TABS = [
    {title: 'DuckDuckGo — Privacy', uri: 'https://duckduckgo.com/'},
    {title: 'Lisa OS — developer portal', uri: 'https://lisa-dev.common.al/'},
];

test('empty input offers nothing', () => {
    assertEq(suggestionsFor('', TABS).length, 0);
});

test('a URL offers navigation first, search last', () => {
    const s = suggestionsFor('https://example.org', TABS);
    assertEq(s[0].kind, 'url');
    assertEq(s.at(-1).kind, 'search');
});

test('typing part of an open tab offers the switch', () => {
    const s = suggestionsFor('duck', TABS);
    const tab = s.find((x) => x.kind === 'tab');
    assert(tab, `expected a tab row: ${JSON.stringify(s)}`);
    assertEq(tab.index, 0);
});

test('a refused scheme offers no navigation row at all', () => {
    const s = suggestionsFor('javascript:alert(1)', TABS);
    assert(!s.some((x) => x.kind === 'url'), `nothing navigable: ${JSON.stringify(s)}`);
    assertEq(s.at(-1).kind, 'search', 'searching for hostile text is harmless');
});

test('free text offers matching tabs and search, no url row', () => {
    const s = suggestionsFor('developer portal', TABS);
    assert(!s.some((x) => x.kind === 'url'));
    assert(s.some((x) => x.kind === 'tab' && x.index === 1));
});

test('the cap holds with many matching tabs', () => {
    const many = Array.from({length: 20}, (_, i) => ({title: `dup ${i}`, uri: `https://d${i}.test/`}));
    const s = suggestionsFor('dup', many, 6);
    assertEq(s.length, 6);
    assertEq(s.at(-1).kind, 'search', 'search survives the cap');
});

finish('surfer/omnibox');
