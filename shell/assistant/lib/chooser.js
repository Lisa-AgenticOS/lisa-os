// Lisa Assistant — what a file chooser actually handed back. Pure logic,
// no GNOME imports, like the rest of lib/: runs under gjs (the app) and
// node/jsc (unit tests on any dev host).
//
// The window's three choosers each end in the same two-line shape:
//
//     const file = d.open_finish(res);
//     if (file) this._attachPath(file.get_path());
//
// and `get_path()` returns NULL for anything that is not a file on this
// machine — a Google Drive mount, an sftp:// share, a gphoto2:// camera,
// a recent-files entry that has gone stale. Nothing in the shape says
// so, so all three read that null as "nothing to do" (#234): attaching
// from Drive did nothing at all, choosing a non-local working folder
// SILENTLY REVOKED the grant, and export swallowed it under "Dismissed".
//
// This module exists so the distinction has a name and a test. A chooser
// callback has three outcomes and they are not interchangeable.

/**
 * What came back from a `Gtk.FileDialog` callback.
 *
 * `file` is duck-typed rather than imported: `Gio.File` here, a stub in
 * the tests, and this module stays runnable on a dev host with no GTK.
 *
 * @param {?{get_path: function(): ?string, get_uri: function(): ?string}} file
 * @returns {{kind: 'dismissed'}|{kind: 'local', path: string}|{kind: 'remote', uri: string}}
 *   `dismissed` — the person closed the dialog; do nothing, quietly.
 *   `local`     — a real path on this machine.
 *   `remote`    — a real choice that has no local path. NOT a dismissal:
 *                 the person picked something and is owed an answer.
 */
export function chosenPath(file) {
    if (!file)
        return {kind: 'dismissed'};
    const path = file.get_path?.() ?? null;
    if (path)
        return {kind: 'local', path};
    // A file object with no local path: the person DID choose, and this
    // window cannot open it. Reporting that as a dismissal is the whole
    // of #234 — three callers then did nothing, quietly, in three
    // different wrong ways.
    return {kind: 'remote', uri: file.get_uri?.() ?? ''};
}

/**
 * What to tell someone who picked a location this window cannot read or
 * write. Says WHERE, says WHY, and says what would work — a note that
 * only says "failed" costs the same round trip as saying nothing.
 *
 * @param {string} verb  what was being attempted: 'attach', 'work in', 'save to'
 * @param {string} uri   the location they picked, or '' if unknown
 * @returns {string}
 */
export function remoteLocationNote(verb, uri) {
    const where = uri ? `“${uri}”` : 'that location';
    return `Cannot ${verb} ${where} — it is not a file on this machine. ` +
        'Copy it to a local folder first (Files can do that), then try again.';
}
