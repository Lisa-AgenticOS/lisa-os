// Unit tests for the overlay's Agent Bus client logic (PLAN §5.4/§5.7.1,
// ADR-0013): prompt→tool routing, schema-driven arg filling, outcome
// formatting, and the disposition→consent-UI mapping.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    tokenize, scoreTool, decideAction, fillArgs, quotedSpans, residualText,
    formatValue, formatExecuted, reasonText, appShort, consentView, safeParse,
    NAME_HIT_SCORE,
} from '../lib/agent.js';

// ToolRef-shaped fixtures (what Agent1.Discover returns), mirroring
// apps/notes/app.lisaos.notes.json and agentd's calendar test fixture.
const NOTES = [
    {app_id: 'app.lisaos.notes', name: 'create_note', tier: 'write',
        description: 'Create a note with a title and optional body', undoable: true,
        input_schema: {type: 'object', required: ['title'],
            properties: {title: {type: 'string'}, body: {type: 'string'}}}},
    {app_id: 'app.lisaos.notes', name: 'list_notes', tier: 'read',
        description: 'List active notes (id, title, created), oldest first',
        undoable: false,
        input_schema: {type: 'object', properties: {}}},
    {app_id: 'app.lisaos.notes', name: 'delete_note', tier: 'write',
        description: 'Soft-delete a note; restorable via restore_note',
        undoable: true,
        input_schema: {type: 'object', required: ['id'],
            properties: {id: {type: 'integer'}}}},
    {app_id: 'app.lisaos.notes', name: 'restore_note', tier: 'write',
        description: 'Restore a soft-deleted note', undoable: false,
        input_schema: {type: 'object', required: ['id'],
            properties: {id: {type: 'integer'}}}},
];

const CALENDAR = [
    {app_id: 'org.gnome.Calendar', name: 'add_event', tier: 'write',
        description: 'Add an event to the calendar', undoable: true,
        input_schema: {type: 'object', required: ['title', 'start'],
            properties: {title: {type: 'string'}, start: {type: 'string'}}}},
    {app_id: 'org.gnome.Calendar', name: 'list_events', tier: 'read',
        description: 'List calendar events', undoable: false,
        input_schema: {type: 'object', properties: {}}},
    {app_id: 'org.gnome.Calendar', name: 'delete_event', tier: 'destructive',
        description: 'Delete an event', undoable: false,
        input_schema: {type: 'object', required: ['id'],
            properties: {id: {type: 'string'}}}},
];

test('tokenize mirrors agentd registry tokens()', () => {
    assertEq(tokenize('Add a Calendar-Event!'), ['add', 'a', 'calendar', 'event']);
    assertEq(tokenize('  '), []);
    assertEq(tokenize(null), []);
    assertEq(tokenize('app.lisaos.notes'), ['app', 'lisaos', 'notes']);
});

test('scoreTool weighs name tokens 3, description/app-id 1 (registry parity)', () => {
    const query = tokenize('add a calendar event');
    const add = scoreTool(query, CALENDAR[0]);
    const list = scoreTool(query, CALENDAR[1]);
    const del = scoreTool(query, CALENDAR[2]);
    assert(add > list && add > del, `name-token hit ranks first (${add}/${list}/${del})`);
    assertEq(scoreTool(tokenize('photosynthesis'), CALENDAR[0]), 0);
});

test('decideAction routes an imperative prompt to the scoring tool', () => {
    const d = decideAction('list my notes', NOTES);
    assertEq(d.tool.name, 'list_notes');
    assertEq(d.args, {});

    const create = decideAction('create a note buy milk', NOTES);
    assertEq(create.tool.name, 'create_note');
    assertEq(create.args, {title: 'buy milk'});
    assert(create.score >= NAME_HIT_SCORE, 'crossed the routing threshold');

    const del = decideAction('delete note 3', NOTES);
    assertEq(del.tool.name, 'delete_note');
    assertEq(del.args, {id: 3});
});

test('decideAction keeps chat and weak hits on the inference path', () => {
    assertEq(decideAction('what is the capital of france', NOTES), null);
    assertEq(decideAction('with body', NOTES), null, 'description-only hit is below threshold');
    assertEq(decideAction('', NOTES), null);
    assertEq(decideAction('list my notes', []), null);
    assertEq(decideAction('list my notes', safeParse('not json')), null);
});

test('decideAction refuses args it cannot fill without the model', () => {
    // add_event needs title AND start; one bare utterance cannot split.
    assertEq(decideAction('add a calendar event', CALENDAR), null);
    // Quoted spans fill string props in order — that routes.
    const d = decideAction('add event "dentist" "2026-07-24T10:00:00Z"', CALENDAR);
    assertEq(d.tool.name, 'add_event');
    assertEq(d.args, {title: 'dentist', start: '2026-07-24T10:00:00Z'});
});

test('fillArgs: no required props → empty call', () => {
    assertEq(fillArgs('list my notes', NOTES[1]), {});
    assertEq(fillArgs('anything', {name: 't'}), {}, 'missing schema defaults to no args');
});

test('fillArgs: quoted spans, colon, and residual feed a string prop', () => {
    const create = NOTES[0];
    assertEq(fillArgs('create a note "buy milk"', create), {title: 'buy milk'});
    assertEq(fillArgs('note: buy milk', create), {title: 'buy milk'});
    assertEq(fillArgs('create a note called buy milk', create), {title: 'buy milk'});
    assertEq(fillArgs('create note', create), null, 'no argument material → do not guess');
});

test('fillArgs: integer/number props take the first number in the prompt', () => {
    assertEq(fillArgs('delete note 3', NOTES[2]), {id: 3});
    assertEq(fillArgs('delete note three', NOTES[2]), null);
    const gauge = {name: 'set_gauge', input_schema: {required: ['value'],
        properties: {value: {type: 'number'}}}};
    assertEq(fillArgs('set gauge 2.5', gauge), {value: 2.5});
    const flag = {name: 'set_flag', input_schema: {required: ['on'],
        properties: {on: {type: 'boolean'}}}};
    assertEq(fillArgs('set flag on', flag), null, 'booleans need the model');
});

test('quotedSpans extracts double- and single-quoted text in order', () => {
    assertEq(quotedSpans('add "dentist" then \'friday\''), ['dentist', 'friday']);
    assertEq(quotedSpans('no quotes'), []);
    assertEq(quotedSpans('""'), []);
});

test('residualText strips the matched tokens and leading glue words', () => {
    const create = NOTES[0];
    assertEq(residualText('create a note buy milk', create), 'buy milk');
    assertEq(residualText('create a note called buy milk', create), 'buy milk');
    assertEq(residualText('note: buy milk', create), 'buy milk');
    assertEq(residualText('create note buy milk.', create), 'buy milk');
    assertEq(residualText('create note', create), '');
});

test('formatValue renders notes-shaped results compactly', () => {
    assertEq(formatValue('pong'), 'pong');
    assertEq(formatValue(3), '3');
    assertEq(formatValue(null), '');
    assertEq(formatValue([]), '(none)');
    assertEq(formatValue([{id: 1, title: 'first'}, {id: 2, title: 'second'}]),
        '· #1 first\n· #2 second');
    assertEq(formatValue({id: 4, restored: true}), 'id: 4, restored: true');
    assertEq(formatValue({notes: [{id: 1, title: 'first'}]}), '· #1 first',
        'a single-key array wrapper unwraps');
});

test('formatExecuted parses result + ledger_ref, tolerates garbage', () => {
    assertEq(formatExecuted('{"result":{"id":1,"title":"first"},"ledger_ref":42}'),
        {text: '#1 first', ledgerRef: 42});
    assertEq(formatExecuted('{"result":"ok","ledger_ref":7}'), {text: 'ok', ledgerRef: 7});
    assertEq(formatExecuted('not json'), {text: 'not json', ledgerRef: null});
});

test('reasonText reads denied/failed detail_json', () => {
    assertEq(reasonText('{"reason":"denied by user"}'), 'denied by user');
    assertEq(reasonText('{"error":"boom","ledger_ref":5}'), 'boom');
    assertEq(reasonText('raw reason'), 'raw reason');
});

test('appShort takes the last app-id segment', () => {
    assertEq(appShort('app.lisaos.notes'), 'notes');
    assertEq(appShort('org.gnome.Calendar'), 'Calendar');
});

test('consentView maps a chip spec to chip-weight UI data', () => {
    const view = consentView(JSON.stringify({
        call_id: 1, actor: 'overlay', app_id: 'app.lisaos.notes', tool: 'create_note',
        description: 'Create a note with a title and optional body',
        args: {title: 'buy milk'}, tier: 'write', effective_tier: 'write',
        confirmation: 'chip', escalated: false, chain: ['user'], undoable: true,
    }));
    assertEq(view.modal, false);
    assertEq(view.title, 'notes wants to create_note');
    assertEq(view.argsText, 'title: buy milk');
    assertEq(view.warnings, []);
});

test('consentView maps modal/escalated/destructive specs to warnings', () => {
    const view = consentView(JSON.stringify({
        app_id: 'org.gnome.Calendar', tool: 'delete_event', description: 'Delete an event',
        args: {id: 'evt-1'}, tier: 'destructive', effective_tier: 'destructive',
        confirmation: 'modal', escalated: true, undoable: false,
    }));
    assertEq(view.modal, true);
    assert(view.warnings.includes('requested via untrusted context'), 'escalation called out');
    assert(view.warnings.includes('destructive — this can delete data'), 'tier called out');
    assertEq(view.argsText, 'id: evt-1');

    const notUndoable = consentView(JSON.stringify({
        app_id: 'app.lisaos.notes', tool: 'restore_note', args: {id: 3},
        tier: 'write', effective_tier: 'write', confirmation: 'chip',
        escalated: false, undoable: false,
    }));
    assertEq(notUndoable.modal, false);
    assertEq(notUndoable.warnings, ['not undoable']);

    const garbage = consentView('not json');
    assertEq(garbage.modal, false, 'garbage degrades to a plain chip');
    assertEq(garbage.argsText, '');
});

finish('overlay agent');
