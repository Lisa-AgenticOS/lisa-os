// Downloads — every decision, none of the plumbing (#146 follow-up).
//
// # Why this is a security module and not a convenience one
//
// A download is a WRITE TO DISK, driven by a remote server. Three things
// have to be impossible, and they are the three reasons this is a tested
// module rather than a signal handler:
//
//   1. **A page choosing where its bytes land.** `suggested_filename`
//      comes from `Content-Disposition`, which is attacker-controlled
//      text. `../../.config/autostart/x.desktop` is a filename as far as
//      the header is concerned. `safeFilename` reduces anything to a
//      single path component with no separators, and `destinationPath`
//      refuses to join one that still has any — belt and braces, because
//      the interesting failure is the one where the first check is
//      changed and the second is not.
//
//   2. **A download silently replacing a file that is already there.**
//      `destinationFor` NEVER returns `save` for a path that exists. A
//      conflict is a question for the person, and `resolveConflict`
//      defaults to `cancel` for any answer it does not recognise.
//
//   3. **An agent causing either.** `agentDriven` is the deterministic
//      test the window applies before a download is allowed to start.
//      See the note at the bottom of this file — it is the reason Surfer
//      has no `download` tool.
//
// No gi:// import: every rule here runs under `just shell-test` on any
// host, and `exists` is injected so the conflict rule can be tested
// without a filesystem.

/// How many rows the downloads list keeps. Old rows are history, not
/// state — the files are still on disk.
export const DOWNLOAD_LIMIT = 200;

/// The longest filename we will write. ext4's limit is 255 BYTES, and a
/// UTF-8 name is up to four bytes a character; 200 characters of the
/// worst case is over that, so the cap is applied to the encoded length
/// where the runtime can measure it and to the character count where it
/// cannot.
const MAX_FILENAME = 120;

/// Characters that must not survive into a filename: path separators,
/// NUL, and the C0/C1 control ranges. `/` and NUL are the only two Linux
/// actually forbids; the rest go because a filename you cannot type is a
/// file you cannot delete, and because a `\r` in a name is how a
/// directory listing gets rewritten under somebody's cursor.
///
/// TWO constants for one class, on purpose: the `g` flag makes `.test()`
/// stateful — it advances `lastIndex` and answers `false` on every other
/// call — and a security check that is right on alternate invocations is
/// worse than no check at all. Built from a string so the source file
/// holds no literal control characters; the first version of this line
/// did, and `grep` reported downloads.js as a binary file.
const UNSAFE_CLASS = '/\\\\\\x00-\\x1f\\x7f-\\x9f';
const UNSAFE_G = new RegExp('[' + UNSAFE_CLASS + ']', 'g');
const UNSAFE = new RegExp('[' + UNSAFE_CLASS + ']');

/// The last path component of a URI, decoded — the fallback when a
/// server suggests nothing.
function nameFromUri(uri) {
    const u = String(uri ?? '');
    // Strip query and fragment first: `?v=2` is not part of a name.
    const bare = u.split('#')[0].split('?')[0];
    // Then the scheme and authority. For `https://example.org/` the last
    // slash-separated piece is the HOST, and naming a download after the
    // site it came from is not naming it after the file.
    const authority = /^[a-z][a-z0-9+.-]*:\/\/[^/]*(\/.*)?$/i.exec(bare);
    const path = authority ? (authority[1] ?? '') : bare;
    const seg = path.split('/').filter(s => s !== '').pop() ?? '';
    try {
        return decodeURIComponent(seg);
    } catch {
        // A malformed percent-escape is not worth failing a download over.
        return seg;
    }
}

/// Cut a name to length without eating its extension.
function capLength(name) {
    if (name.length <= MAX_FILENAME) return name;
    const dot = name.lastIndexOf('.');
    // Only treat it as an extension if it is short and not the whole name.
    if (dot > 0 && name.length - dot <= 12) {
        const ext = name.slice(dot);
        return name.slice(0, MAX_FILENAME - ext.length) + ext;
    }
    return name.slice(0, MAX_FILENAME);
}

/// The filename a download may use: one path component, always.
///
/// Order matters. The separator strip happens AFTER the URI fallback is
/// decoded, because `%2F` decodes to `/` and a decode that happens after
/// the strip hands back exactly the separator that was removed.
export function safeFilename(suggested, uri) {
    let name = typeof suggested === 'string' ? suggested : '';
    if (name.trim() === '') name = nameFromUri(uri);
    name = name.replace(UNSAFE_G, '');
    // Leading dots: `.` and `..` are the traversal, and any other
    // leading dot makes a file the person's file manager hides. A page
    // does not get to decide that its download is invisible.
    name = name.replace(/^\.+/, '');
    name = name.trim();
    if (name === '') return 'download';
    return capLength(name);
}

/// Join a directory and a filename that has already been through
/// `safeFilename`.
///
/// Throws rather than sanitising: by this point a separator means a
/// caller skipped the vetting step, and quietly fixing it up is how the
/// vetting step gets deleted as redundant six months later.
export function destinationPath(dir, filename) {
    const d = String(dir ?? '');
    const f = String(filename ?? '');
    if (d === '') throw new Error('no download directory');
    if (f === '' || f === '.' || f === '..' || UNSAFE.test(f)) {
        throw new Error(
            `refusing to build a path from ${JSON.stringify(f)}: ` +
            'a download filename is one path component and nothing else');
    }
    return `${d.replace(/\/+$/, '')}/${f}`;
}

/// `name.ext` → `name (1).ext`, `name (2).ext`, …
function numbered(filename, n) {
    const dot = filename.lastIndexOf('.');
    if (dot > 0 && filename.length - dot <= 12)
        return `${filename.slice(0, dot)} (${n})${filename.slice(dot)}`;
    return `${filename} (${n})`;
}

/// The first path under `dir` that `exists()` says is free.
///
/// Bounded, and it throws at the bound rather than looping: a thousand
/// files by one name is a bug or an attack, and either way the answer is
/// not to keep trying.
export function uniquePath(dir, filename, exists) {
    const first = destinationPath(dir, filename);
    if (!exists(first)) return first;
    for (let n = 1; n < 1000; n++) {
        const candidate = destinationPath(dir, numbered(filename, n));
        if (!exists(candidate)) return candidate;
    }
    throw new Error(`a thousand files are already named like ${JSON.stringify(filename)}`);
}

/// Where a download should go, and whether anybody has to be asked.
///
/// Two outcomes and no third:
///
///   `{action: 'save', path}`      nothing is there; write it.
///   `{action: 'conflict', …}`     something IS there; ask.
///
/// **`save` is never returned for a path that exists.** That is the
/// whole contract, and it is what `tests/downloads.test.js` mutates
/// first.
export function destinationFor({suggested, uri, dir, exists}) {
    const filename = safeFilename(suggested, uri);
    const path = destinationPath(dir, filename);
    if (!exists(path))
        return {action: 'save', path, filename};
    return {
        action: 'conflict',
        path,                                   // what the server asked for
        filename,
        suggestion: uniquePath(dir, filename, exists), // the free name next to it
    };
}

/// The person's answer to a conflict.
///
/// Default deny: anything that is not one of the two affirmative
/// answers — a dialog dismissed with Escape, a close button, a value
/// nobody thought about — is a cancel. `allowOverwrite` is handed back
/// explicitly so the caller sets WebKit's `allow-overwrite` from a
/// decision rather than from a default (it defaults FALSE, and a
/// download that fails because the file exists is a download the person
/// asked to replace failing for no visible reason).
export function resolveConflict(choice, decision) {
    if (choice === 'keep-both' && decision?.suggestion)
        return {action: 'save', path: decision.suggestion, allowOverwrite: false};
    if (choice === 'replace' && decision?.path)
        return {action: 'save', path: decision.path, allowOverwrite: true};
    return {action: 'cancel'};
}

// ---------------------------------------------------------------------
// The agent boundary
// ---------------------------------------------------------------------

/// Did an agent cause this download?
///
/// Surfer exposes no `download` tool (see below), but `navigate` and
/// `click` are enough: an http URL that answers
/// `Content-Disposition: attachment` starts a download, and a page the
/// model was steered by can be that URL. So the window stamps a view
/// whenever an agent-driven action touches it, and a download that
/// starts inside the stamp's lifetime is cancelled.
///
/// **The rule itself moved to `lib/causation.js` (#260)** and is
/// re-exported here rather than copied: passwords need the identical
/// question — was this submit, was this autofill, an agent's doing — and
/// two timers with two skew rules is two things to keep correct. The
/// download name stays because that is what the download code, its
/// tests and the README all call it.
export {agentDriven} from './causation.js';
export {AGENT_ACTION_WINDOW_MS as AGENT_DOWNLOAD_WINDOW_MS} from './causation.js';

// ---------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------

/// A new row for a download that has just started.
export function startedDownload({id, uri, filename, path, startedAt}) {
    return {
        id: String(id),
        uri: String(uri ?? ''),
        filename: String(filename ?? ''),
        path: String(path ?? ''),
        state: 'running',
        received: 0,
        total: 0,
        startedAt: typeof startedAt === 'number' ? startedAt : 0,
        endedAt: 0,
    };
}

function patch(list, id, fn) {
    const items = Array.isArray(list) ? list : [];
    return items.map(e => (e && e.id === String(id) ? fn(e) : e));
}

export function updateDownload(list, id, {received, total, path} = {}) {
    return patch(list, id, e => ({
        ...e,
        received: typeof received === 'number' ? received : e.received,
        total: typeof total === 'number' && total > 0 ? total : e.total,
        path: typeof path === 'string' && path !== '' ? path : e.path,
    }));
}

export function completeDownload(list, id, at = 0) {
    return patch(list, id, e => ({
        ...e,
        state: 'done',
        endedAt: at,
        // A finished download's total is what it actually received;
        // servers that send no Content-Length would otherwise leave the
        // row reading "4.2 MB of 0 B" forever.
        total: e.total > 0 ? e.total : e.received,
    }));
}

export function failDownload(list, id, reason, at = 0) {
    return patch(list, id, e => ({
        ...e,
        state: 'failed',
        reason: String(reason ?? 'failed'),
        endedAt: at,
    }));
}

/// Drop one row. The FILE is not touched — a downloads list is a record
/// of what happened, and a browser that deletes your files when you
/// tidy its list is a browser nobody tidies twice.
export function removeDownload(list, id) {
    return (Array.isArray(list) ? list : []).filter(e => e && e.id !== String(id));
}

/// Drop every row that is not still running.
export function clearFinished(list) {
    return (Array.isArray(list) ? list : []).filter(e => e && e.state === 'running');
}

export function trimDownloads(list, max = DOWNLOAD_LIMIT) {
    const items = Array.isArray(list) ? list : [];
    return items.length <= max ? items : items.slice(0, max);
}

/// What is worth writing to disk.
///
/// A `running` row is a fact about THIS process. Persisting it would
/// bring back a progress bar for a transfer that no longer exists, so it
/// is written down as what it became: interrupted.
export function persistableDownloads(list) {
    return (Array.isArray(list) ? list : []).map(e => (
        e && e.state === 'running'
            ? {...e, state: 'failed', reason: 'interrupted'}
            : e));
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function formatBytes(n) {
    let v = typeof n === 'number' && Number.isFinite(n) && n > 0 ? n : 0;
    let u = 0;
    while (v >= 1024 && u < UNITS.length - 1) { v /= 1024; u += 1; }
    const rounded = u === 0 ? Math.round(v) : Math.round(v * 10) / 10;
    return `${rounded} ${UNITS[u]}`;
}

/// The one line a downloads row shows under its filename.
export function downloadLabel(entry) {
    const e = entry ?? {};
    if (e.state === 'done') return formatBytes(e.total || e.received);
    if (e.state === 'failed') return `Failed — ${e.reason ?? 'unknown error'}`;
    if (e.total > 0)
        return `${formatBytes(e.received)} of ${formatBytes(e.total)}`;
    return formatBytes(e.received);
}

/// 0..1 for a progress bar, or `null` when the server sent no length —
/// which is a pulsing bar, not a bar stuck at zero.
export function downloadFraction(entry) {
    const e = entry ?? {};
    if (e.state === 'done') return 1;
    if (!(e.total > 0)) return null;
    return Math.min(1, Math.max(0, e.received / e.total));
}

// ---------------------------------------------------------------------
// Why there is no `download` tool on the Agent Bus
// ---------------------------------------------------------------------
//
// It would be one manifest row and about ten lines of window code, and
// it is deliberately absent.
//
// A `download` tool is not "a browser feature the model can use". It is
// **arbitrary bytes, from an address the model chose, written to a path
// on the person's disk** — a primitive the guard's path rules
// (cli/lisa/src/guard.rs) were never shaped for, because nothing else in
// the system hands the model a write of unbounded content. The tier
// machinery would call it `write`, and `write` currently means "changes
// something the user can see and undo"; a file in ~/Downloads is neither
// visible at the moment it lands nor undone by anything.
//
// The narrower rule is the one implemented above: an agent cannot cause
// a download even indirectly, through `navigate` or `click` at a URL
// that answers with an attachment. That is enforced by `agentDriven` in
// deterministic code the model cannot reach (CLAUDE.md 6a), not by a
// tool being absent from a list.
//
// If this is revisited, the thing that changes it is not a browser
// argument: it is a system answer to "where may a model write, and how
// does the person see it afterwards", and that is an ADR.
