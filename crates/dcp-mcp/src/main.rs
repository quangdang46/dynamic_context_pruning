//! `dcp-mcp` - MCP server exposing dynamic_context_pruning via the Model Context Protocol.
//!
//! Exposed tools:
//! - `compress` - compress ranges or messages
//! - `decompress` - deactivate a committed block
//! - `recompress` - re-activate a user-deactivated block
//! - `dcp_context` - show session breakdown
//! - `dcp_stats` - cumulative statistics
//! - `dcp_sweep` - manual sweep trigger
//!
//! Resources:
//! - `dcp://session/{id}/state` - session state as JSON
//! - `dcp://session/{id}/blocks` - active compression blocks
//!
//! Transport:
//! - stdio (default)
//! - HTTP (optional via `--transport http --port`)

use std::sync::Arc;

use anyhow::Result;
use dcp_compress::{CompressArgs, MessageEntry, RangeEntry};
use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_notification::format::{format_stats_header, format_token_count};
use dcp_notification::build_compress_visual_output;
use dcp_types::BlockId;
use rmcp::model::{
    Content, Implementation, InitializeResult, ListResourcesResult, ListToolsResult,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer, Service};
use rmcp::transport::stdio as make_stdio_transport;
use serde_json::Value as JsonValue;

#[derive(Debug)]
struct Cli {
    transport: String,
    port: Option<u16>,
}

impl Cli {
    fn parse() -> Self {
        let mut transport = "stdio".to_string();
        let mut port = None;

        let args: Vec<String> = std::env::args().collect::<Vec<_>>();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--transport" => {
                    if i + 1 < args.len() {
                        transport = args[i + 1].clone();
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--port" => {
                    if i + 1 < args.len() {
                        port = args[i + 1].parse().ok();
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }

        Self { transport, port }
    }
}

fn make_input_schema(properties: &[(&str, &str)]) -> Arc<rmcp::model::JsonObject> {
    let mut obj = serde_json::Map::new();
    obj.insert("type".into(), JsonValue::String("object".into()));
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for (k, t) in properties {
        let mut p = serde_json::Map::new();
        p.insert("type".into(), JsonValue::String((*t).into()));
        props.insert((*k).into(), JsonValue::Object(p));
        required.push(JsonValue::String((*k).into()));
    }
    obj.insert("properties".into(), JsonValue::Object(props));
    obj.insert("required".into(), JsonValue::Array(required));
    Arc::new(obj)
}

pub struct DcpMcpServer {
    inner: Arc<std::sync::Mutex<DcpMcpServerInner>>,
}

pub struct DcpMcpServerInner {
    pruner: ContextPruner,
    session_id: String,
}

impl DcpMcpServer {
    pub fn new() -> Result<Self> {
        let config = Config::load_default().unwrap_or_else(|_| Config::default());
        let pruner = ContextPruner::new(config)?;
        let session_id = "default".to_string();
        Ok(Self {
            inner: Arc::new(std::sync::Mutex::new(DcpMcpServerInner {
                pruner,
                session_id,
            })),
        })
    }

    fn run_compress(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let messages = match args_json.get("messages") {
            Some(v) => {
                let arr = match v {
                    JsonValue::Array(a) => a,
                    _ => {
                        return rmcp::model::CallToolResult::error(vec![Content::text(
                            "messages must be an array".to_string(),
                        )]);
                    }
                };
                arr.iter()
                    .filter_map(|v| {
                        let obj = v.as_object()?;
                        let id = obj.get("id")?.as_str()?.to_string();
                        let role = match obj.get("role")?.as_str()? {
                            "user" => dcp_types::Role::User,
                            "assistant" => dcp_types::Role::Assistant,
                            _ => return None,
                        };
                        let time = obj.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
                        let parts: Vec<dcp_types::Part> = obj
                            .get("parts")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|p| {
                                        let p_obj = p.as_object()?;
                                        let t = p_obj.get("type")?.as_str()?;
                                        match t {
                                            "text" => Some(dcp_types::Part::Text(
                                                p_obj.get("text")?.as_str()?.to_string(),
                                            )),
                                            "reasoning" => Some(dcp_types::Part::Reasoning(
                                                p_obj
                                                    .get("text")
                                                    .or_else(|| p_obj.get("reasoning"))?
                                                    .as_str()?
                                                    .to_string(),
                                            )),
                                            "tool_call" | "tool" => {
                                                let call_id =
                                                    p_obj.get("call_id")?.as_str()?.to_string();
                                                let tool = p_obj.get("tool")?.as_str()?.to_string();
                                                let input = p_obj
                                                    .get("input")
                                                    .or_else(|| p_obj.get("state"))
                                                    .cloned()
                                                    .unwrap_or(JsonValue::Null);
                                                Some(dcp_types::Part::ToolCall {
                                                    call_id,
                                                    tool,
                                                    input,
                                                })
                                            }
                                            "tool_result" => {
                                                let call_id =
                                                    p_obj.get("call_id")?.as_str()?.to_string();
                                                let status = p_obj
                                                    .get("status")
                                                    .or_else(|| {
                                                        p_obj
                                                            .get("state")
                                                            .and_then(|s| s.get("status"))
                                                    })
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| match s {
                                                        "completed" => {
                                                            dcp_types::ToolStatus::Completed
                                                        }
                                                        "error" => dcp_types::ToolStatus::Error,
                                                        _ => dcp_types::ToolStatus::Pending,
                                                    })
                                                    .unwrap_or(dcp_types::ToolStatus::Pending);
                                                let output = p_obj
                                                    .get("output")
                                                    .or_else(|| {
                                                        p_obj
                                                            .get("state")
                                                            .and_then(|s| s.get("output"))
                                                    })
                                                    .and_then(|v| v.as_str())
                                                    .map(String::from);
                                                let error = p_obj
                                                    .get("error")
                                                    .or_else(|| {
                                                        p_obj
                                                            .get("state")
                                                            .and_then(|s| s.get("error"))
                                                    })
                                                    .and_then(|v| v.as_str())
                                                    .map(String::from);
                                                Some(dcp_types::Part::ToolResult {
                                                    call_id,
                                                    status,
                                                    output,
                                                    error,
                                                })
                                            }
                                            _ => None,
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        Some(dcp_types::Message::new(id, role, parts, time))
                    })
                    .collect::<Vec<_>>()
            }
            None => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "missing required field: messages".to_string(),
                )]);
            }
        };

        let topic = args_json
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let mode = args_json
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("range")
            .to_string();

        let args = if mode == "message" {
            let entries: Vec<MessageEntry> = args_json
                .get("content")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|r| {
                            let obj = r.as_object()?;
                            Some(MessageEntry {
                                message_id: obj.get("messageId")?.as_str()?.to_string(),
                                topic: obj.get("topic")?.as_str()?.to_string(),
                                summary: obj.get("summary")?.as_str()?.to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            CompressArgs::Message {
                topic,
                content: entries,
            }
        } else {
            let ranges: Vec<RangeEntry> = args_json
                .get("content")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|r| {
                            let obj = r.as_object()?;
                            Some(RangeEntry {
                                start_id: obj.get("startId")?.as_str()?.to_string(),
                                end_id: obj.get("endId")?.as_str()?.to_string(),
                                summary: obj
                                    .get("summary")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                                    .unwrap_or_default(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            CompressArgs::Range {
                topic,
                content: ranges,
            }
        };

        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        // Extract session message IDs from the input messages
        let session_message_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();

        match inner.pruner.handle_compress(args, &messages) {
            Ok(result) => {
                let state = inner.pruner.state();
                let visual = build_compress_visual_output(state, &result.blocks, &session_message_ids);
                rmcp::model::CallToolResult::success(vec![Content::text(visual)])
            }
            Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!(
                "compress error: {:?}",
                e
            ))]),
        }
    }

    fn run_decompress(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let block_id = match args_json.get("blockId") {
            Some(v) => {
                let s = v
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("b")
                    .map(|s| &s[1..])
                    .unwrap_or_else(|| v.as_str().unwrap_or(""));
                match s.parse::<u32>() {
                    Ok(id) => BlockId::new(id),
                    Err(_) => {
                        return rmcp::model::CallToolResult::error(vec![Content::text(
                            "invalid blockId format".to_string(),
                        )]);
                    }
                }
            }
            None => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "missing required field: blockId".to_string(),
                )]);
            }
        };

        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        match inner.pruner.decompress(block_id) {
            Ok(result) => {
                let state = inner.pruner.state();
                let header = format_stats_header(state.stats.total_prune_tokens, 0);
                let msg = format!(
                    "{}\n  ✓ Block {} restored — anchor: {}",
                    header,
                    result.block_id,
                    result.anchor_message_id
                );
                rmcp::model::CallToolResult::success(vec![Content::text(msg)])
            }
            Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!(
                "decompress error: {:?}",
                e
            ))]),
        }
    }

    fn run_recompress(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let block_id = match args_json.get("blockId") {
            Some(v) => {
                let s = v
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("b")
                    .map(|s| &s[1..])
                    .unwrap_or_else(|| v.as_str().unwrap_or(""));
                match s.parse::<u32>() {
                    Ok(id) => BlockId::new(id),
                    Err(_) => {
                        return rmcp::model::CallToolResult::error(vec![Content::text(
                            "invalid blockId format".to_string(),
                        )]);
                    }
                }
            }
            None => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "missing required field: blockId".to_string(),
                )]);
            }
        };

        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        match inner.pruner.recompress(block_id) {
            Ok(result) => {
                let state = inner.pruner.state();
                let header = format_stats_header(state.stats.total_prune_tokens, 0);
                let msg = format!(
                    "{}\n  ↻ Block {} re-compressed — anchor: {}",
                    header,
                    result.block_id,
                    result.anchor_message_id
                );
                rmcp::model::CallToolResult::success(vec![Content::text(msg)])
            }
            Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!(
                "recompress error: {:?}",
                e
            ))]),
        }
    }

    fn run_manual_toggle(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let mode = args_json
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("toggle");

        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        match mode {
            "on" => {
                inner.pruner.set_manual_mode(true);
            }
            "off" => {
                inner.pruner.set_manual_mode(false);
            }
            "toggle" => {
                let current = inner.pruner.state().manual_mode.enabled;
                inner.pruner.set_manual_mode(!current);
            }
            _ => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "invalid mode: use on/off/toggle".to_string(),
                )]);
            }
        }

        let state = inner.pruner.state().manual_mode.enabled;
        let msg = format!(
            "Manual mode: {}",
            if state { "ACTIVE" } else { "INACTIVE" }
        );
        rmcp::model::CallToolResult::success(vec![Content::text(msg)])
    }

    fn run_dcp_context(&self) -> rmcp::model::CallToolResult {
        let inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        let state = inner.pruner.state();
        let stats = inner.pruner.stats();

        let total_tokens_saved = stats.total_prune_tokens as u64
            + stats.dedup_pruned as u64
            + stats.purge_errors_pruned as u64
            + stats.stale_file_reads_pruned as u64;

        let header = format_stats_header(total_tokens_saved, stats.total_prune_tokens as u64);
        let divider = "─".repeat(header.len().saturating_sub(4).max(32));

        let context_text = format!(
            "\
{header}
{divider}
Session: {}
Turn: {}
Messages: {} total | {} blocks | {} active
Stats:
  ├─ prune tokens: {}
  ├─ dedup pruned: {}
  ├─ purge errors: {}
  ├─ stale reads: {}
  ├─ compress runs: {}
  └─ blocks committed: {}",
            inner.session_id,
            state.current_turn,
            state.message_ids.by_raw_id.len(),
            state.prune.messages.blocks_by_id.len(),
            state.prune.messages.active_block_ids.len(),
            format_token_count(stats.total_prune_tokens as u64, true),
            format_token_count(stats.dedup_pruned as u64, true),
            format_token_count(stats.purge_errors_pruned as u64, true),
            format_token_count(stats.stale_file_reads_pruned as u64, true),
            stats.compress_runs,
            stats.compress_blocks_committed,
        );

        rmcp::model::CallToolResult::success(vec![Content::text(context_text)])
    }

    fn run_dcp_stats(&self) -> rmcp::model::CallToolResult {
        let inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        let stats = inner.pruner.stats();
        let config = inner.pruner.config();

        let total_saved = stats.total_prune_tokens.saturating_add(stats.compress_oversized as u64).saturating_add(stats.compress_useful as u64);

        let output = format!(
            "{}
─────────────────────────────────
Config:
  ├─ enabled: {}
  └─ debug: {}

Prune Stats:
  ├─ total prune tokens: {}
  ├─ dedup pruned: {}
  ├─ purge errors: {}
  ├─ stale file reads: {}
  └─ orphan tool results: {}

Compression Stats:
  ├─ runs: {}
  ├─ blocks committed: {}
  ├─ oversized: {}
  ├─ useful: {}
  └─ compactions observed: {}

Errors:
  ├─ cache bust events: {}
  ├─ dropped invalid: {}
  ├─ invalid status transitions: {}
  └─ storage save failed: {}

Path Handling:
  ├─ null byte stripped: {}
  └─ depth clamped: {}",
            format_stats_header(total_saved, 0),
            config.enabled,
            config.debug,
            format_token_count(stats.total_prune_tokens, true),
            format_token_count(stats.dedup_pruned as u64, true),
            format_token_count(stats.purge_errors_pruned as u64, true),
            format_token_count(stats.stale_file_reads_pruned as u64, true),
            format_token_count(stats.orphan_tool_results as u64, true),
            stats.compress_runs,
            stats.compress_blocks_committed,
            format_token_count(stats.compress_oversized as u64, true),
            format_token_count(stats.compress_useful as u64, true),
            stats.compactions_observed,
            stats.cache_bust_events,
            stats.dropped_invalid,
            stats.invalid_status_transitions,
            stats.storage_save_failed,
            stats.path_null_byte_stripped,
            stats.normalize_depth_clamped,
        );

        rmcp::model::CallToolResult::success(vec![Content::text(output)])
    }

    fn run_dcp_sweep(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let _count = args_json
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let mut inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => {
                return rmcp::model::CallToolResult::error(vec![Content::text(
                    "failed to acquire lock".to_string(),
                )]);
            }
        };

        if let Err(e) = inner.pruner.force_apply() {
            return rmcp::model::CallToolResult::error(vec![Content::text(format!(
                "sweep error: {:?}",
                e
            ))]);
        }

        let msg = format!(
            "✓ Sweep triggered\n  {} prune decisions pending.",
            _count
        );
        rmcp::model::CallToolResult::success(vec![Content::text(msg)])
    }
}

impl Service<RoleServer> for DcpMcpServer {
    #[allow(clippy::manual_async_fn)]
    fn handle_request(
        &self,
        request: rmcp::model::ClientRequest,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<
        Output = std::result::Result<rmcp::model::ServerResult, rmcp::ErrorData>,
    > + Send
    + '_ {
        async move {
            match request {
                rmcp::model::ClientRequest::InitializeRequest(_req) => {
                    let caps = ServerCapabilities::builder()
                        .enable_tools()
                        .enable_resources()
                        .build();
                    let result = InitializeResult::new(caps)
                        .with_server_info(
                            Implementation::new("dcp-mcp", env!("CARGO_PKG_VERSION"))
                                .with_title("Dynamic Context Pruning MCP Server")
                                .with_description(
                                    "MCP server exposing dynamic context pruning transform",
                                ),
                        )
                        .with_instructions(
                            "Use compress to compress ranges or messages. \
                             Use decompress/recompress to deactivate/reactivate blocks. \
                             Use dcp_context for session breakdown. \
                             Use dcp_stats for cumulative statistics. \
                             Use dcp_sweep to trigger a manual sweep.",
                        );
                    Ok(rmcp::model::ServerResult::InitializeResult(result))
                }
                rmcp::model::ClientRequest::ListToolsRequest(_req) => {
                    let tools = vec![
                        Tool::new(
                            "compress",
                            "Compress contiguous ranges or individual messages into block summaries",
                            make_input_schema(&[
                                ("messages", "array"),
                                ("topic", "string"),
                                ("mode", "string"),
                                ("content", "array"),
                            ]),
                        ),
                        Tool::new(
                            "decompress",
                            "Deactivate a committed compression block (restore anchor verbatim)",
                            make_input_schema(&[("blockId", "string")]),
                        ),
                        Tool::new(
                            "recompress",
                            "Re-activate a user-deactivated compression block",
                            make_input_schema(&[("blockId", "string")]),
                        ),
                        Tool::new(
                            "dcp_context",
                            "Show session context breakdown (turn, messages, blocks)",
                            make_input_schema(&[]),
                        ),
                        Tool::new(
                            "dcp_stats",
                            "Return cumulative DCP statistics",
                            make_input_schema(&[]),
                        ),
                        Tool::new(
                            "dcp_sweep",
                            "Trigger a manual sweep (apply pending prune decisions)",
                            make_input_schema(&[("count", "number")]),
                        ),
                        Tool::new(
                            "manual_toggle",
                            "Toggle manual compression mode (on/off/toggle)",
                            make_input_schema(&[("mode", "string")]),
                        ),
                    ];
                    Ok(rmcp::model::ServerResult::ListToolsResult(
                        ListToolsResult::with_all_items(tools),
                    ))
                }
                rmcp::model::ClientRequest::CallToolRequest(req) => {
                    let args_json: JsonValue = req
                        .params
                        .arguments
                        .unwrap_or_else(rmcp::model::JsonObject::new)
                        .into();
                    let result = match req.params.name.as_ref() {
                        "compress" => self.run_compress(&args_json),
                        "decompress" => self.run_decompress(&args_json),
                        "recompress" => self.run_recompress(&args_json),
                        "dcp_context" => self.run_dcp_context(),
                        "dcp_stats" => self.run_dcp_stats(),
                        "dcp_sweep" => self.run_dcp_sweep(&args_json),
                        "manual_toggle" => self.run_manual_toggle(&args_json),
                        _ => rmcp::model::CallToolResult::error(vec![Content::text(format!(
                            "unknown tool: {}",
                            req.params.name
                        ))]),
                    };
                    Ok(rmcp::model::ServerResult::CallToolResult(result))
                }
                rmcp::model::ClientRequest::ListResourcesRequest(_req) => {
                    use rmcp::model::{RawResource, Resource};

                    let inner = match self.inner.lock() {
                        Ok(i) => i,
                        Err(_) => {
                            return Err(rmcp::ErrorData::internal_error(
                                "failed to acquire lock",
                                None,
                            ));
                        }
                    };

                    let session_id = &inner.session_id;

                    let state_resource = Resource::new(
                        RawResource::new(
                            format!("dcp://session/{}/state", session_id),
                            "DCP Session State",
                        )
                        .with_description("Current DCP session state as JSON")
                        .with_mime_type("application/json"),
                        None,
                    );

                    let blocks_resource = Resource::new(
                        RawResource::new(
                            format!("dcp://session/{}/blocks", session_id),
                            "DCP Compression Blocks",
                        )
                        .with_description("Active compression blocks in the session")
                        .with_mime_type("application/json"),
                        None,
                    );

                    Ok(rmcp::model::ServerResult::ListResourcesResult(
                        ListResourcesResult::with_all_items(vec![state_resource, blocks_resource]),
                    ))
                }
                rmcp::model::ClientRequest::ReadResourceRequest(req) => {
                    let inner = match self.inner.lock() {
                        Ok(i) => i,
                        Err(_) => {
                            return Err(rmcp::ErrorData::internal_error(
                                "failed to acquire lock",
                                None,
                            ));
                        }
                    };

                    let state = inner.pruner.state();
                    let uri_path = req.params.uri.as_str();
                    let text = if uri_path.contains("/state") {
                        serde_json::to_string_pretty(state).unwrap_or_default()
                    } else if uri_path.contains("/blocks") {
                        let blocks: Vec<&dcp_types::CompressionBlock> = state
                            .prune
                            .messages
                            .blocks_by_id
                            .values()
                            .filter(|b| b.active)
                            .collect::<Vec<_>>();
                        serde_json::to_string_pretty(&blocks).unwrap_or_default()
                    } else {
                        return Err(rmcp::ErrorData::invalid_request(
                            format!("unknown resource: {}", req.params.uri),
                            None,
                        ));
                    };

                    let contents = vec![ResourceContents::text(text, req.params.uri)];
                    Ok(rmcp::model::ServerResult::ReadResourceResult(
                        ReadResourceResult::new(contents),
                    ))
                }
                _ => Err(rmcp::ErrorData::method_not_found::<
                    rmcp::model::CallToolRequestMethod,
                >()),
            }
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn handle_notification(
        &self,
        _notification: <RoleServer as rmcp::service::ServiceRole>::PeerNot,
        _context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<(), rmcp::ErrorData>> + Send + '_
    {
        async move { Ok(()) }
    }

    fn get_info(&self) -> ServerInfo {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        InitializeResult::new(caps).with_server_info(
            Implementation::new("dcp-mcp", env!("CARGO_PKG_VERSION"))
                .with_title("Dynamic Context Pruning MCP Server")
                .with_description("MCP server exposing dynamic context pruning transform"),
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let server = DcpMcpServer::new()?;
    let (stdin, stdout) = make_stdio_transport();

    match cli.transport.as_str() {
        "stdio" => {
            let running = rmcp::service::serve_directly(server, (stdin, stdout), None);
            let token = running.cancellation_token();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                token.cancel();
            });
            let result = running.waiting().await;
            if let Err(e) = result {
                if !e.is_cancelled() {
                    return Err(anyhow::anyhow!("server error: {:?}", e));
                }
            }
        }
        "http" => {
            let port = cli.port.unwrap_or(7820);
            let addr = format!("127.0.0.1:{}", port);
            eprintln!("Starting MCP server on http://{}", addr);
            return Err(anyhow::anyhow!("HTTP transport not yet implemented"));
        }
        _ => {
            return Err(anyhow::anyhow!("Unknown transport: {}", cli.transport));
        }
    }

    Ok(())
}
