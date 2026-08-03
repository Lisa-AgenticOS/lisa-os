// Peek decisions for text, html and the generic card — pure.
//
// The extension list (lib/formats.js) is the first gate; these are the
// second: a .log that is actually a gzip must land on the card, not in
// a text view full of mojibake, and a 400 MB trace must not be slurped
// into a GtkTextBuffer whole.

/// Does this look like binary data? The classic sniff: a NUL byte in
/// the head. Checked on raw bytes BEFORE any decode — a decoder's
/// replacement characters would hide exactly the evidence this looks
/// for.
export function looksBinary(bytes, probe = 8000) {
    const n = Math.min(bytes.length, probe);
    for (let i = 0; i < n; i++)
        if (bytes[i] === 0) return true;
    return false;
}

/// Cap text for display. Returns what to show and whether it was cut —
/// the caller says so out loud; a truncation the reader cannot see is
/// a file they think they have read (the Surfer extract.js lesson).
export function truncateText(text, cap = 1_000_000) {
    if (typeof text !== 'string') return {text: '', truncated: false};
    if (text.length <= cap) return {text, truncated: false};
    return {text: text.slice(0, cap), truncated: true};
}

/// Bytes -> a human size for the card. Binary units, one decimal above
/// KiB, none for bytes — the shapes people actually read.
export function humanSize(bytes) {
    if (!Number.isFinite(bytes) || bytes < 0) return '';
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KiB', 'MiB', 'GiB', 'TiB'];
    let v = bytes;
    for (const unit of units) {
        v /= 1024;
        if (v < 1024 || unit === 'TiB')
            return `${v.toFixed(1)} ${unit}`;
    }
    return '';
}

/// The card's description line: "content type · size", dropping the
/// parts that are unknown rather than printing placeholders.
export function cardSubtitle(typeDescription, sizeBytes) {
    const parts = [];
    if (typeDescription) parts.push(typeDescription);
    const size = humanSize(sizeBytes);
    if (size) parts.push(size);
    return parts.join(' · ');
}

/// A folder's card line. `capped` means the enumeration stopped early
/// (a 100k-entry directory must not hang a peek): the count is then a
/// floor and says so; the size is omitted because a partial sum
/// presented as THE size is a lie.
export function folderSubtitle(count, capped, sizeBytes) {
    const items = capped
        ? `${count}+ items`
        : `${count} item${count === 1 ? '' : 's'}`;
    if (capped) return items;
    const size = humanSize(sizeBytes);
    return size ? `${items} · ${size}` : items;
}

/// Media duration (microseconds, as Gtk.MediaStream reports it) -> a
/// clock string. Zero/unknown renders as nothing rather than 0:00.
export function mediaClock(us) {
    if (!Number.isFinite(us) || us <= 0) return '';
    const s = Math.round(us / 1e6);
    const m = Math.floor(s / 60), sec = s % 60;
    const h = Math.floor(m / 60);
    if (h > 0) return `${h}:${String(m % 60).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
    return `${m}:${String(sec).padStart(2, '0')}`;
}
