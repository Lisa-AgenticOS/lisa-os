// Is this a credential field? (#260, and the shape #212 left behind.)
//
// Four properties carry weight and each has been watched go red:
//
//   * a password field is refused as a `fill` target, with the rule id;
//   * renaming it, retyping it, masking it in CSS or hiding it behind a
//     custom element does not make it fillable;
//   * the detector the PAGE runs is the same function this file tests —
//     not a copy of it in a template string;
//   * an ordinary form field still fills, because a guard that refuses
//     everything is a guard somebody deletes.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {
    CREDENTIAL_RULE, credentialGuardPreamble, credentialRefusal,
    fillVerdict, isCredentialField,
} from '../lib/credentials.js';
import {fillScript, clickScript} from '../lib/actions.js';

/// A plain text input nobody would call a credential.
const plain = (over = {}) => ({
    tag: 'input', type: 'text', attrType: 'text', autocomplete: '',
    name: 'q', id: 'q', ariaLabel: '', placeholder: 'Search', title: '',
    label: '', textSecurity: 'none', formHasPassword: false,
    shadowHasPassword: false,
    ...over,
});

test('the obvious case: type=password is refused', () => {
    const why = isCredentialField(plain({type: 'password', attrType: 'password'}));
    assert(why !== null, 'a password input must be refused');
    assert(why.includes('type password'), `unhelpful reason: ${why}`);
});

test('renaming the field does not make it fillable (#212 shape)', () => {
    // This is the exact bug: `fill(selector:"#q")` — the string the
    // consent dialog showed — landing in a credential field. The page
    // owned the JS world then; the isolated world fixed the
    // retargeting, and this is the half underneath it. The field is
    // called `q`, is labelled Search, and is still a password box.
    const why = isCredentialField(plain({
        type: 'password', attrType: 'password', name: 'q', id: 'q',
        placeholder: 'Search', ariaLabel: 'Search',
    }));
    assert(why !== null, 'a password input named q is still a password input');
});

test('a bogus type= attribute falls back to text and is still caught', () => {
    // `type="paßword"` is not a known input type, so the IDL `type`
    // reports `text` while the attribute keeps the string. Checking
    // only one of the two spellings misses one of the two attacks.
    assert(isCredentialField(plain({type: 'password', attrType: 'paßword'})) !== null,
        'the IDL type is authoritative');
    assert(isCredentialField(plain({type: 'text', attrType: 'password'})) !== null,
        'the attribute alone is enough to refuse');
});

test('autocomplete is a declaration, and it is honoured', () => {
    for (const token of ['current-password', 'new-password', 'one-time-code', 'cc-csc']) {
        const why = isCredentialField(plain({autocomplete: token}));
        assert(why !== null, `autocomplete=${token} must be refused`);
    }
    // Real pages write several tokens.
    assert(isCredentialField(plain({autocomplete: 'section-blue current-password'})) !== null);
    // …and `username` is not one of them on its own.
    assertEq(isCredentialField(plain({autocomplete: 'username'})), null,
        'a username field alone is not a credential field');
});

test('a text input masked by CSS is a password box to the person', () => {
    // -webkit-text-security renders a type=text input as dots. It looks
    // like a password field and reads as a text field to anything that
    // only checks `type` — and this engine is WebKit, so it is not a
    // hypothetical.
    assert(isCredentialField(plain({textSecurity: 'disc'})) !== null);
    assert(isCredentialField(plain({textSecurity: 'circle'})) !== null);
    assertEq(isCredentialField(plain({textSecurity: 'none'})), null);
    assertEq(isCredentialField(plain({textSecurity: ''})), null,
        'no computed style is not evidence of masking');
});

test('what the field calls itself, across every string a person reads', () => {
    for (const named of [
        {name: 'password'}, {name: 'passwd'}, {id: 'passphrase'},
        {name: 'user_pwd'}, {placeholder: 'PIN'}, {ariaLabel: 'One-time code'},
        {label: 'Verification code'}, {title: 'CVV'}, {name: 'totp_token'},
        {name: 'api_secret'}, {id: 'credential-1'}, {name: 'passcode'},
    ]) {
        const why = isCredentialField(plain(named));
        assert(why !== null, `${JSON.stringify(named)} must be refused`);
    }
});

test('a form that holds a password is a sign-in, and its other fields too', () => {
    // The username half of a sign-in is half of a sign-in. An agent
    // filling it is doing the thing #260 refuses, one field at a time.
    assert(isCredentialField(plain({formHasPassword: true})) !== null);
    assertEq(isCredentialField(plain({formHasPassword: false})), null);
    // `true` and only `true` — a string is what a forged descriptor
    // looks like.
    assertEq(isCredentialField(plain({formHasPassword: 'true'})), null,
        'the descriptor comes from our own code; a string here is a bug, not a rule');
});

test('a custom element hiding a password input in its shadow root', () => {
    // document.querySelector cannot see into a shadow root, so the
    // selector that reaches such a component names the HOST — and the
    // host forwards what it is given.
    assert(isCredentialField(plain({tag: 'my-login', shadowHasPassword: true})) !== null);
});

test('ordinary fields still fill — a guard that refuses everything is deleted', () => {
    for (const ordinary of [
        plain(),
        plain({name: 'search', placeholder: 'Search the web'}),
        plain({type: 'email', attrType: 'email', name: 'newsletter_email',
            autocomplete: 'email'}),
        plain({tag: 'textarea', type: '', attrType: '', name: 'message'}),
        plain({name: 'compass_bearing'}),
        plain({name: 'passenger_count'}),
    ]) {
        assertEq(isCredentialField(ordinary), null,
            `over-refused ${JSON.stringify(ordinary.name)}`);
    }
});

test('junk descriptors fail closed into "not a credential", not into a crash', () => {
    // A missing descriptor must not throw inside a page script: an
    // exception there is a fill that reports a page error and a person
    // who learns nothing. It is safe for this one to answer null —
    // there is no element, so there is nothing to fill.
    assertEq(isCredentialField(null), null);
    assertEq(isCredentialField(undefined), null);
    assertEq(isCredentialField('password'), null);
    assertEq(isCredentialField(42), null);
});

test('a refusal names the rule id, so the Ledger and `lisa guard list` agree', () => {
    const v = fillVerdict(plain({type: 'password', attrType: 'password'}));
    assertEq(v.filled, false);
    assertEq(v.refused, true);
    assertEq(v.rule, 'fill.password_field');
    assertEq(CREDENTIAL_RULE, 'fill.password_field',
        'the id is the Agent Bus id from lisa_guard::BUS_RULES — one rule, not two');
    assert(v.reason.includes('sign in yourself'),
        'a refusal has to say what the person should do instead');
    // The allowed case says nothing about a rule.
    assertEq(fillVerdict(plain()).refused, false);
});

test('the fill script CONTAINS the detector — not a copy of it', () => {
    // The load-bearing anti-drift check. If somebody re-implements the
    // classification inside the template string, this goes red; the
    // suite would otherwise keep testing a function the browser no
    // longer runs.
    const script = fillScript('#q', 'hello');
    assert(script.includes(isCredentialField.toString()),
        'the page runs a different detector from the one tested here');
    assert(script.includes(credentialRefusal.toString()),
        'the refusal object is built by a copy');
    assert(script.includes('isCredentialField(describeField(el))'),
        'the fill script does not actually call the detector');
    assert(script.includes(JSON.stringify(CREDENTIAL_RULE)),
        'the rule id is not in the script');
});

test('classify and fill happen in ONE script — no round trip to race', () => {
    // Two evaluate_javascript calls would leave a gap in which the page
    // swaps the element: the ADR-0033 shape lib/target.js exists for.
    // The proof available to a unit test is structural — the refusal
    // returns BEFORE any assignment to `.value` appears.
    const script = fillScript('#q', 'hello');
    const guard = script.indexOf('return JSON.stringify(credentialRefusal(why))');
    const write = script.indexOf('el.value = v');
    assert(guard > 0, 'no refusal in the fill script');
    assert(write > 0, 'no fill in the fill script');
    assert(guard < write, 'the fill is written before the guard runs');
    assert(!/await|then\s*\(/.test(script),
        'an await in the fill script is a gap a page can run in');
});

test('the serialised detector stands alone — no free variables', () => {
    // Its own source text is what runs in the page. A reference to a
    // module constant would be a ReferenceError in the browser and an
    // ALLOWED fill in production, which is the worst direction to fail
    // in. So evaluate the preamble the way the page does and use it.
    const run = new Function(`${credentialGuardPreamble()}
        return {detect: isCredentialField, refuse: credentialRefusal};`)();
    assert(run.detect(plain({type: 'password', attrType: 'password'})) !== null,
        'the serialised copy does not refuse a password field');
    assertEq(run.detect(plain()), null, 'the serialised copy over-refuses');
    assertEq(run.refuse('x').rule, CREDENTIAL_RULE,
        'the serialised refusal carries a different rule id');
    // describeField is source text, so it must at least parse and be
    // callable on a duck-typed element.
    const el = {
        tagName: 'INPUT', type: 'password', id: 'q',
        getAttribute: (n) => (n === 'type' ? 'password' : null),
        ownerDocument: null, labels: null, form: null, shadowRoot: null,
        closest: () => null,
    };
    const described = new Function(
        `${credentialGuardPreamble()} return describeField(arguments[0]);`)(el);
    assertEq(described.type, 'password');
    assert(isCredentialField(described) !== null,
        'describeField output does not classify');
});

test('click is untouched: reading a page is not typing a credential', () => {
    // Worth pinning. A guard bolted onto `click` too would refuse
    // pressing the "Sign in with…" BUTTON, which fills nothing.
    assert(!clickScript('#go').includes('isCredentialField'));
});

finish('surfer/credentials');
