// Notes' view model. Pure, so it runs under node — the window itself
// needs a display and is not tested here.

import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {ordered, preview, displayTitle, isWorthSaving, matches, groupByPeriod} from '../lib/model.js';

test('newest first, and identical timestamps do not reshuffle', () => {
    const a = {id: 'a', updated_at: '2026-08-01T10:00:00Z'};
    const b = {id: 'b', updated_at: '2026-08-06T10:00:00Z'};
    const c = {id: 'c', updated_at: '2026-08-06T10:00:00Z'};
    assertEq(ordered([a, b, c]).map((n) => n.id).join(''), 'bca');
    // Same input, same order — a list that reorders when nothing
    // changed reads as data loss.
    assertEq(ordered([c, b, a]).map((n) => n.id).join(''), 'bca');
});

test('a note with no updated_at falls back to created_at', () => {
    const old = {id: 'old', created_at: '2026-01-01T00:00:00Z'};
    const recent = {id: 'new', created_at: '2026-08-06T00:00:00Z'};
    assertEq(ordered([old, recent])[0].id, 'new');
});

test('preview flattens newlines instead of cutting at the first one', () => {
    // A note whose first line is blank would preview as empty and look
    // like it had lost its body.
    assertEq(preview('\n\nthe actual content'), 'the actual content');
    assertEq(preview('one\ntwo'), 'one two');
});

test('preview truncates with an ellipsis and never exceeds the limit', () => {
    const out = preview('x'.repeat(200), 20);
    assert(out.length <= 20, `got ${out.length}`);
    assert(out.endsWith('…'));
});

test('an untitled note shows its body, not an empty row', () => {
    assertEq(displayTitle({title: '  ', body: 'buy milk'}), 'buy milk');
    assertEq(displayTitle({title: 'Groceries', body: 'x'}), 'Groceries');
    // Nothing at all is still legible.
    assertEq(displayTitle({}), 'Untitled note');
});

test('an empty draft is not saved', () => {
    assert(!isWorthSaving({title: '', body: ''}));
    assert(!isWorthSaving({title: '   ', body: '\n'}));
    assert(isWorthSaving({title: '', body: 'something'}));
    assert(isWorthSaving({title: 'something', body: ''}));
});

test('filtering looks at title and body, and an empty query matches all', () => {
    const n = {title: 'Groceries', body: 'oat milk'};
    assert(matches(n, ''));
    assert(matches(n, 'groc'));
    assert(matches(n, 'MILK'));
    assert(!matches(n, 'bicycle'));
});

// --- date grouping, the way macOS Notes does it -----------------------
// `now` is fixed here rather than mocked: the function takes it, which
// is the whole reason it can be tested at all.
const NOW = new Date('2026-08-06T12:00:00Z');
const at = (id, iso) => ({id, title: `n${id}`, created: iso});
const labels = (g) => g.map((x) => x.label);

test('the recent buckets are Apple\'s, in Apple\'s order', () => {
    const g = groupByPeriod([
        at(1, '2026-08-06T09:00:00Z'),  // today
        at(2, '2026-08-05T09:00:00Z'),  // yesterday
        at(3, '2026-08-02T09:00:00Z'),  // previous 7
        at(4, '2026-07-20T09:00:00Z'),  // previous 30
    ], NOW);
    assertEq(labels(g).join(' | '),
        'Today | Yesterday | Previous 7 Days | Previous 30 Days');
});

test('older notes fall back to month names this year, then years', () => {
    const g = groupByPeriod([
        at(1, '2026-03-02T09:00:00Z'),
        at(2, '2026-01-15T09:00:00Z'),
        at(3, '2025-11-10T09:00:00Z'),
        at(4, '2024-01-01T09:00:00Z'),
    ], NOW);
    assertEq(labels(g).join(' | '), 'March | January | 2025 | 2024');
});

test('empty groups are not rendered', () => {
    const g = groupByPeriod([at(1, '2026-08-06T09:00:00Z')], NOW);
    assertEq(labels(g).join(''), 'Today');
});

test('a note with an unreadable date is kept, not dropped', () => {
    // Losing a note from the list because its timestamp was malformed is
    // the worst possible way to handle bad data.
    const g = groupByPeriod([at(1, 'not-a-date'), at(2, '2026-08-06T09:00:00Z')], NOW);
    assertEq(labels(g).join(' | '), 'Today | Older');
    assertEq(g.find((x) => x.label === 'Older').notes[0].id, 1);
});

test('every note lands in exactly one group', () => {
    const notes = [
        at(1, '2026-08-06T09:00:00Z'), at(2, '2026-08-05T01:00:00Z'),
        at(3, '2026-08-01T09:00:00Z'), at(4, '2026-07-11T09:00:00Z'),
        at(5, '2026-02-01T09:00:00Z'), at(6, '2023-02-01T09:00:00Z'),
        at(7, 'rubbish'),
    ];
    const g = groupByPeriod(notes, NOW);
    const seen = g.flatMap((x) => x.notes.map((n) => n.id)).sort();
    assertEq(seen.join(','), '1,2,3,4,5,6,7');
});

test('within a group, newest is still first', () => {
    const g = groupByPeriod([
        at(1, '2026-08-06T08:00:00Z'),
        at(2, '2026-08-06T11:00:00Z'),
    ], NOW);
    assertEq(g[0].notes.map((n) => n.id).join(','), '2,1');
});

finish('notes/model');
