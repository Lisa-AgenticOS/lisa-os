// Write-tier tools resolve to the SAME plans the buttons do (#167).
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {agentMayRead, parseMessageId, planFor} from '../lib/agent-actions.js';

const msg = (over = {}) => ({
    folder: 'INBOX', dir: 'cur', filename: '123.abc:2,S',
    flagged: false, seen: true, ...over,
});
const FOLDERS = ['INBOX', 'Archive', 'Trash'];

test('an id round-trips from search_mail to a tool', () => {
    assertEq(parseMessageId('INBOX/123.abc:2,S'),
        {folder: 'INBOX', unique: '123.abc:2,S'});
});

test('a malformed id is refused rather than guessed at', () => {
    for (const bad of ['', 'INBOX', '/leading', 'trailing/', null, undefined])
        assert(parseMessageId(bad).error, `${JSON.stringify(bad)} should error`);
});

test('a slash in the filename cannot smuggle a path', () => {
    // Split on the FIRST slash only; messagePath then refuses a
    // filename containing one, so ../ never reaches the disk.
    const {folder, unique} = parseMessageId('INBOX/../../etc/passwd');
    assertEq(folder, 'INBOX');
    assert(unique.includes('/'), 'the remainder keeps its slashes for messagePath to refuse');
});

test('pin and unpin resolve to a flag change', () => {
    const plan = planFor('flag_message', msg(), {flagged: true}, FOLDERS);
    assertEq(plan.kind, 'flag');
    assert(plan.change.toName.includes('F'), 'F is added');
});

test('an action that would change nothing says so instead of reporting success', () => {
    // The whole point: a tool that returns ok having done nothing is
    // the failure mode this repo keeps finding.
    const already = planFor('flag_message', msg({flagged: true, filename: '123.abc:2,FS'}),
        {flagged: true}, FOLDERS);
    assert(already.noop, 'already-pinned is a noop with a reason');
    assert(!already.kind, 'and carries no plan to perform');
});

test('archive and trash move to their folders', () => {
    assertEq(planFor('archive_message', msg(), {}, FOLDERS).toFolder, 'Archive');
    assertEq(planFor('trash_message', msg(), {}, FOLDERS).toFolder, 'Trash');
});

test('a folder that does not exist is an error, not a folder we create', () => {
    // Inventing a folder on a synced Maildir makes one the server has
    // never heard of, and the next sync either deletes it or spreads it.
    assert(planFor('move_message', msg(), {folder: 'Invented'}, FOLDERS).error);
    assert(planFor('archive_message', msg(), {}, ['INBOX']).error, 'no Archive folder here');
});

test('a message already in the target folder is a noop, not a move', () => {
    assert(planFor('trash_message', msg({folder: 'Trash'}), {}, FOLDERS).noop);
});

test('an unknown tool name errors', () => {
    assert(planFor('delete_everything', msg(), {}, FOLDERS).error);
});

test('a missing message errors rather than throwing', () => {
    assert(planFor('archive_message', null, {}, FOLDERS).error);
});

test('the folders contextd refuses to index are not served to an agent either (#237)', () => {
    // #185 decided Spam is "a corpus that exists to be hostile" and
    // Trash is what the user already chose to forget, so `lisa mail
    // index` skips both (cli/lisa/src/mail.rs, UNINDEXED_FOLDERS). The
    // Agent Bus tools walked every folder the store had, which handed a
    // model the exact corpus the retrieval path declines — through a
    // second door, with no index in between.
    for (const folder of ['INBOX', 'Sent', 'Drafts', 'Archive', 'Newsletters'])
        assert(agentMayRead(folder), `${folder} is ordinary mail`);
    for (const folder of ['Spam', 'Junk', 'Trash', 'spam', 'JUNK', 'trash'])
        assert(!agentMayRead(folder), `${folder} must not be served to a model`);
    // The same spellings a synced account uses for them.
    assert(!agentMayRead('[Gmail]/Spam'), 'nested is still spam');
    assert(!agentMayRead('acct/Junk/cur'), 'any component decides it');
    // Nothing is a folder name, and a folder nobody named is not one
    // of these.
    assert(agentMayRead(''), 'an empty name is refused elsewhere, not here');
});

finish('mail/agent-actions');
