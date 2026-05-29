#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `dcp-notification` — User-facing notifications for pruning and compression events.
//!
//! This crate ports the TypeScript UI notification system:
//! - format: Formatting functions (stats header, token count, progress bar)
//! - notification: Notification builders (prune notification, compress notification)
//!
//! Unlike the TS version which sends via client.tui.showToast(), the Rust version
//! produces formatted strings and lets the host decide delivery.

pub mod format;
pub mod notification;

pub use format::*;
pub use notification::*;
