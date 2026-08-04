// Attachments: the parts the reader used to throw away (#211, #169).
//
// The fixture is a REAL message shape, not a tidy synthetic one. This
// repo has been bitten twice by fixtures whose names contained nothing
// worth sanitising — `uniqueMatchesId` (#210) passed for months because
// every test filename was alphanumeric, so the sanitiser under test was
// a no-op. So the message below carries what the reference device
// actually receives: a boundary with `,U=` and `=` in it, an RFC 2231
// filename with a percent-encoded `ë` and parentheses, an encoded-word
// filename with a quoted comma, a cid: inline logo, and a part whose
// sender-chosen filename is a path traversal.
//
// Hostile characters are written as escapes (`\u0000`, `\u202E`) rather
// than as themselves: a literal NUL in this file makes every tool that
// reads it — grep, diff, the editor — treat the suite as binary.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    attachmentBytes, attachments, listedAttachments, safeFilename,
} from '../lib/attachments.js';
import {
    MAX_PART_BYTES, decodeTransfer, decodedSize, headerParam, leafParts, messageText,
} from '../lib/rfc822.js';

/// Bytes as one char per byte, so a test can assert on content without
/// depending on TextDecoder (the macOS jsc runner has no Web APIs).
function latin1(bytes) {
    let out = '';
    for (let i = 0; i < bytes.length; i++)
        out += String.fromCharCode(bytes[i]);
    return out;
}

/// A 396-byte PDF that really is a PDF: header, catalog, one page, and
/// %%EOF. Base64 wrapped at 76 columns, the way every mailer wraps it.
const PDF_B64 = [
    'JVBERi0xLjQKMSAwIG9iajw8L1R5cGUvQ2F0YWxvZy9QYWdlcyAyIDAgUj4+ZW5kb2JqCjIgMCBv',
    'Ymo8PC9UeXBlL1BhZ2VzL0tpZHNbMyAwIFJdL0NvdW50IDE+PmVuZG9iagozIDAgb2JqPDwvVHlw',
    'ZS9QYWdlL1BhcmVudCAyIDAgUi9NZWRpYUJveFswIDAgNTk1IDg0Ml0vQ29udGVudHMgNCAwIFIv',
    'UmVzb3VyY2VzPDwvRm9udDw8L0YxIDUgMCBSPj4+Pj4+ZW5kb2JqCjQgMCBvYmo8PC9MZW5ndGgg',
    'NjQ+PnN0cmVhbQpCVCAvRjEgMTggVGYgNzIgNzYwIFRkIChGYXR1cmEgRi0yMDI2LTAwMDcpIFRq',
    'IEVUCmVuZHN0cmVhbSBlbmRvYmoKNSAwIG9iajw8L1R5cGUvRm9udC9TdWJ0eXBlL1R5cGUxL0Jh',
    'c2VGb250L0hlbHZldGljYT4+ZW5kb2JqCnRyYWlsZXI8PC9Sb290IDEgMCBSPj4KJSVFT0YK',
];

/// A 69-byte PNG, the inline logo a business mailer puts in its footer.
const PNG_B64 =
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGPIdT0JAAKeAXyVHzc8AAAAAElFTkSuQmCC';

const INVOICE = [
    'Return-Path: <faturimi@negenet.al>',
    'From: "Negenet Sh.p.k." <faturimi@negenet.al>',
    'To: flakerimi@basecode.al',
    'Subject: =?UTF-8?Q?Fatura_F-2026-0007?=',
    'Date: Mon, 03 Aug 2026 09:14:22 +0200',
    'MIME-Version: 1.0',
    'Content-Type: multipart/mixed; boundary="=_MIXED_8f2c1b9a,U=8407"',
    '',
    'This is a multi-part message in MIME format.',
    '--=_MIXED_8f2c1b9a,U=8407',
    'Content-Type: multipart/alternative; boundary="=_ALT_2b7d"',
    '',
    '--=_ALT_2b7d',
    'Content-Type: text/plain; charset=utf-8',
    'Content-Transfer-Encoding: quoted-printable',
    '',
    'Dokumenti =C3=ABsht=C3=AB bashk=C3=ABngjitur si PDF.',
    '',
    '--=_ALT_2b7d',
    'Content-Type: text/html; charset=utf-8',
    'Content-Transfer-Encoding: quoted-printable',
    '',
    '<p>Dokumenti =C3=ABsht=C3=AB bashk=C3=ABngjitur si PDF.</p>',
    '',
    '--=_ALT_2b7d--',
    '',
    '--=_MIXED_8f2c1b9a,U=8407',
    'Content-Type: application/pdf; name="=?UTF-8?Q?Fatura_F-2026-0007.pdf?="',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment;',
    "\tfilename*=UTF-8''Fatura%20F-2026-0007%20%28mars%29.pdf",
    '',
    ...PDF_B64,
    '',
    '--=_MIXED_8f2c1b9a,U=8407',
    'Content-Type: text/plain; charset=utf-8; name="shenime.txt"',
    'Content-Transfer-Encoding: quoted-printable',
    'Content-Disposition: attachment; filename="=?UTF-8?Q?sh=C3=ABnime=2C_mars.txt?="',
    '',
    'Fatura, U=3D8407 =E2=80=94 totali 1=2C200 LEK.',
    'Rreshti i dyt=C3=AB me nj=C3=AB rresht t=C3=AB gjat=C3=AB q=C3=AB thyhet =',
    'k=C3=ABtu.',
    '',
    '--=_MIXED_8f2c1b9a,U=8407',
    'Content-Type: image/png; name="logo.png"',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: inline; filename="logo.png"',
    'Content-ID: <logo-2026@negenet.al>',
    '',
    PNG_B64,
    '',
    '--=_MIXED_8f2c1b9a,U=8407',
    'Content-Type: application/octet-stream',
    'Content-Transfer-Encoding: base64',
    'Content-Disposition: attachment; filename="../../.ssh/authorized_keys"',
    '',
    'c3NoLXJzYSBBQUFBQjNOemFDMXljMkVBQUFBQg==',
    '',
    '--=_MIXED_8f2c1b9a,U=8407--',
    '',
].join('\r\n');

test('the parts a reader used to throw away are listed', () => {
    const list = attachments(INVOICE);
    // The PDF, the note, the inline logo, the hostile one. NOT the
    // text/plain and text/html halves of the body.
    assertEq(list.length, 4, JSON.stringify(list.map((a) => a.filename)));
    assert(!list.some((a) => a.mimeType === 'text/html'), 'the html body is not an attachment');

    const pdf = list[0];
    assertEq(pdf.mimeType, 'application/pdf');
    // RFC 2231 beats the Content-Type `name` parameter: it is the one
    // with the charset, and the one with the parentheses in it.
    assertEq(pdf.filename, 'Fatura F-2026-0007 (mars).pdf');
    assertEq(pdf.disposition, 'attachment');
    assertEq(pdf.size, 396);
});

test('an encoded-word filename decodes, quoted comma and all', () => {
    // Illegal per RFC 2047 in a parameter, and sent by half the world.
    const note = attachments(INVOICE)[1];
    assertEq(note.filename, 'shënime, mars.txt');
    assertEq(note.mimeType, 'text/plain');
    // A text/plain part with a filename is an attachment, not the body.
    assertEq(note.disposition, 'attachment');
});

test('an RFC 2231 continuation is reassembled before it is decoded', () => {
    const value = "attachment; filename*0*=UTF-8''Deklarata%20e%20t; " +
        'filename*1*=%C3%ABrheqjes; filename*2*=.pdf';
    assertEq(headerParam(value, 'filename'), 'Deklarata e tërheqjes.pdf');
    // The plain spelling still works, and an absent parameter is ''.
    assertEq(headerParam('attachment; filename="plain.pdf"', 'filename'), 'plain.pdf');
    assertEq(headerParam('attachment', 'filename'), '');
    assertEq(headerParam('', 'filename'), '');
});

test('the PDF comes back byte for byte', () => {
    const pdf = attachments(INVOICE)[0];
    const bytes = attachmentBytes(INVOICE, pdf.path);
    assertEq(bytes.length, 396);
    assertEq(bytes.length, pdf.size, 'the listed size is the size you get');
    const text = latin1(bytes);
    assert(text.startsWith('%PDF-1.4\n'), text.slice(0, 16));
    assert(text.endsWith('%%EOF\n'), text.slice(-16));
    assert(text.includes('Fatura F-2026-0007'), 'the page text survived');
});

test('a quoted-printable part decodes, soft line breaks and all', () => {
    const note = attachments(INVOICE)[1];
    const text = latin1(attachmentBytes(INVOICE, note.path));
    // `=3D` is an equals sign, not a soft break, and `=2C` is a comma.
    assert(text.includes('U=8407'), text);
    assert(text.includes('1,200 LEK'), text);
    // The soft break at end of line joined the two halves of the word.
    assert(text.includes('thyhet k'), text);
    assert(!text.includes('thyhet ='), 'the soft break is gone');
    // UTF-8 bytes, undecoded: this is a byte stream, not a string.
    assert(text.includes('Ã«'), 'ë survived as its two UTF-8 bytes');
    assertEq(note.size, attachmentBytes(INVOICE, note.path).length);
});

test('the inline logo is kept but not listed as an attachment', () => {
    const all = attachments(INVOICE);
    const logo = all.find((a) => a.mimeType === 'image/png');
    assertEq(logo.disposition, 'inline');
    assertEq(logo.contentId, 'logo-2026@negenet.al');
    assertEq(logo.size, 69);
    // A newsletter's twenty tracking pixels are not twenty attachments.
    const listed = listedAttachments(all);
    assertEq(listed.length, 3);
    assert(!listed.some((a) => a.contentId === 'logo-2026@negenet.al'), 'cid part is not listed');
});

test('a sender-chosen filename cannot escape the folder it is saved in', () => {
    const hostile = attachments(INVOICE).find((a) => a.mimeType === 'application/octet-stream');
    assertEq(hostile.filename, '../../.ssh/authorized_keys');
    const safe = safeFilename(hostile.filename, hostile.mimeType);
    assertEq(safe, 'authorized_keys');
    assert(!safe.includes('/') && !safe.includes('\\'), safe);
    assert(!safe.startsWith('.'), safe);
});

test('every shape of hostile filename comes back as a plain name', () => {
    const cases = [
        ['../../.ssh/authorized_keys', 'authorized_keys'],
        ['..\\..\\Windows\\System32\\drivers\\etc\\hosts', 'hosts'],
        ['/etc/passwd', 'passwd'],
        ['.bashrc', 'bashrc'],
        // A space inside a real filename is not an attack, and stripping
        // it would rename the user's own invoice.
        ['  Fatura F-2026-0007 (mars).pdf  ', 'Fatura F-2026-0007 (mars).pdf'],
        ['line\nbreak.pdf', 'linebreak.pdf'],
        ['nul\u0000byte.pdf', 'nulbyte.pdf'],
        // The right-to-left override that makes `exe` read as `pdf`.
        ['fatura\u202Egnp.exe', 'faturagnp.exe'],
    ];
    for (const [given, want] of cases)
        assertEq(safeFilename(given, ''), want, JSON.stringify(given));
    // Nothing usable left: a name is invented from the declared type,
    // never taken from the sender.
    assertEq(safeFilename('..', 'application/pdf'), 'attachment.pdf');
    assertEq(safeFilename('', 'image/jpeg'), 'attachment.jpg');
    assertEq(safeFilename('   ', 'text/plain'), 'attachment.txt');
    assertEq(safeFilename('/', 'application/x-not-a-thing'), 'attachment.bin');
    // A name long enough to blow a path limit is cut, extension kept.
    const long = safeFilename(`${'a'.repeat(400)}.pdf`, 'application/pdf');
    assert(long.length <= 120, `${long.length}`);
    assert(long.endsWith('.pdf'), long);
});

test('the declared type is shown, and cannot carry markup into the row', () => {
    // The Content-Type is the sender's claim and it is rendered in a
    // label. A claim of `application/pdf<b>` is not a claim about mail.
    const raw = [
        'Content-Type: application/pdf<b>onclick=x</b>; name="x.pdf"',
        'Content-Disposition: attachment; filename="x.pdf"',
        '', 'Zm9v',
    ].join('\r\n');
    const [att] = attachments(raw);
    assert(!att.mimeType.includes('<'), att.mimeType);
    assert(att.mimeType.length <= 80, att.mimeType);
});

test('a single-part message that IS an attachment is listed', () => {
    const raw = [
        'From: a@b.test',
        'Content-Type: application/pdf; name="invoice.pdf"',
        'Content-Transfer-Encoding: base64',
        'Content-Disposition: attachment; filename="invoice.pdf"',
        '',
        ...PDF_B64,
    ].join('\r\n');
    const [att] = attachments(raw);
    assertEq(att.filename, 'invoice.pdf');
    assertEq(attachmentBytes(raw, att.path).length, 396);
});

test('an ordinary message has no attachments and says so', () => {
    const plain = ['From: a@b.test', 'Content-Type: text/plain', '', 'Just words.'].join('\r\n');
    assertEq(attachments(plain), []);
    assertEq(attachments(''), []);
    assertEq(attachments(null), []);
    // A path that addresses no part is null, not a throw and not bytes.
    assertEq(attachmentBytes(plain, [7]), null);
    assertEq(attachmentBytes(plain, 'not a path'), null);
});

test('transfer decoding is a separate, total function', () => {
    assertEq(latin1(decodeTransfer('SGVsbG8gd29ybGQ=', 'base64')), 'Hello world');
    // Wrapped, padded, and with CRLF in it — how it arrives on the wire.
    assertEq(latin1(decodeTransfer('SGVsbG8g\r\nd29ybGQ=\r\n', 'BASE64')), 'Hello world');
    assertEq(latin1(decodeTransfer('a=3Db=20c', 'quoted-printable')), 'a=b c');
    // An unknown or absent encoding is the bytes as they stand.
    assertEq(latin1(decodeTransfer('as sent', '7bit')), 'as sent');
    assertEq(latin1(decodeTransfer('as sent', '')), 'as sent');
    // Total: garbage in, something out, never an exception.
    assertEq(decodeTransfer(null, 'base64').length, 0);
    assert(decodeTransfer('!!!not base64!!!', 'base64').length >= 0, 'garbage decodes to something');
    assertEq(latin1(decodeTransfer('=ZZ', 'quoted-printable')), '=ZZ');
    assertEq(decodedSize('SGVsbG8gd29ybGQ=', 'base64'), 11);
    assertEq(decodedSize('a=3Db=20c', 'quoted-printable'), 5);
    assertEq(decodedSize('as sent', '8bit'), 7);
});

test('an enormous part is bounded rather than allowed to eat the window', () => {
    // 6 MB of base64 is 4.5 MB of bytes. A reading pane must not decode
    // it whole just because a sender said to.
    const huge = 'QUFB'.repeat(1500000);
    const bytes = decodeTransfer(huge, 'base64', {maxBytes: 1024});
    assertEq(bytes.length, 1024);
    assertEq(latin1(bytes).slice(0, 6), 'AAAAAA');
});

test('an attachment past the bound is refused, never silently truncated (#238)', () => {
    // `decodedSize` is uncapped and `decodeTransfer` stops at the
    // bound, and NOTHING compared them: the row said 71,303,169 bytes
    // and Save As wrote 67,108,864 of them, with no error anywhere.
    // Silently writing three quarters of somebody's file is the worst
    // failure in this app — the file opens far enough to look like
    // their problem.
    const pdf = attachments(INVOICE)[0];
    assertEq(pdf.size, 396);
    const short = attachmentBytes(INVOICE, pdf.path, {maxBytes: 100});
    assert(short === null,
        `a bound it cannot meet must be a refusal, got ${short?.length} bytes of a PDF`);
    // Exactly the size it declares is not "past" anything.
    assertEq(attachmentBytes(INVOICE, pdf.path, {maxBytes: 396}).length, 396);
    assertEq(attachmentBytes(INVOICE, pdf.path).length, 396);
});

test('the DEFAULT bound is the one a real attachment crosses (#238)', () => {
    // The old test passed `maxBytes` explicitly, so the bound that
    // actually ships was never crossed by anything. This part is one
    // byte over it, sent as-is — the shape a phone's video attachment
    // arrives in.
    // Concatenated rather than joined, and built once: this fixture is
    // 64 MiB and a suite that copies it four times is a suite nobody
    // runs.
    const raw = 'Content-Type: video/mp4; name="clip.mp4"\r\n' +
        'Content-Transfer-Encoding: binary\r\n' +
        'Content-Disposition: attachment; filename="clip.mp4"\r\n\r\n' +
        'x'.repeat(MAX_PART_BYTES + 1);
    const [att] = attachments(raw);
    assertEq(att.size, MAX_PART_BYTES + 1, 'the row shows the whole file');
    const bytes = attachmentBytes(raw, att.path);
    assert(bytes === null, `saving must refuse rather than write ${bytes?.length} of ${att.size}`);
});

test('the size in the row is the number of bytes that get written', () => {
    // The invariant behind #238, asserted over every part of a real
    // message rather than one of them: what is listed is what is saved,
    // or nothing is.
    for (const att of attachments(INVOICE)) {
        const bytes = attachmentBytes(INVOICE, att.path);
        assertEq(bytes.length, att.size, `${att.filename} lied about its size`);
    }
    // …and the count is BYTES, not characters. `ë` is two bytes on the
    // wire, and every earlier fixture was ASCII, where the two numbers
    // coincide and the assertion proved nothing.
    const raw = messageText(new Uint8Array([
        ...'Content-Type: text/plain; charset=utf-8; name="n.txt"\r\n'.split('')
            .map((c) => c.charCodeAt(0)),
        ...'Content-Disposition: attachment; filename="n.txt"\r\n\r\n'.split('')
            .map((c) => c.charCodeAt(0)),
        0x50, 0xc3, 0xab, 0x72, // P ë r
    ]));
    const [note] = attachments(raw);
    assertEq(note.size, 4, 'four bytes, three characters');
    assertEq(attachmentBytes(raw, note.path).length, 4);
});

test('a malformed message is parsed, not thrown at', () => {
    // No closing boundary, junk where base64 should be, a quote that is
    // never closed. Every one of these arrives eventually.
    const broken = [
        'Content-Type: multipart/mixed; boundary="x"',
        '',
        '--x',
        'Content-Type: application/pdf',
        'Content-Disposition: attachment; filename="broken.pdf',
        '',
        'not base64 at all !!!',
    ].join('\r\n');
    const list = attachments(broken);
    assert(list.length >= 1, JSON.stringify(list));
    assert(attachmentBytes(broken, list[0].path) !== null, 'bytes come back anyway');

    // A part bomb: 40 nested multiparts. Bounded depth, no hang.
    let nested = 'the middle';
    for (let i = 40; i > 0; i--) {
        nested = [
            `Content-Type: multipart/mixed; boundary="b${i}"`, '',
            `--b${i}`, nested, `--b${i}--`, '',
        ].join('\r\n');
    }
    assert(Array.isArray(attachments(nested)), 'a nesting bomb returns a list');
    // …and the leaves it did keep are bounded in number.
    assert(leafParts(nested).length <= 200, `${leafParts(nested).length}`);
});

finish('mail/attachments');
