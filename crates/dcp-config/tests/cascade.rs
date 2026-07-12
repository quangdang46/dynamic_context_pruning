//! Integration tests for the cascade resolver using tempdirs and env
//! var manipulation. SPEC.md §10.1.
//!
//! These tests serialise on the global env (env vars are
//! process-wide), so the `env::set_var` / `env::remove_var` lives
//! inside a global `Mutex`.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use dcp_config::{
    CONFIG_FILE_NAME, ENV_DCP_CONFIG_DIR, PROJECT_DIR_NAME, ResolvedPaths, load_default,
    load_default_at, load_with_paths,
};
use tempfile::TempDir;

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

/// Acquire a process-wide lock so cascade tests don't race on the env.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Restore env-var state captured at test start.
struct EnvGuard {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn capture(keys: &[&'static str]) -> Self {
        let captured = keys.iter().map(|k| (*k, std::env::var(*k).ok())).collect();
        Self { keys: captured }
    }

    fn set(&self, key: &str, value: &str) {
        // SAFETY: env-var mutation is process-global and is serialised
        // across cascade tests by `env_lock()`.
        unsafe { std::env::set_var(key, value) };
    }

    fn unset(&self, key: &str) {
        // SAFETY: same as `set`.
        unsafe { std::env::remove_var(key) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in &self.keys {
            // SAFETY: same as `EnvGuard::set`.
            unsafe {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn write_jsonc(dir: &Path, body: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(CONFIG_FILE_NAME), body).unwrap();
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[test]
fn defaults_when_no_files_present() {
    let _g = env_lock();
    let env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR, "HOME"]);
    let temp = TempDir::new().unwrap();
    env.set("XDG_CONFIG_HOME", temp.path().join("xdg").to_str().unwrap());
    env.unset(ENV_DCP_CONFIG_DIR);

    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).unwrap();
    // Force the project marker so the project search stops here.
    fs::write(project_root.join("Cargo.toml"), "").unwrap();

    let cfg = load_default_at(&project_root).expect("load_default must succeed");
    assert!(cfg.enabled);
    assert!(!cfg.debug);
    assert_eq!(cfg.compress.nudge_frequency, 5);
}

#[test]
fn project_overrides_global() {
    let _g = env_lock();
    let env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR, "HOME"]);
    let temp = TempDir::new().unwrap();

    // Global layer: nudgeFrequency = 7, debug = true.
    let xdg = temp.path().join("xdg");
    let global_dir = xdg.join("dynamic_context_pruning");
    write_jsonc(
        &global_dir,
        r#"{
            "debug": true,
            "compress": { "nudgeFrequency": 7 }
        }"#,
    );
    env.set("XDG_CONFIG_HOME", xdg.to_str().unwrap());
    env.unset(ENV_DCP_CONFIG_DIR);

    // Project layer: nudgeFrequency = 11 — overrides global.
    let project_root = temp.path().join("project");
    let project_dir = project_root.join(PROJECT_DIR_NAME);
    write_jsonc(
        &project_dir,
        r#"{
            "compress": { "nudgeFrequency": 11 }
        }"#,
    );

    let cfg = load_default_at(&project_root).expect("load");
    assert!(cfg.debug, "global debug=true must propagate");
    assert_eq!(cfg.compress.nudge_frequency, 11, "project must win");
    // Defaults preserved where neither layer mentions them.
    assert_eq!(cfg.compress.iteration_nudge_threshold, 15);
}

#[test]
fn custom_dir_overrides_global_under_project() {
    let _g = env_lock();
    let env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR, "HOME"]);
    let temp = TempDir::new().unwrap();

    // Global: enabled=true (the default). nudgeFrequency=2.
    let xdg = temp.path().join("xdg");
    let global_dir = xdg.join("dynamic_context_pruning");
    write_jsonc(&global_dir, r#"{ "compress": { "nudgeFrequency": 2 } }"#);
    env.set("XDG_CONFIG_HOME", xdg.to_str().unwrap());

    // Custom: nudgeFrequency=3, manualMode.enabled=true.
    let custom = temp.path().join("custom");
    write_jsonc(
        &custom,
        r#"{
            "compress": { "nudgeFrequency": 3 },
            "manualMode": { "enabled": true }
        }"#,
    );
    env.set(ENV_DCP_CONFIG_DIR, custom.to_str().unwrap());

    // Project: nudgeFrequency=4 — overrides custom.
    let project_root = temp.path().join("project");
    let project_dir = project_root.join(PROJECT_DIR_NAME);
    write_jsonc(&project_dir, r#"{ "compress": { "nudgeFrequency": 4 } }"#);

    let cfg = load_default_at(&project_root).expect("load");
    assert_eq!(cfg.compress.nudge_frequency, 4, "project wins");
    assert!(cfg.manual_mode.enabled, "custom propagates manualMode");
}

#[test]
fn load_with_paths_explicit_layers() {
    let _g = env_lock();
    let _env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR]);
    let temp = TempDir::new().unwrap();
    let global = temp.path().join("global.jsonc");
    fs::write(
        &global,
        r#"{ "debug": true, "compress": { "nudgeFrequency": 8 } }"#,
    )
    .unwrap();
    let project = temp.path().join("project.jsonc");
    fs::write(&project, r#"{ "compress": { "nudgeFrequency": 9 } }"#).unwrap();

    let paths = ResolvedPaths {
        global: Some(global),
        custom: None,
        project: Some(project),
        overlays: vec![],
    };
    let cfg = load_with_paths(&paths).expect("load");
    assert!(cfg.debug);
    assert_eq!(cfg.compress.nudge_frequency, 9);
}

#[test]
fn jsonc_supports_comments_and_trailing_commas() {
    let _g = env_lock();
    let _env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR]);
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.jsonc");
    fs::write(
        &path,
        r#"
        // This is a JSONC comment.
        {
            /* block comment */
            "enabled": true,
            "debug": true,            // trailing line comment
            "compress": {
                "nudgeFrequency": 7,  // trailing comma below is intentional
            },
        }
        "#,
    )
    .unwrap();

    let paths = ResolvedPaths {
        global: Some(path),
        custom: None,
        project: None,
        overlays: vec![],
    };
    let cfg = load_with_paths(&paths).expect("jsonc parses");
    assert!(cfg.debug);
    assert_eq!(cfg.compress.nudge_frequency, 7);
}

#[test]
fn invalid_config_value_is_rejected_at_load() {
    let _g = env_lock();
    let _env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR]);
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.jsonc");
    // nudgeFrequency = 0 is out of range.
    fs::write(&path, r#"{ "compress": { "nudgeFrequency": 0 } }"#).unwrap();

    let paths = ResolvedPaths {
        global: Some(path),
        custom: None,
        project: None,
        overlays: vec![],
    };
    let err = load_with_paths(&paths).expect_err("validation must fail");
    let msg = format!("{err}");
    assert!(msg.contains("nudgeFrequency"), "got: {msg}");
}

#[test]
fn unknown_keys_are_ignored_silently() {
    let _g = env_lock();
    let _env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR]);
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("config.jsonc");
    fs::write(
        &path,
        r#"{
            "futureKey": 42,
            "compress": { "nudgeFrequency": 7, "tomorrowField": "?" }
        }"#,
    )
    .unwrap();

    let paths = ResolvedPaths {
        global: Some(path),
        custom: None,
        project: None,
        overlays: vec![],
    };
    let cfg = load_with_paths(&paths).expect("unknown keys must not block load");
    assert_eq!(cfg.compress.nudge_frequency, 7);
}

#[test]
fn project_config_walks_up_to_marker() {
    let _g = env_lock();
    let _env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR]);
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repo");
    fs::write(temp.path().join("noisy_outside"), "").unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(".git"), "").unwrap(); // marker
    let pkg_dir = root.join("crates").join("inner");
    fs::create_dir_all(&pkg_dir).unwrap();
    write_jsonc(&root.join(PROJECT_DIR_NAME), r#"{ "debug": true }"#);

    // Searching from a subdirectory must find the project config at the
    // marker root.
    let cfg = load_default_at(&pkg_dir).expect("load");
    assert!(cfg.debug);
}

#[test]
fn load_default_smoke_test() {
    // The bare entry point must at least not panic when no config files
    // are present.
    let _g = env_lock();
    let env = EnvGuard::capture(&["XDG_CONFIG_HOME", ENV_DCP_CONFIG_DIR, "HOME"]);
    let temp = TempDir::new().unwrap();
    env.set(
        "XDG_CONFIG_HOME",
        temp.path().join("nope").to_str().unwrap(),
    );
    env.unset(ENV_DCP_CONFIG_DIR);
    let _ = load_default();
}
