#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-messages` — Message query, shape, sync, priority, injection, and utilities.
//!
//! This crate ports the TypeScript message processing pipeline:
//! - query: Message querying helpers (getLastUserMessage, isIgnored, etc.)
//! - shape: Message validation and filtering
//! - sync: Compression block synchronization
//! - priority: Message priority classification
//! - utils: Synthetic messages and DCP tag hallucination stripping
//! - inject_utils: Nudge injection utilities
//! - inject: Main injection orchestrator (nudges + message IDs)
//! - subagents: Subagent result expansion
//! - reasoning_strip: Stale provider metadata removal

pub mod inject;
pub mod inject_utils;
pub mod priority;
pub mod query;
pub mod reasoning_strip;
pub mod shape;
pub mod subagents;
pub mod sync;
pub mod utils;

pub use inject::*;
pub use reasoning_strip::*;
pub use subagents::*;
pub use utils::*;
