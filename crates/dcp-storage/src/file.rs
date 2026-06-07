//! File-based [`StatePersistence`] backend (SPEC.md §9.3).
//!
//! State is stored as JSON, one file per session, under a configurable
//! directory. Saves use the atomic write protocol:
//!
//! 1. Compute `target = <dir>/<session_id>.json`.
//! 2. Compute `tmp = <dir>/<session_id>.json.tmp.<random>`.
//! 3. If a backup is enabled and `target` already exists, copy `target` to
//!    `<dir>/<session_id>.json.bak`.
//! 4. Write the serialised JSON to `tmp` and `fsync` the file.
//! 5. Rename `tmp` → `target` (POSIX-atomic).
//! 6. `fsync` the parent directory.
//!
//! On rename failure the temp file is left in place; the next save uses a
//! fresh suffix (also documented in SPEC.md §9.3). On load, only files with
//! the `.json` extension are considered — stray `.tmp.*` siblings are
//! ignored.

#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dcp_traits::{PersistedState, PersistenceError, StatePersistence};

use crate::migrate;

/// File-based persistence backend.
///
/// Construct with [`FileStateStore::new`]; toggle backups with
/// [`FileStateStore::with_backup`].
///
/// # Example
///
/// ```no_run
/// use dcp_storage::FileStateStore;
///
/// let store = FileStateStore::new(dcp_storage::default_storage_dir());
/// // store implements `StatePersistence`.
/// # let _ = store;
/// ```
#[derive(Debug, Clone)]
pub struct FileStateStore {
    /// Directory holding `<session>.json` files.
    dir: PathBuf,
    /// When true (the default), each save copies the previous `target` to
    /// `<session>.json.bak` before writing the new file. SPEC.md §9.3
    /// `config.persistence.keep_backup` default = true.
    keep_backup: bool,
}

impl FileStateStore {
    /// Construct a backend rooted at `dir`. The directory does not have to
    /// exist; it is created on first save (mode `0700` on POSIX).
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            keep_backup: true,
        }
    }

    /// Toggle the `<session>.json.bak` snapshot taken before each save.
    ///
    /// Default is `true` per SPEC.md §9.3.
    pub fn with_backup(mut self, keep: bool) -> Self {
        self.keep_backup = keep;
        self
    }

    /// Borrow the directory the store writes into.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether backups are retained on save.
    pub fn keep_backup(&self) -> bool {
        self.keep_backup
    }

    fn ensure_dir(&self) -> Result<(), PersistenceError> {
        if self.dir.exists() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir).map_err(|e| {
            PersistenceError::Io(format!("create_dir_all {}: {e}", self.dir.display()))
        })?;
        // Best-effort `0700` per SPEC.md §9.3 edge cases.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Failure here is non-fatal: the file backend still works on
            // platforms / filesystems that ignore the bit.
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    fn target_path(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.json"))
    }

    fn backup_path(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.json.bak"))
    }

    fn temp_path(&self, session_id: &str) -> PathBuf {
        self.dir
            .join(format!("{session_id}.json.tmp.{}", next_temp_suffix()))
    }

    /// Best-effort `fsync` of the parent directory after a successful
    /// rename (SPEC.md §9.3 step 6).
    #[cfg(unix)]
    fn fsync_dir(&self) {
        if let Ok(d) = File::open(&self.dir) {
            let _ = d.sync_all();
        }
    }

    #[cfg(not(unix))]
    fn fsync_dir(&self) {
        // No-op on platforms where directory fsync is not meaningful.
    }
}

impl StatePersistence for FileStateStore {
    fn load(&self, session_id: &str) -> Result<Option<PersistedState>, PersistenceError> {
        let path = self.target_path(session_id);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(PersistenceError::Io(format!(
                    "read {}: {e}",
                    path.display()
                )));
            }
        };
        let parsed: PersistedState = serde_json::from_slice(&bytes)
            .map_err(|e| PersistenceError::Serde(format!("decode {}: {e}", path.display())))?;
        // SPEC.md §9.2: every load passes through the migration trampoline
        // so callers can trust the returned variant is the current one.
        Ok(Some(PersistedState::V1(migrate(parsed))))
    }

    fn save(&self, session_id: &str, state: &PersistedState) -> Result<(), PersistenceError> {
        self.ensure_dir()?;
        let target = self.target_path(session_id);
        let tmp = self.temp_path(session_id);

        // Step 3: snapshot the previous target into `.bak` (best-effort).
        if self.keep_backup && target.exists() {
            let bak = self.backup_path(session_id);
            // SPEC.md §9.3: "Backup copy fails: log warning, proceed with
            // write." We do not have a logger wired here; surface the
            // failure on stderr but never abort the save.
            if let Err(e) = fs::copy(&target, &bak) {
                eprintln!(
                    "dcp-storage: backup copy {} -> {} failed: {e}",
                    target.display(),
                    bak.display()
                );
            }
        }

        // Step 4: serialise + write to tmp + fsync.
        let json = serde_json::to_vec_pretty(state)
            .map_err(|e| PersistenceError::Serde(format!("encode: {e}")))?;
        {
            let mut f = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp)
                .map_err(|e| PersistenceError::Io(format!("create {}: {e}", tmp.display())))?;
            f.write_all(&json)
                .map_err(|e| PersistenceError::Io(format!("write {}: {e}", tmp.display())))?;
            f.sync_all()
                .map_err(|e| PersistenceError::Io(format!("fsync {}: {e}", tmp.display())))?;
        }

        // Step 5: rename tmp → target. Atomic on POSIX.
        if let Err(e) = fs::rename(&tmp, &target) {
            // Per SPEC.md §9.3: do not auto-delete the temp file; the
            // next save will reuse a fresh suffix. Bubble up the error so
            // the caller can react.
            return Err(PersistenceError::Io(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                target.display()
            )));
        }

        // Step 6: fsync parent dir.
        self.fsync_dir();

        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.dir)
            .map_err(|e| PersistenceError::Io(format!("read_dir {}: {e}", self.dir.display())))?;
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| PersistenceError::Io(format!("read_dir entry: {e}")))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Filter to exactly `*.json` (no `.bak`, no `.tmp.*`).
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // The file stem is the session id. `Path::file_stem` strips
            // exactly one extension, so for `s.json` it returns `s`.
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                sessions.push(stem.to_string());
            }
        }
        sessions.sort();
        Ok(sessions)
    }

    fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
        let target = self.target_path(session_id);
        match fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PersistenceError::Io(format!(
                "remove {}: {e}",
                target.display()
            ))),
        }
    }
}

/// Generate a unique suffix for `*.tmp.<suffix>` files.
///
/// Combines pid, monotonic-ish nanoseconds since epoch, and a per-process
/// atomic counter so that concurrent saves within the same process and
/// rapid-fire saves across processes never collide on a temp filename.
fn next_temp_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{pid}-{nanos}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_traits::PersistedStateV1;
    use tempfile::tempdir;

    fn sample(id: &str, turn: u32) -> PersistedState {
        PersistedState::V1(PersistedStateV1 {
            session_id: id.into(),
            last_updated: "2024-01-01T00:00:00Z".into(),
            current_turn: turn,
            next_block_id: 1,
            next_run_id: 1,
            next_message_ref: 1,
            ..Default::default()
        })
    }

    #[test]
    fn round_trip_save_load_in_tempdir() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());
        let s = sample("sess", 3);

        // Empty initially.
        assert!(store.load("sess").unwrap().is_none());

        store.save("sess", &s).unwrap();

        // The on-disk file exists and is named `<id>.json`.
        let target = dir.path().join("sess.json");
        assert!(
            target.is_file(),
            "target not created at {}",
            target.display()
        );

        // No leftover temp files.
        for entry in fs::read_dir(dir.path()).unwrap() {
            let p = entry.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                !name.contains(".tmp."),
                "unexpected leftover temp file {name}"
            );
        }

        // Load returns the saved value.
        assert_eq!(store.load("sess").unwrap(), Some(s.clone()));

        // Overwrite path: backup of the previous version is created.
        let s2 = sample("sess", 9);
        store.save("sess", &s2).unwrap();
        assert_eq!(store.load("sess").unwrap(), Some(s2));
        let bak = dir.path().join("sess.json.bak");
        assert!(bak.is_file(), "backup not created at {}", bak.display());
        // The .bak holds the *previous* contents (state with turn=3).
        let bak_bytes = fs::read(&bak).unwrap();
        let bak_state: PersistedState = serde_json::from_slice(&bak_bytes).unwrap();
        assert_eq!(bak_state, s);
    }

    #[test]
    fn save_creates_directory_if_missing() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("sessions");
        let store = FileStateStore::new(nested.clone());
        assert!(!nested.exists());

        store.save("s", &sample("s", 0)).unwrap();
        assert!(nested.is_dir());
        assert!(nested.join("s.json").is_file());
    }

    #[test]
    fn with_backup_false_skips_bak_file() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf()).with_backup(false);

        store.save("sess", &sample("sess", 1)).unwrap();
        store.save("sess", &sample("sess", 2)).unwrap();

        assert!(!dir.path().join("sess.json.bak").exists());
        assert!(dir.path().join("sess.json").is_file());
    }

    #[test]
    fn atomic_write_interrupt_keeps_old_target_and_load_value() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());

        // First successful save.
        let original = sample("sess", 1);
        store.save("sess", &original).unwrap();

        // Simulate a crashed save: mid-protocol the writer would have
        // produced a `<id>.json.tmp.<random>` file with possibly partial
        // contents but never reached the rename step.
        let stray_tmp = dir.path().join("sess.json.tmp.crashy");
        fs::write(&stray_tmp, b"{ \"schema_version\": \"1\", \"truncated...").unwrap();

        // The target file must still hold the previous, valid content.
        let target_bytes = fs::read(dir.path().join("sess.json")).unwrap();
        let on_disk: PersistedState = serde_json::from_slice(&target_bytes).unwrap();
        assert_eq!(on_disk, original);

        // load() recovers the original (NOT the corrupted temp).
        assert_eq!(store.load("sess").unwrap(), Some(original.clone()));

        // list_sessions ignores the .tmp file and only reports the .json.
        assert_eq!(store.list_sessions().unwrap(), vec!["sess".to_string()]);

        // A subsequent successful save still works (uses a fresh suffix).
        let next = sample("sess", 2);
        store.save("sess", &next).unwrap();
        assert_eq!(store.load("sess").unwrap(), Some(next));

        // The crash-leftover temp from before is still present (per SPEC
        // §9.3: "the temp file is *not* automatically deleted"). The
        // store does not see it on list_sessions because the extension
        // is not `.json`.
        assert!(stray_tmp.exists());
        assert_eq!(store.list_sessions().unwrap(), vec!["sess".to_string()]);
    }

    #[test]
    fn list_sessions_returns_sorted_and_filters_non_json() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());

        // Insert in non-alphabetical order.
        store.save("gamma", &sample("gamma", 0)).unwrap();
        store.save("alpha", &sample("alpha", 0)).unwrap();
        store.save("beta", &sample("beta", 0)).unwrap();

        // Drop in some non-`.json` siblings that must be ignored.
        fs::write(dir.path().join("notes.txt"), b"hi").unwrap();
        fs::write(dir.path().join("alpha.json.bak"), b"old").unwrap();
        fs::write(dir.path().join("delta.json.tmp.123"), b"partial").unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(
            sessions,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn list_sessions_on_missing_dir_returns_empty() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("never-created");
        let store = FileStateStore::new(missing);
        assert_eq!(store.list_sessions().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn delete_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());

        store.save("x", &sample("x", 0)).unwrap();
        assert!(dir.path().join("x.json").is_file());

        store.delete("x").unwrap();
        assert!(!dir.path().join("x.json").exists());

        // Repeat delete is a no-op.
        store.delete("x").unwrap();
        store.delete("never-existed").unwrap();
    }

    #[test]
    fn load_corrupted_target_returns_serde_error() {
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());

        store.ensure_dir().unwrap();
        fs::write(dir.path().join("bad.json"), b"{ not json").unwrap();

        let err = store.load("bad").unwrap_err();
        assert!(matches!(err, PersistenceError::Serde(_)), "got: {err:?}");
    }

    #[test]
    fn migration_trampoline_runs_on_load() {
        // Today the trampoline is a no-op for V1, but verify a V1 doc
        // round-trips through `load`.
        let dir = tempdir().unwrap();
        let store = FileStateStore::new(dir.path().to_path_buf());
        let s = sample("mig", 11);
        store.save("mig", &s).unwrap();
        let loaded = store.load("mig").unwrap().unwrap();
        // Both are PersistedState::V1 and equal.
        assert_eq!(loaded, s);
    }

    /// Compile-time assertion: `FileStateStore` is `Send + Sync` and can
    /// stand in for a `dyn StatePersistence + Send + Sync`.
    #[test]
    fn store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FileStateStore>();
    }

    #[test]
    fn temp_suffix_is_unique_across_calls() {
        // Smoke test: two suffixes generated back-to-back differ.
        let a = next_temp_suffix();
        let b = next_temp_suffix();
        assert_ne!(a, b);
    }
}
