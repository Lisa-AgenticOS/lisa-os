// Smart grouping: a rule that is wrong is not a crash, it is a message
// the user never sees.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {parseHeaders} from '../lib/rfc822.js';
import {GROUPS, classify, grouped, unreadCount} from '../lib/smart.js';
import {listFolder, messageId, messagePath, parseFilename, previewOf} from '../lib/maildir.js';

const h = (text) => parseHeaders(text);
const msg = (over = {}) => ({seen: false, flagged: false, draft: false, trashed: false, ...over});

test('List-Unsubscribe makes it a newsletter, whoever it claims to be from', () => {
    assertEq(
        classify(msg(), h('From: Kaleidoscope <news@example.test>\nList-Unsubscribe: <https://x/u>')),
        'Newsletters');
    assertEq(classify(msg(), h('From: x@y.test\nList-Id: <list.example.test>')), 'Newsletters');
    assertEq(classify(msg(), h('From: x@y.test\nPrecedence: bulk')), 'Newsletters');
});

test('a newsletter from a noreply address is still a newsletter', () => {
    // Order matters: the unsubscribe header is the stronger signal, and
    // most newsletters are also sent from noreply@.
    assertEq(
        classify(msg(), h('From: noreply@example.test\nList-Unsubscribe: <https://x/u>')),
        'Newsletters');
});

test('automated mail with no unsubscribe is a notification', () => {
    assertEq(classify(msg(), h('From: no-reply@google.test')), 'Notifications');
    assertEq(classify(msg(), h('From: notifications@linkedin.test')), 'Notifications');
    assertEq(classify(msg(), h('From: x@y.test\nAuto-Submitted: auto-generated')), 'Notifications');
    // `Auto-Submitted: no` is the explicit "a person sent this".
    assertEq(classify(msg(), h('From: x@y.test\nAuto-Submitted: no')), 'People');
});

test('a person writing about a security alert is not filed as a notification', () => {
    // The failure this ordering exists to avoid: classification by
    // subject keyword would bury a real message from a colleague.
    assertEq(
        classify(msg(), h('From: Jane <jane@example.test>\nSubject: Security alert for your account')),
        'People');
});

test('pinned outranks everything, and seen outranks classification', () => {
    const newsletter = 'From: x@y.test\nList-Unsubscribe: <https://x/u>';
    assertEq(classify(msg({flagged: true}), h(newsletter)), 'Pinned');
    // Read: it drops out of the working set whatever it is.
    assertEq(classify(msg({seen: true}), h(newsletter)), 'Seen');
    // Pinned beats seen — the user's decision is not undone by reading it.
    assertEq(classify(msg({flagged: true, seen: true}), h(newsletter)), 'Pinned');
});

test('empty groups are dropped, and the order is the declared one', () => {
    const items = [
        {id: '1', group: 'Seen'},
        {id: '2', group: 'People'},
        {id: '3', group: 'Seen'},
    ];
    const out = grouped(items);
    assertEq(out.map((g) => g.name), ['People', 'Seen']);
    assertEq(out[1].items.length, 2);
    assertEq(grouped([]), []);
    // Every group name used by classify is one the display knows.
    for (const name of ['Pinned', 'People', 'Newsletters', 'Notifications', 'Seen'])
        assert(GROUPS.includes(name), `${name} is not a display group`);
});

test('unread counts what a person would call unread', () => {
    assertEq(unreadCount([
        {seen: false}, {seen: true}, {seen: false, draft: true}, {seen: false, trashed: true},
    ]), 1);
});

test('maildir flags come from the filename, and new/ is unread by definition', () => {
    const cur = parseFilename('1753900000.M1P2.host:2,SF', 'cur');
    assert(cur.seen && cur.flagged, JSON.stringify(cur));
    assert(!cur.replied && !cur.draft, JSON.stringify(cur));
    // Same flags, but sitting in new/ — which nothing has read.
    assertEq(parseFilename('1753900000.M1P2.host:2,S', 'new').seen, false);
    // No info section at all.
    assertEq(parseFilename('1753900000.M1P2.host', 'new').seen, false);
});

test('the list is newest first and trashed messages are not in it', () => {
    const out = listFolder('INBOX', [
        {dir: 'cur', name: '1753900001.a.host:2,S'},
        {dir: 'new', name: '1753900003.c.host'},
        {dir: 'cur', name: '1753900002.b.host:2,ST'},
        {dir: 'tmp', name: '1753900004.d.host'},
    ]);
    assertEq(out.map((m) => m.unique), ['1753900003.c.host', '1753900001.a.host']);
    assertEq(out[0].seen, false);
    assertEq(out[0].id, 'INBOX/1753900003.c.host');
});

test('a message id cannot become a path outside the maildir', () => {
    // The id round-trips from a model in read_message, and a model that
    // read a hostile message can be talked into asking for anything.
    for (const bad of ['..', '.', '', 'a/b', 'a\\b', 'x\0y']) {
        assertEq(messagePath('/m', bad, 'cur', 'f'), null, `folder ${bad}`);
        assertEq(messagePath('/m', 'INBOX', 'cur', bad), null, `file ${bad}`);
    }
    // Only the two real maildir subdirectories.
    assertEq(messagePath('/m', 'INBOX', 'tmp', 'f'), null);
    assertEq(messagePath('/m', 'INBOX', '../cur', 'f'), null);
    assertEq(messagePath('/m', 'INBOX', 'cur', '1753900001.a.host'),
        '/m/INBOX/cur/1753900001.a.host');
    // The id normalises the hostile characters a maildir name can carry.
    assert(!messageId('IN/BOX', '1.a:2,S').includes('/1.a:'), messageId('IN/BOX', '1.a:2,S'));
});

test('the preview drops quoted replies and signatures', () => {
    const body = ['Thanks, that works.', '', '> the original question', '> second quoted line',
        '-- ', 'Jane Doe', 'Example Ltd'].join('\n');
    const p = previewOf(body);
    assert(p.startsWith('Thanks, that works.'), p);
    assert(!p.includes('original question'), p);
    assert(!p.includes('Example Ltd'), p);
    // Long bodies are cut with an ellipsis, not mid-nothing.
    assert(previewOf('x'.repeat(400)).endsWith('…'));
    assertEq(previewOf(''), '');
});

finish('mail/smart');
