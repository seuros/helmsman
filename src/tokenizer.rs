//! Token counting for rendered templates.

use tiktoken_rs::cl100k_base;

/// Count tokens in text using cl100k_base encoding.
/// Returns token count or 0 on encoding error.
pub fn count_tokens(text: &str) -> usize {
    cl100k_base()
        .map(|bpe| bpe.encode_with_special_tokens(text).len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
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
}
