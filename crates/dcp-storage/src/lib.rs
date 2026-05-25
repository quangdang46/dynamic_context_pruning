#![forbid(unsafe_code)]
//! `dcp-storage` — `StatePersistence` implementations.
//!
//! Default backends:
//!
//! - `InMemoryStore` — useful for tests and short-lived sessions.
//! - `FileStateStore` — XDG-compliant JSON-on-disk, atomic write via
//!   `tmp` + `rename`, with a `.bak` snapshot on each save.
//!
//! Optional feature backends (Phase 2+):
//!
//! - `sled` feature — embedded key-value store.
//! - `sqlite` feature — embedded SQL store.
//!
//! Phase 0 scaffold: implementations will land in Phase 2.
