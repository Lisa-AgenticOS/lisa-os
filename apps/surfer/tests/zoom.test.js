// Page zoom (#146 follow-up). Small, and a module rather than two lines
// in a signal handler because `level * 1.1` compounds float error until
// Ctrl+0 no longer lands exactly on 100%, and nothing stops it at forty
// presses.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    DEFAULT_ZOOM, ZOOM_STEPS, zoomIn, zoomLabel, zoomOut, zoomReset,
} from '../lib/zoom.js';

test('zooming in and out walks the steps', () => {
    assertEq(zoomIn(1), 1.1);
    assertEq(zoomIn(1.1), 1.2);
    assertEq(zoomOut(1), 0.9);
    assertEq(zoomOut(1.1), 1, 'zooming out from 110% lands ON 100%, not near it');
});

test('the steps have ends', () => {
    const top = ZOOM_STEPS[ZOOM_STEPS.length - 1];
    const bottom = ZOOM_STEPS[0];
    assertEq(zoomIn(top), top, 'forty presses of Ctrl+plus is still 300%');
    assertEq(zoomOut(bottom), bottom);
});

test('Ctrl+0 is exactly 100%', () => {
    assertEq(zoomReset(), 1);
    assertEq(DEFAULT_ZOOM, 1);
});

test('a level that is not on a step still steps sanely', () => {
    // A restored or externally set zoom must not be a dead end.
    assertEq(zoomIn(1.06), 1.2, 'nearest step is 1.1, so in goes to 1.2');
    assertEq(zoomOut(1.06), 1);
    // Exactly between two steps, the LOWER one is the anchor — so the
    // first press after a stray value moves in the direction asked for
    // rather than appearing to do nothing.
    assertEq(zoomIn(1.05), 1.1);
    assertEq(zoomOut(1.05), 0.9);
    assertEq(zoomIn(0), 1.1, 'nonsense anchors at 100%');
    assertEq(zoomIn(null), 1.1);
    assertEq(zoomOut(NaN), 0.9);
});

test('the label reads like a percentage and not like a float', () => {
    assertEq(zoomLabel(1), '100%');
    assertEq(zoomLabel(1.33), '133%');
    assertEq(zoomLabel(0.67), '67%');
    assertEq(zoomLabel(null), '100%');
    assert(!zoomLabel(1.33).includes('.'));
});

finish('surfer/zoom');
