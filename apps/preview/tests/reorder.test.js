import {movePage, removePage, isIdentity, orderChanged, qpdfPageSpec} from '../lib/reorder.js';

let passed = 0, failed = 0;
function ok(cond, name, got) {
    if (cond) { passed++; console.log(`  ok    ${name}`); }
    else { failed++; console.log(`  FAIL  ${name}${got !== undefined ? ` (got ${JSON.stringify(got)})` : ''}`); }
}
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

console.log(`preview/reorder: ${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
