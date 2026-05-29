//! `/dcp …` slash-command surface (PLAN.md §9).
//!
//! The host calls [`crate::ContextPruner::handle_command`] with the
//! command name and arguments; the dispatcher returns a
//! [`CommandOutcome`] which the host translates to its UI surface (toast,
//! chat message, exit code, …).
//!
//! The dispatcher is intentionally minimal: it does not parse any
//! free-form arguments, it just maps `(cmd, args)` to behaviour and
//! collects the resulting summary text. Anything that mutates state
//! lives on [`crate::ContextPruner`] proper.

use serde::{Deserialize, Serialize};

use dcp_types::{BlockId, SessionState, Stats};

use crate::pruner::ContextPruner;
use dcp_compress::{CompressArgs, CompressResult, RangeEntry};

/// One-shot result returned by [`ContextPruner::handle_command`].
///
/// Variants are `#[non_exhaustive]` so additional commands can land
/// without breaking downstream `match` arms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommandOutcome {
    /// `/dcp context` — short human-readable breakdown of the current
    /// session (turn, blocks, frontier, pending tokens).
    Context {
        /// Current turn count (SPEC.md §3.2).
        current_turn: u32,
        /// Number of currently active blocks.
        active_blocks: usize,
        /// Number of all blocks (active + audit-only).
        total_blocks: usize,
        /// Cumulative tokens accumulated in the pending prune snapshot.
        pending_tokens: u64,
        /// `m####` reference of the current pruning frontier.
        frontier: Option<String>,
        /// Active cache stability mode.
        cache_stability_mode: String,
    },

    /// `/dcp stats` — dump the persisted [`Stats`] counters.
    Stats(Stats),

    /// `/dcp sweep [count]` — request a manual flush of pending
    /// strategies. Returns the number of pending tool ids that were
    /// applied.
    Sweep {
        /// Number of pending tool ids that the apply phase will strip
        /// on the next transform.
        applied_ids: u32,
    },

    /// `/dcp manual <on|off>` — toggle [`SessionState::manual_mode`].
    Manual {
        /// New value of the manual-mode flag.
        enabled: bool,
    },

    /// `/dcp compress …` — invoked the compress tool through the
    /// command surface. The host owns the argument parsing; this
    /// outcome simply forwards the resulting [`CompressResult`].
    Compress(CompressResult),

    /// `/dcp decompress <block_id>` — deactivated a block.
    Decompress {
        /// The block id that was deactivated.
        block_id: BlockId,
    },

    /// `/dcp recompress <block_id>` — re-activated a previously
    /// user-decompressed block.
    Recompress {
        /// The block id that was re-activated.
        block_id: BlockId,
    },

    /// The command name was not recognised. The host typically prints
    /// help text to the user.
    Unknown {
        /// Command name as supplied by the host.
        command: String,
    },

    /// Command attempted but rejected — usually because the arguments
    /// were missing or malformed.
    Error {
        /// Human-readable reason.
        message: String,
    },
}

impl CommandOutcome {
    /// True when the outcome is a successful command match (i.e. not
    /// [`CommandOutcome::Unknown`] or [`CommandOutcome::Error`]).
    pub fn is_ok(&self) -> bool {
        !matches!(self, Self::Unknown { .. } | Self::Error { .. })
    }
}

/// Dispatch a `/dcp …` slash command to the pruner.
///
/// `cmd` is the command word (no leading slash); `args` are the
/// whitespace-split tokens following it. `raw_messages` is the host's
/// current message list — required by `compress` so the resolver can see
/// the conversation. Other commands accept and ignore it for a uniform
/// signature.
pub fn handle_command(
    pruner: &mut ContextPruner,
    cmd: &str,
    args: &[&str],
    raw_messages: &[dcp_types::Message],
) -> CommandOutcome {
    match cmd {
        "context" | "ctx" => render_context(pruner),
        "stats" => CommandOutcome::Stats(pruner.state().stats.clone()),
        "sweep" => render_sweep(pruner),
        "manual" => render_manual(pruner, args),
        "compress" => render_compress(pruner, args, raw_messages),
        "decompress" => render_decompress(pruner, args),
        "recompress" => render_recompress(pruner, args),
        other => CommandOutcome::Unknown {
            command: other.to_string(),
        },
    }
}

fn render_context(pruner: &ContextPruner) -> CommandOutcome {
    let s: &SessionState = pruner.state();
    let pending_tokens = s
        .pending_prune
        .as_ref()
        .map(|p| p.cumulative_tokens)
        .unwrap_or(0);
    CommandOutcome::Context {
        current_turn: s.current_turn,
        active_blocks: s.prune.messages.active_block_ids.len(),
        total_blocks: s.prune.messages.blocks_by_id.len(),
        pending_tokens,
        frontier: s.prune.messages.frontier_message_ref.clone(),
        cache_stability_mode: pruner.config().cache_stability_mode.as_str().to_string(),
    }
}

fn render_sweep(pruner: &mut ContextPruner) -> CommandOutcome {
    let pending = pruner
        .state()
        .pending_prune
        .as_ref()
        .map(|p| p.tool_ids.len() as u32)
        .unwrap_or(0);
    pruner.set_force_apply();
    CommandOutcome::Sweep {
        applied_ids: pending,
    }
}

fn render_manual(pruner: &mut ContextPruner, args: &[&str]) -> CommandOutcome {
    let enable = match args.first().copied() {
        Some("on") | Some("true") | Some("1") => true,
        Some("off") | Some("false") | Some("0") => false,
        Some(other) => {
            return CommandOutcome::Error {
                message: format!("expected on/off, got {other:?}"),
            };
        }
        None => !pruner.state().manual_mode.enabled,
    };
    pruner.set_manual_mode(enable);
    CommandOutcome::Manual { enabled: enable }
}

fn render_compress(
    pruner: &mut ContextPruner,
    args: &[&str],
    raw_messages: &[dcp_types::Message],
) -> CommandOutcome {
    // Slash-command form is intentionally limited: it accepts a single
    // range `<startId> <endId> [topic ...]`. The model normally calls
    // the tool directly through `handle_compress`; this is a debug
    // affordance for the host CLI.
    if args.len() < 2 {
        return CommandOutcome::Error {
            message: "usage: /dcp compress <startId> <endId> [topic words ...]".into(),
        };
    }
    let start = args[0].to_string();
    let end = args[1].to_string();
    let topic = if args.len() > 2 {
        args[2..].join(" ")
    } else {
        "manual compress".to_string()
    };
    let cargs = CompressArgs::Range {
        topic: topic.clone(),
        content: vec![RangeEntry {
            start_id: start,
            end_id: end,
            summary: format!("Manual compression: {topic}"),
        }],
    };
    match pruner.handle_compress(cargs, raw_messages) {
        Ok(result) => CommandOutcome::Compress(result),
        Err(e) => CommandOutcome::Error {
            message: format!("compress failed: {e}"),
        },
    }
}

fn render_decompress(pruner: &mut ContextPruner, args: &[&str]) -> CommandOutcome {
    let Some(raw) = args.first() else {
        return CommandOutcome::Error {
            message: "usage: /dcp decompress <blockId>".into(),
        };
    };
    let id = match parse_block_id(raw) {
        Ok(id) => id,
        Err(m) => return CommandOutcome::Error { message: m },
    };
    match pruner.decompress(id) {
        Ok(result) => CommandOutcome::Decompress {
            block_id: result.block_id,
        },
        Err(e) => CommandOutcome::Error {
            message: format!("decompress failed: {e}"),
        },
    }
}

fn render_recompress(pruner: &mut ContextPruner, args: &[&str]) -> CommandOutcome {
    let Some(raw) = args.first() else {
        return CommandOutcome::Error {
            message: "usage: /dcp recompress <blockId>".into(),
        };
    };
    let id = match parse_block_id(raw) {
        Ok(id) => id,
        Err(m) => return CommandOutcome::Error { message: m },
    };
    match pruner.recompress(id) {
        Ok(result) => CommandOutcome::Recompress {
            block_id: result.block_id,
        },
        Err(e) => CommandOutcome::Error {
            message: format!("recompress failed: {e}"),
        },
    }
}

fn parse_block_id(raw: &str) -> Result<BlockId, String> {
    let body = raw.strip_prefix('b').unwrap_or(raw);
    body.parse::<u32>()
        .map(BlockId::new)
        .map_err(|_| format!("invalid block id: {raw:?}"))
}
