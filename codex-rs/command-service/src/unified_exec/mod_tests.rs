use super::take_last_chars;

#[test]
fn running_command_context_tail_keeps_latest_text() {
    assert_eq!(take_last_chars("abcdef", 3), "def");
    assert_eq!(take_last_chars("abcdef", 20), "abcdef");
}
