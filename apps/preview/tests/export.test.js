import {EXPORT_FORMATS, exportFormats, saveOptions, exportName, rasterScale, pageExportNames,
    rotatedExtent, pageExportRender} from '../lib/export.js';
import {rotatePageBy, movePage} from '../lib/reorder.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, name, got) =>
    test(name, () => assert(cond, got !== undefined ? `got ${JSON.stringify(got)}` : ''));

const onDevice = exportFormats(['bmp', 'ico', 'avif', 'jpeg', 'png', 'tiff', 'webp']);
ok(onDevice.length === 5, 'the device set offers all five worthwhile formats', onDevice.map(f => f.key));
ok(!onDevice.some(f => f.key === 'bmp'), 'bmp is writable but not offered — nobody exports to bmp on purpose');
ok(exportFormats(['png']).length === 1, 'a machine that only writes png only offers png');
ok(exportFormats([]).length === 0, 'no writers, no menu — never a format we cannot deliver');

ok(saveOptions('jpeg')[0][0] === 'quality', 'jpeg gets a quality option');
ok(saveOptions('png')[0].length === 0, 'png takes no options');

ok(exportName('/d/report.pdf', 'png', 3) === 'report — page 3.png', 'page exports carry the page number', exportName('/d/report.pdf', 'png', 3));
ok(exportName('/d/photo.jpg', 'webp') === 'photo.webp', 'image converts swap the extension');
ok(exportName('/d/photo.png', 'png') === 'photo (exported).png',
    'same-format export never suggests the source name — that invites overwriting the original');
ok(exportName('/d/noext', 'png') === 'noext.png', 'extensionless sources still name cleanly');

ok(Math.abs(rasterScale(150) - 150 / 72) < 1e-9, '150 dpi is the default raster scale');
ok(rasterScale(72) === 1, '72 dpi is 1:1 with PDF points');

const names = pageExportNames('/d/doc.pdf', 'png', 3);
ok(names.length === 3 && names[2] === 'doc — page 3.png', 'all-pages export numbers every file', names);

ok(EXPORT_FORMATS.every(f => f.key && f.label && f.ext), 'every format entry is complete');

// --- rotatedExtent -------------------------------------------------
// The view drew the rotation and the exporter did not (#299). One
// function now answers for both, so they cannot disagree again.
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
ok(eq(rotatedExtent(400, 600, 0), {width: 400, height: 600, angle: 0}),
    'no rotation, no swap');
ok(eq(rotatedExtent(400, 600, 90), {width: 600, height: 400, angle: 90}),
    'a quarter turn swaps width and height', rotatedExtent(400, 600, 90));
ok(eq(rotatedExtent(400, 600, 270), {width: 600, height: 400, angle: 270}),
    'three quarters swaps too');
ok(eq(rotatedExtent(400, 600, 180), {width: 400, height: 600, angle: 180}),
    'a half turn keeps the shape but is still a rotation to apply');
ok(rotatedExtent(400, 600, -90).angle === 270, 'a negative angle normalizes');
ok(rotatedExtent(400, 600, 450).angle === 90, 'past a full turn normalizes');
ok(rotatedExtent(400, 600).angle === 0, 'the default is no rotation');

// --- pageExportRender ----------------------------------------------
// THE #299 test. An agent that rotates a page and then exports it must
// not get the unrotated page and an `ok`. The page and the angle are
// one value precisely so that an exporter cannot pick the first without
// the second.
const order = [0, 1, 2];
const rotated = rotatePageBy({}, 1, 90);
ok(pageExportRender(order, rotated, 1).angle === 90,
    'a pending rotation reaches the export render',
    pageExportRender(order, rotated, 1));
ok(pageExportRender(order, rotated, 0).angle === 0,
    'an unrotated page still exports unrotated');
ok(pageExportRender(order, {}, 2).page === 2, 'the ORIGINAL page index is what renders');
ok(pageExportRender(order, rotated, 1).dpi === 150, 'the export dpi travels with it');

// Rotations are keyed by ORIGINAL page, so moving the page must carry
// its rotation to the new display position — the #195 mistake (a
// display index used to fetch an original page) applied to rotation.
const moved = movePage(order, 1, 2);
ok(pageExportRender(moved, rotated, 2).angle === 90,
    'the rotation follows its page when the page is moved',
    pageExportRender(moved, rotated, 2));
ok(pageExportRender(moved, rotated, 1).angle === 0,
    'and does not stay behind on whatever page landed in its old slot');

// Four quarter turns is a document that is genuinely unrotated.
ok(pageExportRender(order, rotatePageBy(rotatePageBy(rotatePageBy(rotated, 1, 90), 1, 90), 1, 90),
    1).angle === 0, 'four quarter turns export unrotated');

finish('preview/export');
