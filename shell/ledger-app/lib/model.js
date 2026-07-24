// Ledger app — pure view-model logic (PLAN §5.7.6).
//
// Input is the CLI's JSON (`lisa ledger --json`), i.e. serialized
// lisa-ledger Entry rows, newest first: {id, ts, kind, app_id, model,
// input_hash, preview, status, detail, ref_id, output_tokens,
// duration_ms}. Completion entries (kind "*.complete") reference their
// start entry via ref_id; the timeline shows start entries enriched
// with their completion. No GTK imports — unit-tests under
// gjs/node/jsc.

/** @param {number} ms @returns {string} local YYYY-MM-DD */
export function dayOf(ms) {
    const d = new Date(ms);
    const pad = n => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** @param {number} ms @returns {string} local YYYY-MM-DD HH:MM:SS */
export function formatTs(ms) {
    const d = new Date(ms);
    const pad = n => String(n).padStart(2, '0');
    return `${dayOf(ms)} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/**
 * Fold completion entries into their start entries. Returns timeline
 * rows (newest first, completions absorbed): each row is the original
 * entry plus, when a completion referenced it, {completion} and
 * rolled-up status/tokens/duration.
 *
 * @param {object[]} entries newest-first Entry rows
 * @returns {object[]}
 */
export function buildTimeline(entries) {
    const completions = new Map();
    for (const e of entries ?? []) {
        if (e.kind.endsWith('.complete') && e.ref_id != null)
            completions.set(e.ref_id, e);
    }
    const rows = [];
    for (const e of entries ?? []) {
        if (e.kind.endsWith('.complete') && e.ref_id != null)
            continue; // absorbed into its start entry
        const completion = completions.get(e.id) ?? null;
        rows.push({
            ...e,
            completion,
            effective_status: completion ? completion.status : e.status,
            effective_tokens: completion ? completion.output_tokens : e.output_tokens,
            effective_duration_ms: completion ? completion.duration_ms : e.duration_ms,
        });
    }
    return rows;
}

/**
 * Filter timeline rows. Every field optional; 'all' means no filter
 * (matches the dropdowns' first item).
 *
 * @param {object[]} rows
 * @param {{app?: string, kind?: string, day?: string}} f
 * @returns {object[]}
 */
export function filterRows(rows, f) {
    return (rows ?? []).filter(r => {
        if (f?.app && f.app !== 'all' && r.app_id !== f.app)
            return false;
        if (f?.kind && f.kind !== 'all' && r.kind !== f.kind)
            return false;
        if (f?.day && f.day !== 'all' && dayOf(r.ts) !== f.day)
            return false;
        return true;
    });
}

/**
 * Free-text search over timeline rows: case-insensitive substring
 * match against preview, detail, model, and kind. An empty (or
 * whitespace-only) query matches everything, so the dropdown-filtered
 * view passes through unchanged.
 *
 * @param {object[]} rows
 * @param {string} [query]
 * @returns {object[]}
 */
export function searchRows(rows, query) {
    const q = String(query ?? '').trim().toLowerCase();
    if (!q)
        return rows ?? [];
    return (rows ?? []).filter(r =>
        [r.preview, r.detail, r.model, r.kind]
            .some(v => String(v ?? '').toLowerCase().includes(q)));
}

/**
 * Distinct values of a field across rows, sorted, for filter dropdowns.
 * @param {object[]} rows @param {string} field @returns {string[]}
 */
export function distinctValues(rows, field) {
    return [...new Set((rows ?? []).map(r => String(r[field])))].sort();
}

/**
 * Usage stats (§5.7.6: "tokens by app"): output tokens summed per
 * app_id (from completions, where tokens live) and event counts per
 * kind, both sorted descending.
 *
 * @param {object[]} entries raw entries (not timeline rows)
 * @returns {{tokensByApp: [string, number][], countsByKind: [string, number][]}}
 */
export function usageStats(entries) {
    const tokens = new Map();
    const counts = new Map();
    for (const e of entries ?? []) {
        counts.set(e.kind, (counts.get(e.kind) ?? 0) + 1);
        if (e.output_tokens > 0)
            tokens.set(e.app_id, (tokens.get(e.app_id) ?? 0) + e.output_tokens);
    }
    const desc = (a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1);
    return {
        tokensByApp: [...tokens.entries()].sort(desc),
        countsByKind: [...counts.entries()].sort(desc),
    };
}

/** One-line row summary for the timeline list. */
export function describeRow(row) {
    const tokens = row.effective_tokens > 0 ? ` · ${row.effective_tokens} tok` : '';
    const dur = row.effective_duration_ms > 0 ? ` · ${row.effective_duration_ms} ms` : '';
    return `${row.kind} · ${row.app_id} · ${row.effective_status}${tokens}${dur}`;
}
