#![forbid(unsafe_code)]
//! `dcp-nudges` — three kinds of nudge injection (SPEC.md §8).
//!
//! - **Context-limit nudge** — fired when token usage exceeds the
//!   configured `maxContextLimit` and re-fired every `nudgeFrequency`
//!   fetches.
//! - **Turn nudge** — fired once per uncompressed `(user, assistant)`
//!   pair so the model is reminded that compression is available.
//! - **Iteration nudge** — fired when message count since the last user
//!   message exceeds `iterationNudgeThreshold`.
//!
//! Nudges may be `soft` or `strong` and are placed near specific anchor
//! messages chosen by the priority builder.
//!
//! Phase 0 scaffold: implementations will land in Phase 4.
