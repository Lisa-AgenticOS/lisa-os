// The reading pane's shape, and who owns the Space key (#247).
//
// `lisa-mail.js` cannot be imported without a GTK host, so the parts of
// it worth pinning live here instead and the app is built FROM them.
// That is the point: an `append()` sequence is untestable and a list of
// slot names is not, so the order became data. If someone reorders the
// reader by editing this array, these tests judge it; if someone
// reorders it by editing `lisa-mail.js`, they cannot, because
// `lisa-mail.js` no longer contains an order to edit.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    READER_ORDER, orderedReaderChildren, spaceAction,
} from '../lib/reader.js';

// ---- where the attachment bar sits ----------------------------------

test('attachments render below the message body', () => {
    // The reported defect: attachments sat between the sender and the
    // message, which is where no mail client puts them.
    const body = READER_ORDER.indexOf('body');
    const attachments = READER_ORDER.indexOf('attachments');
    assert(body !== -1, 'the reader has no body slot');
    assert(attachments !== -1, 'the reader has no attachments slot');
    assert(attachments > body,
        `attachments (${attachments}) must come after the body (${body})`);
});

test('the remote-images banner stays with the body it is about', () => {
    // It says what was withheld from THIS message's rendering, so it
    // belongs directly above the thing it is talking about — not
    // stranded above the attachment bar as it was.
    assertEq(READER_ORDER[READER_ORDER.indexOf('body') - 1], 'remote-banner');
});

test('the reader has no duplicate or empty slots', () => {
    assertEq(new Set(READER_ORDER).size, READER_ORDER.length);
    for (const slot of READER_ORDER)
        assert(typeof slot === 'string' && slot.length > 0, `bad slot ${slot}`);
});

test('the child list is built from the order, in the order', () => {
    const widgets = {
        'title': 'T', 'from': 'F', 'remote-banner': 'B',
        'body': 'BODY', 'attachments': 'ATT',
    };
    assertEq(orderedReaderChildren(widgets), ['T', 'F', 'B', 'BODY', 'ATT']);
});

test('a slot with no widget is a crash, not a silently missing bar', () => {
    // The failure this prevents is the one that started #247: a widget
    // that is built, wired, and then not visible where anyone expects
    // it. Dropping the attachment bar must be loud.
    const widgets = {
        'title': 'T', 'from': 'F', 'remote-banner': 'B', 'body': 'BODY',
    };
    let threw = '';
    try {
        orderedReaderChildren(widgets);
    } catch (e) {
        threw = e.message;
    }
    assert(threw.includes('attachments'),
        `expected the missing slot to be named, got: ${threw || '(no throw)'}`);
});

// ---- who gets the Space key -----------------------------------------

test('Space peeks the attachment the person is on', () => {
    assertEq(spaceAction({focusedAttachment: 'a.pdf'}),
        {action: 'peek', attachment: 'a.pdf'});
    // Focus on the list itself rather than a row: the selected row is
    // what the person means. This is the path that only became
    // reachable once a single click stopped opening the attachment —
    // before that you could not select without opening.
    assertEq(spaceAction({selectedAttachment: 'a.pdf'}),
        {action: 'peek', attachment: 'a.pdf'});
});

test('a focused row wins over a stale selection', () => {
    // Which one gets peeked, not merely that one does. This assertion
    // exists because its first draft could not fail: `spaceAction`
    // returned 'peek' either way and `lisa-mail.js` re-derived the
    // choice, so reversing the precedence killed no test.
    assertEq(spaceAction({focusedAttachment: 'a.pdf', selectedAttachment: 'b.pdf'}),
        {action: 'peek', attachment: 'a.pdf'});
});

test('a focused Save or Open button keeps Space', () => {
    // GtkButton binds Space to activate. The controller is in CAPTURE
    // phase, so it sees the key before the button does and has to hand
    // it back deliberately — otherwise the keyboard route to the two
    // actions in the row is dead.
    assertEq(spaceAction({focusIsButton: true, focusedAttachment: 'a.pdf'}),
        {action: 'pass'});
    assertEq(spaceAction({focusIsButton: true, selectedAttachment: 'a.pdf'}),
        {action: 'pass'});
});

test('Space with nothing to peek is passed on, never swallowed', () => {
    // The controller lives on the attachment list, so a key press in the
    // message list, the search entry or any text field is not on its
    // path at all. This pins the remaining case: focus IS in the list,
    // but there is no attachment under it — Space must still behave like
    // Space rather than vanishing.
    assertEq(spaceAction({}), {action: 'pass'});
    assertEq(spaceAction({focusedAttachment: null, selectedAttachment: null}),
        {action: 'pass'});
});

finish('mail/reader');
