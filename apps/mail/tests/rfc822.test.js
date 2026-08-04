// Parsing untrusted mail: the tests are mostly about not being fooled.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    decodeWords, htmlToText, messageText, parseAddress, parseHeaders, readableBody,
    renderableBody, splitMessage,
} from '../lib/rfc822.js';

/// A message as it is on disk: BYTES, one character per byte.
///
/// Every other fixture in this app is a JS string literal that is
/// already correctly decoded, which makes the byte→character step —
/// the one that was wrong (#232) — invisible by construction. This
/// helper is the only way to write a fixture that is not already the
/// answer, so the non-ASCII cases below build their bytes by hand.
function bytesOf(...parts) {
    const out = [];
    for (const part of parts) {
        if (typeof part === 'string') {
            for (let i = 0; i < part.length; i++) {
                const c = part.charCodeAt(i);
                if (c > 0x7f)
                    throw new Error(`${JSON.stringify(part)} is not ASCII — write the bytes`);
                out.push(c);
            }
        } else {
            out.push(...part);
        }
    }
    return new Uint8Array(out);
}

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

// ---------------------------------------------------------------------
// #221 — an attachment's raw bytes must never become the body.
//
// `bodyOfPart`'s last resort was `decoded.find((d) => d.text?.trim())`
// with NO type filter. Ordinary "here is the file" mail — where the
// text parts EXIST and are empty — therefore resolved to the PDF, the
// JPEG or the .docx. On the reference device that put a 3,145,615-
// character body starting `PK\x03\x04` into `read_message` and
// `%PDF-1.4 %Ǭ… /FlateDecode` into the list's preview column: 117 of
// 25,207 messages, 90 MB of decoded binary.
// ---------------------------------------------------------------------

/// `%PDF-1.4\n` and a little more, base64 — a body that announces
/// itself, so a test can say exactly what leaked.
const PDF_B64 = 'JVBERi0xLjQKJcTl8uXrp/Og0MTGCjEgMCBvYmoK';

const ATTACHMENT_ONLY = [
    'From: sender@example.test',
    'Subject: Fatura',
    'Content-Type: multipart/mixed; boundary="b"',
    '',
    '--b',
    'Content-Type: text/plain; charset=utf-8',
    'Content-Transfer-Encoding: 7bit',
    '',
    '   ',
    '',
    '--b',
    'Content-Type: application/pdf; name="fatura.pdf"',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment; filename="fatura.pdf"',
    '',
    PDF_B64,
    '--b--',
    '',
].join('\r\n');

test('a message whose text parts are empty has an empty body, not a PDF (#221)', () => {
    const body = readableBody(ATTACHMENT_ONLY);
    assert(!body.includes('%PDF'), `the document became the body: ${JSON.stringify(body.slice(0, 40))}`);
    assertEq(body.trim(), '', 'an honest empty body, and the attachment listed beside it');
    // The same fallback, spelled a second time, for the window's half.
    const {text} = renderableBody(ATTACHMENT_ONLY);
    assert(!text.includes('%PDF'), text.slice(0, 40));
    assertEq(text.trim(), '');
});

test('a message that is nothing but an attachment has no body either (#221)', () => {
    // Single part, no multipart wrapper: the leaf branch had no type
    // check at all, so the whole decoded file came back as prose.
    const raw = [
        'From: sender@example.test',
        'Content-Type: application/pdf; name="fatura.pdf"',
        'Content-Transfer-Encoding: base64',
        '',
        PDF_B64,
    ].join('\r\n');
    const body = readableBody(raw);
    assert(!body.includes('%PDF'), JSON.stringify(body.slice(0, 40)));
    assertEq(body.trim(), '');
});

test('an image with a filename does not become the body of the mail it came in', () => {
    // The JPEG shape, 8bit rather than base64: the bytes are the part.
    const raw = messageText(bytesOf(
        'From: a@b.test\r\n',
        'Content-Type: multipart/mixed; boundary="b"\r\n\r\n',
        '--b\r\nContent-Type: text/plain\r\n\r\n\r\n',
        '--b\r\nContent-Type: image/jpeg; name="photo.jpg"\r\n',
        'Content-Transfer-Encoding: binary\r\n\r\n',
        [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46],
        '\r\n--b--\r\n'));
    assertEq(readableBody(raw).trim(), '');
});

test('a nested multipart still supplies the body — the fallback that must survive', () => {
    // multipart/mixed[ multipart/alternative[plain, html], pdf ] is the
    // commonest invoice shape there is. The plain part is two levels
    // down, so the top level finds neither text/plain nor text/html and
    // must fall through to the sub-multipart's own choice. A naive
    // "text/* only" filter would return nothing here, which is the
    // regression the #221 fix could easily have introduced.
    //
    // The Albanian text is written as the BYTES a mailer sends, not as
    // a JS string literal somebody already decoded: `ë` is 0xC3 0xAB on
    // the wire, and a fixture that skips that is a fixture that cannot
    // fail the way real mail does.
    const raw = messageText(bytesOf(
        'Content-Type: multipart/mixed; boundary="MIX"\r\n\r\n',
        '--MIX\r\nContent-Type: multipart/alternative; boundary="ALT"\r\n\r\n',
        '--ALT\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n',
        'Dokumenti ', [0xc3, 0xab], 'sht', [0xc3, 0xab], ' bashk', [0xc3, 0xab],
        'ngjitur si PDF.\r\n',
        '--ALT\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Dokumenti</p>\r\n',
        '--ALT--\r\n',
        '--MIX\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n',
        `${PDF_B64}\r\n`,
        '--MIX--\r\n'));
    assert(readableBody(raw).includes('bashkëngjitur'), readableBody(raw));
    assert(!readableBody(raw).includes('%PDF'), readableBody(raw));
});

// ---------------------------------------------------------------------
// #232 — non-UTF-8 mail was destroyed before parsing.
//
// The app read every message file through a lossy `TextDecoder('utf-8')`,
// so `decodeBytes`' Latin-1 path was unreachable and a `charset=
// ISO-8859-1; 8bit` message arrived as `P<FFFD>rsh<FFFD>ndetje`. 198 of
// 25,207 messages on the reference device carry replacement characters.
// ---------------------------------------------------------------------

test('a file’s bytes become one character per byte, whatever is in them', () => {
    const raw = messageText(new Uint8Array([0x41, 0x00, 0xeb, 0xff, 0x80]));
    assertEq(raw.length, 5);
    assertEq(raw.charCodeAt(2), 0xeb, 'a Latin-1 e-diaeresis is not a replacement character');
    assertEq(raw.charCodeAt(4), 0x80);
    assert(!raw.includes('�'), 'nothing was lost on the way in');
    // Total, like everything else here: no bytes is an empty message.
    assertEq(messageText(new Uint8Array([])), '');
    assertEq(messageText(null), '');
});

test('an ISO-8859-1 8bit body is charset-decoded, not mangled (#232)', () => {
    // Byte 0xEB is `ë` in Latin-1 and an illegal lead byte in UTF-8.
    const raw = messageText(bytesOf(
        'From: dega@example.al\r\n',
        'Content-Type: text/plain; charset=ISO-8859-1\r\n',
        'Content-Transfer-Encoding: 8bit\r\n\r\n',
        'P', [0xeb], 'rsh', [0xeb], 'ndetje\r\n'));
    const body = readableBody(raw);
    assertEq(body.trim(), 'Përshëndetje');
    assert(!body.includes('�'), JSON.stringify(body));
});

test('a UTF-8 8bit body is decoded from its bytes too', () => {
    // The same message the other way round: 0xC3 0xAB is `ë` in UTF-8,
    // and reading the file as bytes must not leave it as two characters.
    const raw = messageText(bytesOf(
        'Content-Type: text/plain; charset=utf-8\r\n',
        'Content-Transfer-Encoding: 8bit\r\n\r\n',
        'P', [0xc3, 0xab], 'rsh', [0xc3, 0xab], 'ndetje\r\n'));
    assertEq(readableBody(raw).trim(), 'Përshëndetje');
});

test('an ISO-8859-1 html part is decoded before it is flattened', () => {
    const raw = messageText(bytesOf(
        'Content-Type: text/html; charset=iso-8859-1\r\n',
        'Content-Transfer-Encoding: 8bit\r\n\r\n',
        '<p>P', [0xeb], 'rshendetje</p>\r\n'));
    assertEq(readableBody(raw).trim(), 'Përshendetje');
    assert(renderableBody(raw).html.includes('Përshendetje'), renderableBody(raw).html);
});

test('a body that is already characters is not decoded a second time', () => {
    // `readableBody` is called on strings from other places too. A
    // string carrying a character above U+00FF cannot have come out of
    // a byte, so re-decoding it would be the corruption this exists to
    // prevent.
    const raw = ['Content-Type: text/plain; charset=utf-8', '', 'Përshëndetje 中文'].join('\r\n');
    assertEq(readableBody(raw).trim(), 'Përshëndetje 中文');
});

finish('mail/rfc822');
