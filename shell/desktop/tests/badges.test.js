// Dock badges from the Unity LauncherEntry convention (#190).
//
// The protocol is not ours and that is the point: every toolkit and
// Electron app already emits `com.canonical.Unity.LauncherEntry.Update`,
// so adopting it badges third-party apps with zero Lisa-specific code.
// Inventing one would have been the same feature with a smaller reach
// (rule 8's spirit — no invented references, and no invented protocols
// where a real one exists).
//
// Everything here is the DECISION, not the drawing: what a payload off
// the bus means, and what it must never be trusted to say. A signal is
// something any session peer can emit, so this is a parser for hostile
// input, not a settings reader.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {BadgeState, badgeFor, desktopIdFromUri, isEmpty} from '../lib/badges.js';

test('an application:// uri yields the desktop id the dock keys on', () => {
    assertEq(desktopIdFromUri('application://app.lisaos.Mail.desktop'),
        'app.lisaos.Mail.desktop');
    // Some emitters omit the scheme; some add a trailing slash.
    assertEq(desktopIdFromUri('app.lisaos.Mail.desktop'), 'app.lisaos.Mail.desktop');
    assertEq(desktopIdFromUri('application://app.lisaos.Mail.desktop/'),
        'app.lisaos.Mail.desktop');
});

test('a uri that is not a desktop id is refused, not guessed at', () => {
    // The dock matches this against real items. A guess that happens to
    // collide with an installed app lets any peer badge somebody else's
    // icon, which is a small lie told with our own UI.
    assertEq(desktopIdFromUri('application://../../etc/passwd'), null);
    assertEq(desktopIdFromUri('application://app.lisaos.Mail'), null, 'no .desktop suffix');
    assertEq(desktopIdFromUri(''), null);
    assertEq(desktopIdFromUri(null), null);
    assertEq(desktopIdFromUri('application://a b.desktop'), null, 'a space is not an id');
});

test('count-visible false means no badge, whatever the count says', () => {
    // The convention's own way of saying "clear it". An emitter that
    // sets count 5 and count-visible false is clearing a badge, and
    // rendering 5 would make the badge impossible to dismiss.
    assertEq(badgeFor({count: 5, 'count-visible': false}).count, null);
    assertEq(badgeFor({count: 5}).count, null, 'absent count-visible is not visible');
    assertEq(badgeFor({count: 5, 'count-visible': true}).count, 5);
});

test('zero is no badge even when visible', () => {
    // A badge reading 0 is worse than none: it draws the eye to say
    // nothing is there.
    assertEq(badgeFor({count: 0, 'count-visible': true}).count, null);
});

test('the label caps, and the count does not', () => {
    // 99+ is what fits in a 20px pill. The number itself stays exact so
    // a tooltip or an accessible name can be honest about it.
    const b = badgeFor({count: 1284, 'count-visible': true});
    assertEq(b.count, 1284);
    assertEq(b.label, '99+');
    assertEq(badgeFor({count: 99, 'count-visible': true}).label, '99');
});

test('a hostile count is clamped rather than believed', () => {
    // Anything on the session bus can emit this. A negative count, a
    // float, a string or an absurd number must not reach the renderer.
    assertEq(badgeFor({count: -3, 'count-visible': true}).count, null);
    assertEq(badgeFor({count: 2.7, 'count-visible': true}).count, 2, 'truncated, not rounded up');
    assertEq(badgeFor({count: '5', 'count-visible': true}).count, null, 'a string is not a count');
    assertEq(badgeFor({count: Number.MAX_SAFE_INTEGER, 'count-visible': true}).label, '99+');
    assertEq(badgeFor({count: NaN, 'count-visible': true}).count, null);
});

test('progress is a fraction, clamped, and only when visible', () => {
    assertEq(badgeFor({progress: 0.5, 'progress-visible': true}).progress, 0.5);
    assertEq(badgeFor({progress: 0.5}).progress, null, 'absent progress-visible is not visible');
    assertEq(badgeFor({progress: 1.7, 'progress-visible': true}).progress, 1);
    assertEq(badgeFor({progress: -1, 'progress-visible': true}).progress, 0);
    assertEq(badgeFor({progress: 'half', 'progress-visible': true}).progress, null);
});

test('an empty or absent payload clears everything', () => {
    // How an app says "nothing to report" — and what a malformed signal
    // must degrade to rather than leaving a stale badge on screen.
    const b = badgeFor({});
    assertEq(b.count, null);
    assertEq(b.progress, null);
    assertEq(b.label, null);
    assertEq(badgeFor(null).count, null);
    assertEq(badgeFor('nonsense').count, null);
});

test('urgent is carried through as a flag, not as a count', () => {
    // The convention has it and Lisa does not render it yet. Carried so
    // the dock can decide later, and NOT invented into a count — an
    // urgent app with nothing unread must not sprout a number.
    assertEq(badgeFor({urgent: true}).urgent, true);
    assertEq(badgeFor({urgent: true}).count, null);
    assertEq(badgeFor({}).urgent, false);
    assertEq(badgeFor({urgent: 'yes'}).urgent, false, 'only a real boolean counts');
});

// ---- BadgeState: what survives the Dash throwing its icons away -------

test('a badge is remembered so a Dash rebuild can redraw it', () => {
    // The Dash rebuilds its whole box when favourites change, when an
    // app starts, when the icon size settles. Drawing on receipt and
    // nothing else meant a badge lasted until the next unrelated app
    // launched and then silently vanished — and Mail only emits on
    // sync, so "unread mail, no badge" would be the normal state of the
    // dock within minutes.
    const state = new BadgeState();
    const mail = badgeFor({count: 3, 'count-visible': true});
    state.set('app.lisaos.Mail.desktop', mail);
    assertEq(state.get('app.lisaos.Mail.desktop').label, '3');
    assertEq(state.entries(), [['app.lisaos.Mail.desktop', mail]]);
});

test('clearing a badge forgets it rather than storing a blank', () => {
    // "Nothing to report" is the resting state of every app on the
    // machine. Storing it for each one fills the map with apps that
    // have nothing to say.
    const state = new BadgeState();
    state.set('app.lisaos.Mail.desktop', badgeFor({count: 3, 'count-visible': true}));
    state.set('app.lisaos.Mail.desktop', badgeFor({count: 0, 'count-visible': true}));
    assertEq(state.get('app.lisaos.Mail.desktop'), null);
    assertEq(state.entries(), []);
    assertEq(state.size, 0);
});

test('progress alone is worth remembering, even with no count', () => {
    // A download at 40% draws an arc and no number. `isEmpty` must not
    // read that as nothing.
    const state = new BadgeState();
    const dl = badgeFor({progress: 0.4, 'progress-visible': true});
    assert(!isEmpty(dl));
    state.set('app.lisaos.Surfer.desktop', dl);
    assertEq(state.size, 1);
});

test('the store is bounded, because the emitter list is not ours', () => {
    // Any session peer may emit for any uri it likes. An unbounded map
    // is a memory leak with a public trigger.
    const state = new BadgeState(3);
    for (let i = 0; i < 50; i++)
        state.set(`spam${i}.desktop`, badgeFor({count: 1, 'count-visible': true}));
    assertEq(state.size, 3);
});

test('re-publishing refreshes an app\'s place, so the quiet one is evicted', () => {
    // Insertion order decides who goes when the bound is hit. A plain
    // `Map.set` on an existing key does NOT move it, so without an
    // explicit re-insert the app that keeps publishing is exactly the
    // app that gets thrown out — which is Mail, badging every sync,
    // losing its slot to something that spoke once and went quiet.
    //
    // An earlier version of this test looped "publish Mail, publish
    // junk" and passed even with the re-insert removed: Mail was
    // evicted mid-loop and re-added by its own next call, so the final
    // state looked identical. It asserted nothing. This spells the
    // ordering out instead.
    const state = new BadgeState(3);
    const badge = badgeFor({count: 1, 'count-visible': true});
    state.set('a.desktop', badge);
    state.set('quiet.desktop', badge);
    state.set('c.desktop', badge);
    // `a` speaks again; `quiet` is now the oldest thing in the store.
    state.set('a.desktop', badgeFor({count: 2, 'count-visible': true}));
    state.set('newcomer.desktop', badge);

    assertEq(state.size, 3);
    assert(state.get('a.desktop') !== null, 'the app that kept publishing stayed');
    assertEq(state.get('a.desktop').label, '2', 'and kept its newest number');
    assertEq(state.get('quiet.desktop'), null, 'the one that went quiet went');
});

test('a nameless app cannot occupy a slot', () => {
    const state = new BadgeState();
    state.set('', badgeFor({count: 1, 'count-visible': true}));
    state.set(null, badgeFor({count: 1, 'count-visible': true}));
    assertEq(state.size, 0);
});

test('clear() takes everything off, so disable() is a real undo', () => {
    const state = new BadgeState();
    state.set('a.desktop', badgeFor({count: 1, 'count-visible': true}));
    state.set('b.desktop', badgeFor({count: 2, 'count-visible': true}));
    state.clear();
    assertEq(state.entries(), []);
});

finish('desktop/badges');
