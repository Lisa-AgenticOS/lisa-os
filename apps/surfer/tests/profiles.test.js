// Browsing profiles, and which one an agent gets (#259, #181).
//
// The load-bearing one is the AGENT profile, and it is a security
// property rather than a convenience. Today a `navigate`/`click`/`fill`
// driven by the model runs inside the session where the person is
// logged into everything — so a prompt-injected page can ask the model
// to act as them. With an agent profile the worst case is a logged-out
// browser.
//
// #181's wording is "never as the user by default", and the whole point
// of putting this in a tested module is that the DEFAULT is the
// security boundary. A default that is right only when someone
// remembers to pass an argument is not a boundary.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    AGENT_PROFILE, DEFAULT_PROFILE, dataDirFor, isReserved,
    profileForAgent, profileNamesFrom, validProfileName,
} from '../lib/profiles.js';

test('an agent gets the agent profile, and nothing else has to be right', () => {
    // The security default. Passing nothing, passing junk, or passing a
    // profile that does not exist all land in the agent profile — never
    // in the person's session.
    assertEq(profileForAgent(), AGENT_PROFILE);
    assertEq(profileForAgent(null), AGENT_PROFILE);
    assertEq(profileForAgent({}), AGENT_PROFILE);
    assertEq(profileForAgent({handedTab: false}), AGENT_PROFILE);
    assertEq(profileForAgent({profile: 'personal'}), AGENT_PROFILE,
        'ASKING for the user profile is not being handed a tab');
});

test('only a tab the person handed over escapes the agent profile', () => {
    // #181's exception, and the only one: the person deliberately gave
    // this tab to the agent, so it acts in whatever session that tab is
    // already in. The tab's profile comes from the tab, never from the
    // request — a request field would be the model choosing its own
    // session, which is the whole thing being prevented.
    assertEq(profileForAgent({handedTab: true, tabProfile: 'personal'}), 'personal');
    assertEq(profileForAgent({handedTab: true, tabProfile: 'work'}), 'work');
    // Handed a tab with no profile on it: fail closed, not open.
    assertEq(profileForAgent({handedTab: true}), AGENT_PROFILE);
    assertEq(profileForAgent({handedTab: true, tabProfile: ''}), AGENT_PROFILE);
    // `handedTab` must be a real boolean — a truthy string is what a
    // JSON payload from a tool call looks like.
    assertEq(profileForAgent({handedTab: 'yes', tabProfile: 'personal'}), AGENT_PROFILE);
});

test('the agent profile cannot be created, renamed or deleted as an ordinary one', () => {
    // If a person can delete it, an agent that asks them to delete it
    // can get its session back. It is reserved, like the default.
    assert(isReserved(AGENT_PROFILE));
    assert(isReserved(DEFAULT_PROFILE));
    assert(!isReserved('work'));
    assert(!validProfileName(AGENT_PROFILE), 'reserved names are not available');
    assert(!validProfileName(DEFAULT_PROFILE));
});

test('a profile name has to be safe as a directory component', () => {
    // The name becomes a path under the data dir. A name that can
    // escape it is a profile that reads somebody else's cookies.
    assert(validProfileName('work'));
    assert(validProfileName('Work Laptop'));
    assert(validProfileName('client-a_2026'));
    assert(!validProfileName('..'));
    assert(!validProfileName('../../etc'));
    assert(!validProfileName('a/b'));
    assert(!validProfileName(''));
    assert(!validProfileName('   '));
    assert(!validProfileName(null));
    assert(!validProfileName('.hidden'), 'a leading dot hides it from its own manager');
    assert(!validProfileName('x'.repeat(200)), 'a name longer than a filename is not one');
});

test('each profile gets its own data directory, and they never collide', () => {
    const base = '/home/me/.local/share/lisa-surfer';
    assertEq(dataDirFor(DEFAULT_PROFILE, base), base,
        'the default profile keeps the path it already had — nobody loses a login on upgrade');
    assertEq(dataDirFor(AGENT_PROFILE, base), `${base}/profiles/agent`);
    assertEq(dataDirFor('work', base), `${base}/profiles/work`);
    // A space is legal in a name and must survive into the path
    // untouched rather than being silently collapsed into another
    // profile's directory.
    assert(dataDirFor('Work Laptop', base) !== dataDirFor('WorkLaptop', base));
});

test('an unsafe name never reaches a path', () => {
    const base = '/home/me/.local/share/lisa-surfer';
    assertEq(dataDirFor('../../etc', base), null);
    assertEq(dataDirFor('', base), null);
    assertEq(dataDirFor(null, base), null);
});

test('the profile list always contains the two reserved ones, in order', () => {
    // Whatever is on disk, the person always has their own session and
    // the agent always has one to be confined to. A config that lists
    // neither must not produce a browser with no profile at all.
    assertEq(profileNamesFrom(null).join(','), `${DEFAULT_PROFILE},${AGENT_PROFILE}`);
    assertEq(profileNamesFrom([]).join(','), `${DEFAULT_PROFILE},${AGENT_PROFILE}`);
    assertEq(profileNamesFrom(['work']).join(','),
        `${DEFAULT_PROFILE},${AGENT_PROFILE},work`);
    // Duplicates and unsafe names are dropped rather than rejecting the
    // whole list — one bad entry must not cost somebody every profile.
    assertEq(profileNamesFrom(['work', 'work', '../etc', 'personal']).join(','),
        `${DEFAULT_PROFILE},${AGENT_PROFILE},work`);
});

finish('surfer/profiles');
