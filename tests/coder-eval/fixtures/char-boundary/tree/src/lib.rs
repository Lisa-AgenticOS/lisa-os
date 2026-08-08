//! Listing titles for a conversation sidebar.

/// Clip `text` to at most `max` bytes for a compact listing title,
/// appending an ellipsis when something was cut.
pub fn clip_title(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    format!("{}…", &text[..max])
}
