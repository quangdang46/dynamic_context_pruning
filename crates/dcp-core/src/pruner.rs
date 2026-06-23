//! [`ContextPruner`] — the public facade owning the entire pruning
//! pipeline (PLAN.md §4.2).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use dcp_compress::{CompressArgs, CompressResult};
use dcp_config::Config;
use dcp_storage::{FileStateStore, default_storage_dir};
use dcp_prompts::{PromptStore, Prompts};
use dcp_telemetry::Telemetry;
use dcp_traits::{
    CacheAccountant, MemoryRetriever, PruneStrategy, StatePersistence, Tokenizer,
    defaults::NoopStorage,
};
use dcp_types::{BlockId, Message, Part, Role, SessionState, Stats};

use crate::error::Error;
use crate::pipeline::{self, TransformResult};
use crate::tokenizer::Char4Tokenizer;

// ─────────────────────────────────────────────────────────────────────────
// Decompress / recompress result types
// ─────────────────────────────────────────────────────────────────────────

/// Result of [`ContextPruner::decompress`] — the block has been
/// deactivated and its anchor will render verbatim again on the next
/// `transform_messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecompressResult {
    /// The block id that was deactivated.
    pub block_id: BlockId,
    /// The raw anchor message id whose content is restored.
    pub anchor_message_id: String,
}

/// Result of [`ContextPruner::recompress`] — a previously
/// user-decompressed block was re-activated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecompressResult {
    /// The block id that was re-activated.
    pub block_id: BlockId,
    /// The raw anchor message id that re-renders as the block summary.
    pub anchor_message_id: String,
}

// ─────────────────────────────────────────────────────────────────────────
// ContextPruner
// ─────────────────────────────────────────────────────────────────────────

/// The main facade.
///
/// Constructed via [`ContextPruner::new`] (defaults wiring) or
/// [`ContextPruner::builder`] for full control. Owns:
///
/// * The [`SessionState`] (`pub` accessor [`Self::state`]).
/// * The [`Config`] (`pub` accessor [`Self::config`]).
/// * The [`Prompts`] surface (override-aware).
/// * Pluggable [`Tokenizer`], [`StatePersistence`], optional
///   [`MemoryRetriever`], and optional [`CacheAccountant`].
/// * A live [`Telemetry`] counter (snapshot via [`Self::telemetry`]).
///
/// The host wires this into its agent in three lines:
///
/// ```no_run
/// use dcp_core::{ContextPruner, Config};
///
/// let cfg = Config::default();
/// let mut pruner = ContextPruner::new(cfg).unwrap();
/// // … later, on every turn:
/// // let pruned = pruner.transform_messages(messages)?;
/// ```
pub struct ContextPruner {
    pub(crate) state: SessionState,
    pub(crate) config: Config,
    pub(crate) prompts: Prompts,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
    pub(crate) persistence: Arc<dyn StatePersistence>,
    pub(crate) memory: Option<Arc<dyn MemoryRetriever>>,
    pub(crate) cache_accountant: Option<Arc<Mutex<dyn CacheAccountant>>>,
    pub(crate) custom_strategies: Vec<Box<dyn PruneStrategy<Config>>>,
    pub(crate) telemetry: Telemetry,
}

impl ContextPruner {
    // ──────────────────────────────────────────────────────────────────
    // Constructors
    // ──────────────────────────────────────────────────────────────────

    /// Construct a [`ContextPruner`] with the bundled defaults:
    ///
    /// * `tokenizer` = [`Char4Tokenizer`] (no external deps).
    /// * `persistence` = [`NoopStorage`] (in-memory).
    /// * `memory` = `None`.
    /// * `cache_accountant` = `None`.
    /// * `prompts` = [`Prompts::default`].
    ///
    /// For full control use [`ContextPruner::builder`].
    pub fn new(config: Config) -> Result<Self, Error> {
        Self::builder().config(config).build()
    }

    /// Start a [`ContextPrunerBuilder`] for incremental wiring.
    pub fn builder() -> ContextPrunerBuilder {
        ContextPrunerBuilder::default()
    }

    // ──────────────────────────────────────────────────────────────────
    // Config
    // ──────────────────────────────────────────────────────────────────

    /// Borrow the active configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replace the configuration in place. Re-runs validation and
    /// rebuilds the cached protection sets.
    pub fn update_config(&mut self, mut config: Config) -> Result<(), Error> {
        config.rebuild_cache()?;
        config.validate()?;
        self.config = config;
        Ok(())
    }

    // ──────────────────────────────────────────────────────────────────
    // Hot path
    // ──────────────────────────────────────────────────────────────────

    /// Run the 10-phase pipeline (SPEC.md §5.4 + PLAN.md §6.4).
    ///
    /// Returns a fresh `Vec<Message>` — the input is consumed but not
    /// mutated by the host's perspective.
    pub fn transform_messages(&mut self, messages: Vec<Message>) -> Result<Vec<Message>, Error> {
        let result = self.transform_messages_inner(messages)?;
        Ok(result.messages)
    }

    /// Transform messages and return a diff of what changed.
    ///
    /// This is the same as [`transform_messages()`][Self::transform_messages]
    /// but also returns a [`TransformResult`] with details about what was
    /// pruned (removed message IDs, tool IDs, tokens saved, new block IDs).
    ///
    /// jcode uses this to update its CompactionManager budget and show
    /// notifications about what was pruned.
    pub fn transform_messages_with_diff(
        &mut self,
        messages: Vec<Message>,
    ) -> Result<TransformResult, Error> {
        let input_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        let stats_before = self.stats().clone();
        let result = self.transform_messages_inner(messages)?;
        let output_ids: std::collections::HashSet<String> =
            result.messages.iter().map(|m| m.id.clone()).collect();
        let removed: Vec<String> = input_ids
            .into_iter()
            .filter(|id| !output_ids.contains(id))
            .collect();
        let stats_after = self.stats();
        let tokens_saved = stats_after
            .total_prune_tokens
            .saturating_sub(stats_before.total_prune_tokens);
        let changed = !removed.is_empty() || tokens_saved > 0;
        let new_block_ids: Vec<BlockId> = self
            .state
            .prune
            .messages
            .active_block_ids
            .iter()
            .cloned()
            .collect();
        Ok(TransformResult {
            messages: result.messages,
            removed_message_ids: removed,
            pruned_tool_ids: result.pruned_tool_ids,
            tokens_saved,
            new_block_ids,
            changed,
        })
    }

    fn transform_messages_inner(
        &mut self,
        messages: Vec<Message>,
    ) -> Result<TransformResult, Error> {
        let mut pruned_tool_ids: Vec<String> = Vec::new();

        if !self.config.enabled {
            return Ok(TransformResult {
                messages,
                removed_message_ids: Vec::new(),
                pruned_tool_ids: Vec::new(),
                tokens_saved: 0,
                new_block_ids: Vec::new(),
                changed: false,
            });
        }

        let now_ms = chrono::Utc::now().timestamp_millis();

        let valid = pipeline::filter_valid_messages(messages, &mut self.state);
        pipeline::detect_compaction(&mut self.state, &valid, now_ms);

        pipeline::sync_state(&mut self.state, &self.config, &valid);
        let system_addendum = pipeline::render_system_addendum(&self.prompts, &self.config);
        pipeline::cache_system_prompt_tokens(
            &mut self.state,
            self.tokenizer.as_ref(),
            &system_addendum,
        );

        if self.state.is_subagent && !self.config.experimental.allow_subagents {
            return Ok(TransformResult {
                messages: valid,
                removed_message_ids: Vec::new(),
                pruned_tool_ids: Vec::new(),
                tokens_saved: 0,
                new_block_ids: Vec::new(),
                changed: false,
            });
        }

        let before_pruned = self.state.prune.tools.len();
        let apply_now = pipeline::should_apply_now(&self.state, &self.config);
        if apply_now {
            let outcomes =
                pipeline::run_strategies(&mut self.state, &self.config, &mut self.telemetry);
            for outcome in &outcomes {
                for id in &outcome.pruned_ids {
                    pruned_tool_ids.push(id.clone());
                }
            }
            for strat in &self.custom_strategies {
                let outcome = strat
                    .apply(&mut self.state, &valid, &self.config)
                    .map_err(Error::from)?;
                if outcome.reason_skipped.is_none() {
                    self.telemetry.record(dcp_telemetry::EventKind::Prune {
                        strategy: outcome.strategy,
                    });
                }
            }
            self.telemetry
                .record(dcp_telemetry::EventKind::ApplyTrigger {
                    mode: self.config.cache_stability_mode.as_str().to_string(),
                });
            self.state.last_apply_turn = Some(self.state.current_turn);
            self.state.force_apply_requested = false;
            self.state.pending_prune = None;
        } else {
            let _outcomes =
                pipeline::run_strategies(&mut self.state, &self.config, &mut self.telemetry);
            pipeline::accumulate_pending(&mut self.state, before_pruned);
        }

        let pruned = if apply_now {
            pipeline::apply_prune(&self.state, &valid)
        } else {
            valid.clone()
        };

        let pruned = pipeline::expand_compressed(pruned, &self.state);
        let pruned = pipeline::inject_subagent_results(&self.state, &self.config, pruned);

        let mut pruned = pruned;
        pipeline::inject_nudges_and_ids(
            &mut self.state,
            &self.config,
            &self.prompts,
            self.tokenizer.as_ref(),
            &mut pruned,
            &mut self.telemetry,
        );

        if let Some(trigger) = self.state.pending_manual_trigger.take()
            && trigger.force_apply
        {
            // Already applied above when the gate opened; nothing more
            // to do here.
        }

        pipeline::strip_internal_metadata(&mut pruned);

        if let Some(session_id) = self.state.session_id.clone() {
            let envelope = pipeline::build_persisted(&self.state);
            if let Err(e) = self.persistence.save(&session_id, &envelope) {
                self.state.stats.storage_save_failed =
                    self.state.stats.storage_save_failed.saturating_add(1);
                self.telemetry.record(dcp_telemetry::EventKind::Other {
                    name: format!("persistence_save_failed:{e}"),
                });
            }
        }

        self.telemetry.record(dcp_telemetry::EventKind::Other {
            name: "transform_messages".into(),
        });

        let new_block_ids: Vec<BlockId> = self
            .state
            .prune
            .messages
            .active_block_ids
            .iter()
            .cloned()
            .collect();

        Ok(TransformResult {
            messages: pruned,
            removed_message_ids: Vec::new(),
            pruned_tool_ids,
            tokens_saved: 0,
            new_block_ids,
            changed: false,
        })
    }

    /// Append the system-prompt addendum (SPEC.md §6.6). The addendum
    /// renders the protected-tools block, the manual-mode note (if
    /// active), and the sub-agent note (if enabled).
    ///
    /// The function trims the addendum's leading/trailing whitespace
    /// before append; if `system` already ends with a non-whitespace
    /// byte, a blank line is inserted as a separator.
    pub fn transform_system(&self, system: &mut String) {
        let addendum = pipeline::render_system_addendum(&self.prompts, &self.config);
        let trimmed = addendum.trim();
        if trimmed.is_empty() {
            return;
        }
        if !system.is_empty() && !system.ends_with("\n\n") {
            if system.ends_with('\n') {
                system.push('\n');
            } else {
                system.push_str("\n\n");
            }
        }
        system.push_str(trimmed);
    }

    // ──────────────────────────────────────────────────────────────────
    // Compress tool
    // ──────────────────────────────────────────────────────────────────

    /// Build the JSON-schema descriptor used to register the compress
    /// tool with the LLM (SPEC.md §6.1 / §6.2).
    pub fn compress_tool_schema(&self) -> crate::pipeline::ToolSchema {
        pipeline::compress_tool_schema(&self.prompts, &self.config)
    }

    /// Execute the compress tool. The host typically forwards the
    /// model's tool call here and returns the result back to the
    /// model.
    pub fn handle_compress(
        &mut self,
        args: CompressArgs,
        raw_messages: &[Message],
    ) -> Result<CompressResult, Error> {
        if self.config.compress.permission == dcp_config::Permission::Deny {
            return Err(Error::PermissionDenied);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = dcp_compress::handle_compress(
            args,
            &mut self.state,
            raw_messages,
            &self.config,
            now_ms,
        )?;
        let mode = match self.config.compress.mode {
            dcp_config::CompressMode::Range => "range",
            dcp_config::CompressMode::Message => "message",
        };
        self.telemetry.record(dcp_telemetry::EventKind::Compress {
            mode: mode.to_string(),
        });
        Ok(result)
    }

    /// Mark a previously committed block as user-deactivated. The block
    /// stays in `blocks_by_id` for audit; subsequent `transform_messages`
    /// calls expand the original anchor verbatim again.
    pub fn decompress(&mut self, block_id: BlockId) -> Result<DecompressResult, Error> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let block = self
            .state
            .prune
            .messages
            .blocks_by_id
            .get_mut(&block_id)
            .ok_or(Error::BlockNotFound(block_id))?;
        if !block.active {
            return Err(Error::BlockNotFound(block_id));
        }
        block.active = false;
        block.deactivated_by_user = true;
        block.deactivated_at = Some(now_ms);
        let anchor = block.anchor_message_id.clone();
        self.state.prune.messages.active_block_ids.remove(&block_id);
        if self
            .state
            .prune
            .messages
            .active_by_anchor_message_id
            .get(&anchor)
            == Some(&block_id)
        {
            self.state
                .prune
                .messages
                .active_by_anchor_message_id
                .remove(&anchor);
        }
        Ok(DecompressResult {
            block_id,
            anchor_message_id: anchor,
        })
    }

    /// Re-activate a block previously deactivated via [`Self::decompress`].
    pub fn recompress(&mut self, block_id: BlockId) -> Result<RecompressResult, Error> {
        let block = self
            .state
            .prune
            .messages
            .blocks_by_id
            .get_mut(&block_id)
            .ok_or(Error::BlockNotFound(block_id))?;
        if block.active {
            return Err(Error::BlockNotFound(block_id));
        }
        // Only allow re-activation of user-driven decompressions; blocks
        // that were consumed by a parent stay consumed.
        if !block.deactivated_by_user {
            return Err(Error::BlockNotFound(block_id));
        }
        block.active = true;
        block.deactivated_at = None;
        block.deactivated_by_user = false;
        let anchor = block.anchor_message_id.clone();
        self.state.prune.messages.active_block_ids.insert(block_id);
        self.state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert(anchor.clone(), block_id);
        Ok(RecompressResult {
            block_id,
            anchor_message_id: anchor,
        })
    }

    // ──────────────────────────────────────────────────────────────────
    // Slash commands
    // ──────────────────────────────────────────────────────────────────

    /// Dispatch a `/dcp …` slash command. See [`crate::commands`].
    pub fn handle_command(
        &mut self,
        cmd: &str,
        args: &[&str],
        raw_messages: &[Message],
    ) -> crate::commands::CommandOutcome {
        crate::commands::handle_command(self, cmd, args, raw_messages)
    }

    // ──────────────────────────────────────────────────────────────────
    // Sub-agent
    // ──────────────────────────────────────────────────────────────────

    /// Fold the result of a sub-agent run into a single synthetic
    /// assistant message.
    ///
    /// SPEC.md §11.6 — the parent receives a brief synthesis of the
    /// sub-agent's transcript so it can continue the parent task
    /// without inheriting the sub-agent's full token cost.
    ///
    /// The default implementation concatenates every assistant text
    /// part from `subagent_messages`, separated by blank lines, and
    /// returns the result wrapped in a `<dcp-subagent-result>` block.
    /// Hosts that need richer behaviour (e.g. structured outputs) can
    /// pre-process `subagent_messages` before calling this method.
    pub fn fold_subagent(&mut self, subagent_messages: Vec<Message>) -> Result<Message, Error> {
        if !self.config.experimental.allow_subagents {
            return Err(Error::SubagentsDisabled);
        }
        let mut buf = String::from("<dcp-subagent-result>\n");
        let mut seen_any = false;
        for msg in &subagent_messages {
            if msg.role != Role::Assistant {
                continue;
            }
            for part in &msg.parts {
                if let Part::Text(t) = part {
                    let trimmed = t.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if seen_any {
                        buf.push_str("\n\n");
                    }
                    buf.push_str(trimmed);
                    seen_any = true;
                }
            }
        }
        if !seen_any {
            buf.push_str("(empty)");
        }
        buf.push_str("\n</dcp-subagent-result>");

        let now_ms = chrono::Utc::now().timestamp_millis();
        let id = format!("subagent-{}", now_ms);
        Ok(Message::new(
            id,
            Role::Assistant,
            vec![Part::Text(buf)],
            now_ms,
        ))
    }

    // ──────────────────────────────────────────────────────────────────
    // Introspection
    // ──────────────────────────────────────────────────────────────────

    /// Borrow the persisted [`Stats`] counters. SPEC.md §9.1.
    pub fn stats(&self) -> &Stats {
        &self.state.stats
    }

    /// Borrow the live [`SessionState`].
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Snapshot the live [`Telemetry`] counter.
    pub fn telemetry(&self) -> dcp_telemetry::TelemetrySnapshot {
        self.telemetry.snapshot()
    }

    /// Borrow the bundled [`Prompts`].
    pub fn prompts(&self) -> &Prompts {
        &self.prompts
    }

    /// Count total tokens across all message parts using the installed tokenizer.
    ///
    /// Convenience method so jcode doesn't need to access the tokenizer directly.
    pub fn count_messages_tokens(&self, messages: &[Message]) -> u64 {
        let mut total = 0u64;
        let tokenizer = self.tokenizer.as_ref();
        for msg in messages {
            for part in &msg.parts {
                match part {
                    Part::Text(text) | Part::Reasoning(text) => {
                        total = total.saturating_add(tokenizer.count(text) as u64);
                    }
                    Part::ToolCall { tool, input, .. } => {
                        total = total.saturating_add(tokenizer.count(tool) as u64);
                        total = total.saturating_add(
                            tokenizer.count(&serde_json::to_string(input).unwrap_or_default())
                                as u64,
                        );
                    }
                    Part::ToolResult { output, error, .. } => {
                        if let Some(o) = output {
                            total = total.saturating_add(tokenizer.count(o) as u64);
                        }
                        if let Some(e) = error {
                            total = total.saturating_add(tokenizer.count(e) as u64);
                        }
                    }
                    Part::Image { .. } => {
                        total = total.saturating_add(self.config.image_tokens() as u64);
                    }
                    _ => {}
                }
            }
        }
        total
    }

    /// Return the kind of the most recently injected nudge, if any.
    ///
    /// Return the kind of the most recently injected nudge, if any.
    ///
    /// jcode can check this after `transform_messages()` to decide
    /// whether to show a context-limit warning in the TUI.
    pub fn last_nudge_kind(&self) -> Option<String> {
        self.state.nudges.last_nudge_kind.clone()
    }

    /// Borrow the optional [`MemoryRetriever`] hook installed via the
    /// builder. Returns `None` when the host has not provided one.
    pub fn memory(&self) -> Option<&Arc<dyn MemoryRetriever>> {
        self.memory.as_ref()
    }

    /// Borrow the optional [`CacheAccountant`] hook installed via the
    /// builder. Returns `None` when the host has not provided one.
    pub fn cache_accountant(&self) -> Option<&Arc<Mutex<dyn CacheAccountant>>> {
        self.cache_accountant.as_ref()
    }

    /// Number of host-supplied custom prune strategies registered via
    /// [`ContextPrunerBuilder::add_strategy`].
    pub fn custom_strategy_count(&self) -> usize {
        self.custom_strategies.len()
    }

    // ──────────────────────────────────────────────────────────────────
    // Lifecycle
    // ──────────────────────────────────────────────────────────────────

    /// Reset the in-memory state to its post-construction shape. Storage
    /// is **not** modified.
    pub fn reset(&mut self) {
        self.state = SessionState::default();
        self.telemetry.reset();
    }

    /// Flush the live state to the storage backend. Called automatically
    /// at the end of each `transform_messages`; hosts may also call this
    /// explicitly e.g. on graceful shutdown.
    pub fn save(&self) -> Result<(), Error> {
        let Some(session_id) = self.state.session_id.clone() else {
            return Ok(());
        };
        let envelope = pipeline::build_persisted(&self.state);
        self.persistence
            .save(&session_id, &envelope)
            .map_err(Error::from)
    }

    // ──────────────────────────────────────────────────────────────────
    // Manual control
    // ──────────────────────────────────────────────────────────────────

    /// Force the next `transform_messages` to apply pending prune
    /// decisions regardless of the configured cache-stability mode.
    pub fn force_apply(&mut self) -> Result<(), Error> {
        self.state.force_apply_requested = true;
        Ok(())
    }

    /// Flip the manual-mode flag. SPEC.md §5 — when enabled and
    /// `manualMode.automaticStrategies == false`, strategies are
    /// suspended.
    pub fn set_manual_mode(&mut self, enabled: bool) {
        self.state.manual_mode.enabled = enabled;
    }

    /// Explicitly set the host-assigned session id. The next
    /// `transform_messages` invocation will persist under this key.
    /// When unset, the library derives one from the last message id
    /// (SPEC.md §3.1).
    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.state.session_id = Some(id.into());
    }

    /// Check whether DCP has pending prune decisions or pending work.
    ///
    /// Returns `true` when:
    /// - There is a pending prune snapshot waiting to be applied
    /// - There are pending tool-level prune decisions (`prune.tools` non-empty)
    ///
    /// jcode can call this before `transform_messages()` to decide whether
    /// the DCP transform is needed for the current turn.
    pub fn has_pending_work(&self) -> bool {
        let state = &self.state;
        state.pending_prune.is_some() || !state.prune.tools.is_empty()
    }

    // Internal helper used by the slash-command dispatcher.
    pub(crate) fn set_force_apply(&mut self) {
        self.state.force_apply_requested = true;
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────

/// Builder for [`ContextPruner`] (PLAN.md §4.3).
///
/// Constructed via [`ContextPruner::builder`] or [`Default::default`].
/// Each builder method consumes `self` to keep the type signature
/// compact; chain calls and finish with [`ContextPrunerBuilder::build`].
#[derive(Default)]
pub struct ContextPrunerBuilder {
    config: Option<Config>,
    tokenizer: Option<Arc<dyn Tokenizer>>,
    persistence: Option<Arc<dyn StatePersistence>>,
    memory: Option<Arc<dyn MemoryRetriever>>,
    cache_accountant: Option<Arc<Mutex<dyn CacheAccountant>>>,
    custom_strategies: Vec<Box<dyn PruneStrategy<Config>>>,
    prompts: Option<Prompts>,
    prompt_store: Option<PromptStore>,
}

impl ContextPrunerBuilder {
    /// Set the configuration. If omitted, [`Config::default`] is used.
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Install a custom tokenizer. Default: [`Char4Tokenizer`].
    pub fn tokenizer(mut self, tokenizer: Arc<dyn Tokenizer>) -> Self {
        self.tokenizer = Some(tokenizer);
        self
    }

    /// Install a custom storage backend. Default: in-memory
    /// [`NoopStorage`].
    pub fn storage(mut self, storage: Arc<dyn StatePersistence>) -> Self {
        self.persistence = Some(storage);
        self
    }

    /// Install a [`MemoryRetriever`] (off by default).
    pub fn memory(mut self, retriever: Arc<dyn MemoryRetriever>) -> Self {
        self.memory = Some(retriever);
        self
    }

    /// Install a [`CacheAccountant`] (off by default).
    pub fn cache_accountant(mut self, accountant: Arc<Mutex<dyn CacheAccountant>>) -> Self {
        self.cache_accountant = Some(accountant);
        self
    }

    /// Append a host-defined [`PruneStrategy`] to the pipeline. Custom
    /// strategies run after the three built-in strategies in
    /// declaration order.
    pub fn add_strategy(mut self, strategy: Box<dyn PruneStrategy<Config>>) -> Self {
        self.custom_strategies.push(strategy);
        self
    }

    /// Override the bundled prompts wholesale. Mutually exclusive with
    /// [`Self::prompt_store`] — the last one wins.
    pub fn prompts(mut self, prompts: Prompts) -> Self {
        self.prompts = Some(prompts);
        self.prompt_store = None;
        self
    }

    /// Install a fully-resolved [`PromptStore`] (e.g. from
    /// `PromptStore::with_overrides`).
    pub fn prompt_store(mut self, store: PromptStore) -> Self {
        self.prompts = None;
        self.prompt_store = Some(store);
        self
    }

    /// Finish building. Returns [`Error::Config`] when configuration
    /// validation fails; otherwise the wired-up [`ContextPruner`].
    pub fn build(self) -> Result<ContextPruner, Error> {
        let mut config = self.config.unwrap_or_default();
        config.rebuild_cache()?;
        config.validate()?;

        let prompts = match (self.prompts, self.prompt_store) {
            (Some(p), _) => p,
            (None, Some(store)) => store.into_prompts(),
            (None, None) => Prompts::default(),
        };

        let tokenizer: Arc<dyn Tokenizer> = self
            .tokenizer
            .unwrap_or_else(|| Arc::new(Char4Tokenizer::new()));

        let persistence: Arc<dyn StatePersistence> = if self.persistence.is_some() {
            self.persistence.unwrap()
        } else if config.persistence.enabled {
            let dir = config
                .persistence
                .path
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(default_storage_dir);
            Arc::new(
                FileStateStore::new(dir).with_backup(config.persistence.keep_backup),
            )
        } else {
            Arc::new(NoopStorage::new())
        };

        Ok(ContextPruner {
            state: SessionState::default(),
            config,
            prompts,
            tokenizer,
            persistence,
            memory: self.memory,
            cache_accountant: self.cache_accountant,
            custom_strategies: self.custom_strategies,
            telemetry: Telemetry::now(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests (unit)
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dcp_types::{Part, Role};

    fn pruner() -> ContextPruner {
        ContextPruner::new(Config::default()).unwrap()
    }

    #[test]
    fn new_uses_defaults() {
        let p = pruner();
        assert!(p.config().enabled);
        assert_eq!(p.state().current_turn, 0);
        assert_eq!(p.telemetry().total_events(), 0);
    }

    #[test]
    fn transform_passes_through_when_disabled() {
        let mut cfg = Config::default();
        cfg.enabled = false;
        cfg.rebuild_cache().unwrap();
        let mut p = ContextPruner::new(cfg).unwrap();
        let messages = vec![Message::user_text("u1", 0, "hi")];
        let out = p.transform_messages(messages.clone()).unwrap();
        assert_eq!(out, messages);
    }

    #[test]
    fn transform_basic_roundtrip() {
        let mut p = pruner();
        let messages = vec![
            Message::user_text("u1", 0, "hello"),
            Message::assistant_text("a1", 0, "hi there"),
        ];
        let out = p.transform_messages(messages).unwrap();
        // Two messages flow through; counters reflect at least one
        // transform.
        assert!(!out.is_empty());
        assert!(p.telemetry().total_events() >= 1);
    }

    #[test]
    fn transform_assigns_message_refs() {
        let mut p = pruner();
        let messages = vec![
            Message::user_text("u1", 0, "hi"),
            Message::assistant_text("a1", 0, "hello"),
        ];
        let _ = p.transform_messages(messages).unwrap();
        assert_eq!(
            p.state()
                .message_ids
                .by_raw_id
                .get("u1")
                .map(|s| s.as_str()),
            Some("m0001")
        );
        assert_eq!(
            p.state()
                .message_ids
                .by_raw_id
                .get("a1")
                .map(|s| s.as_str()),
            Some("m0002")
        );
    }

    #[test]
    fn transform_drops_invalid_messages() {
        let mut p = pruner();
        // A user-role message carrying a tool_call is invalid.
        let messages = vec![
            Message::new(
                "u1",
                Role::User,
                vec![Part::tool_call("c1", "read", serde_json::json!({}))],
                0,
            ),
            Message::user_text("u2", 0, "hi"),
        ];
        let out = p.transform_messages(messages).unwrap();
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["u2"]);
        assert_eq!(p.state().stats.dropped_invalid, 1);
    }

    #[test]
    fn transform_system_appends_addendum() {
        let p = pruner();
        let mut sys = String::from("You are a helpful assistant.");
        p.transform_system(&mut sys);
        assert!(sys.contains("Context-pruning support"));
        assert!(sys.starts_with("You are a helpful assistant."));
    }

    #[test]
    fn compress_tool_schema_returns_range_default() {
        let p = pruner();
        let schema = p.compress_tool_schema();
        assert_eq!(schema.name, "compress");
        // Range default has `startId`/`endId` properties.
        let s = serde_json::to_string(&schema.parameters).unwrap();
        assert!(s.contains("startId"));
        assert!(s.contains("endId"));
    }

    #[test]
    fn manual_mode_toggle() {
        let mut p = pruner();
        assert!(!p.state().manual_mode.enabled);
        p.set_manual_mode(true);
        assert!(p.state().manual_mode.enabled);
        p.set_manual_mode(false);
        assert!(!p.state().manual_mode.enabled);
    }

    #[test]
    fn force_apply_sets_flag() {
        let mut p = pruner();
        assert!(!p.state().force_apply_requested);
        p.force_apply().unwrap();
        assert!(p.state().force_apply_requested);
    }

    #[test]
    fn reset_clears_state() {
        let mut p = pruner();
        let messages = vec![Message::user_text("u1", 0, "hi")];
        let _ = p.transform_messages(messages).unwrap();
        assert!(!p.state().message_ids.by_raw_id.is_empty());
        p.reset();
        assert!(p.state().message_ids.by_raw_id.is_empty());
        assert_eq!(p.telemetry().total_events(), 0);
    }

    #[test]
    fn update_config_rebuilds_cache_and_validates() {
        let mut p = pruner();
        let mut new_cfg = Config::default();
        new_cfg.protected_file_patterns.push("Cargo.toml".into());
        p.update_config(new_cfg).unwrap();
        assert!(p.config().protected_paths().is_protected("Cargo.toml"));
    }

    #[test]
    fn update_config_rejects_invalid() {
        let mut p = pruner();
        let mut bad = Config::default();
        bad.compress.nudge_frequency = 0;
        let err = p.update_config(bad).unwrap_err();
        assert!(matches!(err, Error::Config(_)));
    }

    #[test]
    fn fold_subagent_disabled_by_default() {
        let mut p = pruner();
        let err = p.fold_subagent(vec![]).unwrap_err();
        assert!(matches!(err, Error::SubagentsDisabled));
    }

    #[test]
    fn fold_subagent_concatenates_assistant_text() {
        let mut cfg = Config::default();
        cfg.experimental.allow_subagents = true;
        cfg.rebuild_cache().unwrap();
        let mut p = ContextPruner::new(cfg).unwrap();
        let folded = p
            .fold_subagent(vec![
                Message::user_text("u1", 0, "ignored"),
                Message::assistant_text("a1", 0, "first finding"),
                Message::assistant_text("a2", 0, "second finding"),
            ])
            .unwrap();
        match &folded.parts[0] {
            Part::Text(t) => {
                assert!(t.contains("first finding"));
                assert!(t.contains("second finding"));
                assert!(t.contains("<dcp-subagent-result>"));
            }
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn decompress_round_trip() {
        let mut p = pruner();
        // Install a fake block.
        use dcp_types::{BlockId, CompressionBlock, CompressionMode, RunId};
        let block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "summary",
            "m0001",
            "m0002",
            "raw1",
            "raw2",
        );
        let bid = block.block_id;
        p.state.prune.messages.blocks_by_id.insert(bid, block);
        p.state.prune.messages.active_block_ids.insert(bid);
        p.state
            .prune
            .messages
            .active_by_anchor_message_id
            .insert("raw1".into(), bid);

        let result = p.decompress(bid).unwrap();
        assert_eq!(result.block_id, bid);
        assert!(!p.state().prune.messages.active_block_ids.contains(&bid));

        let result = p.recompress(bid).unwrap();
        assert_eq!(result.block_id, bid);
        assert!(p.state().prune.messages.active_block_ids.contains(&bid));
    }

    #[test]
    fn decompress_unknown_returns_error() {
        let mut p = pruner();
        let err = p.decompress(BlockId::new(99)).unwrap_err();
        assert!(matches!(err, Error::BlockNotFound(_)));
    }

    #[test]
    fn handle_command_context_returns_snapshot() {
        let mut p = pruner();
        let outcome = p.handle_command("context", &[], &[]);
        match outcome {
            crate::commands::CommandOutcome::Context { current_turn, .. } => {
                assert_eq!(current_turn, 0);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn handle_command_unknown() {
        let mut p = pruner();
        let outcome = p.handle_command("nope", &[], &[]);
        assert!(matches!(
            outcome,
            crate::commands::CommandOutcome::Unknown { .. }
        ));
    }

    #[test]
    fn handle_command_manual_toggle() {
        let mut p = pruner();
        let outcome = p.handle_command("manual", &["on"], &[]);
        assert!(matches!(
            outcome,
            crate::commands::CommandOutcome::Manual { enabled: true }
        ));
        assert!(p.state().manual_mode.enabled);
    }

    #[test]
    fn save_with_no_session_id_is_noop() {
        let p = pruner();
        // No transform has happened yet → session_id is None.
        p.save().unwrap();
    }

    // Ensure the public type is Send + Sync — required for the async
    // facade to wrap it via spawn_blocking.
    #[test]
    fn context_pruner_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ContextPruner>();
    }

    #[test]
    fn has_pending_work_false_on_fresh() {
        let pruner = ContextPruner::new(Config::default()).unwrap();
        assert!(!pruner.has_pending_work());
    }

    #[test]
    fn transform_with_diff_no_changes() {
        let mut pruner = ContextPruner::new(Config::default()).unwrap();
        let msgs = vec![
            Message::user_text("hello", 0, "m0001"),
            Message::assistant_text("hi", 0, "m0002"),
        ];
        let result = pruner.transform_messages_with_diff(msgs).unwrap();
        assert!(!result.changed);
        assert!(result.removed_message_ids.is_empty());
        assert_eq!(result.messages.len(), 2);
    }

    #[test]
    fn count_messages_tokens_empty() {
        let pruner = ContextPruner::new(Config::default()).unwrap();
        let tokens = pruner.count_messages_tokens(&[]);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn count_messages_tokens_simple() {
        let pruner = ContextPruner::new(Config::default()).unwrap();
        let msgs = vec![Message::user_text("hello world", 0, "m0001")];
        let tokens = pruner.count_messages_tokens(&msgs);
        assert!(tokens > 0);
    }

    #[test]
    fn last_nudge_kind_none_on_fresh() {
        let pruner = ContextPruner::new(Config::default()).unwrap();
        assert!(pruner.last_nudge_kind().is_none());
    }
}
