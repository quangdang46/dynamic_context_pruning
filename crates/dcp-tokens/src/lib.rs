#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-tokens` — concrete [`Tokenizer`] implementations for
//! `dynamic_context_pruning` (PLAN.md Phase 1).
//!
//! Backends, in roughly increasing order of accuracy and weight:
//!
//! - [`Char4Tokenizer`] (always available): a `chars / 4` heuristic. Zero
//!   dependencies, suitable for budget estimation when the host has not
//!   wired a real encoder.
//! - [`HuggingFaceTokenizer`] (feature `tokenizers`): wraps a
//!   `tokenizers::Tokenizer`. Loads any `tokenizer.json` from disk or from
//!   an in-memory JSON string; counts via `encode(...)` + `Encoding::len()`.
//! - [`TiktokenTokenizer`] (feature `tiktoken-fast`): wraps the
//!   `tiktoken` crate (pure-Rust BPE; *not* `tiktoken-rs`). Constructors
//!   for `cl100k_base` and `o200k_base`.
//! - [`ClaudeTokenizer`] (feature `claude-tokens`): wraps the
//!   `claude-tokenizer` crate, which embeds Anthropic's Claude v3
//!   tokenizer JSON.
//!
//! ## Choosing a backend at runtime
//!
//! [`default_tokenizer`] returns the best backend that can be constructed
//! without host input, falling back to [`Char4Tokenizer`] when no feature
//! is active. The literal precedence is `tokenizers > tiktoken-fast >
//! claude-tokens > Char4`, but the bare `tokenizers` feature does not
//! ship a default model, so it is skipped at runtime.
//!
//! ## Errors
//!
//! Backend constructors return [`TokenizerError`]; the `count(...)`
//! implementations are infallible and never panic — encode failures are
//! surfaced as a `0` token count, matching the SPEC requirement that the
//! prune pipeline tolerates degenerate inputs.

use std::sync::Arc;

use dcp_traits::Tokenizer;
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────

/// Errors returned by tokenizer constructors in this crate.
///
/// Per SPEC §6.x the `count(...)` path itself is infallible; this type is
/// only surfaced when wiring a backend (loading `tokenizer.json`,
/// resolving an encoding name, etc.).
#[derive(Debug, Error)]
pub enum TokenizerError {
    /// The backend could not be constructed (missing file, unknown
    /// encoding name, malformed JSON, …).
    #[error("tokenizer construction failed: {reason}")]
    ConstructionFailed {
        /// Human-readable description of the construction failure.
        reason: String,
    },
    /// An encode/tokenize call failed at runtime. Currently unused by the
    /// shipped backends (they swallow encode errors as `0`), but exposed
    /// for downstream wrappers.
    #[error("tokenizer encoding failed: {reason}")]
    EncodingFailed {
        /// Human-readable description of the encode failure.
        reason: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────
// Char4 — UTF-8-safe `chars / 4` heuristic, zero deps.
// ─────────────────────────────────────────────────────────────────────────

/// `chars().count() / 4` heuristic tokenizer.
///
/// This is the always-available default. It uses `str::chars()` so the
/// count is UTF-8-safe (counts Unicode scalar values rather than bytes)
/// and well-defined for emoji, CJK ideographs, Vietnamese diacritics,
/// etc. The `/4` factor matches OpenAI's "rough English token ratio"
/// rule of thumb cited in SPEC §6.1.
#[derive(Debug, Default, Clone, Copy)]
pub struct Char4Tokenizer;

impl Char4Tokenizer {
    /// Construct a fresh `Char4Tokenizer`. Equivalent to `default()`.
    pub const fn new() -> Self {
        Self
    }
}

impl Tokenizer for Char4Tokenizer {
    fn count(&self, text: &str) -> usize {
        text.chars().count() / 4
    }
}

// ─────────────────────────────────────────────────────────────────────────
// HuggingFace `tokenizers` backend.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(feature = "tokenizers")]
mod hf {
    use super::{Tokenizer, TokenizerError};
    use std::path::Path;
    use std::str::FromStr;

    /// HuggingFace `tokenizers::Tokenizer` adapter.
    ///
    /// The wrapped tokenizer is fully owned; the wrapper is `Send + Sync`
    /// and cheap to share via `Arc<dyn Tokenizer>`.
    #[derive(Debug)]
    pub struct HuggingFaceTokenizer {
        /// The underlying HuggingFace tokenizer.
        pub inner: tokenizers::Tokenizer,
    }

    impl HuggingFaceTokenizer {
        /// Load a tokenizer from a `tokenizer.json` file on disk.
        pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, TokenizerError> {
            let inner = tokenizers::Tokenizer::from_file(path.as_ref()).map_err(|e| {
                TokenizerError::ConstructionFailed {
                    reason: format!(
                        "tokenizers::Tokenizer::from_file({}) failed: {e}",
                        path.as_ref().display()
                    ),
                }
            })?;
            Ok(Self { inner })
        }

        /// Parse a `tokenizer.json` blob from an in-memory JSON string.
        ///
        /// Despite the `from_pretrained_json` name, this is purely local:
        /// no HTTP request is made, so the `http` feature on the upstream
        /// `tokenizers` crate is *not* required.
        pub fn from_pretrained_json(json: &str) -> Result<Self, TokenizerError> {
            let inner = tokenizers::Tokenizer::from_str(json).map_err(|e| {
                TokenizerError::ConstructionFailed {
                    reason: format!("tokenizers::Tokenizer::from_str failed: {e}"),
                }
            })?;
            Ok(Self { inner })
        }
    }

    impl Tokenizer for HuggingFaceTokenizer {
        fn count(&self, text: &str) -> usize {
            // `encode(text, false)` — no special tokens. We swallow encode
            // errors as `0` to keep `count` infallible (SPEC §6.1).
            match self.inner.encode(text, false) {
                Ok(encoding) => encoding.len(),
                Err(_) => 0,
            }
        }
    }
}

#[cfg(feature = "tokenizers")]
pub use hf::HuggingFaceTokenizer;

// ─────────────────────────────────────────────────────────────────────────
// `tiktoken` backend.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(feature = "tiktoken-fast")]
mod tiktoken_backend {
    use super::{Tokenizer, TokenizerError};

    /// `tiktoken` (the goliajp pure-Rust port; *not* `tiktoken-rs`)
    /// adapter.
    ///
    /// The upstream `tiktoken::get_encoding(...)` returns a
    /// `&'static CoreBpe` (the encoding tables are baked into the binary
    /// at compile time and live for the program's lifetime), so the
    /// wrapper just stores the reference.
    pub struct TiktokenTokenizer {
        inner: &'static tiktoken::CoreBpe,
        encoding_name: &'static str,
    }

    // `tiktoken::CoreBpe` does not implement `Debug`; we provide a
    // minimal hand-rolled impl so `Result<TiktokenTokenizer, _>` can be
    // unwrapped in tests.
    impl std::fmt::Debug for TiktokenTokenizer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TiktokenTokenizer")
                .field("encoding_name", &self.encoding_name)
                .finish_non_exhaustive()
        }
    }

    impl TiktokenTokenizer {
        /// Construct a tokenizer for an encoding name known to the
        /// `tiktoken` crate (e.g. `"cl100k_base"`, `"o200k_base"`,
        /// `"p50k_base"`, `"llama3"`, …).
        pub fn from_encoding(name: &'static str) -> Result<Self, TokenizerError> {
            let inner =
                tiktoken::get_encoding(name).ok_or_else(|| TokenizerError::ConstructionFailed {
                    reason: format!("tiktoken::get_encoding({name:?}) returned None"),
                })?;
            Ok(Self {
                inner,
                encoding_name: name,
            })
        }

        /// Construct a `cl100k_base` tokenizer (GPT-4, GPT-3.5-turbo,
        /// `text-embedding-*`).
        pub fn cl100k_base() -> Result<Self, TokenizerError> {
            Self::from_encoding("cl100k_base")
        }

        /// Construct an `o200k_base` tokenizer (GPT-4o, GPT-4o-mini,
        /// o1/o3/o4 series).
        pub fn o200k_base() -> Result<Self, TokenizerError> {
            Self::from_encoding("o200k_base")
        }

        /// Returns the tiktoken encoding name this tokenizer was built
        /// from (e.g. `"cl100k_base"`).
        pub fn encoding_name(&self) -> &'static str {
            self.encoding_name
        }
    }

    impl Tokenizer for TiktokenTokenizer {
        fn count(&self, text: &str) -> usize {
            // `count` is the zero-alloc fast path on `CoreBpe`.
            self.inner.count(text)
        }
    }
}

#[cfg(feature = "tiktoken-fast")]
pub use tiktoken_backend::TiktokenTokenizer;

// ─────────────────────────────────────────────────────────────────────────
// `claude-tokenizer` backend.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(feature = "claude-tokens")]
mod claude {
    use super::{Tokenizer, TokenizerError};

    /// Anthropic Claude tokenizer (via the `claude-tokenizer` crate).
    ///
    /// The upstream crate embeds `claude-v3-tokenizer.json` at compile
    /// time and exposes a fully-built `tokenizers::Tokenizer` via
    /// `claude_tokenizer::get_tokenizer()`. We hold that owned tokenizer
    /// for the lifetime of the wrapper so each `count(...)` call avoids
    /// re-parsing the embedded JSON.
    #[derive(Debug)]
    pub struct ClaudeTokenizer {
        inner: tokenizers::Tokenizer,
    }

    impl ClaudeTokenizer {
        /// Construct a Claude v3 tokenizer using the embedded JSON.
        ///
        /// The upstream `get_tokenizer` panics on internal corruption of
        /// the embedded blob, so a `Result` is structurally redundant —
        /// but we still expose a `Result` for symmetry with the other
        /// backends and to leave room for future fallibility.
        pub fn new() -> Result<Self, TokenizerError> {
            // `claude_tokenizer::get_tokenizer()` panics only if the
            // embedded JSON is corrupt, which would be a bug in the
            // upstream crate's build. Wrapping in a closure + `unwind`
            // would be excessive; we trust the upstream invariant here
            // and expose the `Result` only for API symmetry.
            let inner = claude_tokenizer::get_tokenizer();
            Ok(Self { inner })
        }
    }

    impl Tokenizer for ClaudeTokenizer {
        fn count(&self, text: &str) -> usize {
            // Mirror the upstream `count_tokens` implementation, but
            // reuse our cached `Tokenizer` instead of rebuilding it on
            // every call.
            match self.inner.encode(text, false) {
                Ok(encoding) => encoding.len(),
                Err(_) => 0,
            }
        }
    }
}

#[cfg(feature = "claude-tokens")]
pub use claude::ClaudeTokenizer;

// ─────────────────────────────────────────────────────────────────────────
// `default_tokenizer` — runtime backend selection.
// ─────────────────────────────────────────────────────────────────────────

/// Return the best [`Tokenizer`] available given the enabled features.
///
/// The literal precedence per the spec is
/// `tokenizers > tiktoken-fast > claude-tokens > Char4`. The bare
/// `tokenizers` feature has no embedded default model — it requires a
/// `tokenizer.json` from the host — so at runtime it is skipped and the
/// next backend is tried. The observable order is therefore:
///
/// 1. `tiktoken-fast` (`cl100k_base`) if enabled and constructable;
/// 2. `claude-tokens` if enabled;
/// 3. otherwise [`Char4Tokenizer`].
///
/// The return type is `Arc<dyn Tokenizer>` so the value is cheap to
/// clone, share across threads, and stash inside a long-lived `dcp-core`
/// `ContextPruner`.
pub fn default_tokenizer() -> Arc<dyn Tokenizer> {
    #[cfg(feature = "tiktoken-fast")]
    {
        if let Ok(t) = TiktokenTokenizer::cl100k_base() {
            return Arc::new(t);
        }
    }

    #[cfg(feature = "claude-tokens")]
    {
        if let Ok(t) = ClaudeTokenizer::new() {
            return Arc::new(t);
        }
    }

    Arc::new(Char4Tokenizer)
}

// ─────────────────────────────────────────────────────────────────────────
// tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Char4Tokenizer ──────────────────────────────────────────────────

    #[test]
    fn char4_empty_returns_zero() {
        let t = Char4Tokenizer::new();
        assert_eq!(t.count(""), 0);
    }

    #[test]
    fn char4_ascii_lengths_use_floor_div_4() {
        let t = Char4Tokenizer;
        assert_eq!(t.count("a"), 0); // 1 / 4 = 0
        assert_eq!(t.count("abcd"), 1); // 4 / 4 = 1
        assert_eq!(t.count("abcde"), 1); // 5 / 4 = 1
        assert_eq!(t.count("abcdefgh"), 2); // 8 / 4 = 2
        assert_eq!(t.count(&"x".repeat(40)), 10);
    }

    #[test]
    fn char4_counts_unicode_scalar_values_not_bytes() {
        let t = Char4Tokenizer;

        // Vietnamese: 5 scalar values (ti, ế, n, g, V), 4 chars stripped
        // for clarity. We test a longer phrase below; here we just verify
        // the multi-byte rounding behaviour.
        let viet = "Tiếng Việt"; // 10 chars (one is combining)
        assert_eq!(viet.chars().count(), 10);
        assert_eq!(t.count(viet), 10 / 4);

        // Emoji: each codepoint is one char; bytes are 4 (😀 = U+1F600).
        let emoji = "😀😀😀😀"; // 4 chars, 16 bytes
        assert_eq!(emoji.chars().count(), 4);
        assert_eq!(emoji.len(), 16); // confirm bytes != chars
        assert_eq!(t.count(emoji), 1);

        // Chinese: 8 chars, 24 bytes.
        let chinese = "你好世界你好世界"; // 8 chars
        assert_eq!(chinese.chars().count(), 8);
        assert!(chinese.len() > chinese.chars().count());
        assert_eq!(t.count(chinese), 2);
    }

    #[test]
    fn char4_default_matches_new() {
        let a = Char4Tokenizer;
        // Use the trait directly so clippy doesn't (rightly) complain
        // that `Char4Tokenizer::default()` is the unit struct value.
        let b: Char4Tokenizer = Default::default();
        let c = Char4Tokenizer::new();
        assert_eq!(a.count("hello world"), b.count("hello world"));
        assert_eq!(b.count("hello world"), c.count("hello world"));
    }

    #[test]
    fn char4_count_batch_default_sums_per_string_counts() {
        let t = Char4Tokenizer;
        // The default `Tokenizer::count_batch` sums per-string counts.
        // Each empty string contributes 0; "abcdefgh" contributes 2.
        assert_eq!(t.count_batch(&[]), 0);
        assert_eq!(t.count_batch(&["abcdefgh"]), 2);
        assert_eq!(t.count_batch(&["abcdefgh", "", "abcd"]), 3);
    }

    // ── default_tokenizer ───────────────────────────────────────────────

    #[test]
    fn default_tokenizer_returns_something_usable() {
        let t = default_tokenizer();
        // The returned tokenizer must successfully count *some* string
        // without panicking. The exact count depends on which feature is
        // active; we only assert non-panic and basic plausibility for an
        // ASCII payload.
        let _ = t.count("");
        let _ = t.count("hello world");
        // For the no-feature default (Char4), 100 ASCII chars → 25.
        // For tiktoken cl100k_base, 100 'a' chars → ~?? but >0.
        // For Claude, similar. We just assert non-zero on a long input.
        let big = "the quick brown fox jumps over the lazy dog. ".repeat(20);
        assert!(t.count(&big) > 0, "expected nonzero count on long input");
    }

    #[test]
    fn default_tokenizer_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn Tokenizer>();
        let _t: Arc<dyn Tokenizer> = default_tokenizer();
    }

    // ── TokenizerError ─────────────────────────────────────────────────

    #[test]
    fn tokenizer_error_display_messages() {
        let e = TokenizerError::ConstructionFailed {
            reason: "missing file".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("construction failed"));
        assert!(s.contains("missing file"));

        let e = TokenizerError::EncodingFailed {
            reason: "bad utf-8".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("encoding failed"));
        assert!(s.contains("bad utf-8"));
    }

    // ── Per-feature smoke tests ─────────────────────────────────────────

    #[cfg(feature = "tokenizers")]
    #[test]
    fn hf_constructor_compile_check_rejects_garbage_json() {
        // We do not bundle a real tokenizer.json (those models are
        // multi-MB). The most we can verify here is that the constructor
        // surface compiles and that bad inputs are reported as
        // `ConstructionFailed`.
        let err = HuggingFaceTokenizer::from_pretrained_json("not json").unwrap_err();
        assert!(matches!(err, TokenizerError::ConstructionFailed { .. }));

        let err = HuggingFaceTokenizer::from_file("/nonexistent/tokenizer.json").unwrap_err();
        assert!(matches!(err, TokenizerError::ConstructionFailed { .. }));
    }

    #[cfg(feature = "tiktoken-fast")]
    #[test]
    fn tiktoken_constructors_compile_and_count() {
        // `cl100k_base` ships embedded vocab data, so this should always
        // succeed — but we tolerate construction failure in case a future
        // tiktoken release renames or removes the encoding.
        if let Ok(t) = TiktokenTokenizer::cl100k_base() {
            assert_eq!(t.encoding_name(), "cl100k_base");
            assert!(t.count("hello world") > 0);
            assert_eq!(t.count(""), 0);
        }
        let _ = TiktokenTokenizer::o200k_base();

        // Unknown encoding → ConstructionFailed.
        let err = TiktokenTokenizer::from_encoding("definitely-not-an-encoding").unwrap_err();
        assert!(matches!(err, TokenizerError::ConstructionFailed { .. }));
    }

    #[cfg(feature = "claude-tokens")]
    #[test]
    fn claude_tokenizer_compile_check() {
        // The upstream embeds the JSON, so `new()` should always succeed
        // when the crate compiles. We only verify shape; the exact token
        // counts depend on the embedded model and are intentionally not
        // pinned here.
        let t = ClaudeTokenizer::new().expect("embedded JSON should parse");
        let _ = t.count("");
        let _ = t.count("hello");
    }
}
