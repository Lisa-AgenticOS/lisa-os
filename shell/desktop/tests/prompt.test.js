// The prompt half of the dock bar (ADR-0035 §2).
//
// What is asserted here is the DECISION — where one submission goes and
// what a key press means — not the drawing. The St widget is a dozen
// lines only a live session can exercise; this is the part that can be
// wrong on a dev host.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    ASSISTANT_BUS_NAME, ASSISTANT_METHOD, ASSISTANT_OBJECT_PATH, ASSISTANT_SIGNATURE,
    keyAction, namesApp, submission,
} from '../lib/prompt.js';
// The other end of the hand-off. Two installed extension trees cannot
// import each other at RUNTIME — but they ship from one repo, so a test
// can hold them to the same string.
import {
    OVERLAY_UI_BUS_NAME, OVERLAY_UI_IFACE_XML, OVERLAY_UI_OBJECT_PATH,
} from '../../overlay-extension/lib/iface.js';

const MAIL = {id: 'app.lisaos.Mail.desktop', name: 'Mail'};
const FIREFOX = {id: 'firefox.desktop', name: 'Firefox'};
const INSTALLED = [MAIL, FIREFOX];

test('an empty prompt does nothing at all', () => {
    // A permanent entry collects stray Enters from somebody who clicked
    // the dock by accident. Summoning an empty assistant layer for each
    // one is how a person learns not to click the dock.
    assertEq(submission('').kind, 'none');
    assertEq(submission('   \t ').kind, 'none');
    assertEq(submission(null).kind, 'none');
    assertEq(submission(undefined, INSTALLED).kind, 'none');
});

test('typing a program name launches it', () => {
    // ADR-0035 §2: "typing a program name launches it, typing a
    // sentence routes through intent".
    assertEq(submission('Mail', INSTALLED), {
        kind: 'launch', id: 'app.lisaos.Mail.desktop', prompt: 'Mail',
    });
    assertEq(submission('mail', INSTALLED).kind, 'launch', 'case does not matter');
    assertEq(submission('  firefox  ', INSTALLED).id, 'firefox.desktop',
        'surrounding space is not part of the name');
});

test('the desktop-id stem is a name too', () => {
    // `app.lisaos.Surfer.desktop` is typed "surfer", never
    // "app.lisaos.Surfer". Both spellings resolve, because the id is
    // what a person who reads the filesystem would try.
    const surfer = {id: 'app.lisaos.Surfer.desktop', name: 'Lisa Surfer'};
    assertEq(submission('surfer', [surfer]).kind, 'launch');
    assertEq(submission('Lisa Surfer', [surfer]).kind, 'launch', 'the display name too');
});

test('a sentence goes to the assistant, never to an app', () => {
    assertEq(submission('what is on my calendar today?', INSTALLED), {
        kind: 'ask', prompt: 'what is on my calendar today?',
    });
    assertEq(submission('summarise my unread mail', INSTALLED).kind, 'ask',
        'a sentence containing an app name is still a sentence');
    assertEq(submission('open mail', INSTALLED).kind, 'ask');
});

test('a partial name asks rather than guessing an app', () => {
    // The launcher merge (§2) grows this bar upward into a result list;
    // until it does, there is nothing on screen to disambiguate a prefix
    // against. Opening a window the person never named is a worse
    // failure than a sentence reaching the assistant, which they can
    // read and ignore.
    assertEq(submission('mai', INSTALLED).kind, 'ask');
    assertEq(submission('m', INSTALLED).kind, 'ask');
    assertEq(submission('fire', INSTALLED).kind, 'ask');
});

test('an unknown word asks', () => {
    assertEq(submission('quarterly', INSTALLED), {kind: 'ask', prompt: 'quarterly'});
    assertEq(submission('mail', []).kind, 'ask', 'nothing installed, nothing to launch');
});

test('accents and case fold, so a name typed either way still launches', () => {
    const cafe = {id: 'org.example.Cafe.desktop', name: 'Café'};
    assert(namesApp('café', cafe));
    assert(namesApp('CAFE', cafe));
    assert(namesApp('cafe', cafe));
    assert(!namesApp('caf', cafe), 'still not a prefix match');
});

test('a candidate with a missing name or id is not a match', () => {
    // The lookup is a live GNOME call and can hand back anything.
    assert(!namesApp('mail', {}));
    assert(!namesApp('mail', null));
    assert(!namesApp('', MAIL), 'nothing typed names nothing');
    assertEq(submission('mail', [{}, MAIL]).id, 'app.lisaos.Mail.desktop',
        'a junk candidate does not shadow a real one');
});

test('Enter submits, on every keyboard that has one', () => {
    assertEq(keyAction(0xff0d), 'submit', 'Return');
    assertEq(keyAction(0xff8d), 'submit', 'KP_Enter — the numeric keypad is a keyboard');
    assertEq(keyAction(0xfe34), 'submit', 'ISO_Enter');
});

test('Escape clears first and only then gives the keyboard back', () => {
    // The shape Spotlight and every address bar use. One key that both
    // erases a half-typed thought and moves focus somewhere invisible is
    // a key people stop pressing.
    assertEq(keyAction(0xff1b, {hasText: true}), 'clear');
    assertEq(keyAction(0xff1b, {hasText: false}), 'release');
    assertEq(keyAction(0xff1b), 'release', 'no state reads as empty');
});

test('every other key is the text field\'s, not ours', () => {
    // A dock that swallowed keys it had no plan for would break
    // selection, accents and every input method typed into it.
    assertEq(keyAction(0x061), 'pass', 'a');
    assertEq(keyAction(0xff08), 'pass', 'BackSpace');
    assertEq(keyAction(0xff09), 'pass', 'Tab');
    assertEq(keyAction(0xff52), 'pass', 'Up — history is not built, so it types nothing');
});

test('the dock calls the name the overlay actually exports', () => {
    // The dock's copy of the address, held against the overlay's own.
    // A bus name that is one character off is a method call that raises
    // ServiceUnknown into a callback nobody watches: press Enter, get
    // nothing, no error on screen. That is #244 and #255, twice, and
    // it is the whole reason this assertion exists rather than a
    // comment saying "keep these in sync".
    assertEq(ASSISTANT_BUS_NAME, OVERLAY_UI_BUS_NAME);
    assertEq(ASSISTANT_OBJECT_PATH, OVERLAY_UI_OBJECT_PATH);
});

test('the method and its signature are the ones in the overlay\'s XML', () => {
    // Same failure with a different spelling: a right name, a wrong
    // argument list, and a call that is rejected at the bus.
    assert(OVERLAY_UI_IFACE_XML.includes(`<method name="${ASSISTANT_METHOD}">`),
        'Summon is a method on dev.lisaos.Overlay1.UI');
    // Summon(s prompt, a{sv} options) — the `(sa{sv})` the dock packs.
    const summon = OVERLAY_UI_IFACE_XML.slice(
        OVERLAY_UI_IFACE_XML.indexOf(`<method name="${ASSISTANT_METHOD}">`));
    const args = [...summon.slice(0, summon.indexOf('</method>'))
        .matchAll(/<arg type="([^"]+)"[^>]*direction="in"/g)].map(m => m[1]);
    assertEq(`(${args.join('')})`, ASSISTANT_SIGNATURE);
});

finish('desktop/prompt');
