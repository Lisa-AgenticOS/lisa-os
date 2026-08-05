// Find in page (#146 follow-up). Two things worth pinning: the option
// bits (duplicated from the GIR, so the duplication needs a guard) and
// the counter's three states — a search that has not answered yet is not
// a search that found nothing.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    FIND_BACKWARDS, FIND_CASE_INSENSITIVE, FIND_NONE, FIND_WRAP_AROUND,
    MAX_MATCH_COUNT, findOptions, matchLabel, searchable,
} from '../lib/find.js';

test('the option bits are the ones WebKit defines', () => {
    // Pinned against /usr/share/gir-1.0/WebKit-6.0.gir on the reference
    // iMac (WebKitGTK 2.48), bitfield WebKitFindOptions. This module has
    // no gi:// import, so the values are copied — and a copy with no
    // test is a copy that drifts.
    assertEq(FIND_NONE, 0);
    assertEq(FIND_CASE_INSENSITIVE, 1);
    assertEq(FIND_BACKWARDS, 8);
    assertEq(FIND_WRAP_AROUND, 16);
});

test('Ctrl+F searches the way every Ctrl+F does', () => {
    // Case-insensitive and wrapping, because a search that stops at the
    // bottom of the page reads as broken and a case-sensitive default
    // silently finds less than the person expects.
    assertEq(findOptions(), FIND_CASE_INSENSITIVE | FIND_WRAP_AROUND);
    assertEq(findOptions({}), FIND_CASE_INSENSITIVE | FIND_WRAP_AROUND);
    assertEq(findOptions({matchCase: true}), FIND_WRAP_AROUND);
    assertEq(findOptions({backwards: true}),
        FIND_CASE_INSENSITIVE | FIND_WRAP_AROUND | FIND_BACKWARDS);
    assertEq(findOptions({matchCase: true, wrap: false, backwards: true}),
        FIND_BACKWARDS);
    assertEq(findOptions({matchCase: true, wrap: false}), FIND_NONE);
});

test('an empty box is no search, not a failed one', () => {
    assert(!searchable(''));
    assert(!searchable(null));
    assert(searchable('a'));
    assertEq(matchLabel('', 0), '');
    assertEq(matchLabel(null, 12), '');
});

test('the counter has three states and says which one it is in', () => {
    assertEq(matchLabel('cat', null), 'Searching…',
        'the engine has not answered yet — that is not zero');
    assertEq(matchLabel('cat', undefined), 'Searching…');
    assertEq(matchLabel('cat', 0), 'No results');
    assertEq(matchLabel('cat', 1), '1 match');
    assertEq(matchLabel('cat', 12), '12 matches');
    assertEq(matchLabel('cat', MAX_MATCH_COUNT), '1000+ matches',
        'the count is capped, so the label must not claim it is exact');
    assertEq(matchLabel('cat', MAX_MATCH_COUNT + 5), '1000+ matches');
    assertEq(matchLabel('cat', -1), 'No results');
    assertEq(matchLabel('cat', NaN), 'No results');
});

finish('surfer/find');
