// Extraction shaping (ADR-0037 §2, #146 Phase 2).
import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {pageResult, MAX_TEXT_CHARS} from '../lib/extract.js';

test('a normal page passes through with url and links', () => {
    const r = pageResult({title: 'T', text: 'body', links: [{text: 'a', href: 'https://x/'}]}, 'https://site/');
    assertEq(r, {url: 'https://site/', title: 'T', text: 'body', truncated: false,
                 links: [{text: 'a', href: 'https://x/'}]});
});

test('overlong text is truncated AND says so', () => {
    const r = pageResult({title: '', text: 'x'.repeat(MAX_TEXT_CHARS + 5), links: []}, 'u');
    assertEq(r.text.length, MAX_TEXT_CHARS);
    assertEq(r.truncated, true, 'a truncation the model cannot see is a page it thinks it read');
});

test('garbage from the page does not crash the shaper', () => {
    const r = pageResult({title: 42, text: null, links: 'nope'}, null);
    assertEq(r, {url: null, title: '', text: '', truncated: false, links: []});
});

finish('browser/extract');
