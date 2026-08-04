// Unit tests for the Assistant's file-chooser outcomes (issue #234).
//
// The defect these encode: three callbacks in the window collapsed a
// null `Gio.File.get_path()` into "nothing to do". A null path is what
// GIO returns for every location that is not a file on this machine, so
// attaching from a Drive mount did nothing, choosing a non-local working
// folder SILENTLY REVOKED the grant, and export reported it as
// "Dismissed". Three different things, one silent branch.
import {test, assertEq, assert, finish} from '../../testing/harness.js';
import {chosenPath, remoteLocationNote} from '../lib/chooser.js';

/// A Gio.File stand-in. `path` null is exactly what GIO hands back for
/// anything the local filesystem cannot name.
function fakeFile(path, uri) {
    return {get_path: () => path, get_uri: () => uri};
}

test('a dismissed dialog is a dismissal', () => {
    assertEq(chosenPath(null), {kind: 'dismissed'});
    assertEq(chosenPath(undefined), {kind: 'dismissed'});
});

test('a local file comes back with its path', () => {
    assertEq(chosenPath(fakeFile('/home/me/shot.png', 'file:///home/me/shot.png')),
        {kind: 'local', path: '/home/me/shot.png'});
});

/// The whole of #234. A file the person really did pick, which has no
/// local path, is NOT a dismissal — and the difference has to survive
/// out of this function, because every caller acts on it differently.
test('a chosen location with no local path is not a dismissal', () => {
    const drive = fakeFile(null, 'google-drive://me@example.com/0ABxyz/report.png');
    assertEq(chosenPath(drive).kind, 'remote');
    assertEq(chosenPath(drive).uri, 'google-drive://me@example.com/0ABxyz/report.png');

    const sftp = fakeFile(null, 'sftp://build.local/srv/shot.png');
    assertEq(chosenPath(sftp).kind, 'remote');
});

/// A GFile with no URI either (a stale recent-files entry) is still a
/// choice, not a dismissal: something was picked and could not be used.
test('no path and no uri is still a choice, not a dismissal', () => {
    assertEq(chosenPath(fakeFile(null, null)), {kind: 'remote', uri: ''});
});

test('the note says where, why, and what would work', () => {
    const note = remoteLocationNote('attach', 'sftp://build.local/x.png');
    assert(note.includes('sftp://build.local/x.png'), `no location in: ${note}`);
    assert(note.includes('not a file on this machine'), `no reason in: ${note}`);
    assert(/local folder/i.test(note), `no way forward in: ${note}`);
    // The verb is the caller's, so one note serves attach/work in/save to.
    assert(remoteLocationNote('save to', '').startsWith('Cannot save to'),
        remoteLocationNote('save to', ''));
});

finish();
