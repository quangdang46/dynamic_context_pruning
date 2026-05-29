//! Error type re-exposed by the `dcp-core` facade.
//!
//! Mirrors PLAN.md §4.5 and SPEC.md §1 — every public method on
//! [`crate::ContextPruner`] returns this single enum, so callers have one
//! `Result<T, Error>` shape to match against.

use thiserror::Error;

use dcp_compress::CompressError;
use dcp_config::ConfigError;
use dcp_prompts::PromptError;
use dcp_state::EnsureInitError;
use dcp_traits::{PersistenceError, PruneError, RetrievalError};
use dcp_types::BlockId;

/// Errors returned by the public facade.
///
/// The variants closely mirror PLAN.md §4.5; the bridging `#[from]` impls
/// route every underlying crate's error type into the canonical surface
/// the caller sees.
///
/// `#[non_exhaustive]` so future variants (e.g. an `Async` variant when
/// the async facade gains richer error types) can land without a major
/// version bump.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An input message failed validation per SPEC.md §2.5.
    #[error("invalid message: {0}")]
    InvalidMessage(String),

    /// Compress tool arguments failed validation.
    #[error("invalid compress args: {0}")]
    InvalidCompressArgs(String),

    /// Reference to an unknown / never-allocated block.
    #[error("block {0} not found")]
    BlockNotFound(BlockId),

    /// Two ranges in the same compress call cover overlapping ids.
    #[error("range overlap: {0}")]
    RangeOverlap(String),

    /// A summary placeholder mentioned an unknown / out-of-scope block.
    #[error("placeholder mismatch: {0}")]
    PlaceholderMismatch(String),

    /// Persistence backend failure.
    #[error("storage error: {0}")]
    Storage(#[from] PersistenceError),

    /// Configuration loading or validation failure.
    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    /// Tokenizer subsystem failure.
    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    /// Operation rejected because manual mode is active.
    #[error("manual mode blocks operation")]
    ManualModeBlocked,

    /// Compression was rejected by `compress.permission`.
    #[error("permission denied")]
    PermissionDenied,

    /// A prune strategy reported an internal failure.
    #[error("prune error: {0}")]
    Prune(#[from] PruneError),

    /// Memory retrieval backend failure (only surfaces when the host
    /// installs a [`dcp_traits::MemoryRetriever`]).
    #[error("memory retrieval error: {0}")]
    Memory(#[from] RetrievalError),

    /// Failed to load prompts (override directory or file IO).
    #[error("prompt error: {0}")]
    Prompt(#[from] PromptError),

    /// Subagent execution rejected by `experimental.allowSubagents`.
    #[error("subagents disabled by configuration")]
    SubagentsDisabled,

    /// A required dependency was not provided to the builder.
    #[error("missing required dependency: {0}")]
    MissingDependency(&'static str),

    /// The backing async runtime failed (only surfaced by the
    /// `async` facade).
    #[error("async join error: {0}")]
    AsyncJoin(String),
}

impl From<CompressError> for Error {
    fn from(err: CompressError) -> Self {
        match err {
            CompressError::InvalidCompressArgs(m) => Error::InvalidCompressArgs(m),
            CompressError::UnknownRef(m) => Error::InvalidCompressArgs(format!("unknown ref: {m}")),
            CompressError::RangeOverlap(m) => Error::RangeOverlap(m),
            CompressError::PlaceholderMismatch(m) => Error::PlaceholderMismatch(m),
            CompressError::MessageAlreadyCompressed(m) => {
                Error::InvalidCompressArgs(format!("message {m} is already inside an active block"))
            }
            // `CompressError` is `#[non_exhaustive]`; route any future
            // variant through `InvalidCompressArgs` so callers always
            // see a structured error.
            other => Error::InvalidCompressArgs(other.to_string()),
        }
    }
}

impl From<EnsureInitError> for Error {
    fn from(err: EnsureInitError) -> Self {
        match err {
            EnsureInitError::Persistence(e) => Error::Storage(e),
        }
    }
}
