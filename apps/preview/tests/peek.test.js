import {looksBinary, truncateText, humanSize, cardSubtitle, folderSubtitle, mediaClock}
    from '../lib/peek.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, name, got) =>
    test(name, () => assert(cond, got !== undefined ? `got ${JSON.stringify(got)}` : ''));

ok(!looksBinary(new TextEncoder().encode('plain text\nwith lines')), 'plain text is not binary');
ok(looksBinary(new Uint8Array([0x1f, 0x8b, 0x00, 0x41])), 'a NUL in the head means binary');
ok(!looksBinary(new Uint8Array(0)), 'an empty file is not binary');
const tail = new Uint8Array(9000).fill(65); tail[8500] = 0;
ok(!looksBinary(tail), 'a NUL past the probe window does not flip the verdict — the head decides');

const short = truncateText('abc');
ok(short.text === 'abc' && !short.truncated, 'short text passes through');
const long = truncateText('x'.repeat(50), 10);
ok(long.text.length === 10 && long.truncated, 'long text is cut AND says so');
ok(truncateText(null).text === '' && !truncateText(null).truncated, 'garbage in, empty out');

ok(humanSize(512) === '512 B', 'bytes stay integral', humanSize(512));
ok(humanSize(2048) === '2.0 KiB', 'KiB with one decimal', humanSize(2048));
ok(humanSize(5 * 1024 * 1024) === '5.0 MiB', 'MiB', humanSize(5 * 1024 * 1024));
ok(humanSize(-1) === '' && humanSize(NaN) === '', 'invalid sizes render as nothing');

ok(cardSubtitle('PNG image', 2048) === 'PNG image · 2.0 KiB', 'type and size join with a dot');
ok(cardSubtitle('', 2048) === '2.0 KiB', 'missing type drops cleanly');
ok(cardSubtitle('Folder', NaN) === 'Folder', 'missing size drops cleanly');

ok(folderSubtitle(12, false, 3.5 * 1024 * 1024) === '12 items · 3.5 MiB', 'folder card: items and size', folderSubtitle(12, false, 3.5 * 1024 * 1024));
ok(folderSubtitle(1, false, 100) === '1 item · 100 B', 'singular item');
ok(folderSubtitle(1000, true, 999999) === '1000+ items',
    'a capped count is a floor and drops the size — a partial sum presented as THE size is a lie');
ok(folderSubtitle(0, false, NaN) === '0 items', 'empty folder, no size');

ok(mediaClock(83 * 1e6) === '1:23', 'seconds render as m:ss', mediaClock(83 * 1e6));
ok(mediaClock(3723 * 1e6) === '1:02:03', 'hours render as h:mm:ss', mediaClock(3723 * 1e6));
ok(mediaClock(0) === '' && mediaClock(NaN) === '', 'unknown duration renders as nothing');

finish('preview/peek');
