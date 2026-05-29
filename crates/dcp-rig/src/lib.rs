//! `dcp-rig` — Rig framework adapter for dynamic context pruning.
//!
//! Provides integration between DCP's ContextPruner and the Rig LLM orchestration framework.
//! Rig provides a unified interface for calling multiple LLM providers.
//!
//! This adapter allows Rig users to easily add DCP compression to their existing Rig workflows.

use dcp_config::Config;
use dcp_core::{ContextPruner, Error as CoreError};
use dcp_types::Message;

pub use dcp_types::{Message as DcpMessage, Part, Role};

/// Wraps a Rig CompletionModel with DCP context pruning.
#[allow(dead_code)]
pub struct DcpCompletionModel<M> {
    inner: M,
    pruner: ContextPruner,
}

impl<M> DcpCompletionModel<M> {
    /// Create a new DCP-wrapped completion model.
    pub fn new(inner: M, config: Config) -> Result<Self, anyhow::Error> {
        let pruner = ContextPruner::new(config)?;
        Ok(Self { inner, pruner })
    }

    /// Create with default config.
    pub fn with_default(inner: M) -> Result<Self, anyhow::Error> {
        let config = Config::load_default().unwrap_or_else(|_| Config::default());
        Self::new(inner, config)
    }

    /// Get a reference to the underlying model.
    pub fn inner(&self) -> &M {
        &self.inner
    }

    /// Get a mutable reference to the underlying model.
    pub fn inner_mut(&mut self) -> &mut M {
        &mut self.inner
    }

    /// Get a reference to the pruner.
    pub fn pruner(&self) -> &ContextPruner {
        &self.pruner
    }

    /// Transform messages through DCP and return the compressed result.
    pub fn transform(&mut self, messages: Vec<Message>) -> Result<Vec<Message>, CoreError> {
        self.pruner.transform_messages(messages)
    }
}
