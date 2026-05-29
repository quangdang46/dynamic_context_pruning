//! Enhanced prompt store with 3-tier override cascade.
//!
//! This module provides a [`PromptStore`] that checks for overrides in three
//! locations in cascade order:
//!
//! 1. **Project**: `.opencode/dcp-prompts/overrides/` (relative to `working_directory`)
//! 2. **Config dir**: `$OPENCODE_CONFIG_DIR/dcp-prompts/overrides/` (if env var set)
//! 3. **Global**: `~/.config/opencode/dcp-prompts/overrides/` (via dirs crate)
//!
//! For each prompt type, the first valid override file found wins; if none
//! is found, the bundled default is used.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors returned by [`PromptStore`] operations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// IO error during file operations.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    /// UTF-8 error when reading files.
    #[error("UTF-8 error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
}

/// Override file names recognised by [`PromptStore`], in the same order
/// as the fields of [`RuntimePrompts`].
const OVERRIDE_FILE_NAMES: [&str; 6] = [
    "system.md",
    "compress-range.md",
    "compress-message.md",
    "context-limit-nudge.md",
    "turn-nudge.md",
    "iteration-nudge.md",
];

/// Runtime prompts returned by [`PromptStore::get_runtime_prompts`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePrompts {
    /// System-prompt addendum.
    pub system: String,
    /// Description for the compress tool, range mode.
    pub compress_range: String,
    /// Description for the compress tool, message mode.
    pub compress_message: String,
    /// Context-limit nudge template.
    pub context_limit_nudge: String,
    /// Turn nudge template.
    pub turn_nudge: String,
    /// Iteration nudge template.
    pub iteration_nudge: String,
}

impl Default for RuntimePrompts {
    fn default() -> Self {
        Self {
            system: crate::DEFAULT_SYSTEM.to_string(),
            compress_range: crate::DEFAULT_COMPRESS_RANGE.to_string(),
            compress_message: crate::DEFAULT_COMPRESS_MESSAGE.to_string(),
            context_limit_nudge: crate::DEFAULT_CONTEXT_LIMIT_NUDGE.to_string(),
            turn_nudge: crate::DEFAULT_TURN_NUDGE.to_string(),
            iteration_nudge: crate::DEFAULT_ITERATION_NUDGE.to_string(),
        }
    }
}

/// Prompt store with 3-tier override cascade.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::store::PromptStore;
/// use std::path::Path;
///
/// let store = PromptStore::new(Path::new("/path/to/project"));
/// let prompts = store.get_runtime_prompts();
/// assert!(!prompts.system.is_empty());
/// ```
pub struct PromptStore {
    working_directory: PathBuf,
    bundled_defaults: RuntimePrompts,
    loaded_overrides: HashMap<String, String>,
}

impl PromptStore {
    /// Construct a new store with the given working directory.
    pub fn new(working_directory: &Path) -> Self {
        Self {
            working_directory: working_directory.to_path_buf(),
            bundled_defaults: RuntimePrompts::default(),
            loaded_overrides: HashMap::new(),
        }
    }

    /// Get the runtime prompts, applying the 3-tier override cascade.
    pub fn get_runtime_prompts(&self) -> RuntimePrompts {
        RuntimePrompts {
            system: self.resolve_prompt("system.md", &self.bundled_defaults.system),
            compress_range: self.resolve_prompt("compress-range.md", &self.bundled_defaults.compress_range),
            compress_message: self.resolve_prompt("compress-message.md", &self.bundled_defaults.compress_message),
            context_limit_nudge: self.resolve_prompt("context-limit-nudge.md", &self.bundled_defaults.context_limit_nudge),
            turn_nudge: self.resolve_prompt("turn-nudge.md", &self.bundled_defaults.turn_nudge),
            iteration_nudge: self.resolve_prompt("iteration-nudge.md", &self.bundled_defaults.iteration_nudge),
        }
    }

    /// Reload overrides from the 3-tier cascade.
    pub fn reload(&mut self) -> Result<(), StoreError> {
        let mut overrides = HashMap::new();
        let cascade_dirs = self.build_cascade_dirs();

        for file_name in OVERRIDE_FILE_NAMES {
            if let Some(content) = self.find_override_in_cascade(file_name, &cascade_dirs)? {
                overrides.insert(file_name.to_string(), content);
            }
        }

        self.loaded_overrides = overrides;
        Ok(())
    }

    fn build_cascade_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        let project_dir = self
            .working_directory
            .join(".opencode")
            .join("dcp-prompts")
            .join("overrides");
        dirs.push(project_dir);

        if let Ok(config_dir) = env::var("OPENCODE_CONFIG_DIR") {
            let config_path = PathBuf::from(&config_dir);
            // Reject paths with `..` components or relative paths.
            if config_path.is_absolute()
                && !config_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                let config_override = config_path
                    .join("dcp-prompts")
                    .join("overrides");
                dirs.push(config_override);
            }
        }

        if let Some(home) = dirs::config_dir() {
            let global_dir = home.join("opencode").join("dcp-prompts").join("overrides");
            dirs.push(global_dir);
        }

        dirs
    }

    fn find_override_in_cascade(
        &self,
        file_name: &str,
        cascade_dirs: &[PathBuf],
    ) -> Result<Option<String>, StoreError> {
        for dir in cascade_dirs {
            let path = dir.join(file_name);
            if path.exists() {
                let content = fs::read_to_string(&path)?;
                if content.trim().is_empty() {
                    continue;
                }
                return Ok(Some(content));
            }
        }
        Ok(None)
    }

    fn resolve_prompt(&self, file_name: &str, default: &str) -> String {
        self.loaded_overrides
            .get(file_name)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, PromptStore) {
        let dir = TempDir::new().unwrap();
        let store = PromptStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn test_prompt_store_new_initializes_defaults() {
        let (_dir, store) = create_test_store();
        let prompts = store.get_runtime_prompts();
        assert!(!prompts.system.is_empty());
        assert!(!prompts.compress_range.is_empty());
        assert!(!prompts.compress_message.is_empty());
        assert!(!prompts.context_limit_nudge.is_empty());
        assert!(!prompts.turn_nudge.is_empty());
        assert!(!prompts.iteration_nudge.is_empty());
    }

    #[test]
    fn test_prompt_store_reload_with_no_overrides_uses_defaults() {
        let (_dir, mut store) = create_test_store();
        store.reload().unwrap();
        let prompts = store.get_runtime_prompts();
        assert!(prompts.system.contains("# Context-pruning support"));
    }

    #[test]
    fn test_prompt_store_reload_project_override() {
        let (dir, mut store) = create_test_store();

        let override_dir = dir
            .path()
            .join(".opencode")
            .join("dcp-prompts")
            .join("overrides");
        std::fs::create_dir_all(&override_dir).unwrap();
        std::fs::write(override_dir.join("system.md"), "PROJECT-OVERRIDE-SYSTEM").unwrap();

        store.reload().unwrap();
        let prompts = store.get_runtime_prompts();
        assert_eq!(prompts.system, "PROJECT-OVERRIDE-SYSTEM");
    }

    #[test]
    fn test_prompt_store_reload_global_fallback() {
        let (dir, store) = create_test_store();

        let global_override_dir = dir.path().join("global-overrides");
        std::fs::create_dir_all(&global_override_dir).unwrap();
        std::fs::write(global_override_dir.join("system.md"), "GLOBAL-OVERRIDE-SYSTEM").unwrap();

        let content = store.find_override_in_cascade("system.md", &[global_override_dir.clone()]);
        assert!(content.unwrap().is_some());
    }

    #[test]
    fn test_prompt_store_reload_multiple_overrides() {
        let (dir, mut store) = create_test_store();

        let override_dir = dir
            .path()
            .join(".opencode")
            .join("dcp-prompts")
            .join("overrides");
        std::fs::create_dir_all(&override_dir).unwrap();

        std::fs::write(override_dir.join("system.md"), "PROJECT-SYSTEM").unwrap();
        std::fs::write(override_dir.join("turn-nudge.md"), "PROJECT-TURN").unwrap();

        store.reload().unwrap();
        let prompts = store.get_runtime_prompts();

        assert_eq!(prompts.system, "PROJECT-SYSTEM");
        assert_eq!(prompts.turn_nudge, "PROJECT-TURN");
        assert!(prompts.compress_range.contains("compress"));
    }

    #[test]
    fn test_runtime_prompts_default_has_all_fields() {
        let prompts = RuntimePrompts::default();
        assert!(!prompts.system.is_empty());
        assert!(!prompts.compress_range.is_empty());
        assert!(!prompts.compress_message.is_empty());
        assert!(!prompts.context_limit_nudge.is_empty());
        assert!(!prompts.turn_nudge.is_empty());
        assert!(!prompts.iteration_nudge.is_empty());
    }

    #[test]
    fn test_empty_override_file_skipped() {
        let (dir, mut store) = create_test_store();

        let override_dir = dir
            .path()
            .join(".opencode")
            .join("dcp-prompts")
            .join("overrides");
        std::fs::create_dir_all(&override_dir).unwrap();
        std::fs::write(override_dir.join("system.md"), "   \n\t  ").unwrap();

        store.reload().unwrap();
        let prompts = store.get_runtime_prompts();
        assert!(prompts.system.contains("# Context-pruning support"));
    }

    #[test]
    fn test_cascade_order_first_wins() {
        let (dir, store) = create_test_store();

        let project_dir = dir
            .path()
            .join(".opencode")
            .join("dcp-prompts")
            .join("overrides");
        let global_dir = dir.path().join("global-overrides");

        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        std::fs::write(project_dir.join("system.md"), "PROJECT-SYSTEM").unwrap();
        std::fs::write(global_dir.join("system.md"), "GLOBAL-SYSTEM").unwrap();

        let result = store.find_override_in_cascade("system.md", &[project_dir.clone(), global_dir.clone()]);
        assert_eq!(result.unwrap(), Some("PROJECT-SYSTEM".to_string()));
    }
}