// The id↔filename round trip, with a REAL synced filename.
//
// This suite exists because the previous fixtures were synthetic and
// alphanumeric, which made the sanitiser a no-op and hid the fact that
// `search_mail` handed out ids `read_message` rejected on every real
// maildir (#167).
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {messageId, uniqueMatchesId, messagePath, parseFilename} from '../lib/maildir.js';

// Exactly what mbsync wrote on the reference device, 2026-08-02:
//   ~/Mail/INBOX/cur/1785529483.3297_1.lisa,U=8407:2,PS
const REAL = '1785529483.3297_1.lisa,U=8407:2,PS';
const REAL_UNIQUE = '1785529483.3297_1.lisa,U=8407';

test('a real mbsync filename parses into unique + flags', () => {
    const meta = parseFilename(REAL, 'cur');
    assertEq(meta.unique, REAL_UNIQUE);
    assert(meta.seen, 'S is set');
});

test('the id an agent receives round-trips back to the file it came from', () => {
    // The bug: messageId sanitises `,` and `=` to `_`, and the lookup
    // compared that against the raw name. Nothing matched.
    const id = messageId('INBOX', REAL_UNIQUE);
    assertEq(id, 'INBOX/1785529483.3297_1.lisa_U_8407');
    const idUnique = id.slice(id.indexOf('/') + 1);
    assert(uniqueMatchesId(REAL_UNIQUE, idUnique),
        'the on-disk unique part must match the id it produced');
});

test('a plain alphanumeric name still matches — the case that hid the bug', () => {
    assert(uniqueMatchesId('123.abc', '123.abc'));
});

test('the RAW unique part matches too — the window passes that one (#210)', () => {
    // The other caller, and the one nobody wrote a test for. The window's
    // list rows carry `unique` straight off the disk (listFolder keeps
    // `parseFilename`'s output verbatim), so `showMessage` asks for
    // `...lisa,U=8407` while the tools ask for `...lisa_U_8407`.
    //
    // Sanitising both sides fixed the tool path (#167) and broke this
    // one: every GUI lookup returned null, `showMessage` fell back to
    // the list row, and the row has no body — so EVERY message opened
    // to an empty reading pane on the reference device. Both spellings
    // name the same file and both must match.
    assert(uniqueMatchesId(REAL_UNIQUE, REAL_UNIQUE),
        'the window hands back the raw unique part it read off the disk');
});

test('a different message does not match', () => {
    assert(!uniqueMatchesId('1785529483.3297_1.lisa,U=9999', '1785529483.3297_1.lisa_U_8407'));
    assert(!uniqueMatchesId('', 'anything'));
    assert(!uniqueMatchesId('something', ''));
});

test('messagePath still refuses traversal after all this', () => {
    // The sanitised id is what a model hands back; the path builder is
    // the thing standing between it and the disk.
    assertEq(messagePath('/root', '..', 'cur', 'x'), null);
    assertEq(messagePath('/root', 'INBOX', 'cur', '../../etc/passwd'), null);
    assertEq(messagePath('/root', 'INBOX', 'tmp', 'x'), null, 'only cur and new');
    assert(messagePath('/root', 'INBOX', 'cur', REAL), 'a real filename is allowed');
});

finish('mail/maildir');
