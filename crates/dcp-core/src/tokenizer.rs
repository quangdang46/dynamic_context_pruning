//! Internal default [`Tokenizer`] used by [`crate::ContextPruner`] when
//! the host has not installed a custom one through the builder.
//!
//! The implementation is the dependency-free `char_count / 4` heuristic
//! documented in PLAN.md Decision D3 — accurate enough for budget
//! estimation without pulling in any tokenizer crate.

use dcp_traits::Tokenizer;

/// `char_count / 4` heuristic tokenizer.
///
/// SPEC.md never specifies an exact tokenization; this is the
/// universal-fallback shape suitable for context-budget estimation. The
/// constant `4` matches OpenAI's documented average bytes-per-token for
/// English prose.
///
/// # Example
///
/// ```rust
/// use dcp_core::tokenizer::Char4Tokenizer;
/// use dcp_traits::Tokenizer;
///
/// let t = Char4Tokenizer;
/// assert_eq!(t.count(""), 0);
/// assert_eq!(t.count("abcd"), 1);
/// assert_eq!(t.count("abcde"), 2); // ceiling division
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct Char4Tokenizer;

impl Char4Tokenizer {
    /// Construct a [`Char4Tokenizer`].
    pub const fn new() -> Self {
        Self
    }
}

impl Tokenizer for Char4Tokenizer {
    fn count(&self, text: &str) -> usize {
        let chars = text.chars().count();
        chars.div_ceil(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(Char4Tokenizer.count(""), 0);
    }

    #[test]
    fn rounds_up() {
        assert_eq!(Char4Tokenizer.count("abc"), 1);
        assert_eq!(Char4Tokenizer.count("abcd"), 1);
        assert_eq!(Char4Tokenizer.count("abcde"), 2);
        assert_eq!(Char4Tokenizer.count("abcdefgh"), 2);
    }

    #[test]
    fn counts_chars_not_bytes() {
        // Multi-byte char must count as 1 char.
        assert_eq!(Char4Tokenizer.count("🦀"), 1);
        assert_eq!(Char4Tokenizer.count("🦀🦀🦀🦀"), 1);
        assert_eq!(Char4Tokenizer.count("🦀🦀🦀🦀🦀"), 2);
    }

    #[test]
    fn batch_default_sums_per_string() {
        let t = Char4Tokenizer;
        assert_eq!(t.count_batch(&["abcd", "efgh"]), 2);
        assert_eq!(t.count_batch(&[]), 0);
    }
}
