// Saved passwords (#260, ADR-0037's last undecided piece).
//
// # The store is the system keyring, and that is a decision
//
// The system owns credentials. Surfer keeps no credential file of its
// own: entries live in gnome-keyring through libsecret
// (`org.freedesktop.secrets`), which is the same store Mail, the Shell
// and every other program on this machine already uses, and which is
// locked with the login keyring rather than with something a browser
// invented. `lisa-surfer.js` holds the `gi://Secret` calls; this module
// holds every decision they make, so all of it is testable off a
// display and none of it can be talked out of.
//
// # The four rules #260 asked for, and where each one lives
//
//   1. *A password field is never fillable by an agent.* — not here.
//      `lib/credentials.js`, spliced into the fill script itself.
//   2. *Autofill only from a human gesture.* — `autofillVerdict` below.
//   3. *The keyring is not readable through any agent tool.* —
//      `assertNoCredentialTools` below, called by `lib/mcp.js` while it
//      builds the tool table, so a tool that could read a credential
//      stops the browser from starting rather than shipping quietly.
//   4. *The agent profile has no credentials.* — `credentialsAllowed`,
//      the same shape `recordable` uses for history.
//
// # Why an ORIGIN and not a URL
//
// A saved credential is keyed by origin — scheme, host and non-default
// port, nothing else. Not the path (a login form at `/login` and a
// re-auth at `/account/verify` are the same account), not a hostname
// suffix (suffix matching is how `evil-bank.example` gets
// `bank.example`'s password), and never the userinfo, because
// `https://bank.example@evil.test/` is the oldest trick in the file.
//
// No gi:// import.

import {AGENT_PROFILE} from './profiles.js';
import {agentDriven} from './causation.js';

/// The libsecret schema Surfer's own entries carry. A schema is a
/// namespace: an entry stored under it is one of ours, and a lookup
/// under it cannot return somebody else's WiFi key by accident.
export const KEYRING_SCHEMA = 'app.lisaos.Surfer.Login';

/// The attributes an entry is keyed by. All three, always: `profile` is
/// in the key rather than in the label because a lookup that omitted it
/// would return the person's credential to a profile that must not have
/// one, and a lookup is the thing that has to be safe.
export const KEYRING_ATTRIBUTES = Object.freeze(['origin', 'username', 'profile']);

/// How long a human gesture stays a reason. Short: this is the gap
/// between a person clicking the key button and the page answering,
/// not a session.
export const GESTURE_WINDOW_MS = 5000;

/// The gestures that count as a person asking.
///
/// All three are GTK events on Surfer's OWN chrome — a popover row, a
/// keyboard shortcut, a menu item. Nothing in the page can produce one:
/// a synthetic `click()` inside the document reaches a DOM node, and a
/// DOM node is not a Gtk.Button. That is the actual mechanism; the list
/// below only decides which of OUR widgets may ask.
const HUMAN_GESTURES = ['click', 'key', 'menu'];

// ---------------------------------------------------------------------
// Rule 4: the agent profile has no credentials
// ---------------------------------------------------------------------

/// May this profile have saved credentials at all?
///
/// The profile argument is not optional and not defaulted, for the
/// reason `recordable` states: a default here would put the boundary in
/// whichever caller remembered to pass it.
///
/// The agent profile already browses in its own `NetworkSession`
/// (lib/profiles.js, #181/#259), so it is logged into nothing. This
/// makes that true of the credential store as well: an agent-driven
/// page cannot be filled from the keyring because the profile it runs
/// in has no keyring rows and no way to look any up.
export function credentialsAllowed(profile) {
    if (profile === AGENT_PROFILE) return false;
    return typeof profile === 'string' && profile.trim() !== '';
}

// ---------------------------------------------------------------------
// Rule 3: nothing on the bus reads a credential
// ---------------------------------------------------------------------

/// Words that mean a tool touches stored credentials.
const CREDENTIAL_WORDS = [
    'password', 'passwd', 'passphrase', 'credential', 'keyring', 'secret',
    'login', 'otp', 'totp', 'autofill',
];

/// The tools Surfer serves on the Agent Bus. An ALLOWLIST, matching
/// `app.lisaos.Surfer.json`.
///
/// #260 rule 3 is "the keyring is not readable through any agent tool",
/// and the honest way to hold that is not "we did not add one" — an
/// absence is not a guardrail (CLAUDE.md 6a). It is this list plus the
/// check below, which the socket runs while it wires its handlers.
export const AGENT_TOOLS = Object.freeze([
    'read_page', 'get_selection', 'screenshot', 'navigate', 'click', 'fill',
]);

/// Does this tool name suggest it touches stored credentials?
export function exposesCredentials(name) {
    const n = String(name ?? '').toLowerCase();
    return CREDENTIAL_WORDS.some(w => n.includes(w));
}

/// Throw unless every tool being served is one we meant to serve.
///
/// Called from `McpServer`'s constructor, so the failure mode of adding
/// `read_password` to the tool table is a browser that will not start —
/// loud, immediate, and impossible to miss — rather than a credential
/// read tool quietly appearing on the bus. `fill` is on the allowlist
/// and is the one tool here that could ever aim at a credential; what
/// stops it is `lib/credentials.js`, in the fill script itself.
export function assertNoCredentialTools(names) {
    const list = Array.isArray(names) ? names.map(n => String(n)) : [];
    for (const name of list) {
        if (!AGENT_TOOLS.includes(name)) {
            throw new Error(
                `lisa-surfer: tool ${JSON.stringify(name)} is not in AGENT_TOOLS. ` +
                'Adding a tool means deciding what it may reach; see lib/passwords.js');
        }
        if (exposesCredentials(name)) {
            throw new Error(
                `lisa-surfer: tool ${JSON.stringify(name)} would expose stored ` +
                'credentials to an agent. The keyring is not readable from the bus (#260)');
        }
    }
    return true;
}

// ---------------------------------------------------------------------
// Origins
// ---------------------------------------------------------------------

/// A URL → the origin a credential is keyed by, or `null`.
///
/// `https://host`, `https://host:8443`, `http://localhost:3000`. The
/// default port is dropped so the same site typed two ways is one
/// entry. Anything that is not http or https has no saved password by
/// construction: `file:` and `lisa:` are not sites, and a scheme we do
/// not understand is not one we key on.
export function originOf(input) {
    const raw = String(input ?? '').trim();
    const m = /^([a-z][a-z0-9+.-]*):\/\/([^/?#]*)/i.exec(raw);
    if (!m) return null;
    const scheme = m[1].toLowerCase();
    if (scheme !== 'http' && scheme !== 'https') return null;
    let authority = m[2];
    // Userinfo is not part of an origin. `https://bank.example@evil.test/`
    // reads as the bank to a person and resolves to the attacker, and a
    // keying function that took the first half would hand over the
    // bank's password. LAST `@`, because userinfo may itself contain one.
    const at = authority.lastIndexOf('@');
    if (at >= 0) authority = authority.slice(at + 1);
    if (authority === '') return null;
    let host = authority;
    let port = '';
    const v6 = /^(\[[0-9a-fA-F:.]+\])(?::([0-9]+))?$/.exec(authority);
    if (v6) {
        host = v6[1];
        port = v6[2] ?? '';
    } else {
        const hp = /^([^:]+)(?::([0-9]+))?$/.exec(authority);
        if (!hp) return null;
        host = hp[1];
        port = hp[2] ?? '';
    }
    host = host.toLowerCase();
    if (host === '' || /[\s/\\?#@]/.test(host)) return null;
    if (port !== '' && !/^[0-9]{1,5}$/.test(port)) return null;
    const standard = scheme === 'https' ? '443' : '80';
    if (port === '' || port === standard) return `${scheme}://${host}`;
    return `${scheme}://${host}:${port}`;
}

/// May a credential be SAVED for this origin?
///
/// https, or http on the loopback host. A password typed into an http
/// page has already crossed the wire in the clear, and offering to
/// remember it makes Surfer the second place it leaks from; loopback is
/// exempt because a developer's own machine is not a wire. Autofill
/// needs no separate rule — an entry keyed `https://x` never matches an
/// `http://x` page, because the origins differ.
export function secureOrigin(origin) {
    const o = originOf(origin);
    if (o === null) return false;
    if (o.startsWith('https://')) return true;
    const host = o.slice('http://'.length).split(':')[0];
    return host === 'localhost' || host === '127.0.0.1' || host === '[::1]';
}

// ---------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------

/// One row as the manage surface sees it. The SECRET is never part of
/// this: the list, the search and the labels all work on attributes
/// only, and the password is fetched from the keyring at the moment it
/// is filled. A list that carried secrets would put every saved
/// password in a JS array for as long as the dialog is open.
export function entryOf({origin, username, profile} = {}) {
    const o = originOf(origin);
    if (o === null) return null;
    if (!credentialsAllowed(profile)) return null;
    return {
        origin: o,
        username: typeof username === 'string' ? username.trim() : '',
        profile,
    };
}

/// The keyring attributes for an entry, or `null` if it is not one we
/// would store. Every attribute is a string, because libsecret's
/// schemas are typed and a number here is a lookup that silently
/// matches nothing.
export function keyringAttributes(entry) {
    const e = entryOf(entry ?? {});
    if (e === null) return null;
    return {origin: e.origin, username: e.username, profile: e.profile};
}

/// What a row shows. Never the secret, and never an empty string: an
/// entry with no username is still an entry for a site.
export function entryLabel(entry) {
    const origin = String(entry?.origin ?? '');
    const username = String(entry?.username ?? '').trim();
    if (origin === '') return username || 'unknown site';
    return username === '' ? origin : `${username} — ${origin}`;
}

/// Newest-looking order that is actually stable: by origin, then by
/// username. A list that reorders itself between openings is a list in
/// which people delete the wrong row.
export function sortEntries(list) {
    return (Array.isArray(list) ? list.filter(Boolean) : []).slice().sort((a, b) => {
        const byOrigin = String(a.origin ?? '').localeCompare(String(b.origin ?? ''));
        return byOrigin !== 0
            ? byOrigin
            : String(a.username ?? '').localeCompare(String(b.username ?? ''));
    });
}

/// Substring match over origin and username, case-insensitive. An empty
/// query is the whole list — the dialog opens showing everything.
export function searchEntries(list, query) {
    const items = Array.isArray(list) ? list.filter(Boolean) : [];
    const q = String(query ?? '').trim().toLowerCase();
    if (q === '') return items;
    return items.filter(e =>
        String(e.origin ?? '').toLowerCase().includes(q) ||
        String(e.username ?? '').toLowerCase().includes(q));
}

/// Drop every row matching this (origin, username, profile) — not the
/// first one found. The same rule `forgetUrl` follows, for the same
/// reason: a delete that leaves a duplicate behind is a delete the
/// person will believe happened.
export function removeEntry(list, entry) {
    const key = keyringAttributes(entry);
    if (key === null) return Array.isArray(list) ? list.filter(Boolean) : [];
    return (Array.isArray(list) ? list.filter(Boolean) : []).filter(e =>
        !(e.origin === key.origin && String(e.username ?? '') === key.username &&
          e.profile === key.profile));
}

/// The entries that could fill this page. EXACT origin, never a suffix.
export function matchesFor(list, url, profile) {
    if (!credentialsAllowed(profile)) return [];
    const origin = originOf(url);
    if (origin === null) return [];
    return (Array.isArray(list) ? list.filter(Boolean) : [])
        .filter(e => e.origin === origin && e.profile === profile);
}

// ---------------------------------------------------------------------
// Saving — always asked, never silent
// ---------------------------------------------------------------------

/// What to do about a form that was just submitted with a password in
/// it.
///
/// Three answers and no fourth. `save` and `update` both carry a
/// `prompt`, and there is no answer that writes to the keyring without
/// one: a browser that quietly learns your password is a browser that
/// quietly has it. `tests/passwords.test.js` asserts that property over
/// every outcome rather than over the two it happens to know about.
///
/// `existing` is the ATTRIBUTE list — it carries no secrets, per
/// `entryOf` — so telling "already saved" from "changed" needs the one
/// secret in question, which the caller looks up from the keyring and
/// passes as `existingPassword`. Absent it, a matching entry is treated
/// as an update: offering to re-save something already saved is a
/// harmless extra question, and skipping a real change is a password
/// the person believes is stored and is not.
export function saveDecision({
    url, username, password, existing, existingPassword, profile, agentTouchedAt, now,
} = {}) {
    const decline = (reason) => ({action: 'none', reason});

    // Rule 4 first: the agent profile has no credentials, so there is
    // nothing to ask about.
    if (!credentialsAllowed(profile))
        return decline('this profile does not keep credentials');
    // Rule 2's other half. A form an agent submitted is not a person
    // signing in, and a save prompt is a dialog the model caused to
    // appear over the person's work.
    if (agentDriven({agentTouchedAt, now}))
        return decline('an agent drove this submission');
    const origin = originOf(url);
    if (origin === null)
        return decline('not an http or https page');
    if (!secureOrigin(origin))
        return decline('passwords are only saved for https pages');
    if (typeof password !== 'string' || password === '')
        return decline('no password was submitted');

    const user = typeof username === 'string' ? username.trim() : '';
    const entry = {origin, username: user, profile};
    const prior = (Array.isArray(existing) ? existing.filter(Boolean) : [])
        .find(e => e.origin === origin && String(e.username ?? '') === user &&
                   e.profile === profile);
    if (!prior) {
        return {
            action: 'save',
            entry,
            password,
            prompt: `Save the password for ${user === '' ? origin : `${user} at ${origin}`}?`,
        };
    }
    if (typeof existingPassword === 'string' && existingPassword === password)
        return decline('this password is already saved');
    return {
        action: 'update',
        entry,
        password,
        prompt: `Update the saved password for ${user === '' ? origin : `${user} at ${origin}`}?`,
    };
}

// ---------------------------------------------------------------------
// Rule 2: autofill happens because a person asked
// ---------------------------------------------------------------------

/// Was this a person?
///
/// Every clause fails closed, and `trusted` is compared to `true`
/// rather than coerced, because `{trusted: "yes"}` and `{trusted: 1}`
/// are what a forged gesture looks like when somebody builds one out of
/// JSON.
export function isHumanGesture(gesture, now) {
    const g = gesture && typeof gesture === 'object' ? gesture : null;
    if (g === null) return false;
    if (g.trusted !== true) return false;
    if (!HUMAN_GESTURES.includes(g.kind)) return false;
    if (typeof g.at !== 'number' || !Number.isFinite(g.at) || g.at <= 0) return false;
    if (typeof now !== 'number' || !Number.isFinite(now)) return false;
    if (now < g.at) return false;                      // clock skew: fail closed
    return now - g.at <= GESTURE_WINDOW_MS;
}

/// May this autofill happen?
///
/// The order is the argument. Profile, then gesture, then causation,
/// then whether there is anything to fill — so a refusal never becomes
/// an oracle for *whether a credential exists for this site*, which
/// would be a credential search tool with extra steps (#260 rule 3, and
/// CLAUDE.md 6b's "a refusal must not reveal what exists").
export function autofillVerdict({
    profile, url, gesture, agentTouchedAt, now, entries,
} = {}) {
    const refuse = (reason) => ({fill: false, reason});
    if (!credentialsAllowed(profile))
        return refuse('this profile does not keep credentials');
    if (!isHumanGesture(gesture, now)) {
        return refuse(
            'autofill happens when you ask for it — no tool call, page script or ' +
            'timer can start one');
    }
    if (agentDriven({agentTouchedAt, now}))
        return refuse('an agent action is in flight on this tab');
    const origin = originOf(url);
    if (origin === null)
        return refuse('not an http or https page');
    const matches = matchesFor(entries, url, profile);
    if (matches.length === 0)
        return refuse('nothing saved for this site');
    return {fill: true, origin, matches};
}

// ---------------------------------------------------------------------
// Noticing a sign-in
// ---------------------------------------------------------------------

/// The name of the script-message handler the observer reports through.
export const SUBMIT_HANDLER = 'lisaSurferCredential';

/// The observer that notices a form with a password in it being
/// submitted.
///
/// Injected into the AGENT WORLD, where the handler is registered, for
/// two reasons that are both load-bearing:
///
///   - `window.webkit.messageHandlers.lisaSurferCredential` does not
///     exist in the page's own world, so a page cannot post a
///     submission Surfer never saw. Without that, any page could make
///     the save prompt appear saying whatever it liked — including a
///     different site's name.
///   - the DOM functions it calls are the agent world's, so a page
///     cannot redefine `querySelectorAll` and hand back a different
///     form.
///
/// Capture phase, because a page's own submit handler may
/// `preventDefault` and post the form itself; the credential was still
/// typed either way.
export function submitObserverJs(handler = SUBMIT_HANDLER) {
    const name = JSON.stringify(String(handler));
    return `(() => {
        const post = (detail) => {
            try {
                window.webkit.messageHandlers[${name}].postMessage(JSON.stringify(detail));
            } catch (e) { /* no handler in this world: nothing to report to */ }
        };
        document.addEventListener('submit', (ev) => {
            try {
                const form = ev.target;
                if (!form || !form.querySelectorAll) return;
                const fields = Array.from(form.querySelectorAll('input'));
                const pw = fields.find(f => f.type === 'password' && f.value !== '');
                if (!pw) return;
                const user = fields.find(f =>
                    f !== pw && f.type !== 'password' && f.type !== 'hidden' &&
                    typeof f.value === 'string' && f.value !== '');
                post({
                    url: String(document.location.href || ''),
                    username: user ? String(user.value) : '',
                    password: String(pw.value),
                });
            } catch (e) { /* a page that throws here still gets to submit */ }
        }, true);
    })()`;
}

/// What came back over the message handler → a submission, or `null`.
///
/// Validated rather than trusted. The handler is registered in the
/// agent world only, so nothing hostile should reach it — "should" is
/// why this exists: a shape check costs nothing and the alternative is
/// a save prompt built out of whatever arrived.
export function parseSubmission(raw) {
    let parsed = null;
    if (typeof raw === 'string') {
        try {
            parsed = JSON.parse(raw);
        } catch {
            return null;
        }
    } else if (raw && typeof raw === 'object') {
        parsed = raw;
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
    if (typeof parsed.password !== 'string' || parsed.password === '') return null;
    if (originOf(parsed.url) === null) return null;
    return {
        url: String(parsed.url),
        username: typeof parsed.username === 'string' ? parsed.username : '',
        password: parsed.password,
    };
}

/// The page script that puts a chosen credential into the form.
///
/// Deliberately NOT selector-driven. It works from the password field
/// the person is looking at outwards — nothing chooses a target by
/// name, so there is no argument through which a target could be
/// chosen for it. Runs in the agent world (lib/world.js): the page
/// cannot redefine the DOM functions it calls, and cannot see the
/// script.
///
/// Note what this does NOT protect against and cannot: once the value
/// is in the field, the page has it. That is what filling a form means,
/// and it is why the gesture requirement is the boundary — a person
/// choosing to sign in accepts that the site learns the password they
/// are signing in with.
///
/// The username field is found by its declared spellings first
/// (`autocomplete=username`, `type=email`, a name or id containing
/// `user` or `email`) and then by falling back to the first ordinary
/// text field in the same form. The fallback is not decoration: on the
/// reference device a form whose username input was called `alias` got
/// its password filled and its username left blank, which is a sign-in
/// that fails for a reason nobody can see. Nothing about the fallback
/// is a security relaxation — the person chose this entry for this
/// form, and the only field it can reach is one in that form.
export function autofillScript(username, password) {
    const lit = (v) => JSON.stringify(String(v ?? '')).replace(/</g, '\\u003c');
    return `(() => {
        try {
            const active = document.activeElement;
            const scope = (active && (active.form ||
                (active.closest ? active.closest('form') : null))) || document;
            const pw = scope.querySelector('input[type=password i]');
            if (!pw)
                return JSON.stringify({filled: false, reason: 'no password field here'});
            const setValue = (el, v) => {
                el.value = v;
                el.dispatchEvent(new Event('input', {bubbles: true}));
                el.dispatchEvent(new Event('change', {bubbles: true}));
            };
            // Declared spellings first, then the fallback. See the note
            // above this function.
            const user = scope.querySelector(
                'input[autocomplete~=username i], input[type=email i], ' +
                'input[name*=user i], input[id*=user i], input[name*=email i]') ||
                Array.from(scope.querySelectorAll('input')).find(i =>
                    i !== pw && (i.type === 'text' || i.type === 'email'));
            const wanted = ${lit(username)};
            if (user && wanted !== '') setValue(user, wanted);
            setValue(pw, ${lit(password)});
            return JSON.stringify({filled: true, username: user ? true : false});
        } catch (e) {
            return JSON.stringify({filled: false, reason: String(e.message || e)});
        }
    })()`;
}
