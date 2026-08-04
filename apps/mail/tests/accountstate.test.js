// Honest per-account state (#249), which is a diagnostic before it is a
// preference.
//
// THE FAILURE THIS EXISTS FOR IS SILENT. A person connects Google in
// Settings, opens Mail, and sees nothing. Every layer is working exactly
// as designed and none is in a position to say so: GOA holds an account,
// `lisa mail setup` has not run so mbsync knows nothing about it, the
// Maildir is empty because nothing fills it, and Mail is correct to show
// an empty folder. The failure lives in the gaps, which is where no
// component is looking.
//
// So each row must name the layer that is blocking, and offer the one
// action that unblocks it. Everything here is pure — the facts are
// gathered elsewhere.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {accountState, accountStates} from '../lib/accountstate.js';

const GOA = {identity: 'flakerimi@basecode.al', provider: 'Google', imapUser: 'flakerimi@basecode.al'};

test('connected but never set up names mbsync, and offers to run it', () => {
    // This is the reported case, and the whole reason for the group.
    const row = accountState({goa: GOA, configured: false, root: null, messages: 0});
    assertEq(row.state, 'never-set-up');
    assert(row.detail.includes('Set up sync'), row.detail);
    assertEq(row.action, 'setup');
    assert(!row.ok, 'a connected account with no sync config is not ok');
});

test('set up but never synced names sync, not setup', () => {
    // The distinction matters: telling someone to run setup again when
    // setup already ran is how a person loops.
    const row = accountState({goa: GOA, configured: true, root: '/home/lisa/Mail/x', messages: 0});
    assertEq(row.state, 'never-synced');
    assertEq(row.action, 'sync');
    assert(!row.ok);
});

test('synced says so, with the count', () => {
    const row = accountState({goa: GOA, configured: true, root: '/home/lisa/Mail/x', messages: 8407});
    assertEq(row.state, 'synced');
    assert(row.detail.includes('8,407'), row.detail);
    assertEq(row.action, null, 'nothing to fix, so no button');
    assert(row.ok);
});

test('Mail switched off in Online Accounts is not a Lisa problem', () => {
    // Offering "Set up sync" here would run mbsync against an account
    // whose owner has told GOA not to fetch mail. The fix is in Settings.
    const row = accountState({
        goa: {...GOA, mailDisabled: true}, configured: false, root: null, messages: 0,
    });
    assertEq(row.state, 'mail-off');
    assertEq(row.action, 'online-accounts');
    assert(!row.ok);
});

test('a Maildir with no account behind it is named, not hidden', () => {
    // The orphan tree on the reference device: ~/Mail/{INBOX,Sent,…},
    // 9,125 messages, no mbsync channel pointing at it. It must not be
    // reported as a working account, and it must not be silently
    // dropped either — it is somebody's mail (#224).
    const row = accountState({goa: null, configured: false, root: '/home/lisa/Mail', messages: 9125});
    assertEq(row.state, 'orphaned');
    assert(row.detail.includes('nothing keeps it up to date'), row.detail);
    assertEq(row.action, null, 'removing somebody\'s mail is not a button');
    assert(!row.ok);
});

test('every account gets exactly one row, and the order is the caller\'s', () => {
    const rows = accountStates([
        {goa: GOA, configured: true, root: '/a', messages: 10},
        {goa: {...GOA, identity: 'b@x.test'}, configured: false, root: null, messages: 0},
        {goa: null, configured: false, root: '/home/lisa/Mail', messages: 9125},
    ]);
    assertEq(rows.length, 3);
    assertEq(rows.map((r) => r.state).join(','), 'synced,never-set-up,orphaned');
    assertEq(rows[0].title, 'flakerimi@basecode.al');
    // An orphan has no identity to show, so it is titled by where it is.
    assertEq(rows[2].title, '/home/lisa/Mail');
});

test('nothing at all is not a row saying nothing', () => {
    assertEq(accountStates([]).length, 0);
    assertEq(accountStates(null).length, 0);
});

finish('mail/accountstate');
