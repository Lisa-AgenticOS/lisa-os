use fixture_html_escape::escape;

#[test]
fn entities_are_not_double_escaped() {
    // `<` must become `&lt;` and STAY `&lt;` — an escape pass that
    // rewrites the `&` of its own entities produces `&amp;lt;`.
    assert_eq!(escape("<a & b>"), "&lt;a &amp; b&gt;");
}

#[test]
fn plain_text_is_unchanged() {
    assert_eq!(escape("hello world"), "hello world");
}
