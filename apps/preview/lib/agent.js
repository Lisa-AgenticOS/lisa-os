// Argument validation for Preview's write- and destructive-tier tools —
// pure, so it tests under node in `just shell-test` and so the refusals
// are readable in one place.
//
// This module is NOT the guard, and must not be mistaken for it. Scope
// (is this path inside the caller's home? another user's file? a folder
// the owner put out of bounds?) is decided by `libs/lisa-guard` in Rust,
// deterministically, before the call ever reaches this process — rule 6a.
// Duplicating those checks here would create a second copy of a policy
// to drift out of step with the first.
//
// What IS here is the app's own contract, the part only Preview knows:
//   - a page number the model invented must land in a validated error,
//     not inside poppler as `get_page(undefined)` (#198);
//   - a rotation must be a quarter turn, because PDF /Rotate has no
//     other values and qpdf would refuse the rest at save time, after
//     the user already confirmed;
//   - **an export never overwrites**. A destructive effect reached
//     through a write-tier tool is a tier that lies, so the write tool
//     is made unable to have it rather than the tier being raised: the
//     model gets "that file exists", and the file survives.

/// A 1-based display page from the wire -> `{value}` 0-based, or
/// `{error}`. `1.5`, `"2"`, `true` and `NaN` are all refused: JSON has
/// one number type and a caller that sends a float means something we
/// cannot honour.
export function pageArg(page, pageCount, name = 'page') {
    if (!(pageCount > 0)) return {error: 'no document open'};
    const p = page === undefined ? 1 : page;
    if (typeof p !== 'number' || !Number.isInteger(p) || p < 1 || p > pageCount)
        return {error: `${name} must be an integer 1..${pageCount}`};
    return {value: p - 1};
}

/// A rotation delta -> `{value}` normalized to 90/180/270, or `{error}`.
///
/// 0 and 360 are refused rather than treated as a no-op: a tool call
/// that does nothing and reports success is how a model concludes it
/// rotated a page it did not rotate.
export function rotationArg(degrees) {
    const d = degrees === undefined ? 90 : degrees;
    if (typeof d !== 'number' || !Number.isInteger(d) || d % 90 !== 0)
        return {error: 'degrees must be a whole number of quarter turns (90, 180, 270, -90)'};
    const norm = ((d % 360) + 360) % 360;
    if (norm === 0) return {error: 'degrees must not be a whole turn — that would change nothing'};
    return {value: norm};
}

/// `from`/`to` display pages for a move -> `{from, to}` 0-based, or
/// `{error}`. A move onto itself is refused for the same reason a
/// whole-turn rotation is.
export function moveArg(from, to, pageCount) {
    const f = pageArg(from, pageCount, 'from');
    if (f.error) return f;
    const t = pageArg(to, pageCount, 'to');
    if (t.error) return t;
    if (f.value === t.value) return {error: 'from and to are the same page'};
    return {from: f.value, to: t.value};
}

/// Where an agent-driven export may write — the decisions a string can
/// answer on its own.
///
/// It does NOT ask whether the file exists, and that omission is the fix
/// for #299. It used to take an `exists` boolean the caller computed
/// with `GLib.file_test`, which is check-then-act twice over: the answer
/// is stale by the time the write happens, and `G_FILE_TEST_EXISTS`
/// answers *false* for a dangling symlink, so the write went through the
/// link to a path the guard never judged. Never-overwrite is now
/// enforced by creating the file exclusively (O_CREAT|O_EXCL, which
/// POSIX requires to fail on a symlink whether or not it dangles) and
/// [`exportExistsError`] is the refusal that failure becomes. A rule the
/// filesystem enforces cannot be raced; a rule a boolean enforces can.
///
/// Refusals, in order, and each for its own reason:
///   - not a string / empty: nothing to check;
///   - relative: the app has no working directory a caller could mean,
///     and inventing one is how a file lands somewhere nobody looks;
///   - a `..` segment: a path this module cannot reason about must be
///     refused, not normalized-and-hoped — and the guard upstream judged
///     the string as written, so silently resolving it here would move
///     the target out from under the decision that approved it;
///   - wrong extension: `photo.png` written by the JPEG writer is a file
///     every other program on the machine will open wrong.
export function exportTarget(path, ext) {
    if (typeof path !== 'string' || path.trim() === '')
        return {error: 'path is required'};
    if (!path.startsWith('/'))
        return {error: 'path must be absolute'};
    if (path.split('/').includes('..'))
        return {error: 'path must not contain ".."'};
    const base = path.split('/').pop();
    const dot = base.lastIndexOf('.');
    const have = dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
    if (have !== ext)
        return {error: `path must end in .${ext} for a ${ext} export`};
    return {value: path};
}

/// The refusal an exclusive create's EEXIST becomes: never, ever
/// overwrite, phrased so the model can pick another name instead of
/// retrying the same one.
export function exportExistsError(path) {
    const base = String(path).split('/').pop();
    return {error: `${base} already exists — Preview never overwrites; choose another name`};
}

/// The export format asked for -> the entry from `exportFormats`, or an
/// error naming what this machine can actually write.
///
/// The available list is the one asked of GdkPixbuf at startup (#146):
/// a model that asks for AVIF on a host without the writer gets told so,
/// rather than a save that fails deep inside pixbuf.
export function formatArg(key, available) {
    const names = available.map(f => f.key);
    if (typeof key !== 'string' || !names.includes(key))
        return {error: `format must be one of: ${names.join(', ') || '(none available)'}`};
    return {value: available.find(f => f.key === key)};
}
