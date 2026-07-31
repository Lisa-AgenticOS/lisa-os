// What the Assistant is allowed to ask for, and what it does with the
// answer. The dispositions matter more than the happy path: two of them
// are things this window must refuse to handle by itself.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {READ_TIER, callOptions, forTranscript, interpret, offerable} from '../lib/tools.js';

test('only read-tier tools are offered to the model', () => {
    // Shapes taken from what the reference iMac actually reports, not
    // from memory: the first version of this filter invented three tool
    // names that do not exist.
    const catalog = JSON.stringify([
        {name: 'search_mail', tier: 'read', app_id: 'app.lisaos.Mail'},
        {name: 'create_note', tier: 'write', app_id: 'app.lisaos.notes'},
        {name: 'search_notes', tier: 'read', app_id: 'app.lisaos.notes'},
        {name: 'delete_note', tier: 'destructive', app_id: 'app.lisaos.notes'},
        {name: 'read_page', tier: 'read', app_id: 'app.lisaos.Surfer'},
    ]);
    assertEq(
        offerable(catalog).map((t) => t.name),
        ['search_mail', 'search_notes', 'read_page']);
    assertEq(READ_TIER, 'read');
});

test('a malformed catalog offers nothing rather than something broken', () => {
    // A garbled entry handed to a model becomes a tool call that cannot
    // succeed and an error a person has to read.
    assertEq(offerable('not json'), []);
    assertEq(offerable('{"not":"an array"}'), []);
    assertEq(offerable(JSON.stringify([null, 42, {}, {name: 7}])), []);
    // No tier is not read tier. Assuming would offer a destructive tool;
    // dropping loses a capability somebody notices and reports.
    assertEq(offerable(JSON.stringify([{name: 'search_mail'}])), []);
    assertEq(offerable(undefined), []);
});

test('a parked confirmation is reported, never answered', () => {
    // The Assistant is the peer that asked. Answering its own
    // confirmation is precisely the hole #145 closed, and it must not
    // reappear as a convenience in the client.
    for (const d of ['confirm-chip', 'confirm-modal']) {
        const out = interpret(d, '{"tool":"send_message"}');
        assertEq(out.kind, 'parked', d);
        assert(out.text.includes('confirmation'), out.text);
    }
});

test('a denial is an answer, not a prompt to try again', () => {
    const out = interpret('denied', '{"reason":"guard: escalate.privilege"}');
    assertEq(out.kind, 'denied');
    assert(out.text.includes('escalate.privilege'), out.text);
});

test('results and failures are told apart', () => {
    assertEq(interpret('executed', '{"result":"3 messages"}'), {kind: 'result', text: '3 messages'});
    assertEq(interpret('failed', '{"error":"no such folder"}').kind, 'error');
    // An unknown disposition is an error, not a silent success: agentd
    // may grow one, and guessing would be the wrong direction.
    assertEq(interpret('something-new', '{}').kind, 'error');
    // Unparseable detail must not throw — the window is mid-conversation.
    assertEq(interpret('executed', 'not json').kind, 'result');
    assertEq(interpret('executed', undefined).kind, 'result');
});

test('the provenance claim is stated, because an empty one escalates', () => {
    const o = callOptions();
    assertEq(o.provenance, ['user']);
    assertEq(o.actor, 'user');
    // agentd verifies this against peer credentials (#55); the Assistant
    // is one of Lisa's own programs, so the claim holds. Omitting it
    // would mean "unknown", which escalates — asking a person to confirm
    // reading their own mail.
});

test('a huge result is truncated visibly rather than silently', () => {
    const big = 'x'.repeat(9000);
    const out = forTranscript(big);
    assert(out.length < 4200, out.length);
    assert(out.includes('more characters not shown'), out.slice(-60));
    // Short results pass through untouched.
    assertEq(forTranscript('short'), 'short');
    assertEq(forTranscript(''), '');
});

finish('assistant/tools');
