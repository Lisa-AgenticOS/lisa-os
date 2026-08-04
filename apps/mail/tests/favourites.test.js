// The curated sidebar (#249): which of an account's folders show.
//
// "The sidebar should be a curated subset, not a rendering of the disk."
// Spark's real folder list includes `beanstalk`, `Boomerang-Outbox`,
// `Delegated`, `Snoozed` and `Reminders` — folders nobody wants in a
// sidebar, and at eight accounts that matters more than any nesting
// decision. `reloadFolders` renders every directory the Maildir
// contains, which is the disk, not a decision.
//
// The rule this suite is really pinning: **an unset account shows
// everything.** A curation feature whose default is "show nothing until
// you configure it" turns a working app into an empty one on upgrade,
// and nobody would find the setting that did it.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    favouritesFor, isFavourite, toggleFavourite, visibleFolders,
} from '../lib/favourites.js';

const ROOT = '/home/lisa/Mail/flakerimi_at_basecode.al';
const OTHER = '/home/lisa/Mail/apple_at_example.test';

// What an eight-account Maildir really looks like once a few years of
// server-side rules have run.
const ON_DISK = [
    'INBOX', 'Sent', 'Drafts', 'Archive', 'Spam', 'Trash',
    'beanstalk', 'Boomerang-Outbox', 'Delegated', 'Snoozed', 'Reminders',
];

test('an account nobody has curated shows every folder', () => {
    // The upgrade path. Shipping this with an empty default would empty
    // every existing sidebar and look exactly like data loss.
    assertEq(visibleFolders(ON_DISK, {}, ROOT).join(','), ON_DISK.join(','));
    assertEq(visibleFolders(ON_DISK, null, ROOT).join(','), ON_DISK.join(','));
});

test('a curated account shows its favourites, in the on-disk order', () => {
    const config = {favouriteFolders: {[ROOT]: ['Archive', 'INBOX', 'Sent']}};
    // Order comes from the caller's list (INBOX, Sent, Drafts… then the
    // rest alphabetically), not from the order they were ticked — a
    // sidebar that reorders itself as you configure it is disorienting.
    assertEq(visibleFolders(ON_DISK, config, ROOT).join(','), 'INBOX,Sent,Archive');
});

test('curation is per account', () => {
    const config = {favouriteFolders: {[ROOT]: ['INBOX']}};
    assertEq(visibleFolders(ON_DISK, config, ROOT).join(','), 'INBOX');
    // The account nobody curated is untouched by the one somebody did.
    assertEq(visibleFolders(ON_DISK, config, OTHER).join(','), ON_DISK.join(','));
});

test('a favourite that is no longer on disk simply does not show', () => {
    // Server-side rules delete folders. A stale favourite must not
    // resurrect a row pointing at a directory that is gone.
    const config = {favouriteFolders: {[ROOT]: ['INBOX', 'Delegated', 'GoneAway']}};
    assertEq(visibleFolders(ON_DISK, config, ROOT).join(','), 'INBOX,Delegated');
});

test('curating every folder away still shows the inbox', () => {
    // A mail app with no folders is a broken mail app, and a settings
    // page that can produce one is a trap. INBOX is the floor.
    const config = {favouriteFolders: {[ROOT]: []}};
    assertEq(visibleFolders(ON_DISK, config, ROOT).join(','), 'INBOX');
    // …and if there is no INBOX on disk either, everything comes back
    // rather than nothing: an empty sidebar is never the right answer.
    assertEq(visibleFolders(['Archive', 'Sent'], config, ROOT).join(','), 'Archive,Sent');
});

test('toggling writes only that account, and never mutates the input', () => {
    const before = {favouriteFolders: {[OTHER]: ['INBOX']}};
    const frozen = JSON.stringify(before);
    const after = toggleFavourite(before, ROOT, 'Archive', ON_DISK);
    assertEq(JSON.stringify(before), frozen, 'the caller\'s config is untouched');
    assertEq(after.favouriteFolders[OTHER].join(','), 'INBOX', 'the other account is untouched');
    // First toggle on an uncurated account starts from "everything is a
    // favourite" and removes one — not from nothing and adds one, which
    // would collapse the sidebar to a single folder on the first click.
    assert(!after.favouriteFolders[ROOT].includes('Archive'), 'Archive came off');
    assert(after.favouriteFolders[ROOT].includes('INBOX'), 'the rest stayed');
    assertEq(after.favouriteFolders[ROOT].length, ON_DISK.length - 1);
});

test('toggling back restores it', () => {
    let config = toggleFavourite({}, ROOT, 'Snoozed', ON_DISK);
    assert(!isFavourite(config, ROOT, 'Snoozed'));
    config = toggleFavourite(config, ROOT, 'Snoozed', ON_DISK);
    assert(isFavourite(config, ROOT, 'Snoozed'));
    assertEq(favouritesFor(config, ROOT, ON_DISK).length, ON_DISK.length);
});

test('an uncurated account reports every folder as a favourite', () => {
    // The checkbox column has to render ticked for an account nobody has
    // touched, because that is what the sidebar is actually doing.
    for (const f of ON_DISK)
        assert(isFavourite({}, ROOT, f), `${f} ticked by default`);
});

finish('mail/favourites');
