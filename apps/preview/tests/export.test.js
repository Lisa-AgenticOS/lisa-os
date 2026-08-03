import {EXPORT_FORMATS, exportFormats, saveOptions, exportName, rasterScale, pageExportNames}
    from '../lib/export.js';

let passed = 0, failed = 0;
function ok(cond, name, got) {
    if (cond) { passed++; console.log(`  ok    ${name}`); }
    else { failed++; console.log(`  FAIL  ${name}${got !== undefined ? ` (got ${JSON.stringify(got)})` : ''}`); }
}

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

console.log(`preview/export: ${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
