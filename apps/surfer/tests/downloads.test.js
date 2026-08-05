// Downloads (#146 follow-up). Three properties carry weight and each of
// them has been watched go red:
//
//   * a suggested filename cannot become a path,
//   * `destinationFor` never says `save` for a file that exists,
//   * an agent-driven download is refused.
//
// The rest is bookkeeping and is tested because a downloads list that
// silently loses a row is a file the person cannot find again.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    AGENT_DOWNLOAD_WINDOW_MS, agentDriven, clearFinished, completeDownload,
    destinationFor, destinationPath, downloadFraction, downloadLabel,
    failDownload, formatBytes, persistableDownloads, removeDownload,
    resolveConflict, safeFilename, startedDownload, trimDownloads,
    uniquePath, updateDownload,
} from '../lib/downloads.js';

/// A filesystem that is a Set of paths.
const fsWith = (...paths) => {
    const set = new Set(paths);
    return (p) => set.has(p);
};
const emptyFs = () => false;

test('a server-suggested filename never becomes a path', () => {
    // Content-Disposition is attacker-controlled text. Every one of
    // these is a filename as far as the header is concerned.
    assertEq(safeFilename('../../.config/autostart/x.desktop', ''),
        'configautostartx.desktop',
        'separators go, and the leading dots with them');
    assertEq(safeFilename('/etc/passwd', ''), 'etcpasswd');
    assertEq(safeFilename('..\\..\\windows\\system32', ''), 'windowssystem32');
    assertEq(safeFilename('..', ''), 'download', 'a name that is only dots is not a name');
    assertEq(safeFilename('.', ''), 'download');
    assertEq(safeFilename('.bashrc', ''), 'bashrc',
        'a page does not get to write a file the file manager hides');
    // A control character in a name rewrites a terminal listing.
    assertEq(safeFilename('report\r\n.pdf', ''), 'report.pdf');
    // A NUL is the one byte the kernel forbids outright, and it is
    // also how a name gets truncated by whatever reads it next.
    assertEq(safeFilename('a\u0000b.txt', ''), 'ab.txt');
    // A space is legal in a filename and survives: sanitising it
    // would be this module deciding what people may call their files.
    assertEq(safeFilename('quarterly report.pdf', ''), 'quarterly report.pdf');
});

test('the URI fallback is decoded BEFORE it is stripped, not after', () => {
    // %2F decodes to a separator. Decoding after the strip hands back
    // exactly the character that was removed.
    assertEq(safeFilename('', 'https://evil.example/x/%2F..%2Fpasswd'), 'passwd');
    assertEq(safeFilename(null, 'https://example.org/files/report.pdf'), 'report.pdf');
    assertEq(safeFilename('', 'https://example.org/files/report.pdf?v=2#top'), 'report.pdf',
        'a query string is not part of a filename');
    assertEq(safeFilename('', 'https://example.org/'), 'download',
        'a bare host suggests nothing');
});

test('a long name keeps its extension', () => {
    const long = `${'a'.repeat(400)}.tar.gz`;
    const out = safeFilename(long, '');
    assert(out.length <= 120, `still ${out.length} characters`);
    assert(out.endsWith('.gz'), `lost its extension: ${out}`);
});

test('destinationPath refuses to join anything that is not one component', () => {
    assertEq(destinationPath('/home/me/Downloads', 'report.pdf'),
        '/home/me/Downloads/report.pdf');
    assertEq(destinationPath('/home/me/Downloads/', 'report.pdf'),
        '/home/me/Downloads/report.pdf', 'a trailing slash is not a second one');
    for (const bad of ['../x', 'a/b', '..', '.', '', 'a\\b']) {
        let threw = false;
        try { destinationPath('/home/me/Downloads', bad); } catch { threw = true; }
        assert(threw, `joined ${JSON.stringify(bad)} instead of refusing it`);
    }
});

test('a download never silently overwrites a file that is there', () => {
    const dir = '/home/me/Downloads';
    // Nothing in the way: save, at the obvious path.
    const clean = destinationFor({suggested: 'report.pdf', uri: '', dir, exists: emptyFs});
    assertEq(clean.action, 'save');
    assertEq(clean.path, '/home/me/Downloads/report.pdf');

    // Something IS in the way: never `save`, and the person is asked.
    const clash = destinationFor({
        suggested: 'report.pdf', uri: '', dir,
        exists: fsWith('/home/me/Downloads/report.pdf'),
    });
    assertEq(clash.action, 'conflict',
        'a name that is taken must ask, not overwrite');
    assertEq(clash.path, '/home/me/Downloads/report.pdf');
    assertEq(clash.suggestion, '/home/me/Downloads/report (1).pdf');
});

test('the conflict dialog defaults to doing nothing', () => {
    const decision = {
        action: 'conflict',
        path: '/d/report.pdf',
        suggestion: '/d/report (1).pdf',
    };
    assertEq(resolveConflict('keep-both', decision),
        {action: 'save', path: '/d/report (1).pdf', allowOverwrite: false});
    assertEq(resolveConflict('replace', decision),
        {action: 'save', path: '/d/report.pdf', allowOverwrite: true},
        'replacing is allowed — but only because somebody said so');
    // Escape, the close button, a stale value, a typo: all cancel.
    for (const answer of ['cancel', '', null, undefined, 'Replace', 'yes', 0]) {
        assertEq(resolveConflict(answer, decision), {action: 'cancel'},
            `${JSON.stringify(answer)} was treated as an answer`);
    }
});

test('unique names step past every file that is already there', () => {
    const dir = '/d';
    const fs = fsWith('/d/a.txt', '/d/a (1).txt', '/d/a (2).txt');
    assertEq(uniquePath(dir, 'a.txt', fs), '/d/a (3).txt');
    assertEq(uniquePath(dir, 'b.txt', fs), '/d/b.txt');
    // An extensionless name numbers at the end.
    assertEq(uniquePath(dir, 'LICENSE', fsWith('/d/LICENSE')), '/d/LICENSE (1)');
    // A "double extension" longer than 12 chars is not one.
    assertEq(uniquePath(dir, 'x.verylongextension', fsWith('/d/x.verylongextension')),
        '/d/x.verylongextension (1)');
});

test('uniquePath gives up rather than spinning', () => {
    let threw = false;
    try { uniquePath('/d', 'a.txt', () => true); } catch { threw = true; }
    assert(threw, 'a filesystem where every name is taken looped instead of failing');
});

test('an agent cannot cause a write to disk', () => {
    // navigate/click can reach a URL that answers with an attachment, so
    // the browser stamps a view when an agent touches it and refuses any
    // download that starts inside the stamp.
    assert(agentDriven({agentTouchedAt: 1000, now: 1000}), 'same instant');
    assert(agentDriven({agentTouchedAt: 1000, now: 1000 + AGENT_DOWNLOAD_WINDOW_MS - 1}));
    assert(!agentDriven({agentTouchedAt: 1000, now: 1000 + AGENT_DOWNLOAD_WINDOW_MS}),
        'the window closes; a person clicking later is a person');
    // Never touched by an agent.
    assert(!agentDriven({now: 5000}));
    assert(!agentDriven({agentTouchedAt: 0, now: 5000}));
    assert(!agentDriven({agentTouchedAt: null, now: 5000}));
    // Fail closed on nonsense and on a clock that went backwards.
    assert(agentDriven({agentTouchedAt: 5000, now: 1000}), 'clock skew must fail closed');
    assert(agentDriven({agentTouchedAt: 1000, now: NaN}));
    assert(agentDriven({agentTouchedAt: 1000}));
});

test('the list records what happened to each transfer', () => {
    let list = [startedDownload({
        id: 'd1', uri: 'https://example.org/a.zip', filename: 'a.zip',
        path: '/d/a.zip', startedAt: 10,
    })];
    assertEq(list[0].state, 'running');
    list = updateDownload(list, 'd1', {received: 512, total: 2048});
    assertEq(downloadLabel(list[0]), '512 B of 2 KB');
    assertEq(downloadFraction(list[0]), 0.25);
    list = completeDownload(list, 'd1', 99);
    assertEq(list[0].state, 'done');
    assertEq(list[0].endedAt, 99);
    assertEq(downloadFraction(list[0]), 1);

    // A server that sends no length is a pulsing bar, not a stuck one.
    let none = [startedDownload({id: 'd2', uri: '', filename: 'x', path: '/d/x'})];
    none = updateDownload(none, 'd2', {received: 300});
    assertEq(downloadFraction(none[0]), null);
    assertEq(downloadLabel(none[0]), '300 B');
    none = completeDownload(none, 'd2', 1);
    assertEq(downloadLabel(none[0]), '300 B', 'a finished download knows its own size');
});

test('a failure says what went wrong', () => {
    let list = [startedDownload({id: 'd1', uri: '', filename: 'a', path: '/d/a'})];
    list = failDownload(list, 'd1', 'Network error', 42);
    assertEq(list[0].state, 'failed');
    assertEq(downloadLabel(list[0]), 'Failed — Network error');
});

test('clearing the list removes rows and never touches transfers in flight', () => {
    let list = [
        startedDownload({id: 'a', uri: '', filename: 'a', path: '/d/a'}),
        startedDownload({id: 'b', uri: '', filename: 'b', path: '/d/b'}),
    ];
    list = completeDownload(list, 'b', 1);
    const cleared = clearFinished(list);
    assertEq(cleared.length, 1);
    assertEq(cleared[0].id, 'a', 'a running download is not a finished one');
    assertEq(removeDownload(list, 'a').map(e => e.id), ['b']);
    assertEq(removeDownload(list, 'nope').length, 2);
});

test('a running download is written down as interrupted, not as running', () => {
    // Otherwise the next launch shows a progress bar for a transfer that
    // died with the process.
    const list = [startedDownload({id: 'a', uri: '', filename: 'a', path: '/d/a'})];
    const saved = persistableDownloads(list);
    assertEq(saved[0].state, 'failed');
    assertEq(saved[0].reason, 'interrupted');
    // …and the in-memory list is untouched.
    assertEq(list[0].state, 'running');
});

test('the list is bounded', () => {
    const many = Array.from({length: 260}, (_, i) =>
        startedDownload({id: `d${i}`, uri: '', filename: 'x', path: '/d/x'}));
    assertEq(trimDownloads(many).length, 200);
    assertEq(trimDownloads(many)[0].id, 'd0', 'newest first stays newest first');
});

test('sizes read like sizes', () => {
    assertEq(formatBytes(0), '0 B');
    assertEq(formatBytes(999), '999 B');
    assertEq(formatBytes(1024), '1 KB');
    assertEq(formatBytes(1536), '1.5 KB');
    assertEq(formatBytes(1024 * 1024 * 3.25), '3.3 MB');
    assertEq(formatBytes(-5), '0 B');
    assertEq(formatBytes(null), '0 B');
});

finish('surfer/downloads');
