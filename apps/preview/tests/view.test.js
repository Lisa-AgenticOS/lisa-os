import {zoomStep, fitScale, fitWidthScale, step, rotate, MIN_ZOOM, MAX_ZOOM} from '../lib/view.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, what) => test(what, () => assert(cond));
const near = (a, b) => Math.abs(a - b) < 1e-6;

ok(zoomStep(1, +1) === 1.5, 'zoom in from 100% lands on a ladder step');
ok(zoomStep(1, -1) === 0.67, 'zoom out from 100% lands on a ladder step');
ok(zoomStep(MAX_ZOOM, +1) === MAX_ZOOM, 'zoom clamps at the top');
ok(zoomStep(MIN_ZOOM, -1) === MIN_ZOOM, 'zoom clamps at the bottom');
// A fit ratio is almost never on the ladder — stepping from one must
// still move, or +/- appears dead after "fit to window".
ok(zoomStep(0.73, +1) === 1, 'stepping up from an off-ladder fit ratio moves');
ok(zoomStep(0.73, -1) === 0.67, 'stepping down from an off-ladder fit ratio moves');

ok(near(fitScale({width: 200, height: 100}, {width: 100, height: 100}), 0.5), 'fit uses the tighter axis');
ok(near(fitScale({width: 16, height: 16}, {width: 4000, height: 4000}), 1), 'fit never enlarges — a blown-up favicon is not a fit');
ok(near(fitScale({width: 16, height: 16}, {width: 4000, height: 4000}, 0, true), 250),
    'explicit fit (the button, the 0 key) DOES enlarge — Fit doing nothing on a small icon reads as broken');
ok(near(fitScale({width: 200, height: 100}, {width: 100, height: 100}, 0, true), 0.5),
    'enlarge changes nothing when content is already bigger than the viewport');
ok(near(fitScale({width: 200, height: 100}, {width: 100, height: 100}, 90), 0.5),
    'a quarter turn swaps the axes for fit');
ok(near(fitScale({width: 0, height: 0}, {width: 100, height: 100}), 1), 'a zero-sized page does not divide by zero');

// Fit-width is the reading mode and MAY enlarge: legibility beats
// pixel fidelity when the point is text.
ok(near(fitWidthScale({width: 100, height: 400}, {width: 800, height: 200}), 8), 'fit width fills the width');
ok(near(fitWidthScale({width: 400, height: 100}, {width: 800, height: 200}, 90), 8), 'fit width honours rotation');

ok(step(0, 5, -1) === 0, 'paging back from the first page stays put');
ok(step(4, 5, +1) === 4, 'paging past the last page stays put — no wrap');
ok(step(2, 5, +1) === 3, 'paging forward moves one');
ok(step(0, 0, +1) === 0, 'an empty document has no page to move to');

ok(rotate(0, -90) === 270, 'rotating anticlockwise from 0 gives 270, not -90');
ok(rotate(270, 90) === 0, 'rotation wraps to 0');

finish('preview/view');
