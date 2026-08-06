// Notes' view model: pure, so it tests under node without gi://.
//
// The window is thin on purpose. Everything that can be decided without
// a widget is decided here, because this is the part a test can reach —
// the same split apps/mail/lib and shell/assistant/lib already use.

/// When a note was last touched, as milliseconds, or 0 if unknowable.
///
/// `created` is in that list because it is the field `lisa-notes`
/// ACTUALLY sends. This function used to read `updated_at ?? created_at`
/// only — names invented at the keyboard — so on real data every note
/// scored 0, the sort was a no-op, and the list had been in id order
/// since it was written. Nothing failed; it just was not sorted.
///
/// Same root cause as the `structuredContent` decode bug the first
/// render found: a shape assumed instead of read.
export function timeOf(note) {
    const raw = note?.updated_at ?? note?.updated ?? note?.created_at ?? note?.created;
    return Date.parse(raw ?? '') || 0;
}

/// Notes as the list should show them: newest first, with a one-line
/// preview that does not depend on the body's formatting.
export function ordered(notes) {
    return [...(notes ?? [])].sort((a, b) => {
        const ta = timeOf(a);
        const tb = timeOf(b);
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

/// Group notes into the periods a person actually thinks in, the way
/// macOS Notes does: Today, Yesterday, Previous 7 Days, Previous 30
/// Days, then month names within this year, then years.
///
/// `now` is a parameter, not `Date.now()`, so this is testable without
/// freezing a clock — and so a window left open overnight regroups when
/// it reloads rather than insisting yesterday is still today.
///
/// Returns `[{label, notes}]` in display order, skipping empty groups.
/// A note with no usable date lands in "Older" rather than being
/// dropped: losing a note from the list because its timestamp was
/// malformed is the worst possible way to handle bad data.
export function groupByPeriod(notes, now = new Date()) {
    const t = now instanceof Date ? now : new Date(now);
    const startOfDay = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const today = startOfDay(t);
    const day = 86400000;

    const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June',
        'July', 'August', 'September', 'October', 'November', 'December'];

    const buckets = new Map();
    const push = (label, note) => {
        if (!buckets.has(label))
            buckets.set(label, []);
        buckets.get(label).push(note);
    };

    for (const note of ordered(notes)) {
        const ms = timeOf(note);
        if (!ms) {
            push('Older', note);
            continue;
        }
        const d = new Date(ms);
        const dayStart = startOfDay(d);
        if (dayStart === today)
            push('Today', note);
        else if (dayStart === today - day)
            push('Yesterday', note);
        else if (dayStart > today - 7 * day)
            push('Previous 7 Days', note);
        else if (dayStart > today - 30 * day)
            push('Previous 30 Days', note);
        else if (d.getFullYear() === t.getFullYear())
            push(MONTHS[d.getMonth()], note);
        else
            push(String(d.getFullYear()), note);
    }

    // Display order: the named recents first, then months newest-first
    // within this year, then years descending, then the unparseable.
    const fixed = ['Today', 'Yesterday', 'Previous 7 Days', 'Previous 30 Days'];
    const out = [];
    for (const label of fixed)
        if (buckets.has(label))
            out.push({label, notes: buckets.get(label)});
    for (const m of [...MONTHS].reverse())
        if (buckets.has(m))
            out.push({label: m, notes: buckets.get(m)});
    const years = [...buckets.keys()].filter((k) => /^\d{4}$/.test(k))
        .sort((a, b) => Number(b) - Number(a));
    for (const y of years)
        out.push({label: y, notes: buckets.get(y)});
    if (buckets.has('Older'))
        out.push({label: 'Older', notes: buckets.get('Older')});
    return out;
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
