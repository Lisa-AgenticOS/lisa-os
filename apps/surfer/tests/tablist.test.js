import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {rowLabel} from '../lib/tablist.js';

test('a titled page shows its title', () => {
    assertEq(rowLabel('Lisa OS — developer portal', 'https://lisaos.dev/'), 'Lisa OS — developer portal');
});

test('an untitled page shows its host, never the raw URL', () => {
    assertEq(rowLabel('', 'https://example.org/some/long/path?q=1'), 'example.org');
});

test('no title and no parsable host is a New Tab', () => {
    assertEq(rowLabel('', ''), 'New Tab');
    assertEq(rowLabel('  ', 'about:blank'), 'New Tab');
});

test('long titles truncate with an ellipsis at the limit', () => {
    const long = 'x'.repeat(60);
    const out = rowLabel(long, '');
    assertEq(out.length, 40);
    assertEq(out.at(-1), '…');
});

finish();
