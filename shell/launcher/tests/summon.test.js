// What Super+Space does, given what is already on screen (PLAN §5.7.2,
// issue #255). Pure decision, no GNOME imports — the extension does the
// St/Meta work, this decides what that work should be.
import {assertEq, finish, test} from '../../testing/harness.js';
import {summonAction} from '../lib/summon.js';

test('from the desktop, the key opens the search', () => {
    assertEq(summonAction({overviewVisible: false, searchActive: false}), 'open');
});

test('from a window-picker overview, the key moves into the search', () => {
    // Super+A (app grid) or the hot corner got here. The chord is a
    // search key, not an overview toggle: it must land in the entry
    // rather than closing the overview the user just opened.
    assertEq(summonAction({overviewVisible: true, searchActive: false}), 'open');
});

test('from the search itself, the key dismisses — Spotlight, not a one-way door', () => {
    assertEq(summonAction({overviewVisible: true, searchActive: true}), 'dismiss');
});

test('a search flagged active while the overview is down cannot dismiss', () => {
    // searchActive lags the overview by an animation frame; treating a
    // stale flag as "showing" would eat the key press that was meant to
    // open the search.
    assertEq(summonAction({overviewVisible: false, searchActive: true}), 'open');
});

test('missing state is a request to open, never to close', () => {
    assertEq(summonAction({}), 'open');
    assertEq(summonAction(null), 'open');
    assertEq(summonAction(undefined), 'open');
});

finish('launcher summon');
