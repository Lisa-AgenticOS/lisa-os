import {normalizeRect, isClick, viewToPage, annotRect, savePathFor, unsavedLabel, COLORS}
    from '../lib/annotate.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, name, got) =>
    test(name, () => assert(cond, got !== undefined ? `got ${JSON.stringify(got)}` : ''));
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// --- rects ---------------------------------------------------------
ok(eq(normalizeRect({x: 10, y: 20}, {x: 5, y: 40}), {x1: 5, y1: 20, x2: 10, y2: 40}),
    'a drag up-and-left normalizes to x1<x2, y1<y2');
ok(eq(normalizeRect({x: 1, y: 2}, {x: 3, y: 4}), {x1: 1, y1: 2, x2: 3, y2: 4}),
    'an already-ordered drag passes through');

ok(isClick({x: 10, y: 10}, {x: 11, y: 12}), 'a twitch is a click, not a marquee');
ok(!isClick({x: 10, y: 10}, {x: 30, y: 10}), 'a real horizontal drag is not a click');

ok(eq(viewToPage({x: 100, y: 50}, 2), {x: 50, y: 25}), 'view coords divide by the render scale');

// The one crossing between poppler's two coordinate spaces: top-down
// render coords -> bottom-up PDF annotation coords. Getting the flip
// wrong mirrors every highlight to the bottom of the page.
const flipped = annotRect({x1: 10, y1: 20, x2: 110, y2: 40}, 792);
ok(eq(flipped, {x1: 10, y1: 752, x2: 110, y2: 772}),
    'annotRect flips y around the page height', flipped);
ok(flipped.y2 - flipped.y1 === 20, 'the flip preserves the rect height');
const top = annotRect({x1: 0, y1: 0, x2: 10, y2: 10}, 792);
ok(top.y2 === 792, 'a rect at the visual top lands at the PDF-space top (y2 == pageHeight)', top);

// --- save naming ---------------------------------------------------
ok(savePathFor('/d/report.pdf', []) === '/d/report (edited).pdf', 'first save appends (edited)');
ok(savePathFor('/d/report.pdf', ['report (edited).pdf']) === '/d/report (edited 2).pdf',
    'a taken name counts up');
ok(savePathFor('/d/report.pdf', ['report (edited).pdf', 'report (edited 2).pdf'])
    === '/d/report (edited 3).pdf', 'counting continues past 2');
ok(savePathFor('/d/no-extension', []) === '/d/no-extension (edited)',
    'a file with no extension still gets a sane name');
ok(savePathFor('/d/.hidden', []) === '/d/.hidden (edited)',
    'a leading dot is a hidden file, not an extension');

// --- labels --------------------------------------------------------
ok(unsavedLabel(0, false) === '', 'nothing pending, no label');
ok(unsavedLabel(1, false) === '1 annotation — unsaved', 'singular');
ok(unsavedLabel(3, false) === '3 annotations — unsaved', 'plural');
ok(unsavedLabel(2, true) === '2 annotations, pages reordered — unsaved', 'both kinds combine');
ok(unsavedLabel(0, true) === 'pages reordered — unsaved', 'reorder alone');
ok(unsavedLabel(0, false, true) === 'pages rotated — unsaved', 'rotation alone');
ok(unsavedLabel(0, true, true) === 'pages reordered, pages rotated — unsaved',
    'a rotation is named separately from a reorder — they are undone by different gestures',
    unsavedLabel(0, true, true));
ok(unsavedLabel(2, true, true) === '2 annotations, pages reordered, pages rotated — unsaved',
    'all three combine', unsavedLabel(2, true, true));
// The default keeps every existing caller honest: omitting the third
// argument must not start claiming a rotation nobody made.
ok(unsavedLabel(1, false) === '1 annotation — unsaved', 'rotation defaults to none');

// --- colors --------------------------------------------------------
ok(COLORS.highlight.red === 0xffff && COLORS.note.blue === 0xc9c9,
    'colors are 16-bit channels as PopplerColor wants');

finish('preview/annotate');
