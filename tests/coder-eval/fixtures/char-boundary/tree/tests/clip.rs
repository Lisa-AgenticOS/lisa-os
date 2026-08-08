use fixture_char_boundary::clip_title;

#[test]
fn multibyte_titles_clip_without_panicking() {
    // Three bytes per character: byte 10 is mid-character, which is
    // exactly where a byte slice panics.
    let clipped = clip_title("日本語のテスト", 10);
    assert!(clipped.ends_with('…'), "clipped titles carry an ellipsis");
    let stem = clipped.trim_end_matches('…');
    assert!(
        "日本語のテスト".starts_with(stem),
        "the clipped stem must be a prefix of the original"
    );
}

#[test]
fn short_titles_pass_through_unchanged() {
    assert_eq!(clip_title("hi", 10), "hi");
}
