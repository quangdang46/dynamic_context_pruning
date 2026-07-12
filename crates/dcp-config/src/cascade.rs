//! Cascade resolution per SPEC.md §10.1 and PLAN.md §8.2.
//!
//! Resolution order, each later layer overriding earlier ones:
//!
//! 1. Built-in defaults (compiled into the library).
//! 2. Global config — `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`
//!    with the canonical fallback `~/.config/dynamic_context_pruning/config.jsonc`.
//! 3. Custom directory — `$DCP_CONFIG_DIR/config.jsonc` when set.
//! 4. Project config — `.dynamic_context_pruning/config.jsonc` in the
//!    current working directory or any ancestor up to the filesystem
//!    root or a marker file (`.git`, `Cargo.toml`, `pyproject.toml`,
//!    `package.json`).
//!
//! Programmatic overrides happen on the resulting [`Config`] and live
//! in `dcp-core`.

use std::env;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::Config;
use crate::error::ConfigError;

/// Filename used for every cascade layer (library default).
pub const CONFIG_FILE_NAME: &str = "config.jsonc";

/// Alternate filenames accepted for OpenCode host compatibility
/// (upstream `@tarquinen/opencode-dcp` uses `dcp.jsonc` / `dcp.json`).
pub const OPENCODE_CONFIG_NAMES: &[&str] = &["dcp.jsonc", "dcp.json", CONFIG_FILE_NAME];

/// Project-config directory name (located in the project root).
pub const PROJECT_DIR_NAME: &str = ".dynamic_context_pruning";

/// OpenCode project plugin directory (upstream DCP).
pub const OPENCODE_PROJECT_DIR: &str = ".opencode";

/// Environment variable for the custom-directory layer.
pub const ENV_DCP_CONFIG_DIR: &str = "DCP_CONFIG_DIR";

/// Also honour OpenCode's config-dir env when set.
pub const ENV_OPENCODE_CONFIG_DIR: &str = "OPENCODE_CONFIG_DIR";

/// Project-root marker filenames considered when walking up from CWD.
pub const PROJECT_MARKERS: &[&str] = &[".git", "Cargo.toml", "pyproject.toml", "package.json"];

/// Locations to consult, in cascade order. Public so dcp-core can reuse
/// them (e.g. for `--config` debugging output).
///
/// Layers are applied left-to-right; later paths override earlier ones.
/// OpenCode-compatible paths are interleaved so a drop-in install of
/// this plugin can share `~/.config/opencode/dcp.jsonc` with the TS
/// upstream package.
#[derive(Clone, Debug, Default)]
pub struct ResolvedPaths {
    /// Path to the global config file, when present and readable.
    pub global: Option<PathBuf>,
    /// Path to the custom-directory config file, when present and readable.
    pub custom: Option<PathBuf>,
    /// Path to the project config file, when present and readable.
    pub project: Option<PathBuf>,
    /// Extra overlay paths (OpenCode global / project), applied after
    /// the three primary layers so they win on key conflict.
    pub overlays: Vec<PathBuf>,
}

impl ResolvedPaths {
    /// Discover all cascade paths from the environment and the working
    /// directory.
    pub fn discover() -> Self {
        Self {
            global: discover_global(),
            custom: discover_custom(),
            project: discover_project(env::current_dir().ok().as_deref()),
            overlays: discover_opencode_overlays(env::current_dir().ok().as_deref()),
        }
    }

    /// Like [`Self::discover`] but rooted at a specific project base
    /// directory (used by tests).
    pub fn discover_at(base_dir: &Path) -> Self {
        Self {
            global: discover_global(),
            custom: discover_custom(),
            project: discover_project(Some(base_dir)),
            overlays: discover_opencode_overlays(Some(base_dir)),
        }
    }

    /// Ordered list of every config file that will be merged.
    pub fn all_layers(&self) -> Vec<&PathBuf> {
        let mut out = Vec::new();
        if let Some(p) = &self.global {
            out.push(p);
        }
        if let Some(p) = &self.custom {
            out.push(p);
        }
        if let Some(p) = &self.project {
            out.push(p);
        }
        for p in &self.overlays {
            out.push(p);
        }
        out
    }
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn discover_global() -> Option<PathBuf> {
    let dir = if let Ok(p) = env::var("XDG_CONFIG_HOME").map(PathBuf::from)
        && !p.as_os_str().is_empty()
    {
        Some(p)
    } else {
        dirs::config_dir()
    }?;
    // Prefer library path, then OpenCode global config dir.
    let lib = dir.join("dynamic_context_pruning");
    if let Some(p) = first_existing(&lib, OPENCODE_CONFIG_NAMES) {
        return Some(p);
    }
    let oc = dir.join("opencode");
    first_existing(&oc, OPENCODE_CONFIG_NAMES)
}

fn discover_custom() -> Option<PathBuf> {
    for env_key in [ENV_DCP_CONFIG_DIR, ENV_OPENCODE_CONFIG_DIR] {
        let Ok(raw) = env::var(env_key) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let dir = PathBuf::from(raw);
        if let Some(p) = first_existing(&dir, OPENCODE_CONFIG_NAMES) {
            return Some(p);
        }
    }
    None
}

/// OpenCode-specific overlays applied last (highest priority for shared keys
/// when both library and OpenCode project files exist).
fn discover_opencode_overlays(base: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();

    // Global OpenCode dcp.jsonc (if not already selected as `global`).
    if let Some(config_home) = dirs::config_dir() {
        let oc_global = config_home.join("opencode");
        if let Some(p) = first_existing(&oc_global, &["dcp.jsonc", "dcp.json"]) {
            out.push(p);
        }
    }

    // Project `.opencode/dcp.jsonc` walking up from base.
    if let Some(start) = base {
        let mut current = Some(start.to_path_buf());
        while let Some(dir) = current {
            let candidate_dir = dir.join(OPENCODE_PROJECT_DIR);
            if let Some(p) = first_existing(&candidate_dir, &["dcp.jsonc", "dcp.json"]) {
                out.push(p);
                break;
            }
            let marker_present = PROJECT_MARKERS
                .iter()
                .any(|m| dir.join(m).exists());
            if marker_present {
                break;
            }
            current = dir.parent().map(|p| p.to_path_buf());
        }
    }

    out
}

fn discover_project(base: Option<&Path>) -> Option<PathBuf> {
    let start = base?;
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        // Library project dir first, then OpenCode project dir.
        if let Some(p) = first_existing(&dir.join(PROJECT_DIR_NAME), OPENCODE_CONFIG_NAMES) {
            return Some(p);
        }
        if let Some(p) = first_existing(&dir.join(OPENCODE_PROJECT_DIR), &["dcp.jsonc", "dcp.json"])
        {
            return Some(p);
        }
        // If we hit a project marker without finding the config, stop —
        // SPEC.md §10.1 forbids escaping past the project root.
        let marker_present = PROJECT_MARKERS.iter().any(|m| dir.join(m).exists())
            && !dir.join(PROJECT_DIR_NAME).is_dir()
            && !dir.join(OPENCODE_PROJECT_DIR).is_dir();
        if marker_present {
            return None;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

/// Load the cascade with the SPEC.md §10.1 resolution rules.
///
/// Returns a fully resolved (and validated) [`Config`].
pub fn load_default() -> Result<Config, ConfigError> {
    load_with_paths(&ResolvedPaths::discover())
}

/// Same as [`load_default`] but rooted at a specific project base
/// directory. Mostly useful for tests; production callers should use
/// the env-driven [`load_default`].
pub fn load_default_at(base_dir: &Path) -> Result<Config, ConfigError> {
    load_with_paths(&ResolvedPaths::discover_at(base_dir))
}

/// Load the cascade given a pre-discovered set of paths.
pub fn load_with_paths(paths: &ResolvedPaths) -> Result<Config, ConfigError> {
    // Start from the built-in defaults serialized as a JSON value, so the
    // merge step is a single deep-merge over JSON.
    let mut acc = serde_json::to_value(Config::default())
        .map_err(|e| ConfigError::Deserialize(e.to_string()))?;

    // Dedup paths that may appear in both primary layers and OpenCode overlays.
    let mut seen = std::collections::HashSet::new();
    for path in paths.all_layers() {
        let key = path.to_string_lossy().to_string();
        if !seen.insert(key) {
            continue;
        }
        let overlay = read_layer(path)?;
        // Map OpenCode-only key aliases onto the library schema before merge.
        let overlay = normalize_opencode_aliases(overlay);
        deep_merge(&mut acc, overlay);
    }

    let mut config: Config = serde_json::from_value(acc).map_err(|e| {
        ConfigError::Deserialize(format!("failed to deserialise merged configuration: {e}"))
    })?;
    config.rebuild_cache()?;
    config.validate()?;
    Ok(config)
}

/// Translate legacy / OpenCode-only config keys into the library schema.
///
/// Upstream TS DCP uses `pruneNotification` / `pruneNotificationType` while
/// the Rust port stores `notification.level` / `notification.kind`.
fn normalize_opencode_aliases(mut value: Value) -> Value {
    let Some(obj) = value.as_object_mut() else {
        return value;
    };

    if let Some(level) = obj.remove("pruneNotification") {
        let entry = obj
            .entry("notification".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(n) = entry.as_object_mut() {
            n.entry("level".to_string()).or_insert(level);
        }
    }
    if let Some(kind) = obj.remove("pruneNotificationType") {
        let entry = obj
            .entry("notification".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(n) = entry.as_object_mut() {
            n.entry("kind".to_string()).or_insert(kind);
        }
    }
    // drop autoUpdate — library ignores it silently via default/unknown
    obj.remove("autoUpdate");
    obj.remove("$schema");

    value
}

/// Parse a single JSONC document into a `serde_json::Value` overlay.
pub(crate) fn read_layer(path: &Path) -> Result<Value, ConfigError> {
    let body = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_jsonc_value(&body).map_err(|message| ConfigError::Parse {
        path: path.to_path_buf(),
        message,
    })
}

/// Parse a JSONC / JSON5 string into a `serde_json::Value`.
pub fn parse_jsonc_value(body: &str) -> Result<Value, String> {
    if body.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    json5::from_str::<Value>(body).map_err(|e| e.to_string())
}

/// Deep-merge `overlay` onto `base`. Object fields are merged
/// key-by-key; arrays and scalars are replaced wholesale (SPEC.md
/// §10.1).
pub(crate) fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, other) => {
            *slot = other;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge_replaces_arrays_wholesale() {
        let mut base = serde_json::json!({ "list": [1, 2, 3], "scalar": 1 });
        let overlay = serde_json::json!({ "list": [9], "extra": true });
        deep_merge(&mut base, overlay);
        assert_eq!(
            base,
            serde_json::json!({
                "list": [9],
                "scalar": 1,
                "extra": true,
            })
        );
    }

    #[test]
    fn deep_merge_recurses_into_objects() {
        let mut base = serde_json::json!({ "nested": { "a": 1, "b": 2 }});
        let overlay = serde_json::json!({ "nested": { "b": 20, "c": 30 }});
        deep_merge(&mut base, overlay);
        assert_eq!(
            base,
            serde_json::json!({
                "nested": { "a": 1, "b": 20, "c": 30 },
            })
        );
    }

    #[test]
    fn deep_merge_replaces_scalars() {
        let mut base = serde_json::json!({ "a": 1 });
        deep_merge(&mut base, serde_json::json!({ "a": 2 }));
        assert_eq!(base, serde_json::json!({ "a": 2 }));
    }

    #[test]
    fn parse_jsonc_handles_comments_and_trailing_commas() {
        let body = r#"
            // line comment
            {
                /* block comment */
                "enabled": true,
                "debug": false, // trailing comment
            }
        "#;
        let v = parse_jsonc_value(body).unwrap();
        assert_eq!(v.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(v.get("debug"), Some(&Value::Bool(false)));
    }

    #[test]
    fn parse_empty_returns_empty_object() {
        let v = parse_jsonc_value("").unwrap();
        assert_eq!(v, Value::Object(Map::new()));
    }

    #[test]
    fn parse_invalid_returns_error() {
        assert!(parse_jsonc_value("{ \"a\": }").is_err());
    }
}
