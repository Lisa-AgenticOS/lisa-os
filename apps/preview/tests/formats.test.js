// Runs under gjs, node or jsc — see `just shell-test`.
import {kindOf, siblings, MIME_TYPES} from '../lib/formats.js';

let pass = 0, fail = 0;
function ok(cond, what) {
    if (cond) { pass++; console.log(`  ok    ${what}`); }
    else { fail++; console.log(`  FAIL  ${what}`); }
}
function eq(a, b, what) { ok(JSON.stringify(a) === JSON.stringify(b), `${what} (got ${JSON.stringify(a)})`); }

console.log('preview/formats');

eq(kindOf('/home/lisa/cat.webp'), 'image', 'webp is an image');
eq(kindOf('/home/lisa/scan.PDF'), 'document', 'extension match is case-insensitive');
eq(kindOf('/home/lisa/photo.HEIC'), 'image', 'heic is an image');
eq(kindOf('/home/lisa/notes.txt'), 'text', 'txt is text since the peek slice');
eq(kindOf('/home/lisa/index.html'), 'html', 'html renders through WebKit');
eq(kindOf('/home/lisa/archive.zip'), null, 'an unclaimed type is null, not a guess — the card handles it');
eq(kindOf('/home/lisa/archive.tar.gz'), null, 'only the last extension counts');

// A grey rectangle where a file should be is a bug report about the
// user's file; null is a bug report about us, which is the honest one.
eq(kindOf(''), null, 'the empty path is not an image');
eq(kindOf(null), null, 'a non-string is not an image');
eq(kindOf('/home/lisa/.webp'), null, 'a dotfile named .webp has no extension');
eq(kindOf('/home/lisa/webp'), null, 'a bare name with no dot is not an image');

// Folder browsing: sorted, numeric-aware, and the index must point at
// the file that was actually opened.
const {files, index} = siblings('/p/img10.png',
    ['img2.png', 'img10.png', 'archive.zip', 'img1.png']);
eq(files, ['/p/img1.png', '/p/img2.png', '/p/img10.png'], 'siblings drop unopenable files and sort numerically');
eq(index, 2, 'the opened file keeps its place in the sorted list');

const missing = siblings('/p/gone.png', ['a.png']);
eq(missing.index, -1, 'a file that is not in the listing reports -1, not 0');

// The .desktop MIME list is generated from the same source, so a viewer
// can never claim a type it cannot open.
ok(MIME_TYPES.includes('image/jpeg'), 'jpg maps to image/jpeg, not image/jpg');
ok(MIME_TYPES.includes('image/svg+xml'), 'svg maps to image/svg+xml');
ok(MIME_TYPES.includes('application/pdf'), 'pdf is claimed');
ok(!MIME_TYPES.includes('image/jpg'), 'the invented image/jpg type is not claimed');
// .jpg/.jpeg/.jpe/.jfif all map to image/jpeg — the list must collapse
// them, or the .desktop claims the same type four times.
eq(MIME_TYPES.length, new Set(MIME_TYPES).size, 'the mime list has no duplicates');

console.log(`preview/formats: ${pass} passed, ${fail} failed`);
if (fail) throw new Error(`${fail} failed`);
