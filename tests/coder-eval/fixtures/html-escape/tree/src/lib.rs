//! Text → HTML escaping for rendered transcripts.

/// Escape `text` for embedding into HTML.
pub fn escape(text: &str) -> String {
    text.replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('&', "&amp;")
}
