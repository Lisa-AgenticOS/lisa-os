// What the Assistant remembers about you, as this window needs to see it
// (#157, ADR-0025 phase 4).
//
// Pure: parsing, ordering and wording. The D-Bus calls live in the
// window; everything that can be got wrong is here, where a test can
// reach it.
//
// # Why this surface is not optional
//
// Cross-conversation memory means the assistant carries facts about a
// person from one conversation into the next, indefinitely, without
// being asked again. That is only a feature while the person can see the
// list and take things off it. Without this pane the same code is a
// dossier, so the pane is part of the feature rather than a follow-up:
// `MemoryList`/`MemoryForget` exist on `dev.lisaos.Harness1` and this is
// what calls them.
//
// # Provenance is shown, not implied
//
// Every note carries where it was learned. `user` means the person said
// it; anything else means it came from content — a web page, a mail
// body, a scheduled run — and the assistant's own guardrails already
// treat it as untrusted (harnessd escalates any privileged call made by
// a run that read one). Showing the tag is what lets a person recognise
// the note that should not be there.

/// Its own interface XML rather than the shared one in
/// `shell/overlay-extension/lib/iface.js`, which describes what the
/// OVERLAY needs from the harness. Memory is this window's business:
/// putting it in the shared node would offer it to a surface with no
/// list to show it in.
export const MEMORY_IFACE_XML = `
<node>
  <interface name="dev.lisaos.Harness1">
    <method name="MemoryList">
      <arg type="s" direction="out" name="notes_json"/>
    </method>
    <method name="MemoryForget">
      <arg type="x" direction="in" name="note_id"/>
      <arg type="b" direction="out" name="forgotten"/>
    </method>
    <method name="MemoryForgetAll">
      <arg type="u" direction="out" name="count"/>
    </method>
  </interface>
</node>`;

/// `MemoryList()`'s payload → rows to render.
///
/// Anything unreadable yields an empty list rather than throwing: a
/// person opening this pane is asking a question about their own
/// privacy, and an exception dialog answers it with "something went
/// wrong", which is the least useful possible answer. A row missing its
/// text is dropped; a row missing its provenance is shown as `unknown`
/// and NOT as trusted, because that is the direction that fails safe.
export function parseNotes(payload) {
    let rows;
    try {
        rows = typeof payload === 'string' ? JSON.parse(payload) : payload;
    } catch {
        return [];
    }
    if (!Array.isArray(rows))
        return [];
    return rows
        .filter(r => r && typeof r.text === 'string' && r.text.length > 0)
        .map(r => ({
            id: Number(r.id),
            text: r.text,
            provenance:
                typeof r.provenance === 'string' && r.provenance
                    ? r.provenance
                    : 'unknown',
            // `=== true`, not truthiness: a missing field must read as
            // untrusted, and `undefined` is falsy but `"no"` is not.
            trusted: r.trusted === true,
            ts: Number(r.ts) || 0,
        }))
        .filter(r => Number.isFinite(r.id));
}

/// The line under a note saying where it came from.
///
/// A trusted note gets no line at all. Labelling the ordinary case
/// teaches people to ignore the label, and the label is only worth
/// having on the note that should make them look twice.
export function provenanceNote(note) {
    if (note.trusted)
        return '';
    switch (note.provenance) {
    case 'web':
        return 'Learned from a web page, not from you.';
    case 'mail':
        return 'Learned from a message, not from you.';
    case 'file':
        return 'Learned from a file, not from you.';
    case 'screen':
        return 'Learned from something on screen, not from you.';
    case 'schedule':
    case 'event':
        return 'Learned during a run you did not start.';
    default:
        return 'Where this was learned is not recorded — treated as untrusted.';
    }
}

/// What the pane says when there is nothing to show.
///
/// Two different nothings, and conflating them is the #228 defect in a
/// new place: "I remember nothing about you" and "I could not read what
/// I remember" are opposite reassurances.
export function emptyText(readOk) {
    return readOk
        ? 'Nothing yet. The assistant adds a note when something is worth ' +
          'carrying between conversations.'
        : 'What the assistant remembers could not be read just now. This ' +
          'is not the same as it remembering nothing.';
}

/// The body of the "forget everything" confirmation.
export function forgetAllBody(count) {
    if (count === 1)
        return 'The one thing the assistant remembers about you is removed.';
    return `All ${count} things the assistant remembers about you are ` +
        'removed. Conversations themselves are not touched.';
}

/// Newest first, and stable when two notes share a timestamp.
///
/// The daemon already returns them this way; sorting again is not
/// mistrust, it is that this list is the one place a person audits, and
/// an order that depends on an unstated server-side promise is an order
/// that will silently change.
export function sortNotes(notes) {
    return [...notes].sort((a, b) => b.ts - a.ts || b.id - a.id);
}
