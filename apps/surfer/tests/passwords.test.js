// Saved passwords (#260). The properties that carry weight, each of
// which has been watched go red:
//
//   * autofill needs a HUMAN GESTURE and refuses everything else —
//     no gesture, a forged one, a stale one, or a live agent stamp;
//   * the agent profile has no credentials, at save or at fill;
//   * an origin is an origin: no userinfo, no path, no suffix match;
//   * nothing writes to the keyring without asking first;
//   * no tool on the Agent Bus can read a credential, and the socket
//     refuses to start if one is ever added.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {AGENT_PROFILE, DEFAULT_PROFILE} from '../lib/profiles.js';
import {AGENT_ACTION_WINDOW_MS} from '../lib/causation.js';
import {
    AGENT_TOOLS, GESTURE_WINDOW_MS, KEYRING_ATTRIBUTES, KEYRING_SCHEMA,
    assertNoCredentialTools, autofillScript, autofillVerdict, credentialsAllowed,
    entryLabel, entryOf, exposesCredentials, isHumanGesture, keyringAttributes,
    SUBMIT_HANDLER, matchesFor, originOf, parseSubmission, removeEntry,
    saveDecision, searchEntries, secureOrigin, sortEntries, submitObserverJs,
} from '../lib/passwords.js';

const NOW = 1_000_000;
const hand = (over = {}) => ({kind: 'click', at: NOW - 10, trusted: true, ...over});
const saved = (over = {}) => ({
    origin: 'https://bank.example', username: 'ada', profile: DEFAULT_PROFILE, ...over,
});

// ---------------------------------------------------------------------
// Rule 4 — the agent profile has no credentials
// ---------------------------------------------------------------------

test('the agent profile never has credentials (#260 rule 4)', () => {
    assertEq(credentialsAllowed(AGENT_PROFILE), false);
    assertEq(credentialsAllowed(DEFAULT_PROFILE), true);
    assertEq(credentialsAllowed('work'), true);
    // Everything ambiguous fails closed, the same shape `recordable`
    // uses: a default here would be the boundary living in whichever
    // caller remembered to pass an argument.
    assertEq(credentialsAllowed(undefined), false);
    assertEq(credentialsAllowed(null), false);
    assertEq(credentialsAllowed(''), false);
    assertEq(credentialsAllowed('  '), false);
    assertEq(credentialsAllowed(0), false);
});

test('the agent profile cannot look up, key, save or fill', () => {
    const entries = [saved(), saved({profile: AGENT_PROFILE})];
    assertEq(entryOf(saved({profile: AGENT_PROFILE})), null);
    assertEq(keyringAttributes(saved({profile: AGENT_PROFILE})), null,
        'no attribute set means no lookup is even expressible');
    assertEq(matchesFor(entries, 'https://bank.example/login', AGENT_PROFILE), []);
    assertEq(
        saveDecision({
            url: 'https://bank.example/login', username: 'ada', password: 'x',
            existing: [], profile: AGENT_PROFILE, now: NOW,
        }).action, 'none');
    assertEq(
        autofillVerdict({
            profile: AGENT_PROFILE, url: 'https://bank.example/login',
            gesture: hand(), now: NOW, entries,
        }).fill, false);
});

// ---------------------------------------------------------------------
// Rule 3 — nothing on the bus reads a credential
// ---------------------------------------------------------------------

test('no Agent Bus tool can reach the keyring, and adding one stops the browser', () => {
    // An absence is not a guardrail (CLAUDE.md 6a). This is the
    // mechanism: `lib/mcp.js` runs it while it wires the tool table, so
    // `read_password` is a browser that will not start.
    assertEq(assertNoCredentialTools(AGENT_TOOLS), true);
    for (const bad of [
        'read_password', 'list_passwords', 'search_credentials', 'get_secret',
        'keyring_lookup', 'autofill', 'saved_logins',
    ]) {
        assert(exposesCredentials(bad), `${bad} must read as a credential tool`);
        let threw = '';
        try { assertNoCredentialTools([...AGENT_TOOLS, bad]); } catch (e) { threw = e.message; }
        assert(threw !== '', `${bad} was accepted onto the bus`);
    }
    // …and the allowlist is a list, not a filter: any name we did not
    // put there is refused even if it sounds harmless.
    let threw = '';
    try { assertNoCredentialTools(['download_file']); } catch (e) { threw = e.message; }
    assert(threw.includes('AGENT_TOOLS'), `unhelpful refusal: ${threw}`);
    // The tools we DO serve are not accidentally credential-shaped.
    for (const tool of AGENT_TOOLS)
        assertEq(exposesCredentials(tool), false, `${tool} trips the check`);
});

// ---------------------------------------------------------------------
// Origins
// ---------------------------------------------------------------------

test('an origin is scheme + host + non-default port, and nothing else', () => {
    assertEq(originOf('https://bank.example/login?next=/x#f'), 'https://bank.example');
    assertEq(originOf('https://BANK.example/'), 'https://bank.example');
    assertEq(originOf('https://bank.example:443/'), 'https://bank.example');
    assertEq(originOf('https://bank.example:8443/'), 'https://bank.example:8443');
    assertEq(originOf('http://localhost:3000/app'), 'http://localhost:3000');
    assertEq(originOf('http://example.org:80/'), 'http://example.org');
    assertEq(originOf('https://[::1]:8443/'), 'https://[::1]:8443');
});

test('userinfo is not a host — the oldest trick there is', () => {
    // `https://bank.example@evil.test/` reads as the bank to a person
    // and resolves to the attacker. A keying function that took the
    // first half would hand over the bank's password.
    assertEq(originOf('https://bank.example@evil.test/'), 'https://evil.test');
    assertEq(originOf('https://a@b@evil.test/'), 'https://evil.test',
        'the LAST @ — userinfo may contain one');
    // A fragment or a query that merely looks like a URL is text.
    assertEq(originOf('https://evil.test/#https://bank.example'), 'https://evil.test');
    assertEq(originOf('https://evil.test/?u=https://bank.example'), 'https://evil.test');
});

test('non-http schemes have no origin at all', () => {
    for (const bad of [
        'file:///etc/passwd', 'javascript:alert(1)', 'data:text/html,x',
        'lisa:start', 'about:blank', 'ftp://example.org/', 'bank.example',
        '', null, undefined, 'https://', 'https:///path',
    ]) {
        assertEq(originOf(bad), null, `${String(bad)} must not key an entry`);
    }
});

test('a saved credential is only offered to its EXACT origin', () => {
    const entries = [saved(), saved({origin: 'https://other.example', username: 'bo'})];
    assertEq(matchesFor(entries, 'https://bank.example/login', DEFAULT_PROFILE).length, 1);
    for (const wrong of [
        'https://evil-bank.example/',      // suffix-ish
        'https://bank.example.evil.test/', // prefix-ish
        'https://sub.bank.example/',       // a subdomain is a different origin
        'http://bank.example/',            // a different scheme is a different origin
        'https://bank.example:8443/',      // a different port is a different origin
    ]) {
        assertEq(matchesFor(entries, wrong, DEFAULT_PROFILE), [],
            `${wrong} must not match https://bank.example`);
    }
});

test('passwords are saved for https, and for loopback http, and nothing else', () => {
    assert(secureOrigin('https://bank.example/'));
    assert(secureOrigin('http://localhost:3000/'));
    assert(secureOrigin('http://127.0.0.1:8000/'));
    assert(!secureOrigin('http://bank.example/'),
        'a password typed over http already crossed the wire in the clear');
    assert(!secureOrigin('file:///x'));
});

// ---------------------------------------------------------------------
// Saving — never silent
// ---------------------------------------------------------------------

test('nothing is written to the keyring without asking', () => {
    const base = {
        url: 'https://bank.example/login', username: 'ada', password: 's3cr3t',
        existing: [], profile: DEFAULT_PROFILE, now: NOW,
    };
    const first = saveDecision(base);
    assertEq(first.action, 'save');
    assert(first.prompt.includes('bank.example'), 'the prompt must name the site');
    assert(first.prompt.includes('ada'), 'the prompt must name the account');

    // The property, asserted over EVERY outcome rather than the two we
    // happen to know about: no answer writes without a question.
    const cases = [
        base,
        {...base, existing: [saved()], existingPassword: 'old'},
        {...base, existing: [saved()], existingPassword: 's3cr3t'},
        {...base, password: ''},
        {...base, url: 'http://bank.example/login'},
        {...base, profile: AGENT_PROFILE},
        {...base, agentTouchedAt: NOW - 1},
        {...base, url: 'file:///tmp/x.html'},
        {},
    ];
    for (const c of cases) {
        const d = saveDecision(c);
        assert(['save', 'update', 'none'].includes(d.action), `unknown action ${d.action}`);
        if (d.action !== 'none') {
            assert(typeof d.prompt === 'string' && d.prompt.trim() !== '',
                `a ${d.action} with no prompt is a silent write: ${JSON.stringify(c)}`);
        }
    }
});

test('a changed password is an update; an unchanged one is nothing', () => {
    const base = {
        url: 'https://bank.example/login', username: 'ada',
        existing: [saved()], profile: DEFAULT_PROFILE, now: NOW,
    };
    assertEq(saveDecision({...base, password: 'new', existingPassword: 'old'}).action, 'update');
    assertEq(saveDecision({...base, password: 'same', existingPassword: 'same'}).action, 'none');
    // With no secret to compare against, ask. Offering to re-save
    // something already saved is one extra question; skipping a real
    // change is a password the person believes is stored and is not.
    assertEq(saveDecision({...base, password: 'new'}).action, 'update');
});

test('a form an agent submitted never produces a save prompt (#260 rule 2)', () => {
    const base = {
        url: 'https://bank.example/login', username: 'ada', password: 's3cr3t',
        existing: [], profile: DEFAULT_PROFILE, now: NOW,
    };
    assertEq(saveDecision({...base, agentTouchedAt: NOW}).action, 'none');
    assertEq(saveDecision({...base, agentTouchedAt: NOW - AGENT_ACTION_WINDOW_MS + 1}).action,
        'none');
    // Outside the window it is a person again.
    assertEq(saveDecision({...base, agentTouchedAt: NOW - AGENT_ACTION_WINDOW_MS}).action,
        'save');
});

// ---------------------------------------------------------------------
// Rule 2 — autofill is a person asking
// ---------------------------------------------------------------------

test('a gesture is a person, and everything else is not', () => {
    assert(isHumanGesture(hand(), NOW));
    assert(isHumanGesture(hand({kind: 'key'}), NOW));
    assert(isHumanGesture(hand({kind: 'menu'}), NOW));
    for (const forged of [
        null, undefined, {}, 'click', 42,
        hand({trusted: false}),
        hand({trusted: 'yes'}),          // what a forged gesture looks like
        hand({trusted: 1}),
        hand({trusted: undefined}),
        hand({kind: 'tool'}),            // a bus call is not a gesture
        hand({kind: 'navigate'}),
        hand({kind: 'script'}),
        hand({at: 0}),
        hand({at: '1000000'}),
        hand({at: NaN}),
        hand({at: NOW + 1}),             // from the future: fail closed
        hand({at: NOW - GESTURE_WINDOW_MS - 1}),  // stale
    ]) {
        assert(!isHumanGesture(forged, NOW), `accepted ${JSON.stringify(forged)}`);
    }
    // An unreadable clock is not a reason to fill.
    assert(!isHumanGesture(hand(), NaN));
    assert(!isHumanGesture(hand(), undefined));
});

test('autofill without a gesture is refused, whatever else is true', () => {
    const entries = [saved()];
    const base = {
        profile: DEFAULT_PROFILE, url: 'https://bank.example/login', now: NOW, entries,
    };
    assert(autofillVerdict({...base, gesture: hand()}).fill, 'the person asking must work');
    for (const g of [undefined, null, {}, hand({trusted: false}), hand({kind: 'tool'}),
        hand({at: NOW - GESTURE_WINDOW_MS - 1})]) {
        const v = autofillVerdict({...base, gesture: g});
        assertEq(v.fill, false, `filled on ${JSON.stringify(g)}`);
    }
    // The refusal has to say what would work, or people build a
    // workaround for it.
    const why = autofillVerdict({...base, gesture: null}).reason;
    assert(why.includes('when you ask'), `unhelpful refusal: ${why}`);
});

test('an agent action in flight cancels autofill even WITH a gesture', () => {
    // The stamp survives a click: a page can talk a person into
    // clicking, and `navigate`/`click`/`fill` all stamp the view. The
    // gesture is necessary and not sufficient.
    const base = {
        profile: DEFAULT_PROFILE, url: 'https://bank.example/login', now: NOW,
        entries: [saved()], gesture: hand(),
    };
    assertEq(autofillVerdict({...base, agentTouchedAt: NOW}).fill, false);
    assertEq(autofillVerdict({...base, agentTouchedAt: NOW - 1}).fill, false);
    assertEq(
        autofillVerdict({...base, agentTouchedAt: NOW - AGENT_ACTION_WINDOW_MS + 1}).fill,
        false);
    assertEq(autofillVerdict({...base, agentTouchedAt: NOW + 5000}).fill, false,
        'a clock that went backwards is not a reason to fill');
    // Outside the window the tab is the person's again.
    assertEq(
        autofillVerdict({...base, agentTouchedAt: NOW - AGENT_ACTION_WINDOW_MS}).fill, true);
});

test('a refusal does not say whether a credential exists', () => {
    // CLAUDE.md 6b: a refusal must not reveal what exists. Anything
    // that answered "no gesture" for a site with a saved password and
    // "nothing saved" for one without would be a credential search tool
    // with extra steps.
    const withEntry = autofillVerdict({
        profile: DEFAULT_PROFILE, url: 'https://bank.example/login', now: NOW,
        entries: [saved()], gesture: null,
    });
    const without = autofillVerdict({
        profile: DEFAULT_PROFILE, url: 'https://bank.example/login', now: NOW,
        entries: [], gesture: null,
    });
    assertEq(withEntry.reason, without.reason,
        'the refusal differs depending on whether a password is stored');
});

test('the autofill script chooses its target by focus, not by argument', () => {
    const s = autofillScript('ada', 's3cr3t');
    assert(s.includes('document.activeElement'), 'nothing anchors the fill to the person');
    assert(s.includes('input[type=password i]'));
    // No selector parameter exists, so there is no argument through
    // which a target could be chosen for it.
    assertEq(autofillScript.length, 2, 'autofillScript grew an argument');
    // Values are data. A password with a quote in it must not become
    // script — the same rule lib/actions.js follows.
    const hostile = autofillScript('"];alert(1);//', '</script><script>1');
    assert(!hostile.includes('</script>'), 'raw </script> leaked into the page script');
    assertEq((hostile.match(/\(/g) ?? []).length, (hostile.match(/\)/g) ?? []).length);
});

test('a page cannot post a submission Surfer never saw', () => {
    // The observer is injected into the AGENT world and the handler is
    // registered there, so `messageHandlers.lisaSurferCredential` does
    // not exist in the page's own world. This pins the two halves to the
    // same name — the day they drift, a real sign-in stops being
    // noticed and nothing says why.
    const src = submitObserverJs();
    assert(src.includes(JSON.stringify(SUBMIT_HANDLER)),
        'the observer posts to a different handler than the one registered');
    assert(src.includes("addEventListener('submit'"), 'nothing is observed');
    assert(src.includes('true)'), 'the capture phase is what survives preventDefault');
    // Whatever arrives is validated, not trusted.
    assertEq(parseSubmission(null), null);
    assertEq(parseSubmission('not json'), null);
    assertEq(parseSubmission('[]'), null);
    assertEq(parseSubmission('{"url":"https://a.example","password":""}'), null);
    assertEq(parseSubmission('{"url":"file:///x","password":"p"}'), null,
        'a scheme with no origin has no entry to save');
    assertEq(parseSubmission('{"url":"https://a.example/login","password":"p"}'),
        {url: 'https://a.example/login', username: '', password: 'p'});
    assertEq(parseSubmission({url: 'https://a.example/', username: 7, password: 'p'}),
        {url: 'https://a.example/', username: '', password: 'p'},
        'a non-string username is dropped, not stringified into the prompt');
});

// ---------------------------------------------------------------------
// The manage surface
// ---------------------------------------------------------------------

test('the list carries no secrets', () => {
    const e = entryOf({origin: 'https://bank.example/login?x=1', username: ' ada ',
        profile: DEFAULT_PROFILE});
    assertEq(e, {origin: 'https://bank.example', username: 'ada', profile: DEFAULT_PROFILE});
    assertEq(Object.keys(e).sort(), ['origin', 'profile', 'username']);
    assertEq(KEYRING_ATTRIBUTES.slice().sort(), ['origin', 'profile', 'username']);
    assertEq(KEYRING_SCHEMA, 'app.lisaos.Surfer.Login');
});

test('search, sort, label and delete', () => {
    const list = [
        saved({origin: 'https://zed.example', username: 'zoe'}),
        saved(),
        saved({username: 'bo'}),
    ];
    assertEq(searchEntries(list, '').length, 3, 'an empty query is the whole list');
    assertEq(searchEntries(list, 'BANK').length, 2, 'search is case-insensitive');
    assertEq(searchEntries(list, 'zoe').length, 1);
    assertEq(searchEntries(list, 'nothing').length, 0);
    assertEq(sortEntries(list).map(e => e.username), ['ada', 'bo', 'zoe']);
    assertEq(entryLabel(saved()), 'ada — https://bank.example');
    assertEq(entryLabel(saved({username: ''})), 'https://bank.example',
        'an entry with no username is still an entry');
    // Delete removes EVERY matching row, not the first one found.
    const dupes = [saved(), saved(), saved({username: 'bo'})];
    assertEq(removeEntry(dupes, saved()).length, 1);
    // A row for another profile is another profile's row.
    assertEq(removeEntry([saved({profile: 'work'})], saved()).length, 1);
});

finish('surfer/passwords');
