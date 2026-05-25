#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-protected` — protection helpers shared by every prune strategy.
//!
//! Two kinds of protection are exposed:
//!
//! * [`ToolProtection`] — a small wrapper around a list of tool names
//!   that should be exempt from a strategy. Backed by a `HashSet` so
//!   membership checks are O(1).
//! * [`PathProtection`] — compiled globs (via the `globset` crate, the
//!   same engine used by `ripgrep` / `ignore`). Each glob matches a path
//!   substring; multiple globs are combined into a [`globset::GlobSet`]
//!   for efficient any-match queries.
//!
//! # Example
//!
//! ```rust
//! use dcp_protected::{PathProtection, ToolProtection};
//!
//! let tools = ToolProtection::new(["task", "skill"]);
//! assert!(tools.is_protected("task"));
//! assert!(!tools.is_protected("read"));
//!
//! let paths = PathProtection::compile(&["**/*.config.ts".into(), "Cargo.toml".into()]).unwrap();
//! assert!(paths.is_protected("src/app.config.ts"));
//! assert!(paths.is_protected("Cargo.toml"));
//! assert!(!paths.is_protected("src/main.rs"));
//! ```

use std::collections::HashSet;

use globset::{Glob, GlobSet, GlobSetBuilder};
use thiserror::Error;

/// Errors returned by [`PathProtection::compile`].
#[derive(Debug, Error)]
pub enum ProtectionError {
    /// One of the supplied glob patterns did not parse.
    #[error("invalid glob {pattern:?}: {source}")]
    InvalidGlob {
        /// The offending pattern as supplied by the caller.
        pattern: String,
        /// Underlying parse error from `globset`.
        #[source]
        source: globset::Error,
    },
}

/// Tool-name protection set.
///
/// Used by every prune strategy to skip tool calls whose name matches a
/// configured allow-list (`compress.protectedTools`, plus the per-strategy
/// `protectedTools` knobs). Per SPEC.md §10.2 each strategy keeps its own
/// list, so callers typically construct multiple [`ToolProtection`]s.
#[derive(Clone, Debug, Default)]
pub struct ToolProtection {
    names: HashSet<String>,
}

impl ToolProtection {
    /// Construct a [`ToolProtection`] from an iterator of tool names.
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Return `true` when `tool` is in the protected set.
    pub fn is_protected(&self, tool: &str) -> bool {
        self.names.contains(tool)
    }

    /// True when the protection set is empty.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Compiled file-path protection patterns.
///
/// Internally this is a [`globset::GlobSet`] backed by all configured
/// patterns. The matcher is anchored at the start of the path
/// (`globset`'s default), but `**/` patterns work as users expect.
#[derive(Clone, Debug, Default)]
pub struct PathProtection {
    set: Option<GlobSet>,
}

impl PathProtection {
    /// Compile a list of glob patterns into a [`PathProtection`].
    ///
    /// An empty input list compiles to a "no-op" matcher whose
    /// [`PathProtection::is_protected`] always returns `false`.
    pub fn compile(patterns: &[String]) -> Result<Self, ProtectionError> {
        if patterns.is_empty() {
            return Ok(Self { set: None });
        }
        let mut builder = GlobSetBuilder::new();
        for raw in patterns {
            let glob = Glob::new(raw).map_err(|source| ProtectionError::InvalidGlob {
                pattern: raw.clone(),
                source,
            })?;
            builder.add(glob);
        }
        let set = builder
            .build()
            .map_err(|source| ProtectionError::InvalidGlob {
                pattern: "<set>".into(),
                source,
            })?;
        Ok(Self { set: Some(set) })
    }

    /// Returns `true` when `path` matches any compiled glob.
    ///
    /// Empty patterns or a default [`PathProtection`] always return
    /// `false`.
    pub fn is_protected(&self, path: &str) -> bool {
        match &self.set {
            Some(set) => set.is_match(path),
            None => false,
        }
    }

    /// True when no patterns were compiled.
    pub fn is_empty(&self) -> bool {
        self.set.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- ToolProtection -----

    #[test]
    fn tool_protection_empty_default() {
        let p = ToolProtection::default();
        assert!(p.is_empty());
        assert!(!p.is_protected("anything"));
    }

    #[test]
    fn tool_protection_membership() {
        let p = ToolProtection::new(["task", "skill"]);
        assert!(p.is_protected("task"));
        assert!(p.is_protected("skill"));
        assert!(!p.is_protected("read"));
    }

    // ----- PathProtection -----

    #[test]
    fn path_protection_empty_input_is_noop() {
        let p = PathProtection::compile(&[]).unwrap();
        assert!(p.is_empty());
        assert!(!p.is_protected("Cargo.toml"));
    }

    #[test]
    fn path_protection_matches_globs() {
        let p = PathProtection::compile(&[
            "**/*.config.ts".into(),
            "Cargo.toml".into(),
            "src/secrets/*".into(),
        ])
        .unwrap();
        assert!(p.is_protected("src/app.config.ts"));
        assert!(p.is_protected("a/b/c/x.config.ts"));
        assert!(p.is_protected("Cargo.toml"));
        assert!(p.is_protected("src/secrets/key.pem"));
        assert!(!p.is_protected("src/main.rs"));
    }

    #[test]
    fn path_protection_invalid_glob_errors() {
        // `[` opens a character class that is never closed.
        let err = PathProtection::compile(&["src/[".into()]).unwrap_err();
        match err {
            ProtectionError::InvalidGlob { pattern, .. } => {
                assert_eq!(pattern, "src/[");
            }
        }
    }
}
