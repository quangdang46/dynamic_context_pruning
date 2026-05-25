//! In-memory [`StatePersistence`] backend.
//!
//! Backed by a [`std::collections::BTreeMap`] under a [`std::sync::Mutex`]
//! so it is `Send + Sync` and a single instance can be shared across
//! threads. State does not survive process exit; this backend is intended
//! for tests and for hosts that explicitly opt out of disk persistence.

use std::collections::BTreeMap;
use std::sync::Mutex;

use dcp_traits::{PersistedState, PersistenceError, StatePersistence};

/// In-memory [`StatePersistence`] backend.
///
/// # Example
///
/// ```rust
/// use dcp_storage::{InMemoryStateStore, StatePersistence};
/// use dcp_traits::{PersistedState, PersistedStateV1};
///
/// let store = InMemoryStateStore::new();
/// let state = PersistedState::V1(PersistedStateV1 {
///     session_id: "sess".into(),
///     last_updated: "2024-01-01T00:00:00Z".into(),
///     ..Default::default()
/// });
/// store.save("sess", &state).unwrap();
/// assert_eq!(store.load("sess").unwrap(), Some(state));
/// ```
#[derive(Debug, Default)]
pub struct InMemoryStateStore {
    inner: Mutex<BTreeMap<String, PersistedState>>,
}

impl InMemoryStateStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    /// Number of sessions currently held.
    ///
    /// Convenience for tests.
    pub fn len(&self) -> usize {
        self.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// `true` if no sessions are held.
    pub fn is_empty(&self) -> bool {
        self.lock().map(|g| g.is_empty()).unwrap_or(true)
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, PersistedState>>, PersistenceError> {
        self.inner
            .lock()
            .map_err(|e| PersistenceError::Backend(format!("mutex poisoned: {e}")))
    }
}

impl StatePersistence for InMemoryStateStore {
    fn load(&self, session_id: &str) -> Result<Option<PersistedState>, PersistenceError> {
        let guard = self.lock()?;
        Ok(guard.get(session_id).cloned())
    }

    fn save(&self, session_id: &str, state: &PersistedState) -> Result<(), PersistenceError> {
        let mut guard = self.lock()?;
        guard.insert(session_id.to_string(), state.clone());
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        let guard = self.lock()?;
        // BTreeMap iteration is sorted ascending; preserve that contract.
        Ok(guard.keys().cloned().collect())
    }

    fn delete(&self, session_id: &str) -> Result<(), PersistenceError> {
        let mut guard = self.lock()?;
        guard.remove(session_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_traits::PersistedStateV1;

    fn sample(id: &str, turn: u32) -> PersistedState {
        PersistedState::V1(PersistedStateV1 {
            session_id: id.into(),
            last_updated: "2024-01-01T00:00:00Z".into(),
            current_turn: turn,
            ..Default::default()
        })
    }

    #[test]
    fn round_trip_save_load() {
        let store = InMemoryStateStore::new();
        let s = sample("sess", 3);

        // initially missing
        assert!(store.load("sess").unwrap().is_none());

        // save -> load
        store.save("sess", &s).unwrap();
        assert_eq!(store.load("sess").unwrap(), Some(s.clone()));

        // overwrite
        let s2 = sample("sess", 7);
        store.save("sess", &s2).unwrap();
        assert_eq!(store.load("sess").unwrap(), Some(s2));
    }

    #[test]
    fn list_sessions_returns_sorted_keys() {
        let store = InMemoryStateStore::new();
        // Insert out of order; BTreeMap returns sorted.
        store.save("gamma", &sample("gamma", 0)).unwrap();
        store.save("alpha", &sample("alpha", 0)).unwrap();
        store.save("beta", &sample("beta", 0)).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(
            sessions,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn delete_is_idempotent() {
        let store = InMemoryStateStore::new();
        store.save("x", &sample("x", 0)).unwrap();
        assert!(store.load("x").unwrap().is_some());

        store.delete("x").unwrap();
        assert!(store.load("x").unwrap().is_none());

        // Deleting a missing entry is a no-op.
        store.delete("x").unwrap();
        store.delete("never-existed").unwrap();
    }

    #[test]
    fn len_and_is_empty_track_state() {
        let store = InMemoryStateStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        store.save("a", &sample("a", 0)).unwrap();
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
        store.delete("a").unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn default_matches_new() {
        let _ = InMemoryStateStore::default();
    }

    /// Compile-time assertion: `InMemoryStateStore` is `Send + Sync` so it
    /// can be used as a `dyn StatePersistence + Send + Sync`.
    #[test]
    fn store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemoryStateStore>();
    }
}
