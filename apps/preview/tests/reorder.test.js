import {movePage, removePage, isIdentity, orderChanged, qpdfPageSpec,
    rotatePageBy, rotationOf, rotationsChanged, qpdfRotateArgs} from '../lib/reorder.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, name, got) =>
    test(name, () => assert(cond, got !== undefined ? `got ${JSON.stringify(got)}` : ''));
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// --- moving --------------------------------------------------------
ok(eq(movePage([0, 1, 2, 3], 0, 2), [1, 2, 0, 3]), 'moving forward shifts the pages between');
ok(eq(movePage([0, 1, 2, 3], 3, 0), [3, 0, 1, 2]), 'moving to the front works');
ok(eq(movePage([0, 1, 2], 1, 1), [0, 1, 2]), 'a no-move returns the order unchanged');
ok(movePage([0, 1], 0, 5) === (m => m)([0, 1]) || eq(movePage([0, 1], 0, 5), [0, 1]),
    'an out-of-range target is refused, not clamped mid-drag');

// --- removing ------------------------------------------------------
ok(eq(removePage([0, 1, 2], 1), [0, 2]), 'removing keeps the rest in order');
ok(removePage([0], 0) === null, 'the last page cannot be removed — a 0-page PDF is not a document');
ok(removePage([0, 1], 5) === null, 'an out-of-range removal is refused');

// --- identity ------------------------------------------------------
ok(isIdentity([0, 1, 2]), 'untouched order is identity');
ok(!isIdentity([1, 0, 2]), 'a swap is not identity');
ok(!isIdentity([0, 2]), 'a removal is not identity — the length may match original indices but positions do not');

// --- changed-ness --------------------------------------------------
ok(!orderChanged([0, 1, 2], 3), 'untouched 3-page order is unchanged');
ok(orderChanged([1, 0, 2], 3), 'a swap is a change');
// The trap: removing the LAST page leaves an identity permutation of
// the wrong length. isIdentity alone would call it unchanged and the
// deletion would be silently dropped at save.
ok(orderChanged([0, 1], 3), 'removing the last page IS a change even though [0,1] is identity');

// --- qpdf spec -----------------------------------------------------
ok(qpdfPageSpec([0, 1, 2]) === '1-3', 'consecutive pages compress to one range');
ok(qpdfPageSpec([2, 0, 1]) === '3,1-2', 'a moved page splits the ranges', qpdfPageSpec([2, 0, 1]));
ok(qpdfPageSpec([4]) === '5', 'a single page is 1-based');
ok(qpdfPageSpec([0, 2, 4]) === '1,3,5', 'non-consecutive pages stay separate terms');
ok(qpdfPageSpec([]) === '', 'empty order, empty spec');
// A 400-page document with one page pulled to the front must not become
// 400 comma terms on a command line.
const big = [399, ...Array.from({length: 399}, (_, i) => i)];
ok(qpdfPageSpec(big) === '400,1-399', 'large documents compress', qpdfPageSpec(big));

// --- rotation ------------------------------------------------------
ok(eq(rotatePageBy({}, 2, 90), {2: 90}), 'a first rotation records the page');
ok(eq(rotatePageBy({2: 90}, 2, 90), {2: 180}), 'rotations accumulate');
ok(eq(rotatePageBy({2: 270}, 2, 90), {}),
    'four quarter turns leave NO entry — a document nobody edited must not ask to be saved',
    rotatePageBy({2: 270}, 2, 90));
ok(eq(rotatePageBy({}, 0, -90), {0: 270}), 'a negative turn normalizes into 0..359');
ok(eq(rotatePageBy({1: 90}, 3, 180), {1: 90, 3: 180}), 'pages rotate independently');
// The map is keyed by ORIGINAL page, so it survives a move untouched —
// keying by display position would rotate whatever page landed there.
const rot = rotatePageBy({}, 2, 90);
ok(eq(qpdfRotateArgs(movePage([0, 1, 2], 2, 0), rot), ['--rotate=+90:1']),
    'a rotation follows its page when the page moves',
    qpdfRotateArgs(movePage([0, 1, 2], 2, 0), rot));

ok(rotationOf({2: 90}, 2) === 90, 'rotationOf reads the entry');
ok(rotationOf({}, 2) === 0, 'an unrotated page is 0, not undefined');
ok(rotationOf(undefined, 2) === 0, 'no map at all is 0, not a throw');

ok(!rotationsChanged({}, [0, 1, 2]), 'no rotations, no change');
ok(rotationsChanged({1: 90}, [0, 1, 2]), 'one rotated page is a change');
// A rotation left behind by a REMOVED page cannot rotate anything, so
// it must not keep the document dirty forever.
ok(!rotationsChanged({2: 90}, [0, 1]),
    'a rotation on a page that is no longer in the order is not a change');

// --- qpdf rotate args ----------------------------------------------
ok(eq(qpdfRotateArgs([0, 1, 2], {}), []), 'nothing rotated, no arguments');
ok(eq(qpdfRotateArgs([0, 1, 2], {0: 90, 1: 90, 2: 90}), ['--rotate=+90:1-3']),
    'one angle across every page compresses to a single range',
    qpdfRotateArgs([0, 1, 2], {0: 90, 1: 90, 2: 90}));
ok(eq(qpdfRotateArgs([0, 1, 2], {0: 90, 2: 180}), ['--rotate=+90:1', '--rotate=+180:3']),
    'two angles are two arguments, in ascending angle order',
    qpdfRotateArgs([0, 1, 2], {0: 90, 2: 180}));
// THE trap this function exists for. Device-verified 2026-08-05:
// `qpdf in.pdf --pages . 3,1-2 -- --rotate=+90:1 out.pdf` rotates the
// page that ENDED UP first (input page 3). So the numbers emitted here
// are OUTPUT positions. Reading them as input pages puts every rotation
// on the wrong page the moment a page also moves.
ok(eq(qpdfRotateArgs([2, 0, 1], {2: 90}), ['--rotate=+90:1']),
    'the number is the OUTPUT position — page 2 moved to the front is rotated as page 1',
    qpdfRotateArgs([2, 0, 1], {2: 90}));
ok(eq(qpdfRotateArgs([2, 0, 1], {0: 90}), ['--rotate=+90:2']),
    'original page 0 sitting second is rotated as page 2, not page 1',
    qpdfRotateArgs([2, 0, 1], {0: 90}));

finish('preview/reorder');
