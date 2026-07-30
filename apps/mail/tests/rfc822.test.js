// Parsing untrusted mail: the tests are mostly about not being fooled.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    decodeWords, htmlToText, parseAddress, parseHeaders, readableBody, renderableBody,
    splitMessage,
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


test('a message keeps its HTML for the window and its text for the model', () => {
    // The same message has to serve two readers. A person needs the
    // newsletter to look like a newsletter; a model needs prose, not
    // markup carrying instructions in an alt attribute.
    const msg = [
        'From: a@b.test', 'MIME-Version: 1.0',
        'Content-Type: multipart/alternative; boundary="X"', '',
        '--X', 'Content-Type: text/plain', '', 'Plain version.', '',
        '--X', 'Content-Type: text/html', '', '<p>Rich <b>version</b>.</p>', '',
        '--X--', '',
    ].join('\r\n');
    const {html, text} = renderableBody(msg);
    assert(html.includes('<b>version</b>'), html);
    assertEq(text.trim(), 'Plain version.');
});

test('an html-only message still yields readable text', () => {
    const msg = ['From: a@b.test', 'Content-Type: text/html', '', '<p>Hi <b>there</b></p>'].join('\r\n');
    const {html, text} = renderableBody(msg);
    assert(html.includes('<b>there</b>'), html);
    // …flattened for the model, with no tags left in it.
    assert(!text.includes('<'), text);
    assert(text.includes('there'), text);
});

test('a plain-text message offers no html at all', () => {
    // null, not an empty string: the window decides which widget to use
    // on this, and "" would send it down the HTML path with nothing in it.
    const msg = ['From: a@b.test', 'Content-Type: text/plain', '', 'Just words.'].join('\r\n');
    const {html, text} = renderableBody(msg);
    assertEq(html, null);
    assertEq(text.trim(), 'Just words.');
});

test('a quoted-printable html part is decoded before it is rendered', () => {
    // Undecoded, this renders as literal =3D and =20 across the page.
    const msg = [
        'From: a@b.test', 'Content-Type: text/html; charset=utf-8',
        'Content-Transfer-Encoding: quoted-printable', '',
        '<a href=3D"https://x.test">link=20here</a>',
    ].join('\r\n');
    const {html} = renderableBody(msg);
    assert(html.includes('href="https://x.test"'), html);
    assert(html.includes('link here'), html);
});

finish('mail/rfc822');
