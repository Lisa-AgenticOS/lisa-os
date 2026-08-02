// A send that fails must not lose the message (#168).
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {msmtpArgv, sendOutcome, sentFilename} from '../lib/send.js';

test('recipients come from the message, not from a second list', () => {
    // -t reads To/Cc from the headers. Passing them separately is how a
    // Cc silently goes undelivered while the header still shows it.
    const argv = msmtpArgv('/home/lisa/.config/lisa/msmtprc', 'work');
    assert(argv.includes('-t'), '-t');
    assert(argv.includes('--read-envelope-from'), 'envelope sender from From:');
    assertEq(argv[argv.indexOf('-a') + 1], 'work');
    assertEq(argv[argv.indexOf('--file') + 1], '/home/lisa/.config/lisa/msmtprc');
});

test('no account means no -a, not an empty one', () => {
    const argv = msmtpArgv('/x/msmtprc', '');
    assert(!argv.includes('-a'), 'a bare msmtp still uses the default account');
});

test('success claims only what SMTP can promise', () => {
    const ok = sendOutcome(0);
    assert(ok.sent);
    // "Sent" — not "delivered". Nobody can promise delivery.
    assertEq(ok.message, 'Sent');
});

test('a config error is not retryable and names the fix', () => {
    const out = sendOutcome(78, 'msmtp: account work not found');
    assert(!out.sent);
    assertEq(out.retryable, false, 'retrying a broken config never helps');
    assert(out.message.includes('lisa mail setup'));
});

test('a 5xx refusal is quoted back so the sender can fix the address', () => {
    const out = sendOutcome(1, 'msmtp: server message: 550 5.1.1 no such user\nmsmtp: could not send');
    assert(!out.sent);
    assertEq(out.retryable, false);
    assert(out.message.includes('550'), 'the server said why; repeat it');
});

test('a network failure is retryable', () => {
    const out = sendOutcome(1, 'msmtp: cannot connect to smtp.example.test, port 587: Network unreachable');
    assert(!out.sent);
    assertEq(out.retryable, true);
    assert(out.message.includes('Network unreachable'));
});

test('an exit code with no stderr still says something useful', () => {
    const out = sendOutcome(42, '');
    assert(!out.sent);
    assert(out.message.includes('42'), 'the code is all we have; do not hide it');
});

test('our own sent copy lands in cur, marked seen', () => {
    // new/ means "arrived and untouched"; a copy of your own outgoing
    // mail there shows as unread mail from yourself in every client.
    const name = sentFilename(1785529483, 'abc');
    assert(name.endsWith(':2,S'), 'seen');
    assert(name.startsWith('1785529483.'), 'delivery-time prefix, so it sorts');
});

finish('mail/send');
