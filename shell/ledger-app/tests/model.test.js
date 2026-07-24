// Unit tests for the Ledger app view-model (PLAN §5.7.6).
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    buildTimeline, filterRows, distinctValues, usageStats,
    dayOf, formatTs, describeRow,
} from '../lib/model.js';

// Fixture mirrors lisa-ledger's tests: a generate start entry, its
// completion (ref_id), and an unrelated context.search — newest first,
// as `lisa ledger --json` emits.
const T0 = new Date(2026, 6, 22, 9, 30, 0).getTime(); // local 2026-07-22
const ENTRIES = [
    {id: 3, ts: T0 + 60_000, kind: 'context.search', app_id: 'host', model: '',
     input_hash: 'q1', preview: '', status: 'ok', detail: '', ref_id: null,
     output_tokens: 0, duration_ms: 0},
    {id: 2, ts: T0 + 900, kind: 'inference.complete', app_id: 'dev.lisaos.Overlay',
     model: 'qwen3-0.6b', input_hash: 'h1', preview: 'hello', status: 'ok',
     detail: '', ref_id: 1, output_tokens: 42, duration_ms: 900},
    {id: 1, ts: T0, kind: 'inference.generate', app_id: 'dev.lisaos.Overlay',
     model: 'qwen3-0.6b', input_hash: 'h1', preview: 'hello', status: 'started',
     detail: '', ref_id: null, output_tokens: 0, duration_ms: 0},
];

test('buildTimeline absorbs completions into start entries', () => {
    const rows = buildTimeline(ENTRIES);
    assertEq(rows.length, 2, 'completion row absorbed');
    const gen = rows.find(r => r.id === 1);
    assertEq(gen.effective_status, 'ok');
    assertEq(gen.effective_tokens, 42);
    assertEq(gen.effective_duration_ms, 900);
    assertEq(gen.completion.id, 2);
    const search = rows.find(r => r.id === 3);
    assertEq(search.completion, null);
    assertEq(search.effective_status, 'ok');
});

test('a start entry with no completion stays in-flight', () => {
    const rows = buildTimeline([ENTRIES[2]]);
    assertEq(rows[0].effective_status, 'started');
});

test('filterRows by app, kind, and day', () => {
    const rows = buildTimeline(ENTRIES);
    assertEq(filterRows(rows, {app: 'dev.lisaos.Overlay'}).map(r => r.id), [1]);
    assertEq(filterRows(rows, {kind: 'context.search'}).map(r => r.id), [3]);
    assertEq(filterRows(rows, {day: dayOf(T0)}).length, 2);
    assertEq(filterRows(rows, {day: '1999-01-01'}).length, 0);
    assertEq(filterRows(rows, {app: 'all', kind: 'all', day: 'all'}).length, 2);
    assertEq(filterRows(rows, {}).length, 2);
});

test('distinctValues feeds the dropdowns sorted', () => {
    const rows = buildTimeline(ENTRIES);
    assertEq(distinctValues(rows, 'app_id'), ['dev.lisaos.Overlay', 'host']);
    assertEq(distinctValues(rows, 'kind'), ['context.search', 'inference.generate']);
    assertEq(distinctValues([], 'kind'), []);
});

test('usageStats sums tokens by app and counts by kind', () => {
    const {tokensByApp, countsByKind} = usageStats(ENTRIES);
    assertEq(tokensByApp, [['dev.lisaos.Overlay', 42]]);
    assertEq(countsByKind.length, 3);
    assert(countsByKind.every(([, n]) => n === 1));
});

test('timestamps format as local date and datetime', () => {
    assertEq(dayOf(T0), '2026-07-22');
    assertEq(formatTs(T0), '2026-07-22 09:30:00');
});

test('describeRow is a compact one-liner', () => {
    const rows = buildTimeline(ENTRIES);
    const gen = rows.find(r => r.id === 1);
    assertEq(describeRow(gen),
        'inference.generate · dev.lisaos.Overlay · ok · 42 tok · 900 ms');
    const search = rows.find(r => r.id === 3);
    assertEq(describeRow(search), 'context.search · host · ok');
});

finish('ledger model');
