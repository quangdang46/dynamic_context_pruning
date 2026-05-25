#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-storage` — concrete [`StatePersistence`] backends for the
//! `dynamic_context_pruning` library.
//!
//! Two default backends ship in this crate:
//!
//! * [`FileStateStore`] — XDG-compliant JSON-on-disk persistence with the
//!   atomic write protocol from SPEC.md §9.3 (write-temp → fsync → rename
//!   → fsync-dir, with an optional `.bak` snapshot of the previous save).
//! * [`InMemoryStateStore`] — a `BTreeMap`-backed store useful for tests
//!   and short-lived sessions.
//!
//! Optional backends behind feature flags (Phase 2+):
//!
//! * `sled` — embedded key-value store (skipped in this phase pending a
//!   non-trivial design).
//! * `sqlite` — embedded SQL store.
//!
//! [`StatePersistence`]: dcp_traits::StatePersistence
//!
//! # Choosing a backend
//!
//! ```rust
//! use dcp_storage::{InMemoryStateStore, default_storage_dir, FileStateStore};
//!
//! // Disk-backed (the production default):
//! let _disk = FileStateStore::new(default_storage_dir());
//!
//! // In-memory (for tests):
//! let _mem = InMemoryStateStore::new();
//! ```
//!
//! # Schema migration
//!
//! Persisted documents are tagged with `schema_version`. The crate exposes
//! [`migrate`] as a single entry point; for now it is a no-op trampoline
//! since only `V1` exists. Future versions chain through this function.

use std::path::PathBuf;

use dcp_traits::{PersistedState, PersistedStateV1};

mod file;
mod memory;

pub use file::FileStateStore;
pub use memory::InMemoryStateStore;

// Re-export the trait + schema types so downstream users can import
// everything they need from a single crate.
pub use dcp_traits::{PersistenceError, StatePersistence};

/// Default on-disk location for [`FileStateStore`].
///
/// On every platform this resolves to `<data-dir>/dynamic_context_pruning/sessions`,
/// where `<data-dir>` is the platform-appropriate "user data" directory:
///
/// * **Linux/BSD**: `$XDG_DATA_HOME` (or `~/.local/share` when that env
///   var is unset), per the XDG Base Directory specification.
/// * **macOS**: `~/Library/Application Support`.
/// * **Windows**: `%APPDATA%`.
///
/// If [`dirs::data_dir`] cannot resolve (e.g. `$HOME` is unset), the
/// function falls back to `~/.local/share/dynamic_context_pruning/sessions`
/// to preserve a deterministic shape; if even `$HOME` is unset, the path
/// resolves relative to the current directory (`./.local/share/...`).
///
/// Per SPEC.md §9.3, the directory is created on first write — no I/O is
/// performed by this function.
///
/// # Example
///
/// ```rust
/// let dir = dcp_storage::default_storage_dir();
/// assert!(dir.ends_with("dynamic_context_pruning/sessions"));
/// ```
pub fn default_storage_dir() -> PathBuf {
    if let Some(data) = dirs::data_dir() {
        return data.join("dynamic_context_pruning").join("sessions");
    }
    // Fallback path documented in SPEC.md §9.3.
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".local")
        .join("share")
        .join("dynamic_context_pruning")
        .join("sessions")
}

/// Schema migration trampoline.
///
/// SPEC.md §9.2: every loader runs the persisted document through
/// `migrate` to produce the current canonical schema. Today the only
/// schema version is `V1`, so this is a no-op extraction; new versions
/// will chain through here (`V1 → V2`, `V2 → V3`, …) so callers do not
/// have to know about every intermediate step.
///
/// # Example
///
/// ```rust
/// use dcp_storage::migrate;
/// use dcp_traits::{PersistedState, PersistedStateV1};
///
/// let v1 = PersistedStateV1 {
///     session_id: "s1".into(),
///     last_updated: "2024-01-01T00:00:00Z".into(),
///     ..Default::default()
/// };
/// let migrated = migrate(PersistedState::V1(v1.clone()));
/// assert_eq!(migrated, v1);
/// ```
pub fn migrate(persisted: PersistedState) -> PersistedStateV1 {
    match persisted {
        PersistedState::V1(v1) => v1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_v1_is_noop() {
        let v1 = PersistedStateV1 {
            session_id: "abc".into(),
            last_updated: "2024-06-01T00:00:00Z".into(),
            current_turn: 4,
            next_block_id: 2,
            next_run_id: 1,
            next_message_ref: 5,
            ..Default::default()
        };
        let out = migrate(PersistedState::V1(v1.clone()));
        assert_eq!(out, v1);
    }

    #[test]
    fn default_storage_dir_ends_in_sessions() {
        let dir = default_storage_dir();
        // The function must not perform I/O.
        assert!(dir.ends_with("sessions"));
        // Last two components must be `dynamic_context_pruning/sessions`.
        let mut comps = dir.components().rev();
        assert_eq!(
            comps.next().and_then(|c| c.as_os_str().to_str()),
            Some("sessions")
        );
        assert_eq!(
            comps.next().and_then(|c| c.as_os_str().to_str()),
            Some("dynamic_context_pruning")
        );
    }
}
