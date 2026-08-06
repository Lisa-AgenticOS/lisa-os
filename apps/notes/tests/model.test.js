// Notes' view model. Pure, so it runs under node — the window itself
// needs a display and is not tested here.

import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {ordered, preview, displayTitle, isWorthSaving, matches} from '../lib/model.js';

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

finish('notes/model');
