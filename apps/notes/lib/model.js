// Notes' view model: pure, so it tests under node without gi://.
//
// The window is thin on purpose. Everything that can be decided without
// a widget is decided here, because this is the part a test can reach —
// the same split apps/mail/lib and shell/assistant/lib already use.

/// Notes as the list should show them: newest first, with a one-line
/// preview that does not depend on the body's formatting.
export function ordered(notes) {
    return [...(notes ?? [])].sort((a, b) => {
        const ta = Date.parse(a?.updated_at ?? a?.created_at ?? 0) || 0;
        const tb = Date.parse(b?.updated_at ?? b?.created_at ?? 0) || 0;
        if (tb !== ta)
            return tb - ta;
        // Same timestamp — order by id so the list never reshuffles
        // between two identical reloads. A list that changes order when
        // nothing changed reads as data loss.
        return String(a?.id ?? '').localeCompare(String(b?.id ?? ''));
    });
}

/// One line of body, collapsed, for the row under the title.
///
/// Newlines become spaces rather than being cut at the first one: a
/// note whose first line is blank would otherwise preview as empty and
/// look like it had lost its content.
export function preview(body, limit = 80) {
    const flat = String(body ?? '').replace(/\s+/g, ' ').trim();
    if (flat.length <= limit)
        return flat;
    return `${flat.slice(0, limit - 1).trimEnd()}…`;
}

/// The title to show for a note that has none.
///
/// Untitled notes are normal — you type the body first. Showing an
/// empty row is what makes people think the save failed.
export function displayTitle(note) {
    const t = String(note?.title ?? '').trim();
    if (t)
        return t;
    const p = preview(note?.body, 40);
    return p || 'Untitled note';
}

/// Is this note worth saving? Used to decide whether closing the editor
/// creates one.
///
/// Both fields empty means the person opened the editor and changed
/// their mind; saving that leaves litter they then have to delete.
export function isWorthSaving({title, body}) {
    return Boolean(String(title ?? '').trim() || String(body ?? '').trim());
}

/// Client-side filter for the search box.
///
/// The BACKEND has search_notes, and it is the better answer for a real
/// query — it is indexed and it is what the agent uses. This is for
/// filtering a list already on screen as you type, where a round trip
/// per keystroke would make the list flicker.
export function matches(note, query) {
    const q = String(query ?? '').trim().toLowerCase();
    if (!q)
        return true;
    const hay = `${note?.title ?? ''} ${note?.body ?? ''}`.toLowerCase();
    return hay.includes(q);
}
