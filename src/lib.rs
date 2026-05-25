//! Workspace-root container that hosts the public examples and the
//! end-to-end smoke test for `dynamic_context_pruning`.
//!
//! The examples and tests live at the repository root (per PLAN.md
//! Appendix A) so a fresh contributor can read them without diving
//! into the workspace layout. This file exists only so Cargo treats
//! the root as a package and picks up the explicit `[[example]]` /
//! `[[test]]` entries in the root `Cargo.toml`.
//!
//! Nothing should depend on this crate. For the public API surface,
//! depend on `dynamic_context_pruning` (PLAN.md §4).
