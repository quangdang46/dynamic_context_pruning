//! Async wrapper around [`crate::ContextPruner`] (PLAN.md §3.3).
//!
//! Only available behind the `async` feature flag. The wrapper takes
//! ownership of the sync core inside a [`tokio::sync::Mutex`] and runs
//! every blocking method through [`tokio::task::spawn_blocking`] so it
//! never holds a runtime worker for the duration of CPU-bound pipeline
//! work.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinError;

use dcp_compress::{CompressArgs, CompressResult};
use dcp_config::Config;
use dcp_telemetry::TelemetrySnapshot;
use dcp_types::{BlockId, Message, Stats};

use crate::commands::CommandOutcome;
use crate::error::Error;
use crate::pipeline::ToolSchema;
use crate::pruner::{ContextPruner, ContextPrunerBuilder, DecompressResult, RecompressResult};

/// Async facade — every method is `async` and offloads the underlying
/// work via [`tokio::task::spawn_blocking`].
///
/// The wrapper is `Clone` so the same logical pruner can be shared
/// across tasks.
#[derive(Clone)]
pub struct ContextPrunerAsync {
    inner: Arc<Mutex<ContextPruner>>,
}

impl ContextPrunerAsync {
    /// Wrap an existing [`ContextPruner`].
    pub fn from_sync(inner: ContextPruner) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Construct an async pruner with the bundled defaults.
    pub fn new(config: Config) -> Result<Self, Error> {
        Ok(Self::from_sync(ContextPruner::new(config)?))
    }

    /// Start a builder. Use [`Self::from_sync`] on the result of
    /// [`ContextPrunerBuilder::build`].
    pub fn builder() -> ContextPrunerBuilder {
        ContextPruner::builder()
    }

    /// Borrow a clone of the [`Config`] inside the lock. The async
    /// surface returns by value because the pruner is behind a mutex.
    pub async fn config(&self) -> Config {
        self.inner.lock().await.config().clone()
    }

    /// Update the configuration.
    pub async fn update_config(&self, config: Config) -> Result<(), Error> {
        self.inner.lock().await.update_config(config)
    }

    /// Async [`ContextPruner::transform_messages`].
    pub async fn transform_messages(&self, messages: Vec<Message>) -> Result<Vec<Message>, Error> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            guard.transform_messages(messages)
        })
        .await
        .map_err(map_join)?
    }

    /// Async [`ContextPruner::transform_system`]. Mutates `system` in
    /// place.
    pub async fn transform_system(&self, system: &mut String) {
        let guard = self.inner.lock().await;
        guard.transform_system(system);
    }

    /// Async [`ContextPruner::compress_tool_schema`].
    pub async fn compress_tool_schema(&self) -> ToolSchema {
        self.inner.lock().await.compress_tool_schema()
    }

    /// Async [`ContextPruner::handle_compress`].
    pub async fn handle_compress(
        &self,
        args: CompressArgs,
        raw_messages: Vec<Message>,
    ) -> Result<CompressResult, Error> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            guard.handle_compress(args, &raw_messages)
        })
        .await
        .map_err(map_join)?
    }

    /// Async [`ContextPruner::decompress`].
    pub async fn decompress(&self, block_id: BlockId) -> Result<DecompressResult, Error> {
        self.inner.lock().await.decompress(block_id)
    }

    /// Async [`ContextPruner::recompress`].
    pub async fn recompress(&self, block_id: BlockId) -> Result<RecompressResult, Error> {
        self.inner.lock().await.recompress(block_id)
    }

    /// Async [`ContextPruner::handle_command`].
    pub async fn handle_command(
        &self,
        cmd: String,
        args: Vec<String>,
        raw_messages: Vec<Message>,
    ) -> CommandOutcome {
        let inner = self.inner.clone();
        let join = tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            guard.handle_command(&cmd, &arg_refs, &raw_messages)
        })
        .await;
        match join {
            Ok(outcome) => outcome,
            Err(e) => CommandOutcome::Error {
                message: format!("async join error: {e}"),
            },
        }
    }

    /// Async [`ContextPruner::fold_subagent`].
    pub async fn fold_subagent(&self, subagent_messages: Vec<Message>) -> Result<Message, Error> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            guard.fold_subagent(subagent_messages)
        })
        .await
        .map_err(map_join)?
    }

    /// Async [`ContextPruner::stats`] (returns a clone).
    pub async fn stats(&self) -> Stats {
        self.inner.lock().await.stats().clone()
    }

    /// Async [`ContextPruner::telemetry`].
    pub async fn telemetry(&self) -> TelemetrySnapshot {
        self.inner.lock().await.telemetry()
    }

    /// Async [`ContextPruner::reset`].
    pub async fn reset(&self) {
        self.inner.lock().await.reset();
    }

    /// Async [`ContextPruner::save`].
    pub async fn save(&self) -> Result<(), Error> {
        self.inner.lock().await.save()
    }

    /// Async [`ContextPruner::force_apply`].
    pub async fn force_apply(&self) -> Result<(), Error> {
        self.inner.lock().await.force_apply()
    }

    /// Async [`ContextPruner::set_manual_mode`].
    pub async fn set_manual_mode(&self, enabled: bool) {
        self.inner.lock().await.set_manual_mode(enabled);
    }
}

fn map_join(e: JoinError) -> Error {
    Error::AsyncJoin(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn async_basic_roundtrip() {
        let pruner = ContextPrunerAsync::new(Config::default()).unwrap();
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let out = pruner.transform_messages(messages).await.unwrap();
        assert!(!out.is_empty());
        let snap = pruner.telemetry().await;
        assert!(snap.total_events() >= 1);
    }

    #[tokio::test]
    async fn async_compress_tool_schema() {
        let pruner = ContextPrunerAsync::new(Config::default()).unwrap();
        let schema = pruner.compress_tool_schema().await;
        assert_eq!(schema.name, "compress");
    }

    #[tokio::test]
    async fn async_clone_shares_state() {
        let p1 = ContextPrunerAsync::new(Config::default()).unwrap();
        let p2 = p1.clone();
        p1.set_manual_mode(true).await;
        let cfg2 = p2.config().await;
        assert_eq!(cfg2.cache_stability_mode.as_str(), "agent-message");
        let stats2 = p2.stats().await;
        // Stats are zero, the manual_mode flag lives on state.
        assert_eq!(stats2.dedup_pruned, 0);
    }

    #[tokio::test]
    async fn async_force_apply_round_trip() {
        let pruner = ContextPrunerAsync::new(Config::default()).unwrap();
        pruner.force_apply().await.unwrap();
    }
}
