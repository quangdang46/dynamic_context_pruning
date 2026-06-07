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
//! let tools = ToolProtection::new_exact(["task", "skill"]);
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
use serde_json::Value;
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
///
/// Supports both exact-match names (legacy) and glob patterns for flexible
/// tool-name matching (e.g. `mcp*` to match all MCP tool names).
#[derive(Clone, Debug, Default)]
pub struct ToolProtection {
    /// Exact-match tool names (legacy behavior).
    exact: HashSet<String>,
    /// Compiled glob patterns for pattern-based matching.
    glob_set: Option<GlobSet>,
}

impl ToolProtection {
    /// Backwards-compatible: create from exact-match names only.
    ///
    /// This constructor preserves the original API — all existing call sites
    /// using `ToolProtection::new(...)` continue to work unchanged.
    pub fn new_exact<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let exact: HashSet<String> = names.into_iter().map(Into::into).collect();
        Self {
            exact,
            glob_set: None,
        }
    }

    /// New constructor: exact names + glob patterns.
    ///
    /// Use this when you need both exact matches and glob patterns.
    /// Glob patterns are compiled into a [`GlobSet`] for efficient matching.
    pub fn new<I, S, G>(exact_names: I, glob_patterns: G) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
        G: IntoIterator<Item = String>,
    {
        let exact: HashSet<String> = exact_names.into_iter().map(Into::into).collect();
        let patterns_vec: Vec<String> = glob_patterns.into_iter().collect();
        let glob_set = if patterns_vec.is_empty() {
            None
        } else {
            let mut builder = GlobSetBuilder::new();
            for pattern in &patterns_vec {
                let glob = Glob::new(pattern).expect("valid glob pattern");
                builder.add(glob);
            }
            builder.build().ok()
        };
        Self { exact, glob_set }
    }

    /// Return `true` when `tool` is in the protected set.
    ///
    /// Checks exact match first, then glob patterns if available.
    pub fn is_protected(&self, tool: &str) -> bool {
        self.exact.contains(tool)
            || self
                .glob_set
                .as_ref()
                .is_some_and(|glob_set| glob_set.is_match(tool))
    }

    /// True when the protection set is empty.
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty()
            && self
                .glob_set
                .as_ref()
                .map_or(true, |glob_set| glob_set.is_empty())
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

/// Extract file paths from a tool's parameters JSON.
/// Handles: apply_patch/patchText, multiedit, filePath fields.
pub fn extract_file_paths(tool: &str, parameters: &Value) -> Vec<String> {
    let mut paths = Vec::new();

    match tool {
        "apply_patch" => {
            // Handle patchText field — Claude Code apply_patch format:
            // "*** Add File: path", "*** Update File: path", "*** Delete File: path"
            if let Some(patch_text) = parameters.get("patchText").and_then(|v| v.as_str()) {
                for line in patch_text.lines() {
                    if let Some(rest) = line.strip_prefix("*** Add File: ").or_else(|| {
                        line.strip_prefix("*** Update File: ")
                            .or_else(|| line.strip_prefix("*** Delete File: "))
                    }) {
                        let path = rest.trim();
                        if !path.is_empty() {
                            paths.push(path.to_string());
                        }
                    }
                }
            }
        }
        "multiedit" => {
            // Handle direct path field
            if let Some(path) = parameters
                .get("path")
                .or_else(|| parameters.get("filePath"))
            {
                if let Some(s) = path.as_str() {
                    paths.push(s.to_string());
                }
            }

            // Handle nested edits array (multiedit)
            if let Some(edits) = parameters.get("edits").and_then(|v| v.as_array()) {
                for edit in edits {
                    if let Some(edit_path) = edit.get("path").or_else(|| edit.get("filePath")) {
                        if let Some(s) = edit_path.as_str() {
                            paths.push(s.to_string());
                        }
                    }
                }
            }
        }
        _ => {
            // Default: try filePath field
            if let Some(path) = parameters
                .get("filePath")
                .or_else(|| parameters.get("path"))
            {
                if let Some(s) = path.as_str() {
                    paths.push(s.to_string());
                }
            }
        }
    }

    // Deduplicate while preserving order
    paths.sort();
    paths.dedup();
    paths
}

/// Check whether any extracted path matches the protected set.
pub fn is_file_path_protected(file_paths: &[String], patterns: &PathProtection) -> bool {
    file_paths.iter().any(|p| patterns.is_protected(p))
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
        let p = ToolProtection::new_exact(["task", "skill"]);
        assert!(p.is_protected("task"));
        assert!(p.is_protected("skill"));
        assert!(!p.is_protected("read"));
    }

    #[test]
    fn test_exact_membership() {
        let tp = ToolProtection::new_exact(["task".to_string(), "skill".to_string()]);
        assert!(tp.is_protected("task"));
        assert!(tp.is_protected("skill"));
        assert!(!tp.is_protected("ask"));
    }

    #[test]
    fn test_glob_pattern_match() {
        let tp = ToolProtection::new(Vec::<String>::new(), ["mcp*".to_string()]);
        assert!(tp.is_protected("mcp__fs__read"));
        assert!(tp.is_protected("mcptool"));
        assert!(!tp.is_protected("amcp"));
    }

    #[test]
    fn test_mixed_exact_and_glob() {
        let tp = ToolProtection::new(["task".to_string()], ["mcp*".to_string()]);
        assert!(tp.is_protected("task"));
        assert!(tp.is_protected("mcp__fs__read"));
        assert!(!tp.is_protected("ask"));
    }

    #[test]
    fn test_backwards_compatible_new_exact() {
        let tp = ToolProtection::new_exact(["task", "ask"]);
        assert!(tp.is_protected("task"));
        assert!(!tp.is_protected("mcp__fs__read"));
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

    // ----- extract_file_paths -----

    #[test]
    fn test_extract_file_paths_apply_patch() {
        let patch = "*** Update File: src/main.rs\n@@ -1,3 +1,4 @@\n old\n-old\n+new";
        let params = serde_json::json!({ "patchText": patch });
        let paths = extract_file_paths("apply_patch", &params);
        assert!(paths.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn test_extract_file_paths_multiedit() {
        let params = serde_json::json!({
            "path": "/workspace/file1.txt",
            "edits": [
                { "path": "/workspace/file2.txt" },
                { "filePath": "/workspace/file3.txt" }
            ]
        });
        let paths = extract_file_paths("multiedit", &params);
        assert!(paths.contains(&"/workspace/file1.txt".to_string()));
        assert!(paths.contains(&"/workspace/file2.txt".to_string()));
        assert!(paths.contains(&"/workspace/file3.txt".to_string()));
    }

    #[test]
    fn test_extract_file_paths_simple_path() {
        let params = serde_json::json!({ "filePath": "/tmp/test.txt" });
        let paths = extract_file_paths("read", &params);
        assert_eq!(paths, vec!["/tmp/test.txt"]);
    }

    #[test]
    fn test_extract_file_paths_deduplicates() {
        let params = serde_json::json!({
            "patchText": "*** Update File: src/main.rs\n*** Update File: src/main.rs\n"
        });
        let paths = extract_file_paths("apply_patch", &params);
        // Should be deduplicated
        assert_eq!(paths.iter().filter(|p| *p == "src/main.rs").count(), 1);
    }

    #[test]
    fn test_is_file_path_protected() {
        let pp = PathProtection::compile(&["/protected/*".into()]).unwrap();
        let paths = [
            "/protected/secret.txt".to_string(),
            "/public/file.txt".to_string(),
        ];
        assert!(is_file_path_protected(&paths, &pp));
        assert!(!is_file_path_protected(
            &["/public/file.txt".to_string()],
            &pp
        ));
    }
}
