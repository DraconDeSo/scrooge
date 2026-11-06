#![cfg(target_os = "linux")]

use glib::variant::ToVariant;

#[test]
fn glib_string_variant_iterator_reads_both_ends() {
    let variant = vec!["first", "middle", "last"].to_variant();
    let mut strings = variant.array_iter_str().unwrap();
    assert_eq!(strings.next(), Some("first"));
    assert_eq!(strings.next_back(), Some("last"));
    assert_eq!(strings.next(), Some("middle"));
    assert_eq!(strings.next(), None);
}
