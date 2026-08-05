// Is this a credential field? (#260, and the shape #212 left behind.)
//
// # Why this module exists at all
//
// `fill` is a write-tier agent tool. #212 demonstrated on the device
// that `fill(selector:"#q")` — the exact string the consent dialog
// showed the person — landed in a field named `password`, because the
// page owned `document.querySelector` in the world the script ran in.
// The isolated world (lib/world.js) fixed the *retargeting*. It did not
// answer the question underneath it: **what happens when the agent's
// selector honestly resolves to a credential field?**
//
// #260's answer is that nothing happens. A password field is not a
// confirm-tier target and not a scope question — it is refused, in
// #251's sense, because there is no legitimate agent workflow that
// requires typing a credential. A person who wants to sign in signs in.
//
// The bus already refuses the LEXICAL spelling of this: `judge_action`
// in `libs/lisa-guard/src/action.rs` emits `fill.password_field` when
// the arguments themselves name a credential (`selector: "#password"`,
// `field: "passphrase"`). That check reads the tool call. It cannot read
// the page, and the page is where the truth is: `#q` is an innocent
// selector right up until it resolves to `<input type=password>`.
//
// So this module is the DOM half of the same rule, under the same rule
// id — deliberately the same, because a person reading the Ledger
// should see one rule rather than two spellings of it, which is the
// argument `action.rs` already makes for `escalate.privilege`.
//
// # Where the check runs, and why that is the whole design
//
// Three properties, and losing any one of them loses the guarantee:
//
//   1. **In the agent world.** `describeField` calls `getAttribute`,
//      `getComputedStyle`, `closest`. In the page's own world a page
//      redefines all three and every answer becomes the page's answer.
//      The script this module builds is only ever evaluated through
//      `evaluateInAgentWorld` (lib/world.js).
//
//   2. **In the same synchronous turn as the fill.** If the
//      classification were one `evaluate_javascript` round trip and the
//      fill were a second, a page could swap the element between them —
//      the ADR-0033 shape, a later call acting on state nothing pinned.
//      So `credentialGuardPreamble()` is spliced INTO the fill script:
//      resolve, classify, refuse-or-fill, with no `await` and therefore
//      no point at which page script can run.
//
//   3. **From one source of truth.** `isCredentialField` is a real
//      function this module exports and the tests exercise directly;
//      the page-side copy is `isCredentialField.toString()`, not a
//      re-implementation. `tests/credentials.test.js` asserts the
//      script CONTAINS that source, so the two cannot drift — a copy
//      with no test is a copy that drifts (the same argument
//      `tests/find.test.js` makes about WebKit enums).
//
// No gi:// import: every rule here runs under `just shell-test` on any
// host.

/// The rule id a refusal reports. The SAME id the Agent Bus emits for
/// the lexical case (`lisa_guard::BUS_RULES`, `HARD_NO_RULES`) — one
/// rule, two enforcement points, one thing to look up.
export const CREDENTIAL_RULE = 'fill.password_field';

/// Is this element a credential field, and if so why?
///
/// Takes a plain descriptor rather than an element so it is testable
/// off a browser, and returns a REASON string (or `null`) rather than a
/// boolean so the refusal can say which rule fired — a refusal nobody
/// can explain is a refusal somebody deletes.
///
/// **Self-contained on purpose.** It references no module-level
/// constant and no import, because its own source text is what runs
/// inside the page script. A free variable here would be a
/// `ReferenceError` in the browser and an allowed fill in production,
/// which is the worst possible failure direction. `tests/credentials.js`
/// evaluates the serialised copy to prove it stands alone.
export function isCredentialField(descriptor) {
    const d = descriptor && typeof descriptor === 'object' ? descriptor : {};
    const s = (v) => (typeof v === 'string' ? v : '').toLowerCase();

    // 1. The engine's own answer, in both spellings. `type` is the IDL
    //    property — what the browser actually uses to decide masking and
    //    what to send — and `attrType` is the content attribute. They
    //    differ when the attribute is bogus (`type="paßword"` falls back
    //    to text), so both are asked and either is enough.
    if (s(d.type) === 'password' || s(d.attrType) === 'password')
        return 'it is an input of type password';

    // 2. What the page itself told password managers this field is.
    //    A page that wants Chrome to fill it has to say so here, so a
    //    real login form says so — and a page that lies about it in the
    //    other direction is only lying to itself.
    const tokens = s(d.autocomplete).split(/[\s,]+/).filter(Boolean);
    for (const token of ['current-password', 'new-password', 'one-time-code', 'cc-csc']) {
        if (tokens.includes(token))
            return `it declares autocomplete ${token}`;
    }

    // 3. Masked by CSS rather than by type. `-webkit-text-security: disc`
    //    renders a `type=text` input as dots — it looks like a password
    //    box to the person and reads as an ordinary text field to
    //    anything that only checks `type`. WebKit is where that property
    //    comes from, so it is not a hypothetical on this engine.
    const mask = s(d.textSecurity);
    if (mask !== '' && mask !== 'none')
        return 'it is masked by text-security';

    // 4. What the field calls itself, across every string a person would
    //    read. Renaming defeats this and only this — which is why it is
    //    the fourth rule and not the first.
    //    Punctuation is flattened to spaces first: `user_pwd`, `user-pwd`
    //    and `user.pwd` are one name three ways, and `_` is a word
    //    character so `\bpwd\b` matches none of them without this.
    const named = [d.name, d.id, d.ariaLabel, d.placeholder, d.title, d.label]
        .map(s).join(' ').replace(/[^a-z0-9]+/g, ' ');
    const words = /pass(word|phrase|wd|code)|\bpwd\b|\bpin\b|\botp\b|totp|2fa|mfa|one ?time|verification code|security code|\bcvv\b|\bcvc\b|secret|credential/;
    if (words.test(named))
        return 'it names itself a credential field';

    // 5. It sits in a form that also holds a password field. That form
    //    is a sign-in or a sign-up, and the username half of a sign-in
    //    is half of a sign-in: an agent filling it is doing the thing
    //    #260 refuses, one field at a time.
    if (d.formHasPassword === true)
        return 'it is part of a form that contains a password field';

    // 6. A custom element wrapping a password input in an OPEN shadow
    //    root. `document.querySelector` cannot see into a shadow root,
    //    so the selector that reaches such a component names the HOST —
    //    and the host forwards what it is given to the input inside.
    if (d.shadowHasPassword === true)
        return 'it wraps a password field in its shadow root';

    return null;
}

/// The page-side half: an element → the descriptor above.
///
/// Source text rather than a function, because it runs in the page's
/// document from inside the agent world. Every DOM call here resolves
/// against the agent world's prototypes, which the page cannot reach.
export const DESCRIBE_FIELD_JS = `function describeField(el) {
    const attr = (n) => { try { return el.getAttribute(n); } catch (e) { return null; } };
    let mask = '';
    try {
        const view = el.ownerDocument && el.ownerDocument.defaultView;
        const style = view ? view.getComputedStyle(el) : null;
        if (style)
            mask = style.getPropertyValue('-webkit-text-security') || style.webkitTextSecurity || '';
    } catch (e) { mask = ''; }
    let label = '';
    try {
        const labels = el.labels ? Array.from(el.labels) : [];
        label = labels.map(l => l.innerText || l.textContent || '').join(' ');
    } catch (e) { label = ''; }
    let formHasPassword = false;
    try {
        const form = el.form || (el.closest ? el.closest('form') : null);
        if (form) formHasPassword = !!form.querySelector('input[type=password i]');
    } catch (e) { formHasPassword = false; }
    let shadowHasPassword = false;
    try {
        const root = el.shadowRoot;
        if (root) shadowHasPassword = !!root.querySelector('input[type=password i]');
    } catch (e) { shadowHasPassword = false; }
    return {
        tag: (el.tagName || '').toLowerCase(),
        type: typeof el.type === 'string' ? el.type : '',
        attrType: attr('type') || '',
        autocomplete: attr('autocomplete') || (typeof el.autocomplete === 'string' ? el.autocomplete : '') || '',
        name: attr('name') || '',
        id: el.id || '',
        ariaLabel: attr('aria-label') || '',
        placeholder: attr('placeholder') || '',
        title: attr('title') || '',
        label: label,
        textSecurity: mask,
        formHasPassword: formHasPassword,
        shadowHasPassword: shadowHasPassword,
    };
}`;

/// The refusal, as an object.
///
/// Serialised into the page script alongside the detector, so the
/// object a refused `fill` hands back to the agent is built by THIS
/// function and not by a hand-written copy of it inside a template
/// string. `rule` is read from `CREDENTIAL_RULE`, which the preamble
/// declares — so the id in the Ledger and the id `lisa guard list`
/// prints cannot drift apart either.
export function credentialRefusal(why) {
    return {
        filled: false,
        refused: true,
        rule: CREDENTIAL_RULE,
        reason:
            `refused: ${why}. Nothing an agent legitimately does involves ` +
            'typing a credential — sign in yourself, issue 260',
    };
}

/// Everything a page script needs to answer the question, as source,
/// from one place.
///
/// Spliced into `fillScript` rather than run before it: see property 2
/// in the header. Anything that ever needs to ask "is this a credential
/// field" inside the page must use this, so that adding a second
/// caller cannot add a second, weaker, answer.
export function credentialGuardPreamble() {
    return [
        `const CREDENTIAL_RULE = ${JSON.stringify(CREDENTIAL_RULE)};`,
        isCredentialField.toString(),
        credentialRefusal.toString(),
        DESCRIBE_FIELD_JS,
    ].join('\n');
}

/// The verdict for a descriptor, JS-land side. Same decision the page
/// script makes, from the same function.
export function fillVerdict(descriptor) {
    const why = isCredentialField(descriptor);
    return why === null ? {filled: null, refused: false} : credentialRefusal(why);
}
