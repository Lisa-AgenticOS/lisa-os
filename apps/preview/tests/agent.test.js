// Argument validation for Preview's write- and destructive-tier tools.
//
// These are the refusals a model meets. Every one of them exists because
// the alternative is worse than an error message: an invented page
// number reaching poppler as `get_page(undefined)` (#198), a rotation
// qpdf rejects after the user already confirmed the modal, or — the one
// that matters — an export quietly replacing a file.
import {pageArg, rotationArg, moveArg, exportTarget, formatArg} from '../lib/agent.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, name, got) =>
    test(name, () => assert(cond, got !== undefined ? `got ${JSON.stringify(got)}` : ''));
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// --- pageArg -------------------------------------------------------
ok(pageArg(1, 3).value === 0, '1-based in, 0-based out');
ok(pageArg(3, 3).value === 2, 'the last page is in range');
ok(pageArg(undefined, 3).value === 0, 'an omitted page means the first');
ok(pageArg(0, 3).error !== undefined, 'page 0 does not exist');
ok(pageArg(4, 3).error !== undefined, 'past the end is refused');
ok(pageArg(1.5, 3).error !== undefined, 'a fractional page is refused, not floored');
// A string that JSON.parse did not turn into a number is a caller
// sending something else's page number. `"2" - 1` would have been 1.
ok(pageArg('2', 3).error !== undefined, 'a numeric STRING is refused, not coerced');
ok(pageArg(true, 3).error !== undefined, 'a boolean is refused (Number(true) is 1)');
ok(pageArg(NaN, 3).error !== undefined, 'NaN is refused');
ok(pageArg(1, 0).error === 'no document open', 'no pages means no document, not page 0');
ok(pageArg(5, 3, 'from').error.startsWith('from must be'),
    'the argument name appears in its own error', pageArg(5, 3, 'from').error);

// --- rotationArg ---------------------------------------------------
ok(rotationArg(90).value === 90, 'a quarter turn passes');
ok(rotationArg(undefined).value === 90, 'the default is a clockwise quarter turn');
ok(rotationArg(-90).value === 270, 'a negative turn normalizes');
ok(rotationArg(450).value === 90, 'more than a full turn normalizes');
ok(rotationArg(45).error !== undefined, '45 degrees is not a PDF rotation');
ok(rotationArg(91).error !== undefined, 'nearly-90 is not 90');
// Reporting success for a call that changed nothing is how a model
// concludes it rotated a page it did not rotate.
ok(rotationArg(0).error !== undefined, 'a zero turn is refused, not silently accepted');
ok(rotationArg(360).error !== undefined, 'a whole turn is refused for the same reason');
ok(rotationArg('90').error !== undefined, 'a string rotation is refused');

// --- moveArg -------------------------------------------------------
ok(eq(moveArg(1, 3, 3), {from: 0, to: 2}), 'both ends convert to 0-based');
ok(moveArg(1, 1, 3).error !== undefined, 'moving a page onto itself is refused');
ok(moveArg(0, 2, 3).error.startsWith('from'), 'a bad source names `from`');
ok(moveArg(1, 9, 3).error.startsWith('to'), 'a bad target names `to`');

// --- exportTarget --------------------------------------------------
ok(exportTarget('/home/u/a.png', 'png', false).value === '/home/u/a.png', 'a fresh path passes');
ok(exportTarget('a.png', 'png', false).error !== undefined, 'a relative path is refused');
ok(exportTarget('/home/u/../etc/a.png', 'png', false).error !== undefined,
    'a .. segment is refused rather than normalized behind the guard\'s back');
ok(exportTarget('/home/u/a.jpg', 'png', false).error !== undefined,
    'the extension must match the format asked for');
ok(exportTarget('/home/u/a', 'png', false).error !== undefined, 'no extension at all is refused');
ok(exportTarget('/home/u/A.PNG', 'png', false).value !== undefined,
    'the extension check is case-insensitive');
ok(exportTarget('', 'png', false).error !== undefined, 'an empty path is refused');
ok(exportTarget(null, 'png', false).error !== undefined, 'a null path is refused');
// THE one. A write-tier tool that can clobber a file is a tier that
// lies, so the tool is made unable to do it.
ok(exportTarget('/home/u/a.png', 'png', true).error !== undefined,
    'an EXISTING path is refused — Preview never overwrites');
ok(/already exists/.test(exportTarget('/home/u/a.png', 'png', true).error),
    'and the refusal says why, so the model can pick another name',
    exportTarget('/home/u/a.png', 'png', true).error);

// --- formatArg -----------------------------------------------------
const avail = [{key: 'png', label: 'PNG', ext: 'png'}, {key: 'jpeg', label: 'JPEG', ext: 'jpg'}];
ok(formatArg('png', avail).value.ext === 'png', 'an available format resolves to its entry');
// #146's lesson, at the tool boundary: what the MACHINE can write, not
// what the catalogue wishes it could.
ok(formatArg('avif', avail).error !== undefined,
    'a format this machine cannot write is refused, not attempted');
ok(/png, jpeg/.test(formatArg('avif', avail).error),
    'and the refusal lists what it CAN write', formatArg('avif', avail).error);
ok(formatArg(undefined, avail).error !== undefined, 'a missing format is refused');
ok(formatArg('png', []).error !== undefined, 'no writers at all refuses everything');

finish('preview/agent');
