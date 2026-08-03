import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {GO_PREFIX, START_URI, START_PAGE_HTML, goQuery} from '../lib/startpage.js';

test('the form emits the intercepted scheme, nothing else', () => {
    assert(START_PAGE_HTML.includes(`action="${GO_PREFIX}submit"`));
    assert(!/https?:\/\//.test(START_PAGE_HTML), 'zero network in the start page');
});

test('goQuery decodes the submitted text', () => {
    assertEq(goQuery('lisa-go:submit?q=quarterly+invoice'), 'quarterly invoice');
    assertEq(goQuery('lisa-go:submit?q=https%3A%2F%2Fexample.org'), 'https://example.org');
    assertEq(goQuery('lisa-go:submit?q='), '');
});

test('non-go URIs are not ours', () => {
    assertEq(goQuery('https://example.org/?q=x'), null);
    assertEq(goQuery(START_URI), null);
    assertEq(goQuery(''), null);
});

finish();
