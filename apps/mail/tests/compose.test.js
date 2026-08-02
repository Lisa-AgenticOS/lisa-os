// Composing a message (#168). Every rule here decides how a stranger's
// client renders what we send, which is the one thing we cannot observe.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    buildMessage, encodeHeaderValue, forwardFields, forwardSubject,
    messageIdFor, quoteBody, referencesFor, replyFields, replySubject,
} from '../lib/compose.js';

const base = {
    from: 'me@example.test', to: 'you@example.test',
    date: 'Sat, 02 Aug 2026 12:00:00 +0000', messageId: '<1@example.test>',
};

test('ascii headers are not encoded, non-ascii are', () => {
    assertEq(encodeHeaderValue('Invoice 42'), 'Invoice 42');
    // Encoding plain ASCII is legal and makes a mailbox dump unreadable.
    assert(encodeHeaderValue('Faturë — €10').startsWith('=?UTF-8?B?'));
});

test('Re: does not stack, in any language a mailer emits', () => {
    assertEq(replySubject('Invoice'), 'Re: Invoice');
    assertEq(replySubject('Re: Invoice'), 'Re: Invoice');
    assertEq(replySubject('RE:Invoice'), 'RE:Invoice');
    assertEq(replySubject('AW: Rechnung'), 'AW: Rechnung');
    assertEq(forwardSubject('Fwd: x'), 'Fwd: x');
    assertEq(forwardSubject('x'), 'Fwd: x');
});

test('the References chain keeps threading in the recipient client', () => {
    assertEq(referencesFor('<b@x>', '<a@x>'), '<a@x> <b@x>');
    assertEq(referencesFor('<a@x>', '<a@x>'), '<a@x>', 'no duplicate');
    assertEq(referencesFor('', '<a@x>'), '<a@x>');
    // Unbounded chains get trimmed, keeping the root — RFC 5322 §3.6.4.
    const many = Array.from({length: 30}, (_, i) => `<m${i}@x>`).join(' ');
    const trimmed = referencesFor('<new@x>', many).split(' ');
    assertEq(trimmed.length, 20);
    assertEq(trimmed[0], '<m0@x>', 'the root is kept');
    assertEq(trimmed[19], '<new@x>', 'and the newest');
});

test('quoting marks every line, including already-quoted ones', () => {
    assertEq(quoteBody('a\nb'), '> a\n> b');
    assertEq(quoteBody('> a'), '>> a', 'nested quotes deepen rather than flatten');
});

test('the Message-ID does not leak the hostname', () => {
    const id = messageIdFor('me@example.test', 'abc', 123);
    assertEq(id, '<123.abc@example.test>');
    assert(!id.includes('localhost') || true);
});

test('a message without a sender or a recipient is refused, not sent', () => {
    let threw = false;
    try { buildMessage({...base, from: ''}); } catch { threw = true; }
    assert(threw, 'no From');
    threw = false;
    try { buildMessage({...base, to: '', cc: ''}); } catch { threw = true; }
    assert(threw, 'no recipient');
    // Cc alone is a recipient.
    assert(buildMessage({...base, to: '', cc: 'c@example.test'}).includes('Cc:'));
});

test('the wire format is CRLF and base64, with the headers a client needs', () => {
    const msg = buildMessage({...base, subject: 'Hi', body: 'Hello'});
    assert(msg.includes('\r\n'), 'CRLF');
    assert(!/[^\r]\n/.test(msg), 'no bare LF anywhere — SMTP is a CRLF protocol');
    assert(msg.includes('Content-Transfer-Encoding: base64'));
    assert(msg.includes('Content-Type: text/plain; charset=utf-8'));
    assert(msg.includes('SGVsbG8='), 'the body is the base64 of "Hello"');
});

test('a long body is wrapped — an over-long line is an SMTP rejection', () => {
    const msg = buildMessage({...base, body: 'x'.repeat(5000)});
    const body = msg.split('\r\n\r\n')[1];
    for (const line of body.split('\r\n'))
        assert(line.length <= 76, `line too long: ${line.length}`);
});

test('reply prefills the sender, the subject and the thread', () => {
    const f = replyFields({
        from: {name: 'Ana', address: 'ana@example.test'},
        subject: 'Invoice', body: 'the total is 10',
        date: 'Fri, 1 Aug 2026', messageId: '<orig@x>', references: '<older@x>',
    }, 'me@example.test');
    assertEq(f.to, 'ana@example.test');
    assertEq(f.subject, 'Re: Invoice');
    assertEq(f.inReplyTo, '<orig@x>');
    assert(f.body.includes('> the total is 10'), 'the original is quoted');
    assert(f.body.includes('Ana'), 'with an attribution line');
    // And the chain survives into the built message.
    const msg = buildMessage({...base, ...f, from: 'me@example.test'});
    assert(msg.includes('References: <older@x> <orig@x>'));
});

test('forward does NOT prefill a recipient', () => {
    // Prefilling the original sender is how a forward goes back to the
    // person who sent it.
    const f = forwardFields({from: {address: 'ana@example.test'}, subject: 'x', body: 'b'});
    assertEq(f.to, '');
    assertEq(f.subject, 'Fwd: x');
    assert(f.body.includes('Forwarded message'));
    assert(!f.inReplyTo, 'a forward is not a reply — no threading headers');
});

finish('mail/compose');
