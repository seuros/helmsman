use super::*;

#[test]
fn test_count_tokens_basic() {
    let count = count_tokens("Hello, world!");
    assert!(count > 0);
}

#[test]
fn test_count_tokens_empty() {
    let count = count_tokens("");
    assert_eq!(count, 0);
}
