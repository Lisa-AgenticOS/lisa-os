use fixture_dedupe_first_wins::merge;

fn e(id: &str, label: &str) -> (String, String) {
    (id.to_string(), label.to_string())
}

#[test]
fn the_first_entry_wins_a_duplicate_id() {
    let merged = merge(&[e("m", "Local M")], &[e("m", "Cloud M")]);
    assert_eq!(merged, vec![e("m", "Local M")]);
}

#[test]
fn insertion_order_is_kept() {
    let merged = merge(&[e("a", "A")], &[e("b", "B"), e("c", "C")]);
    assert_eq!(merged, vec![e("a", "A"), e("b", "B"), e("c", "C")]);
}
