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
import {badgeFor, desktopIdFromUri} from '../lib/badges.js';

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

finish('desktop/badges');
