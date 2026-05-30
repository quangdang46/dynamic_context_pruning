#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-nudges` — three kinds of nudge injection (SPEC.md §8).
//!
//! - **Context-limit nudge** — fired when token usage exceeds the
//!   configured `maxContextLimit` and re-fired every `nudgeFrequency`
//!   transforms (SPEC.md §8.1).
//! - **Turn nudge** — fired once per uncompressed `(user, assistant)`
//!   pair so the model is reminded that compression is available
//!   (SPEC.md §8.2).
//! - **Iteration nudge** — fired when the assistant has run more than
//!   `iterationNudgeThreshold` messages since the most recent user
//!   message (SPEC.md §8.3).
//!
//! Per SPEC.md §8.4 only one nudge is emitted per transform; the
//! [`build_priority_map`] function selects the winner.
//!
//! # Public surface
//!
//! ```rust
//! use dcp_nudges::{NudgeConfig, build_priority_map, render_nudge,
//!     inject_nudges, NudgeKind, InjectionMode};
//! use dcp_prompts::Prompts;
//! use dcp_types::{Message, SessionState};
//!
//! let cfg = NudgeConfig::default();
//! let mut state = SessionState::default();
//! let mut messages = vec![
//!     Message::user_text("u1", 0, "hi"),
//!     Message::assistant_text("a1", 0, "hello"),
//! ];
//! let total_tokens = 100;
//! let priorities = build_priority_map(&cfg, &state, &messages, total_tokens);
//! let prompts = Prompts::default();
//! inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
//! ```

use std::collections::HashMap;

use dcp_prompts::{NudgeForce, Prompts};
use dcp_state::{assign_message_refs, collect_turn_nudge_anchors};
use dcp_types::{Message, MessageRef, MessageRefKind, Part, Role, SessionState};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────
// NudgeKind
// ─────────────────────────────────────────────────────────────────────────

/// One of the three nudge kinds the library emits (SPEC.md §8).
///
/// `#[non_exhaustive]` so future kinds can be added without breaking
/// downstream `match` arms.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::NudgeKind;
/// let k = NudgeKind::Iteration { count: 20 };
/// assert!(matches!(k, NudgeKind::Iteration { count: 20 }));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum NudgeKind {
    /// Context limit reached — render with `{tokens}` and `{limit}`
    /// substituted.
    ContextLimit {
        /// Current total token count of the outgoing message stream.
        tokens: u64,
        /// Configured maximum token count for the model.
        limit: u64,
    },
    /// Turn ended without compression — render verbatim from the
    /// `turn_nudge` template.
    Turn,
    /// Iteration threshold exceeded — render with `{count}` substituted.
    Iteration {
        /// Number of assistant messages since the most recent user
        /// message.
        count: u32,
    },
}

impl NudgeKind {
    /// Return a string identifier for this nudge kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            NudgeKind::ContextLimit { .. } => "context_limit",
            NudgeKind::Turn => "turn",
            NudgeKind::Iteration { .. } => "iteration",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// InjectionMode
// ─────────────────────────────────────────────────────────────────────────

/// How rendered nudge text is glued into a message's primary text part.
///
/// The default ([`InjectionMode::Append`]) preserves cache stability for
/// the leading prefix of the existing content; [`InjectionMode::Prepend`]
/// is provided for hosts that want the nudge to appear before the
/// message body; [`InjectionMode::Replace`] swaps the body entirely.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::InjectionMode;
/// assert_eq!(InjectionMode::default(), InjectionMode::Append);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    /// Replace the message's primary text part with the rendered nudge.
    Replace,
    /// Append the rendered nudge after the existing primary text.
    #[default]
    Append,
    /// Prepend the rendered nudge before the existing primary text.
    Prepend,
}

// ─────────────────────────────────────────────────────────────────────────
// NudgeConfig
// ─────────────────────────────────────────────────────────────────────────

/// Subset of the host configuration consumed by this crate.
///
/// `dcp-nudges` does not depend on `dcp-config` (PLAN.md §5.2). When the
/// latter crate lands it will provide a forwarding [`From`] implementation
/// or expose this struct directly.
///
/// Defaults match SPEC.md §10.2.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::{NudgeConfig, InjectionMode};
/// use dcp_prompts::NudgeForce;
///
/// let cfg = NudgeConfig {
///     iteration_nudge_threshold: 10,
///     ..NudgeConfig::default()
/// };
/// assert_eq!(cfg.injection_mode, InjectionMode::Append);
/// assert_eq!(cfg.nudge_force, NudgeForce::Soft);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NudgeConfig {
    /// How rendered nudge text is glued into the anchor's text part.
    pub injection_mode: InjectionMode,
    /// Number of transforms that must elapse between repeated
    /// context-limit / iteration nudges (SPEC.md §8.1, §8.3).
    pub nudge_frequency: u32,
    /// Iteration-nudge threshold: fire when the assistant-since-user
    /// count exceeds this value (SPEC.md §8.3).
    pub iteration_nudge_threshold: u32,
    /// Tone of the nudge text (SPEC.md §8.2).
    pub nudge_force: NudgeForce,
    /// Resolved maximum context token budget (SPEC.md §8.1). Hosts pass
    /// the post-resolution numeric value; percentage strings have
    /// already been multiplied through.
    pub max_context_limit: u64,
    /// Resolved minimum context token budget below which context-limit
    /// nudges never fire (SPEC.md §8.1).
    pub min_context_limit: u64,
}

impl Default for NudgeConfig {
    fn default() -> Self {
        Self {
            injection_mode: InjectionMode::default(),
            nudge_frequency: 5,
            iteration_nudge_threshold: 15,
            nudge_force: NudgeForce::default(),
            max_context_limit: 100_000,
            min_context_limit: 50_000,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// build_priority_map
// ─────────────────────────────────────────────────────────────────────────

/// Build the (anchor message id → nudge kind) map for one transform.
///
/// SPEC.md §8.4 — at most one nudge fires per transform. Priority order,
/// highest to lowest:
///
/// 1. **Context-limit** — when `total_tokens > max_context_limit` and the
///    state's `context_limit_counter` lands on a `nudge_frequency` cycle.
/// 2. **Iteration** — when the assistant-since-user count exceeds
///    `iteration_nudge_threshold` and the iteration counter cycles.
/// 3. **Turn** — when [`NudgeForce::Soft`] is configured and at least one
///    `(user, assistant)` pair has not yet been turn-nudged.
///
/// Past-the-frontier check: candidate anchors at or before
/// `state.prune.messages.frontier_message_ref` are dropped.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::{NudgeConfig, build_priority_map};
/// use dcp_types::{Message, SessionState};
///
/// let cfg = NudgeConfig::default();
/// let state = SessionState::default();
/// let messages: Vec<Message> = Vec::new();
/// let map = build_priority_map(&cfg, &state, &messages, 0);
/// assert!(map.is_empty());
/// ```
pub fn build_priority_map(
    config: &NudgeConfig,
    state: &SessionState,
    messages: &[Message],
    total_tokens: u64,
) -> HashMap<String, NudgeKind> {
    let mut out = HashMap::new();

    // 1) Context-limit (highest priority).
    if let Some((anchor, kind)) = candidate_context_limit(config, state, messages, total_tokens)
        && past_frontier(state, &anchor)
    {
        out.insert(anchor, kind);
        return out;
    }

    // 2) Iteration.
    if let Some((anchor, kind)) = candidate_iteration(config, state, messages)
        && past_frontier(state, &anchor)
    {
        out.insert(anchor, kind);
        return out;
    }

    // 3) Turn (only when force == Soft).
    if config.nudge_force == NudgeForce::Soft
        && let Some((anchor, kind)) = candidate_turn(state, messages)
        && past_frontier(state, &anchor)
    {
        out.insert(anchor, kind);
        return out;
    }

    out
}

/// Decide whether a context-limit nudge should fire on this transform.
///
/// Mirrors SPEC §8.1: gate on `total_tokens > max_context_limit` *and*
/// `total_tokens >= min_context_limit`, then re-fire every
/// `nudge_frequency` transforms by inspecting the state's counter.
fn candidate_context_limit(
    config: &NudgeConfig,
    state: &SessionState,
    messages: &[Message],
    total_tokens: u64,
) -> Option<(String, NudgeKind)> {
    if total_tokens < config.min_context_limit {
        return None;
    }
    if total_tokens <= config.max_context_limit {
        return None;
    }
    if config.nudge_frequency == 0 {
        return None;
    }

    // Counter pacing: SPEC §8.1 — the next fire is the cycle on which the
    // counter rolls back to zero. With counter = 0 + just-incremented = 1
    // we want it to fire when counter % frequency == 0 after increment;
    // build_priority_map is read-only so we model "would the upcoming
    // increment hit a cycle?" The actual increment happens in
    // `inject_nudges` so the same transform that emits the nudge also
    // resets the counter.
    let next_value = state.nudges.context_limit_counter.saturating_add(1);
    if next_value % config.nudge_frequency != 0 {
        return None;
    }

    let anchor = latest_assistant_or_user_anchor(messages)?;
    Some((
        anchor,
        NudgeKind::ContextLimit {
            tokens: total_tokens,
            limit: config.max_context_limit,
        },
    ))
}

/// Decide whether an iteration nudge should fire on this transform.
///
/// SPEC §8.3 — count = number of assistant messages since the most
/// recent user message. Fires when `count > iteration_nudge_threshold`,
/// re-firing every `nudge_frequency` further messages.
fn candidate_iteration(
    config: &NudgeConfig,
    state: &SessionState,
    messages: &[Message],
) -> Option<(String, NudgeKind)> {
    let count = assistant_since_user(messages);
    if count <= config.iteration_nudge_threshold {
        return None;
    }
    if config.nudge_frequency == 0 {
        return None;
    }

    let next_value = state.nudges.iteration_counter.saturating_add(1);
    if next_value % config.nudge_frequency != 0 {
        return None;
    }

    let anchor = latest_assistant_anchor(messages)?;
    Some((anchor, NudgeKind::Iteration { count }))
}

/// Decide whether a turn nudge should fire on this transform.
///
/// SPEC §8.2 — anchor = the assistant of the most recent
/// `(user, assistant)` pair that has not yet been turn-nudged. The pair
/// must be older than the most-recent pair (i.e. not the in-progress
/// turn) so we never nudge the active turn.
fn candidate_turn(state: &SessionState, messages: &[Message]) -> Option<(String, NudgeKind)> {
    let anchors = collect_turn_nudge_anchors(messages);
    if anchors.is_empty() {
        return None;
    }

    // Walk back to find pairs in stream order.
    let pairs = turn_pairs(messages);

    // Skip the most recent pair (the in-progress turn).
    let pairs_to_consider = if pairs.is_empty() {
        return None;
    } else {
        &pairs[..pairs.len().saturating_sub(1)]
    };

    for (user_id, assistant_id) in pairs_to_consider {
        if !anchors.contains(assistant_id) {
            continue;
        }
        if state
            .nudges
            .turn_nudged_pairs
            .contains(&(user_id.clone(), assistant_id.clone()))
        {
            continue;
        }
        return Some((assistant_id.clone(), NudgeKind::Turn));
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────
// render_nudge
// ─────────────────────────────────────────────────────────────────────────

/// Render the prompt text for `kind` against the configured templates.
///
/// Substitutions:
/// - [`NudgeKind::ContextLimit`] → `{tokens}`, `{limit}`
/// - [`NudgeKind::Iteration`] → `{count}`
/// - [`NudgeKind::Turn`] → no substitution (template is used verbatim).
///
/// `state` and `config` are accepted for parity with the broader
/// SPEC-described renderer; the current implementation does not consult
/// either, but future templates may.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::{NudgeConfig, NudgeKind, render_nudge};
/// use dcp_prompts::Prompts;
/// use dcp_types::SessionState;
///
/// let prompts = Prompts::default();
/// let state = SessionState::default();
/// let cfg = NudgeConfig::default();
/// let s = render_nudge(
///     &NudgeKind::ContextLimit { tokens: 1234, limit: 5000 },
///     &prompts,
///     &state,
///     &cfg,
/// );
/// assert!(s.contains("1234"));
/// assert!(s.contains("5000"));
/// ```
pub fn render_nudge(
    kind: &NudgeKind,
    prompts: &Prompts,
    _state: &SessionState,
    _config: &NudgeConfig,
) -> String {
    match kind {
        NudgeKind::ContextLimit { tokens, limit } => prompts
            .context_limit_nudge
            .replace("{tokens}", &tokens.to_string())
            .replace("{limit}", &limit.to_string()),
        NudgeKind::Turn => prompts.turn_nudge.clone(),
        NudgeKind::Iteration { count } => prompts
            .iteration_nudge
            .replace("{count}", &count.to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// inject_nudges
// ─────────────────────────────────────────────────────────────────────────

/// Inject rendered nudge text into the priority anchors.
///
/// For every entry in `priorities`, locate the matching message by id
/// and apply the rendered nudge under [`NudgeConfig::injection_mode`].
/// Updates `state.nudges` counters and `turn_nudged_pairs` so subsequent
/// transforms do not re-emit the same nudge prematurely.
///
/// Messages without a primary text part receive a freshly-created
/// [`Part::Text`] containing only the nudge.
///
/// # Example
///
/// ```rust
/// use std::collections::HashMap;
/// use dcp_nudges::{NudgeConfig, NudgeKind, inject_nudges};
/// use dcp_prompts::Prompts;
/// use dcp_types::{Message, SessionState};
///
/// let mut state = SessionState::default();
/// let mut messages = vec![Message::assistant_text("a1", 0, "ack")];
/// let prompts = Prompts::default();
/// let mut priorities = HashMap::new();
/// priorities.insert("a1".to_string(), NudgeKind::Turn);
///
/// inject_nudges(&mut state, &NudgeConfig::default(), &mut messages, &prompts, &priorities);
/// // The message body now ends with the nudge text.
/// match &messages[0].parts[0] {
///     dcp_types::Part::Text(t) => assert!(t.contains("compress")),
///     other => panic!("expected text part, got {other:?}"),
/// }
/// ```
pub fn inject_nudges(
    state: &mut SessionState,
    config: &NudgeConfig,
    messages: &mut [Message],
    prompts: &Prompts,
    priorities: &HashMap<String, NudgeKind>,
) {
    if priorities.is_empty() {
        return;
    }

    // Resolve `(user_id, assistant_id)` pairs up front so we can mutate
    // `messages` later without re-borrowing it for lookups.
    let pair_map: HashMap<String, String> = priorities
        .iter()
        .filter_map(|(anchor_id, kind)| match kind {
            NudgeKind::Turn => {
                preceding_user_text_id(messages, anchor_id).map(|uid| (anchor_id.clone(), uid))
            }
            _ => None,
        })
        .collect();

    for msg in messages.iter_mut() {
        // Ignored messages are not visible to the LLM — skip injection.
        if msg.ignored {
            continue;
        }
        let Some(kind) = priorities.get(&msg.id) else {
            continue;
        };
        let text = render_nudge(kind, prompts, state, config);
        apply_to_text_part(msg, &text, config.injection_mode);

        // Update counters and bookkeeping. `inject_nudges` is the
        // canonical place where state mutates: build_priority_map is
        // read-only (so it is safe to call repeatedly).
        match kind {
            NudgeKind::ContextLimit { .. } => {
                state.nudges.context_limit_counter = 0;
            }
            NudgeKind::Iteration { .. } => {
                state.nudges.iteration_counter = 0;
            }
            NudgeKind::Turn => {
                if let Some(user_id) = pair_map.get(&msg.id) {
                    state
                        .nudges
                        .turn_nudged_pairs
                        .insert((user_id.clone(), msg.id.clone()));
                }
            }
        }
        state.nudges.last_nudge_kind = Some(kind.as_str().to_string());
    }
}

// ─────────────────────────────────────────────────────────────────────────
// inject_message_ids
// ─────────────────────────────────────────────────────────────────────────

/// Tag each priority anchor with `<dcp-message-id>m####</dcp-message-id>`
/// so the model can reference it from a `compress` call.
///
/// SPEC.md §6.9 — the model addresses messages by their canonical
/// reference. The library renders the tag at the start of the anchor's
/// primary text part, allocating a reference via
/// [`dcp_state::assign_message_refs`] if one does not yet exist.
///
/// `config` is accepted for parity with the broader injection family of
/// helpers; the current implementation does not consult it.
///
/// # Example
///
/// ```rust
/// use std::collections::HashMap;
/// use dcp_nudges::{NudgeConfig, NudgeKind, inject_message_ids};
/// use dcp_types::{Message, SessionState};
///
/// let mut state = SessionState::default();
/// let mut messages = vec![Message::assistant_text("a1", 0, "ack")];
/// let mut priorities = HashMap::new();
/// priorities.insert("a1".to_string(), NudgeKind::Turn);
///
/// inject_message_ids(&mut state, &NudgeConfig::default(), &mut messages, &priorities);
///
/// match &messages[0].parts[0] {
///     dcp_types::Part::Text(t) => assert!(t.contains("<dcp-message-id>m0001</dcp-message-id>")),
///     other => panic!("expected text part, got {other:?}"),
/// }
/// ```
pub fn inject_message_ids(
    state: &mut SessionState,
    _config: &NudgeConfig,
    messages: &mut [Message],
    priorities: &HashMap<String, NudgeKind>,
) {
    if priorities.is_empty() {
        return;
    }

    // Make sure every priority anchor has an `m####` reference.
    assign_message_refs(state, messages);

    for msg in messages.iter_mut() {
        if !priorities.contains_key(&msg.id) {
            continue;
        }
        // Ignored messages are not visible to the LLM — skip tagging.
        if msg.ignored {
            continue;
        }
        let Some(reference) = state.message_ids.by_raw_id.get(&msg.id) else {
            continue;
        };
        let tag = format!("<dcp-message-id>{reference}</dcp-message-id>");
        prepend_unique_tag(msg, &tag);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// inject_extended_subagent_results
// ─────────────────────────────────────────────────────────────────────────

/// Expand stored sub-agent results back into the message stream when
/// `experimental.allowSubagents` is enabled (SPEC.md §11.6).
///
/// Behavior:
///
/// - When `allow_subagents == false`, returns `messages` unchanged.
/// - When enabled, every `Part::ToolResult` whose `call_id` appears in
///   [`SessionState::subagent_result_cache`] is rewritten to embed the
///   cached extended result. Original output is preserved as a prefix
///   so cache stability is not catastrophically broken; the extended
///   payload is appended under a clearly-marked block.
///
/// The function does not mutate the input slice — it returns a new
/// `Vec<Message>` so callers can swap freely.
///
/// # Example
///
/// ```rust
/// use dcp_nudges::inject_extended_subagent_results;
/// use dcp_types::{Message, SessionState};
///
/// let state = SessionState::default();
/// let messages = vec![Message::user_text("u1", 0, "hi")];
/// let out = inject_extended_subagent_results(&state, messages.clone(), false);
/// assert_eq!(out, messages);
/// ```
pub fn inject_extended_subagent_results(
    state: &SessionState,
    messages: Vec<Message>,
    allow_subagents: bool,
) -> Vec<Message> {
    if !allow_subagents {
        return messages;
    }
    if state.subagent_result_cache.is_empty() {
        return messages;
    }

    messages
        .into_iter()
        .map(|mut msg| {
            for part in msg.parts.iter_mut() {
                if let Part::ToolResult {
                    call_id, output, ..
                } = part
                    && let Some(extended) = state.subagent_result_cache.get(call_id)
                {
                    let prefix = output.clone().unwrap_or_default();
                    let mut combined = String::new();
                    if !prefix.is_empty() {
                        combined.push_str(&prefix);
                        combined.push_str("\n\n");
                    }
                    combined.push_str("<dcp-subagent-extended>\n");
                    combined.push_str(extended);
                    combined.push_str("\n</dcp-subagent-extended>");
                    *output = Some(combined);
                }
            }
            msg
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers (private)
// ─────────────────────────────────────────────────────────────────────────

/// Apply `text` to the first [`Part::Text`] in `msg`. If none exists, a
/// new [`Part::Text`] is appended carrying just `text`.
fn apply_to_text_part(msg: &mut Message, text: &str, mode: InjectionMode) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some(part) = msg.parts.iter_mut().find_map(|p| match p {
        Part::Text(t) => Some(t),
        _ => None,
    }) {
        match mode {
            InjectionMode::Replace => {
                *part = trimmed.to_string();
            }
            InjectionMode::Append => {
                if part.is_empty() {
                    *part = trimmed.to_string();
                } else {
                    part.push_str("\n\n");
                    part.push_str(trimmed);
                }
            }
            InjectionMode::Prepend => {
                if part.is_empty() {
                    *part = trimmed.to_string();
                } else {
                    let body = std::mem::take(part);
                    *part = format!("{trimmed}\n\n{body}");
                }
            }
        }
    } else {
        msg.parts.push(Part::Text(trimmed.to_string()));
    }
}

/// Prepend `tag` to the first text part of `msg` if it is not already
/// present; create a text part otherwise.
fn prepend_unique_tag(msg: &mut Message, tag: &str) {
    if let Some(part) = msg.parts.iter_mut().find_map(|p| match p {
        Part::Text(t) => Some(t),
        _ => None,
    }) {
        if part.contains(tag) {
            return;
        }
        if part.is_empty() {
            *part = tag.to_string();
        } else {
            let body = std::mem::take(part);
            *part = format!("{tag}\n{body}");
        }
    } else {
        msg.parts.insert(0, Part::Text(tag.to_string()));
    }
}

/// Most-recent assistant message id with at least one text part, falling
/// back to the most-recent user-text message when no assistant text
/// exists.
fn latest_assistant_or_user_anchor(messages: &[Message]) -> Option<String> {
    if let Some(id) = latest_assistant_anchor(messages) {
        return Some(id);
    }
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User && has_text_part(m))
        .map(|m| m.id.clone())
}

fn latest_assistant_anchor(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && has_text_part(m))
        .map(|m| m.id.clone())
}

fn has_text_part(msg: &Message) -> bool {
    msg.parts.iter().any(|p| matches!(p, Part::Text(_)))
}

/// Number of consecutive assistant messages following the most recent
/// user-text message. Returns `0` when there are no assistants in that
/// suffix or no preceding user message at all.
fn assistant_since_user(messages: &[Message]) -> u32 {
    let last_user = messages
        .iter()
        .rposition(|m| m.role == Role::User && !m.ignored && has_text_part(m));
    let Some(idx) = last_user else {
        return 0;
    };
    messages[idx + 1..]
        .iter()
        .filter(|m| m.role == Role::Assistant && !m.ignored)
        .count() as u32
}

/// Pairs `(user_id, assistant_id)` for every consecutive user-text →
/// assistant-text transition. Anchors that fail [`Part::Text`] gating
/// are skipped just like in `dcp_state::collect_turn_nudge_anchors`.
fn turn_pairs(messages: &[Message]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut last_user: Option<String> = None;
    for m in messages {
        match m.role {
            Role::User if !m.ignored && has_text_part(m) => {
                last_user = Some(m.id.clone());
            }
            Role::Assistant if has_text_part(m) => {
                if let Some(uid) = &last_user {
                    pairs.push((uid.clone(), m.id.clone()));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// Locate the user-text message id immediately preceding `assistant_id`
/// in stream order.
fn preceding_user_text_id(messages: &[Message], assistant_id: &str) -> Option<String> {
    let asst_idx = messages.iter().position(|m| m.id == assistant_id)?;
    messages[..asst_idx]
        .iter()
        .rev()
        .find(|m| m.role == Role::User && !m.ignored && has_text_part(m))
        .map(|m| m.id.clone())
}

/// True when `anchor_id` resolves to a reference strictly past the
/// frontier (so the nudge is not pointing at content the library has
/// already given up trying to compress, per SPEC §8.4).
fn past_frontier(state: &SessionState, anchor_id: &str) -> bool {
    let Some(frontier_raw) = state.prune.messages.frontier_message_ref.as_deref() else {
        return true;
    };
    let Ok(frontier) = MessageRef::parse(frontier_raw) else {
        // Corrupt frontier — fail-safe to "no frontier".
        return true;
    };

    let MessageRefKind::Message(frontier_n) = frontier.kind() else {
        // Block-form frontier — defer to caller; treat as "no frontier"
        // since a block reference does not order against an `m####`.
        return true;
    };

    let Some(anchor_ref) = state.message_ids.by_raw_id.get(anchor_id) else {
        // Anchor has not been allocated a reference yet — definitionally
        // newer than the frontier.
        return true;
    };
    let Ok(anchor) = MessageRef::parse(anchor_ref) else {
        return true;
    };
    match anchor.kind() {
        MessageRefKind::Message(n) => n > frontier_n,
        MessageRefKind::Block(_) => true,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Message, Part, Role, SessionState, ToolStatus};
    use serde_json::json;

    fn user(id: &str, text: &str) -> Message {
        Message::user_text(id, 0, text)
    }

    fn asst(id: &str, text: &str) -> Message {
        Message::assistant_text(id, 0, text)
    }

    // ----- NudgeConfig defaults -----

    #[test]
    fn nudge_config_defaults_match_spec() {
        let cfg = NudgeConfig::default();
        assert_eq!(cfg.injection_mode, InjectionMode::Append);
        assert_eq!(cfg.nudge_frequency, 5);
        assert_eq!(cfg.iteration_nudge_threshold, 15);
        assert_eq!(cfg.nudge_force, NudgeForce::Soft);
        assert_eq!(cfg.max_context_limit, 100_000);
        assert_eq!(cfg.min_context_limit, 50_000);
    }

    // ----- build_priority_map: empty -----

    #[test]
    fn build_priority_map_empty_messages() {
        let cfg = NudgeConfig::default();
        let state = SessionState::default();
        let map = build_priority_map(&cfg, &state, &[], 0);
        assert!(map.is_empty());
    }

    #[test]
    fn build_priority_map_no_signal_returns_empty() {
        let cfg = NudgeConfig::default();
        let state = SessionState::default();
        let messages = vec![user("u1", "hi"), asst("a1", "hello")];
        // Tokens well below limits AND only one pair (the in-progress
        // turn is excluded). Expect: no nudge because the only pair is
        // the latest one.
        let map = build_priority_map(&cfg, &state, &messages, 1_000);
        assert!(map.is_empty());
    }

    // ----- build_priority_map: context-limit -----

    #[test]
    fn context_limit_fires_when_tokens_above_max_and_counter_cycles() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            min_context_limit: 0,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let map = build_priority_map(&cfg, &state, &messages, cfg.max_context_limit + 1);
        assert_eq!(map.len(), 1);
        let (anchor, kind) = map.into_iter().next().unwrap();
        assert_eq!(anchor, "a1");
        assert!(matches!(kind, NudgeKind::ContextLimit { .. }));
    }

    #[test]
    fn context_limit_skips_below_min_threshold() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            min_context_limit: 10_000,
            max_context_limit: 5_000,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![user("u1", "hi"), asst("a1", "ack")];
        // tokens = 7000 is above max but below min_context_limit → no nudge.
        let map = build_priority_map(&cfg, &state, &messages, 7_000);
        assert!(map.is_empty());
    }

    #[test]
    fn context_limit_respects_frequency_pacing() {
        let cfg = NudgeConfig {
            nudge_frequency: 5,
            min_context_limit: 0,
            ..NudgeConfig::default()
        };
        let mut state = SessionState::default();
        let messages = vec![user("u1", "hi"), asst("a1", "ack")];

        // Counter starts at 0, next_value = 1, 1 % 5 != 0 → no fire.
        state.nudges.context_limit_counter = 0;
        let map = build_priority_map(&cfg, &state, &messages, 999_999);
        assert!(map.is_empty());

        // Counter at 4, next_value = 5, 5 % 5 == 0 → fires.
        state.nudges.context_limit_counter = 4;
        let map = build_priority_map(&cfg, &state, &messages, 999_999);
        assert_eq!(map.len(), 1);
    }

    // ----- build_priority_map: iteration -----

    #[test]
    fn iteration_fires_when_count_above_threshold() {
        let cfg = NudgeConfig {
            iteration_nudge_threshold: 2,
            nudge_frequency: 1,
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "go"),
            asst("a1", "step1"),
            asst("a2", "step2"),
            asst("a3", "step3"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 0);
        assert_eq!(map.len(), 1);
        let (anchor, kind) = map.into_iter().next().unwrap();
        assert_eq!(anchor, "a3");
        assert!(matches!(kind, NudgeKind::Iteration { count: 3 }));
    }

    #[test]
    fn iteration_does_not_fire_when_count_at_threshold() {
        let cfg = NudgeConfig {
            iteration_nudge_threshold: 3,
            nudge_frequency: 1,
            nudge_force: NudgeForce::Strong, // suppress turn fallback
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "go"),
            asst("a1", "1"),
            asst("a2", "2"),
            asst("a3", "3"),
        ];
        // count == threshold (3) → iteration does not fire; turn is
        // suppressed via Strong force; context-limit is gated off → empty.
        let map = build_priority_map(&cfg, &state, &messages, 0);
        assert!(map.is_empty());
    }

    // ----- build_priority_map: turn -----

    #[test]
    fn turn_fires_for_older_pair_only() {
        let cfg = NudgeConfig {
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "first"),
            asst("a1", "ack 1"),
            user("u2", "second"),
            asst("a2", "ack 2"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 0);
        // The most recent pair (u2/a2) is excluded; only (u1/a1) is
        // eligible.
        assert_eq!(map.len(), 1);
        let (anchor, kind) = map.into_iter().next().unwrap();
        assert_eq!(anchor, "a1");
        assert!(matches!(kind, NudgeKind::Turn));
    }

    #[test]
    fn turn_skips_already_nudged_pair() {
        let cfg = NudgeConfig {
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let mut state = SessionState::default();
        state
            .nudges
            .turn_nudged_pairs
            .insert(("u1".into(), "a1".into()));
        let messages = vec![
            user("u1", "first"),
            asst("a1", "ack 1"),
            user("u2", "second"),
            asst("a2", "ack 2"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 0);
        // Only eligible non-recent pair (u1/a1) is already nudged → no
        // candidate.
        assert!(map.is_empty());
    }

    #[test]
    fn turn_suppressed_under_strong_force() {
        let cfg = NudgeConfig {
            nudge_force: NudgeForce::Strong,
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "first"),
            asst("a1", "ack 1"),
            user("u2", "second"),
            asst("a2", "ack 2"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 0);
        assert!(map.is_empty());
    }

    // ----- build_priority_map: priority ordering -----

    #[test]
    fn context_limit_wins_over_iteration_and_turn() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            iteration_nudge_threshold: 1,
            min_context_limit: 0,
            max_context_limit: 10,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "go"),
            asst("a1", "1"),
            asst("a2", "2"),
            asst("a3", "3"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 1_000);
        assert_eq!(map.len(), 1);
        let kind = map.values().next().unwrap();
        assert!(matches!(kind, NudgeKind::ContextLimit { .. }));
    }

    #[test]
    fn iteration_wins_over_turn_when_context_limit_inactive() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            iteration_nudge_threshold: 1,
            min_context_limit: u64::MAX,
            max_context_limit: u64::MAX,
            ..NudgeConfig::default()
        };
        let state = SessionState::default();
        let messages = vec![
            user("u1", "go"),
            asst("a1", "1"),
            user("u2", "more"),
            asst("a2", "2"),
            asst("a3", "3"),
        ];
        let map = build_priority_map(&cfg, &state, &messages, 0);
        assert_eq!(map.len(), 1);
        let kind = map.values().next().unwrap();
        assert!(matches!(kind, NudgeKind::Iteration { .. }));
    }

    // ----- build_priority_map: frontier -----

    #[test]
    fn frontier_drops_candidates_at_or_before_it() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            min_context_limit: 0,
            ..NudgeConfig::default()
        };
        let mut state = SessionState::default();
        // Set up message refs: u1 -> m0001, a1 -> m0002.
        state
            .message_ids
            .by_raw_id
            .insert("u1".into(), "m0001".into());
        state
            .message_ids
            .by_raw_id
            .insert("a1".into(), "m0002".into());
        state.message_ids.next_ref = 3;
        // Frontier sits at m0002 (== anchor) → context-limit candidate
        // dropped.
        state.prune.messages.frontier_message_ref = Some("m0002".into());

        let messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let map = build_priority_map(&cfg, &state, &messages, cfg.max_context_limit + 1);
        assert!(map.is_empty());
    }

    #[test]
    fn frontier_allows_candidates_beyond() {
        let cfg = NudgeConfig {
            nudge_frequency: 1,
            min_context_limit: 0,
            ..NudgeConfig::default()
        };
        let mut state = SessionState::default();
        state
            .message_ids
            .by_raw_id
            .insert("u1".into(), "m0001".into());
        state
            .message_ids
            .by_raw_id
            .insert("a1".into(), "m0005".into());
        state.message_ids.next_ref = 6;
        state.prune.messages.frontier_message_ref = Some("m0002".into());

        let messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let map = build_priority_map(&cfg, &state, &messages, cfg.max_context_limit + 1);
        assert_eq!(map.len(), 1);
    }

    // ----- render_nudge -----

    #[test]
    fn render_context_limit_substitutes_placeholders() {
        let prompts = Prompts::default();
        let state = SessionState::default();
        let cfg = NudgeConfig::default();
        let s = render_nudge(
            &NudgeKind::ContextLimit {
                tokens: 1234,
                limit: 5_000,
            },
            &prompts,
            &state,
            &cfg,
        );
        assert!(s.contains("1234"));
        assert!(s.contains("5000"));
        assert!(!s.contains("{tokens}"));
        assert!(!s.contains("{limit}"));
    }

    #[test]
    fn render_iteration_substitutes_count() {
        let prompts = Prompts::default();
        let state = SessionState::default();
        let cfg = NudgeConfig::default();
        let s = render_nudge(&NudgeKind::Iteration { count: 42 }, &prompts, &state, &cfg);
        assert!(s.contains("42"));
        assert!(!s.contains("{count}"));
    }

    #[test]
    fn render_turn_uses_template_verbatim() {
        let prompts = Prompts::default();
        let state = SessionState::default();
        let cfg = NudgeConfig::default();
        let s = render_nudge(&NudgeKind::Turn, &prompts, &state, &cfg);
        assert_eq!(s, prompts.turn_nudge);
    }

    // ----- inject_nudges: modes -----

    #[test]
    fn inject_nudges_append_mode_appends_to_existing_text() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig {
            injection_mode: InjectionMode::Append,
            ..NudgeConfig::default()
        };
        let mut messages = vec![asst("a1", "original")];
        let prompts = Prompts {
            turn_nudge: "NUDGE".into(),
            ..Prompts::default()
        };
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        match &messages[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "original\n\nNUDGE"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn inject_nudges_prepend_mode_prepends_to_existing_text() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig {
            injection_mode: InjectionMode::Prepend,
            ..NudgeConfig::default()
        };
        let mut messages = vec![asst("a1", "body")];
        let prompts = Prompts {
            turn_nudge: "TOP".into(),
            ..Prompts::default()
        };
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        match &messages[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "TOP\n\nbody"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn inject_nudges_replace_mode_replaces_text() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig {
            injection_mode: InjectionMode::Replace,
            ..NudgeConfig::default()
        };
        let mut messages = vec![asst("a1", "old")];
        let prompts = Prompts {
            turn_nudge: "NEW".into(),
            ..Prompts::default()
        };
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        match &messages[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "NEW"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn inject_nudges_creates_text_part_when_missing() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig::default();
        // Message has only a tool_call — no text part.
        let mut messages = vec![Message::new(
            "a1",
            Role::Assistant,
            vec![Part::tool_call("c1", "tool", json!({}))],
            0,
        )];
        let prompts = Prompts {
            turn_nudge: "X".into(),
            ..Prompts::default()
        };
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        let text_count = messages[0]
            .parts
            .iter()
            .filter(|p| matches!(p, Part::Text(_)))
            .count();
        assert_eq!(text_count, 1);
    }

    #[test]
    fn inject_nudges_resets_context_limit_counter() {
        let mut state = SessionState::default();
        state.nudges.context_limit_counter = 4;
        let cfg = NudgeConfig::default();
        let mut messages = vec![asst("a1", "body")];
        let prompts = Prompts::default();
        let mut priorities = HashMap::new();
        priorities.insert(
            "a1".into(),
            NudgeKind::ContextLimit {
                tokens: 100,
                limit: 50,
            },
        );
        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        assert_eq!(state.nudges.context_limit_counter, 0);
    }

    #[test]
    fn inject_nudges_records_turn_nudged_pair() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig::default();
        let mut messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let prompts = Prompts::default();
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_nudges(&mut state, &cfg, &mut messages, &prompts, &priorities);
        assert!(
            state
                .nudges
                .turn_nudged_pairs
                .contains(&("u1".into(), "a1".into()))
        );
    }

    #[test]
    fn inject_nudges_no_priorities_is_noop() {
        let mut state = SessionState::default();
        let mut messages = vec![asst("a1", "body")];
        let original = messages.clone();
        let prompts = Prompts::default();
        let priorities: HashMap<String, NudgeKind> = HashMap::new();
        inject_nudges(
            &mut state,
            &NudgeConfig::default(),
            &mut messages,
            &prompts,
            &priorities,
        );
        assert_eq!(messages, original);
    }

    // ----- inject_message_ids -----

    #[test]
    fn inject_message_ids_tags_priority_anchor() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig::default();
        let mut messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_message_ids(&mut state, &cfg, &mut messages, &priorities);
        // a1 received m0002 (u1 was m0001).
        let part = match &messages[1].parts[0] {
            Part::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(part.starts_with("<dcp-message-id>m0002</dcp-message-id>"));
        assert!(part.contains("ack"));
    }

    #[test]
    fn inject_message_ids_skips_non_priority_messages() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig::default();
        let mut messages = vec![user("u1", "hi"), asst("a1", "ack")];
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_message_ids(&mut state, &cfg, &mut messages, &priorities);
        // u1 is not a priority anchor → no tag.
        match &messages[0].parts[0] {
            Part::Text(t) => assert_eq!(t, "hi"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn inject_message_ids_idempotent() {
        let mut state = SessionState::default();
        let cfg = NudgeConfig::default();
        let mut messages = vec![asst("a1", "ack")];
        let mut priorities = HashMap::new();
        priorities.insert("a1".into(), NudgeKind::Turn);

        inject_message_ids(&mut state, &cfg, &mut messages, &priorities);
        let after_first = messages[0].clone();
        inject_message_ids(&mut state, &cfg, &mut messages, &priorities);
        assert_eq!(messages[0], after_first);
    }

    #[test]
    fn inject_message_ids_no_priorities_is_noop() {
        let mut state = SessionState::default();
        let mut messages = vec![asst("a1", "body")];
        let original = messages.clone();
        inject_message_ids(
            &mut state,
            &NudgeConfig::default(),
            &mut messages,
            &HashMap::new(),
        );
        assert_eq!(messages, original);
        assert_eq!(state.message_ids.next_ref, 0);
    }

    // ----- inject_extended_subagent_results -----

    #[test]
    fn inject_extended_subagent_disabled_passthrough() {
        let mut state = SessionState::default();
        state
            .subagent_result_cache
            .insert("c1".into(), "EXTENDED".into());
        let messages = vec![Message::new(
            "u1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                Some("short".into()),
                None,
            )],
            0,
        )];
        let out = inject_extended_subagent_results(&state, messages.clone(), false);
        assert_eq!(out, messages);
    }

    #[test]
    fn inject_extended_subagent_enabled_appends_extended() {
        let mut state = SessionState::default();
        state
            .subagent_result_cache
            .insert("c1".into(), "EXTENDED".into());
        let messages = vec![Message::new(
            "u1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                Some("short".into()),
                None,
            )],
            0,
        )];
        let out = inject_extended_subagent_results(&state, messages, true);
        match &out[0].parts[0] {
            Part::ToolResult { output, .. } => {
                let text = output.as_deref().unwrap_or("");
                assert!(text.starts_with("short"));
                assert!(text.contains("EXTENDED"));
                assert!(text.contains("<dcp-subagent-extended>"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn inject_extended_subagent_no_match_passthrough() {
        let mut state = SessionState::default();
        state
            .subagent_result_cache
            .insert("OTHER".into(), "EXTENDED".into());
        let messages = vec![Message::new(
            "u1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                Some("short".into()),
                None,
            )],
            0,
        )];
        let out = inject_extended_subagent_results(&state, messages.clone(), true);
        assert_eq!(out, messages);
    }

    #[test]
    fn inject_extended_subagent_empty_cache_passthrough() {
        let state = SessionState::default();
        let messages = vec![Message::new(
            "u1",
            Role::User,
            vec![Part::tool_result(
                "c1",
                ToolStatus::Completed,
                Some("short".into()),
                None,
            )],
            0,
        )];
        let out = inject_extended_subagent_results(&state, messages.clone(), true);
        assert_eq!(out, messages);
    }

    // ----- helpers -----

    #[test]
    fn assistant_since_user_counts_correctly() {
        let messages = vec![
            user("u1", "hi"),
            asst("a1", "1"),
            asst("a2", "2"),
            user("u2", "again"),
            asst("a3", "3"),
            asst("a4", "4"),
            asst("a5", "5"),
        ];
        assert_eq!(assistant_since_user(&messages), 3);
    }

    #[test]
    fn assistant_since_user_zero_when_no_user() {
        let messages = vec![asst("a1", "1"), asst("a2", "2")];
        assert_eq!(assistant_since_user(&messages), 0);
    }

    #[test]
    fn turn_pairs_includes_all_user_assistant_transitions() {
        let messages = vec![
            user("u1", "hi"),
            asst("a1", "ack 1"),
            user("u2", "again"),
            asst("a2", "ack 2"),
            asst("a3", "ack 3"),
        ];
        let pairs = turn_pairs(&messages);
        assert_eq!(
            pairs,
            vec![
                ("u1".into(), "a1".into()),
                ("u2".into(), "a2".into()),
                ("u2".into(), "a3".into()),
            ]
        );
    }

    #[test]
    fn past_frontier_no_frontier_passes_all() {
        let state = SessionState::default();
        assert!(past_frontier(&state, "any-id"));
    }

    #[test]
    fn past_frontier_with_block_form_passes_all() {
        let mut state = SessionState::default();
        state.prune.messages.frontier_message_ref = Some("b3".into());
        assert!(past_frontier(&state, "any-id"));
    }

    // ----- serde -----

    #[test]
    fn nudge_kind_serde_roundtrip() {
        let cases = [
            NudgeKind::ContextLimit {
                tokens: 1,
                limit: 2,
            },
            NudgeKind::Turn,
            NudgeKind::Iteration { count: 5 },
        ];
        for k in cases {
            let s = serde_json::to_string(&k).unwrap();
            let back: NudgeKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
    }

    #[test]
    fn injection_mode_serde_roundtrip() {
        for m in [
            InjectionMode::Replace,
            InjectionMode::Append,
            InjectionMode::Prepend,
        ] {
            let s = serde_json::to_string(&m).unwrap();
            let back: InjectionMode = serde_json::from_str(&s).unwrap();
            assert_eq!(m, back);
        }
    }

    #[test]
    fn nudge_config_serde_roundtrip() {
        let cfg = NudgeConfig {
            iteration_nudge_threshold: 7,
            ..NudgeConfig::default()
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: NudgeConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }
}
