#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-prompts` — the six default prompts plus an override loader.
//!
//! The library renders three categories of model-facing text:
//!
//! 1. **System-prompt addendum** (`system.md`) — appended to the host's
//!    system prompt by the facade's `transform_system` entry point.
//! 2. **Compress tool descriptions** (`compress-range.md`,
//!    `compress-message.md`) — registered with the LLM alongside the
//!    `compress` tool schema.
//! 3. **Nudge templates** (`context-limit-nudge.md`, `turn-nudge.md`,
//!    `iteration-nudge.md`) — short reminders rendered by `dcp-nudges`
//!    to ask the model to call `compress`.
//!
//! Defaults are embedded at compile time via `include_str!`. Hosts can
//! override any of the six by placing a same-named markdown file inside
//! a directory passed to [`PromptStore::with_overrides`], **provided**
//! `experimental.custom_prompts` is enabled in the host configuration.
//! Without that flag the override directory is ignored and the defaults
//! are returned, matching SPEC.md §10.2's "warning only; overrides are
//! ignored" rule.
//!
//! # Quick start
//!
//! ```rust
//! use dcp_prompts::{Prompts, PromptStore, NudgeForce, build_protected_tools_extension,
//!     build_nudge_extension, render_system_prompt};
//!
//! // Use built-in defaults.
//! let prompts = Prompts::default();
//!
//! // Build a system prompt with the protected-tools section.
//! let ext = build_protected_tools_extension(&["task".into(), "skill".into()]);
//! let rendered = render_system_prompt(&prompts, &ext, false, false);
//! assert!(rendered.contains("task"));
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────
// Embedded defaults
// ─────────────────────────────────────────────────────────────────────────

const DEFAULT_SYSTEM: &str = include_str!("../prompts/system.md");
const DEFAULT_COMPRESS_RANGE: &str = include_str!("../prompts/compress-range.md");
const DEFAULT_COMPRESS_MESSAGE: &str = include_str!("../prompts/compress-message.md");
const DEFAULT_CONTEXT_LIMIT_NUDGE: &str = include_str!("../prompts/context-limit-nudge.md");
const DEFAULT_TURN_NUDGE: &str = include_str!("../prompts/turn-nudge.md");
const DEFAULT_ITERATION_NUDGE: &str = include_str!("../prompts/iteration-nudge.md");

/// Override file names recognised by [`PromptStore::with_overrides`],
/// in the same order as the fields of [`Prompts`].
pub const OVERRIDE_FILE_NAMES: [&str; 6] = [
    "system.md",
    "compress-range.md",
    "compress-message.md",
    "context-limit-nudge.md",
    "turn-nudge.md",
    "iteration-nudge.md",
];

// ─────────────────────────────────────────────────────────────────────────
// NudgeForce
// ─────────────────────────────────────────────────────────────────────────

/// Tone of nudge text, controlled by the host's `compress.nudgeForce`
/// configuration (SPEC.md §8.2 / §10.2).
///
/// The variant is consumed by [`build_nudge_extension`] to decide
/// whether the system-prompt addendum should request strong or soft
/// language when the model encounters a nudge.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::NudgeForce;
/// assert_eq!(NudgeForce::default(), NudgeForce::Soft);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NudgeForce {
    /// Strong nudges — used when iteration thresholds are reached.
    Strong,
    /// Soft nudges — the default; informational only.
    #[default]
    Soft,
}

// ─────────────────────────────────────────────────────────────────────────
// Prompts
// ─────────────────────────────────────────────────────────────────────────

/// The six prompt strings the library renders.
///
/// Field names mirror the override file names exactly (with hyphens
/// replaced by underscores). Construct with [`Prompts::default`] for
/// the bundled defaults, or with [`PromptStore`] for host overrides.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::Prompts;
/// let p = Prompts::default();
/// assert!(!p.system.is_empty());
/// assert!(!p.compress_range.is_empty());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prompts {
    /// System-prompt addendum (`system.md`).
    pub system: String,
    /// Description for the compress tool, range mode (`compress-range.md`).
    pub compress_range: String,
    /// Description for the compress tool, message mode
    /// (`compress-message.md`).
    pub compress_message: String,
    /// Context-limit nudge template (`context-limit-nudge.md`). Accepts
    /// the placeholders `{tokens}` and `{limit}`.
    pub context_limit_nudge: String,
    /// Turn nudge template (`turn-nudge.md`).
    pub turn_nudge: String,
    /// Iteration nudge template (`iteration-nudge.md`). Accepts the
    /// placeholder `{count}`.
    pub iteration_nudge: String,
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            system: DEFAULT_SYSTEM.to_string(),
            compress_range: DEFAULT_COMPRESS_RANGE.to_string(),
            compress_message: DEFAULT_COMPRESS_MESSAGE.to_string(),
            context_limit_nudge: DEFAULT_CONTEXT_LIMIT_NUDGE.to_string(),
            turn_nudge: DEFAULT_TURN_NUDGE.to_string(),
            iteration_nudge: DEFAULT_ITERATION_NUDGE.to_string(),
        }
    }
}

impl Prompts {
    /// Same as [`Prompts::default`] — explicit name for callers who
    /// want to be unambiguous about the source of the strings.
    pub fn embedded_defaults() -> Self {
        Self::default()
    }

    /// Mutable accessor by override file name. Used internally by
    /// [`PromptStore::with_overrides`] but exposed for hosts that want
    /// to apply ad-hoc overrides.
    ///
    /// Returns `None` if `file_name` is not one of [`OVERRIDE_FILE_NAMES`].
    pub fn field_mut(&mut self, file_name: &str) -> Option<&mut String> {
        match file_name {
            "system.md" => Some(&mut self.system),
            "compress-range.md" => Some(&mut self.compress_range),
            "compress-message.md" => Some(&mut self.compress_message),
            "context-limit-nudge.md" => Some(&mut self.context_limit_nudge),
            "turn-nudge.md" => Some(&mut self.turn_nudge),
            "iteration-nudge.md" => Some(&mut self.iteration_nudge),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────

/// Errors returned by [`PromptStore::with_overrides`].
#[derive(Debug, Error)]
pub enum PromptError {
    /// The override directory exists but could not be read (permission,
    /// not-a-directory, …).
    #[error("override directory `{path}` could not be read: {source}")]
    OverrideDirRead {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error message.
        #[source]
        source: std::io::Error,
    },
    /// A specific override file existed but could not be read.
    #[error("override file `{path}` could not be read: {source}")]
    OverrideFileRead {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error message.
        #[source]
        source: std::io::Error,
    },
    /// An override file was empty (probably a mistake — refuse).
    #[error("override file `{path}` is empty; remove it to use the default")]
    EmptyOverride {
        /// Path that was empty.
        path: PathBuf,
    },
}

// ─────────────────────────────────────────────────────────────────────────
// PromptStore
// ─────────────────────────────────────────────────────────────────────────

/// Container for the resolved [`Prompts`] plus diagnostics about which
/// fields were taken from overrides versus defaults.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::PromptStore;
///
/// let store = PromptStore::defaults();
/// assert!(store.overridden_files().is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct PromptStore {
    prompts: Prompts,
    overridden_files: Vec<String>,
}

impl PromptStore {
    /// Construct a store using the embedded defaults.
    pub fn defaults() -> Self {
        Self {
            prompts: Prompts::default(),
            overridden_files: Vec::new(),
        }
    }

    /// Construct a store from a fully-formed [`Prompts`] value (no I/O).
    pub fn from_prompts(prompts: Prompts) -> Self {
        Self {
            prompts,
            overridden_files: Vec::new(),
        }
    }

    /// Try to load overrides from `override_dir`.
    ///
    /// Behaviour:
    ///
    /// - When `custom_prompts_enabled == false`, the directory is
    ///   ignored entirely and embedded defaults are returned. This
    ///   matches the `experimental.customPrompts == false` semantics in
    ///   SPEC.md §10.2.
    /// - When `custom_prompts_enabled == true` and the directory does
    ///   not exist, embedded defaults are returned (no error — a
    ///   missing dir is interpreted as "no overrides supplied").
    /// - For each known override file (see [`OVERRIDE_FILE_NAMES`]), if
    ///   the file exists its contents replace the corresponding default.
    /// - Empty override files are rejected with [`PromptError::EmptyOverride`]
    ///   so silent mistakes do not become silent ship-breaking changes.
    pub fn with_overrides(
        override_dir: impl AsRef<Path>,
        custom_prompts_enabled: bool,
    ) -> Result<Self, PromptError> {
        let mut prompts = Prompts::default();
        let mut overridden_files: Vec<String> = Vec::new();

        if !custom_prompts_enabled {
            return Ok(Self {
                prompts,
                overridden_files,
            });
        }

        let dir = override_dir.as_ref();
        if !dir.exists() {
            return Ok(Self {
                prompts,
                overridden_files,
            });
        }

        // If the path exists but is not a directory, treat that as a
        // read error so the host hears about the misconfiguration.
        let meta = fs::metadata(dir).map_err(|source| PromptError::OverrideDirRead {
            path: dir.to_path_buf(),
            source,
        })?;
        if !meta.is_dir() {
            return Err(PromptError::OverrideDirRead {
                path: dir.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "override path is not a directory",
                ),
            });
        }

        for name in OVERRIDE_FILE_NAMES {
            let path = dir.join(name);
            if !path.exists() {
                continue;
            }
            let content =
                fs::read_to_string(&path).map_err(|source| PromptError::OverrideFileRead {
                    path: path.clone(),
                    source,
                })?;
            if content.trim().is_empty() {
                return Err(PromptError::EmptyOverride { path });
            }
            if let Some(field) = prompts.field_mut(name) {
                *field = content;
                overridden_files.push(name.to_string());
            }
        }

        Ok(Self {
            prompts,
            overridden_files,
        })
    }

    /// Borrow the resolved prompts.
    pub fn prompts(&self) -> &Prompts {
        &self.prompts
    }

    /// Consume the store and return the owned [`Prompts`].
    pub fn into_prompts(self) -> Prompts {
        self.prompts
    }

    /// File names (one of [`OVERRIDE_FILE_NAMES`]) whose default value
    /// was replaced by an override during [`PromptStore::with_overrides`].
    pub fn overridden_files(&self) -> &[String] {
        &self.overridden_files
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Renderers
// ─────────────────────────────────────────────────────────────────────────

/// Build the system-prompt addendum the facade installs through
/// `transform_system`.
///
/// Sections:
///
/// 1. The contents of `prompts.system`.
/// 2. `protected_tools_extension` if non-empty (typically produced by
///    [`build_protected_tools_extension`]).
/// 3. A short note when `manual_mode == true` so the model knows the
///    library will not auto-run strategies.
/// 4. A short note when `allow_subagents == true` so the model knows
///    sub-agent results may be folded back into context.
///
/// Sections are separated by a single blank line (`"\n\n"`); leading
/// and trailing whitespace is trimmed for cache stability.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::{Prompts, render_system_prompt, build_protected_tools_extension};
/// let p = Prompts::default();
/// let ext = build_protected_tools_extension(&["task".into()]);
/// let rendered = render_system_prompt(&p, &ext, false, false);
/// assert!(rendered.starts_with("# Context-pruning support"));
/// assert!(rendered.contains("task"));
/// ```
pub fn render_system_prompt(
    prompts: &Prompts,
    protected_tools_extension: &str,
    manual_mode: bool,
    allow_subagents: bool,
) -> String {
    let mut sections: Vec<String> = Vec::with_capacity(4);

    sections.push(prompts.system.trim().to_string());

    let trimmed_ext = protected_tools_extension.trim();
    if !trimmed_ext.is_empty() {
        sections.push(trimmed_ext.to_string());
    }

    if manual_mode {
        sections.push(MANUAL_MODE_NOTE.to_string());
    }

    if allow_subagents {
        sections.push(SUBAGENT_NOTE.to_string());
    }

    sections.join("\n\n")
}

const MANUAL_MODE_NOTE: &str = "\
Manual mode is enabled. The library will not run pruning strategies \
automatically; the user drives compression and pruning via slash \
commands. Treat any nudges as advisory only.";

const SUBAGENT_NOTE: &str = "\
Sub-agent support is enabled. Results from sub-agent runs may be \
folded back into this conversation; treat them as if you produced \
them yourself.";

/// Render the `<dcp-protected-tools>` block listing the tool names whose
/// verbatim output should never be replaced by a summary.
///
/// Returns an empty string when `protected_tools` is empty so that
/// [`render_system_prompt`] can skip the section without inserting
/// extra blank lines (cache stability).
///
/// # Example
///
/// ```rust
/// use dcp_prompts::build_protected_tools_extension;
/// assert!(build_protected_tools_extension(&[]).is_empty());
/// let block = build_protected_tools_extension(&["task".into(), "skill".into()]);
/// assert!(block.contains("- task"));
/// assert!(block.contains("- skill"));
/// ```
pub fn build_protected_tools_extension(protected_tools: &[String]) -> String {
    if protected_tools.is_empty() {
        return String::new();
    }

    let mut buf = String::new();
    buf.push_str("Protected tools (their outputs are preserved verbatim across compression):\n");
    for name in protected_tools {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        buf.push_str("- ");
        buf.push_str(trimmed);
        buf.push('\n');
    }
    // If every entry was blank, return empty so the caller can drop the
    // section entirely.
    if buf.lines().skip(1).all(|line| line.trim().is_empty()) {
        return String::new();
    }
    // Drop the trailing newline so trim() in callers behaves cleanly.
    while buf.ends_with('\n') {
        buf.pop();
    }
    buf
}

/// Render a short system-prompt extension explaining how nudges should
/// be interpreted.
///
/// The text differs between [`NudgeForce::Strong`] and
/// [`NudgeForce::Soft`] so the model knows whether a nudge is advisory
/// or a hard ask.
///
/// # Example
///
/// ```rust
/// use dcp_prompts::{NudgeForce, build_nudge_extension};
/// let strong = build_nudge_extension(NudgeForce::Strong);
/// let soft = build_nudge_extension(NudgeForce::Soft);
/// assert!(strong.contains("compress"));
/// assert!(soft.contains("compress"));
/// assert_ne!(strong, soft);
/// ```
pub fn build_nudge_extension(force: NudgeForce) -> String {
    match force {
        NudgeForce::Strong => "\
Nudge handling: when the library injects a nudge, treat it as a strong \
request to call the `compress` tool on a finished range before \
continuing the current task."
            .to_string(),
        NudgeForce::Soft => "\
Nudge handling: when the library injects a nudge, treat it as advisory. \
Call the `compress` tool only when a finished range is clearly available \
to fold."
            .to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_non_empty() {
        let p = Prompts::default();
        assert!(!p.system.trim().is_empty());
        assert!(!p.compress_range.trim().is_empty());
        assert!(!p.compress_message.trim().is_empty());
        assert!(!p.context_limit_nudge.trim().is_empty());
        assert!(!p.turn_nudge.trim().is_empty());
        assert!(!p.iteration_nudge.trim().is_empty());
    }

    #[test]
    fn defaults_match_embedded_constants() {
        let p = Prompts::default();
        assert_eq!(p.system, DEFAULT_SYSTEM);
        assert_eq!(p.compress_range, DEFAULT_COMPRESS_RANGE);
        assert_eq!(p.compress_message, DEFAULT_COMPRESS_MESSAGE);
        assert_eq!(p.context_limit_nudge, DEFAULT_CONTEXT_LIMIT_NUDGE);
        assert_eq!(p.turn_nudge, DEFAULT_TURN_NUDGE);
        assert_eq!(p.iteration_nudge, DEFAULT_ITERATION_NUDGE);
    }

    #[test]
    fn embedded_defaults_constructor_matches_default() {
        assert_eq!(Prompts::embedded_defaults(), Prompts::default());
    }

    #[test]
    fn nudge_templates_have_expected_placeholders() {
        let p = Prompts::default();
        assert!(p.context_limit_nudge.contains("{tokens}"));
        assert!(p.context_limit_nudge.contains("{limit}"));
        assert!(p.iteration_nudge.contains("{count}"));
        // Turn nudge has no placeholders.
        assert!(!p.turn_nudge.contains('{'));
    }

    #[test]
    fn field_mut_dispatches_by_filename() {
        let mut p = Prompts::default();
        *p.field_mut("system.md").unwrap() = "X".into();
        assert_eq!(p.system, "X");
        *p.field_mut("compress-range.md").unwrap() = "Y".into();
        assert_eq!(p.compress_range, "Y");
        *p.field_mut("compress-message.md").unwrap() = "Z".into();
        assert_eq!(p.compress_message, "Z");
        *p.field_mut("context-limit-nudge.md").unwrap() = "A".into();
        assert_eq!(p.context_limit_nudge, "A");
        *p.field_mut("turn-nudge.md").unwrap() = "B".into();
        assert_eq!(p.turn_nudge, "B");
        *p.field_mut("iteration-nudge.md").unwrap() = "C".into();
        assert_eq!(p.iteration_nudge, "C");
        assert!(p.field_mut("unknown.md").is_none());
    }

    #[test]
    fn override_file_names_match_struct_fields() {
        // Sanity: every name in OVERRIDE_FILE_NAMES dispatches via
        // field_mut, and they cover all six fields.
        let mut p = Prompts::default();
        for name in OVERRIDE_FILE_NAMES {
            assert!(p.field_mut(name).is_some(), "missing dispatch for {name}");
        }
        assert_eq!(OVERRIDE_FILE_NAMES.len(), 6);
    }

    // ----- PromptStore -----

    #[test]
    fn prompt_store_defaults_returns_embedded() {
        let store = PromptStore::defaults();
        assert_eq!(store.prompts(), &Prompts::default());
        assert!(store.overridden_files().is_empty());
    }

    #[test]
    fn prompt_store_from_prompts_preserves_value() {
        let p = Prompts {
            system: "custom".into(),
            ..Prompts::default()
        };
        let store = PromptStore::from_prompts(p.clone());
        assert_eq!(store.prompts(), &p);
        assert!(store.overridden_files().is_empty());
        assert_eq!(store.into_prompts(), p);
    }

    #[test]
    fn prompt_store_with_overrides_disabled_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("system.md"), "OVERRIDE").unwrap();

        let store = PromptStore::with_overrides(dir.path(), false).unwrap();
        assert_eq!(store.prompts(), &Prompts::default());
        assert!(store.overridden_files().is_empty());
    }

    #[test]
    fn prompt_store_with_overrides_enabled_replaces_only_present_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("system.md"), "SYS-OVERRIDE").unwrap();
        std::fs::write(
            dir.path().join("context-limit-nudge.md"),
            "CTX {tokens}/{limit}",
        )
        .unwrap();

        let store = PromptStore::with_overrides(dir.path(), true).unwrap();
        let p = store.prompts();
        assert_eq!(p.system, "SYS-OVERRIDE");
        assert_eq!(p.context_limit_nudge, "CTX {tokens}/{limit}");
        // Untouched fields keep defaults.
        assert_eq!(p.compress_range, DEFAULT_COMPRESS_RANGE);
        assert_eq!(p.compress_message, DEFAULT_COMPRESS_MESSAGE);
        assert_eq!(p.turn_nudge, DEFAULT_TURN_NUDGE);
        assert_eq!(p.iteration_nudge, DEFAULT_ITERATION_NUDGE);

        let mut names = store.overridden_files().to_vec();
        names.sort();
        assert_eq!(names, vec!["context-limit-nudge.md", "system.md"]);
    }

    #[test]
    fn prompt_store_missing_dir_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        let store = PromptStore::with_overrides(&nonexistent, true).unwrap();
        assert_eq!(store.prompts(), &Prompts::default());
        assert!(store.overridden_files().is_empty());
    }

    #[test]
    fn prompt_store_path_is_file_yields_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let err = PromptStore::with_overrides(&file, true).unwrap_err();
        assert!(matches!(err, PromptError::OverrideDirRead { .. }));
    }

    #[test]
    fn prompt_store_empty_override_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("turn-nudge.md"), "   \n\t  ").unwrap();
        let err = PromptStore::with_overrides(dir.path(), true).unwrap_err();
        assert!(matches!(err, PromptError::EmptyOverride { .. }));
    }

    #[test]
    fn prompt_store_unrelated_files_in_override_dir_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "ignore me").unwrap();
        std::fs::write(dir.path().join("system.md"), "ok").unwrap();
        let store = PromptStore::with_overrides(dir.path(), true).unwrap();
        assert_eq!(store.prompts().system, "ok");
        assert_eq!(store.overridden_files(), &["system.md".to_string()]);
    }

    // ----- render_system_prompt -----

    #[test]
    fn render_system_prompt_baseline_uses_default_section() {
        let p = Prompts::default();
        let out = render_system_prompt(&p, "", false, false);
        assert_eq!(out, p.system.trim());
    }

    #[test]
    fn render_system_prompt_includes_protected_tools_extension() {
        let p = Prompts::default();
        let ext = build_protected_tools_extension(&["task".into(), "skill".into()]);
        let out = render_system_prompt(&p, &ext, false, false);
        assert!(out.contains("task"));
        assert!(out.contains("skill"));
        assert!(out.contains("Protected tools"));
    }

    #[test]
    fn render_system_prompt_includes_manual_mode_note() {
        let p = Prompts::default();
        let out = render_system_prompt(&p, "", true, false);
        assert!(out.contains("Manual mode"));
    }

    #[test]
    fn render_system_prompt_includes_subagent_note() {
        let p = Prompts::default();
        let out = render_system_prompt(&p, "", false, true);
        assert!(out.contains("Sub-agent"));
    }

    #[test]
    fn render_system_prompt_combines_all_sections() {
        let p = Prompts::default();
        let ext = build_protected_tools_extension(&["task".into()]);
        let out = render_system_prompt(&p, &ext, true, true);
        assert!(out.contains("Context-pruning support"));
        assert!(out.contains("Protected tools"));
        assert!(out.contains("Manual mode"));
        assert!(out.contains("Sub-agent"));
        // Sections separated by blank lines.
        assert!(out.contains("\n\n"));
    }

    #[test]
    fn render_system_prompt_skips_blank_extension() {
        let p = Prompts::default();
        // Whitespace-only extension is dropped, no trailing blank line.
        let out = render_system_prompt(&p, "   \n  \t  ", false, false);
        assert_eq!(out, p.system.trim());
    }

    // ----- build_protected_tools_extension -----

    #[test]
    fn build_protected_tools_extension_empty_returns_empty() {
        assert!(build_protected_tools_extension(&[]).is_empty());
    }

    #[test]
    fn build_protected_tools_extension_single_tool() {
        let s = build_protected_tools_extension(&["task".into()]);
        assert!(s.contains("- task"));
        // No trailing newline.
        assert!(!s.ends_with('\n'));
    }

    #[test]
    fn build_protected_tools_extension_multiple_tools() {
        let s = build_protected_tools_extension(&[
            "task".into(),
            "skill".into(),
            "memory_search".into(),
        ]);
        assert!(s.contains("- task"));
        assert!(s.contains("- skill"));
        assert!(s.contains("- memory_search"));
        assert!(s.starts_with("Protected tools"));
    }

    #[test]
    fn build_protected_tools_extension_skips_blank_entries() {
        let s = build_protected_tools_extension(&["task".into(), "  ".into(), "skill".into()]);
        assert!(s.contains("- task"));
        assert!(s.contains("- skill"));
        // No "- " on its own.
        assert!(!s.contains("\n- \n"));
    }

    #[test]
    fn build_protected_tools_extension_all_blank_returns_empty() {
        let s = build_protected_tools_extension(&["  ".into(), "\t".into()]);
        assert!(s.is_empty());
    }

    // ----- build_nudge_extension -----

    #[test]
    fn build_nudge_extension_strong_and_soft_differ() {
        let strong = build_nudge_extension(NudgeForce::Strong);
        let soft = build_nudge_extension(NudgeForce::Soft);
        assert!(!strong.is_empty());
        assert!(!soft.is_empty());
        assert_ne!(strong, soft);
        assert!(strong.contains("compress"));
        assert!(soft.contains("compress"));
    }

    #[test]
    fn nudge_force_default_is_soft() {
        assert_eq!(NudgeForce::default(), NudgeForce::Soft);
    }

    #[test]
    fn prompts_serde_roundtrip_preserves_value() {
        let p = Prompts::default();
        let json = serde_json::to_string(&p).unwrap();
        let back: Prompts = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn prompts_serde_partial_doc_uses_defaults() {
        // Only `system` is supplied; remaining fields fall back to
        // embedded defaults thanks to `#[serde(default)]`.
        let json = r#"{"system":"X"}"#;
        let p: Prompts = serde_json::from_str(json).unwrap();
        assert_eq!(p.system, "X");
        assert_eq!(p.compress_range, DEFAULT_COMPRESS_RANGE);
        assert_eq!(p.compress_message, DEFAULT_COMPRESS_MESSAGE);
        assert_eq!(p.context_limit_nudge, DEFAULT_CONTEXT_LIMIT_NUDGE);
        assert_eq!(p.turn_nudge, DEFAULT_TURN_NUDGE);
        assert_eq!(p.iteration_nudge, DEFAULT_ITERATION_NUDGE);
    }

    #[test]
    fn nudge_force_serde_roundtrip() {
        for f in [NudgeForce::Strong, NudgeForce::Soft] {
            let s = serde_json::to_string(&f).unwrap();
            let back: NudgeForce = serde_json::from_str(&s).unwrap();
            assert_eq!(f, back);
        }
        // Lower-case rendering.
        assert_eq!(
            serde_json::to_string(&NudgeForce::Strong).unwrap(),
            "\"strong\""
        );
        assert_eq!(
            serde_json::to_string(&NudgeForce::Soft).unwrap(),
            "\"soft\""
        );
    }
}
