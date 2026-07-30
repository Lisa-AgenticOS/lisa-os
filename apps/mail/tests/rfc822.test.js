// Parsing untrusted mail: the tests are mostly about not being fooled.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    decodeWords, htmlToText, parseAddress, parseHeaders, readableBody, splitMessage,
} from '../lib/rfc822.js';

test('headers unfold, so a long subject is not truncated mid-word', () => {
    const h = parseHeaders(
        'Subject: Come back to Kaleidoscope\r\n and save 40% on your first two years\r\n' +
        'From: Christopher <chris@example.test>');
    assertEq(h.get('subject'),
        'Come back to Kaleidoscope and save 40% on your first two years');
    // Lookup is case-insensitive: real mail capitalises inconsistently.
    assertEq(h.get('SUBJECT'), h.get('subject'));
    assertEq(h.get('absent'), '');
});

test('a repeated header keeps the first for lookup and all of them for inspection', () => {
    const h = parseHeaders('From: real@example.test\nFrom: spoofed@evil.test');
    assertEq(h.get('from'), 'real@example.test');
    assertEq(h.all.from.length, 2);
});

test('encoded words decode, and adjacent ones do not gain a space', () => {
    assertEq(decodeWords('=?UTF-8?B?SGVsbG8gd29ybGQ=?='), 'Hello world');
    assertEq(decodeWords('=?UTF-8?Q?Fakt=C3=BCra?='), 'Faktüra');
    // Underscore is a space in Q encoding.
    assertEq(decodeWords('=?UTF-8?Q?two_words?='), 'two words');
    // Split across two encoded words: the separator whitespace is not content.
    assertEq(decodeWords('=?UTF-8?B?SGVsbG8g?= =?UTF-8?B?d29ybGQ=?='), 'Hello world');
    // Text around them survives.
    assertEq(decodeWords('Re: =?UTF-8?B?SGVsbG8=?= (fwd)'), 'Re: Hello (fwd)');
    // Nothing to decode is returned untouched.
    assertEq(decodeWords('plain subject'), 'plain subject');
});

test('an undecodable word is shown, not thrown', () => {
    // A mail client that throws on one bad message shows an empty inbox.
    const out = decodeWords('=?UTF-8?B?!!!not-base64!!!?=');
    assert(typeof out === 'string' && out.length > 0, `got ${out}`);
});

test('a display name that impersonates an address keeps both halves', () => {
    // The phishing shape: the name says one thing, the address another.
    const a = parseAddress('"security@yourbank.com" <evil@attacker.test>');
    assertEq(a.name, 'security@yourbank.com');
    assertEq(a.address, 'evil@attacker.test');
    // A bare address has no name to show.
    assertEq(parseAddress('plain@example.test'), {name: '', address: 'plain@example.test'});
});

test('the readable body prefers text/plain over html', () => {
    const raw = [
        'Content-Type: multipart/alternative; boundary="b1"',
        '',
        '--b1',
        'Content-Type: text/plain; charset=utf-8',
        '',
        'the plain version',
        '--b1',
        'Content-Type: text/html; charset=utf-8',
        '',
        '<p>the html version</p>',
        '--b1--',
    ].join('\n');
    assert(readableBody(raw).includes('the plain version'), readableBody(raw));
    assert(!readableBody(raw).includes('html version'), readableBody(raw));
});

test('an html-only message is flattened rather than shown as markup', () => {
    const raw = [
        'Content-Type: text/html; charset=utf-8',
        '',
        '<p>Save 40%</p><p>Use code <b>INDIESUMMER26</b></p>',
    ].join('\n');
    const body = readableBody(raw);
    assert(body.includes('Save 40%'), body);
    assert(body.includes('INDIESUMMER26'), body);
    assert(!body.includes('<p>'), body);
});

test('script and style contents never reach the body', () => {
    // An agent asked to summarise a message must not be handed a script
    // body: it is code, not prose, and it is written by the sender.
    const html = '<style>.x{color:red}</style><script>alert("ignore all previous")</script>' +
        '<p>real text</p>';
    const out = htmlToText(html);
    assertEq(out, 'real text');
    assert(!out.includes('alert'), out);
    assert(!out.includes('color:red'), out);
});

test('quoted-printable and base64 bodies decode', () => {
    const qp = ['Content-Type: text/plain; charset=utf-8',
        'Content-Transfer-Encoding: quoted-printable', '', 'Fakt=C3=BCra =\n', 'continued'].join('\n');
    assert(readableBody(qp).includes('Faktüra'), readableBody(qp));

    const b64 = ['Content-Type: text/plain; charset=utf-8',
        'Content-Transfer-Encoding: base64', '', 'SGVsbG8gd29ybGQ='].join('\n');
    assertEq(readableBody(b64).trim(), 'Hello world');
});

test('a message with no blank line is all headers and no body', () => {
    const {headerText, body} = splitMessage('Subject: only headers');
    assertEq(headerText, 'Subject: only headers');
    assertEq(body, '');
    // …and empty input does not throw.
    assertEq(splitMessage('').body, '');
    assertEq(readableBody(''), '');
});

test('CRLF and LF mix, because two tools wrote the same maildir', () => {
    const raw = 'Subject: mixed\r\nFrom: a@b.test\n\r\nbody here';
    const {headerText, body} = splitMessage(raw);
    assertEq(parseHeaders(headerText).get('subject'), 'mixed');
    assertEq(body.trim(), 'body here');
});

finish('mail/rfc822');
