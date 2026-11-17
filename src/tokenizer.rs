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
mod tests;
