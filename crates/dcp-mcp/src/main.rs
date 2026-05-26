//! `dcp-mcp` — MCP server exposing dynamic_context_pruning via stdio transport.
//!
//! Exposed tools: dcp_transform, dcp_compress_range, dcp_stats
//! Exposed resources: dcp://config

use std::sync::Arc;

use dcp_compress::{CompressArgs, RangeEntry};
use dcp_config::Config;
use dcp_core::ContextPruner;
use dcp_types::Message;
use rmcp::model::{
    Content, Implementation, InitializeResult, ListResourcesResult, ListToolsResult,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo,
    Tool,
};
use rmcp::service::{
    NotificationContext, RequestContext, RoleServer, Service,
};
use rmcp::transport::stdio as make_stdio_transport;
use serde_json::Value as JsonValue;

// ─── IR conversion ─────────────────────────────────────────────────────────

fn lift_messages(value: &JsonValue) -> Vec<Message> {
    let arr = match value {
        JsonValue::Array(a) => a,
        _ => return Vec::new(),
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
                    arr.iter().filter_map(|p| {
                        let p_obj = p.as_object()?;
                        let t = p_obj.get("type")?.as_str()?;
                        match t {
                            "text" => Some(dcp_types::Part::Text(
                                p_obj.get("text")?.as_str()?.to_string(),
                            )),
                            "reasoning" => Some(dcp_types::Part::Reasoning(
                                p_obj.get("text")
                                    .or_else(|| p_obj.get("reasoning"))?
                                    .as_str()?
                                    .to_string(),
                            )),
                            "tool_call" | "tool" => {
                                let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                                let tool = p_obj.get("tool")?.as_str()?.to_string();
                                let input = p_obj
                                    .get("input")
                                    .or_else(|| p_obj.get("state"))
                                    .cloned()
                                    .unwrap_or(JsonValue::Null);
                                Some(dcp_types::Part::ToolCall { call_id, tool, input })
                            }
                            "tool_result" => {
                                let call_id = p_obj.get("call_id")?.as_str()?.to_string();
                                let status = p_obj
                                    .get("status")
                                    .or_else(|| p_obj.get("state").and_then(|s| s.get("status")))
                                    .and_then(|v| v.as_str())
                                    .map(|s| match s {
                                        "completed" => dcp_types::ToolStatus::Completed,
                                        "error" => dcp_types::ToolStatus::Error,
                                        _ => dcp_types::ToolStatus::Pending,
                                    })
                                    .unwrap_or(dcp_types::ToolStatus::Pending);
                                let output = p_obj
                                    .get("output")
                                    .or_else(|| p_obj.get("state").and_then(|s| s.get("output")))
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                                let error = p_obj
                                    .get("error")
                                    .or_else(|| p_obj.get("state").and_then(|s| s.get("error")))
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                                Some(dcp_types::Part::ToolResult { call_id, status, output, error })
                            }
                            _ => None,
                        }
                    }).collect()
                })
                .unwrap_or_default();
            Some(dcp_types::Message::new(id, role, parts, time))
        })
        .collect()
}

fn lower_message(msg: &Message) -> JsonValue {
    serde_json::json!({
        "id": msg.id,
        "role": match msg.role {
            dcp_types::Role::User => "user",
            dcp_types::Role::Assistant => "assistant",
            _ => "",
        },
        "parts": msg.parts.iter().map(|p| match p {
            dcp_types::Part::Text(t) => serde_json::json!({ "type": "text", "text": t }),
            dcp_types::Part::Reasoning(r) => serde_json::json!({ "type": "reasoning", "reasoning": r }),
            dcp_types::Part::ToolCall { call_id, tool, input } => serde_json::json!({
                "type": "tool_call", "call_id": call_id, "tool": tool, "input": input,
            }),
            dcp_types::Part::ToolResult { call_id, status, output, error } => {
                let mut obj = serde_json::json!({
                    "type": "tool_result",
                    "call_id": call_id,
                    "status": match status {
                        dcp_types::ToolStatus::Pending => "pending",
                        dcp_types::ToolStatus::Completed => "completed",
                        dcp_types::ToolStatus::Error => "error",
                        _ => "unknown",
                    },
                });
                if let Some(o) = output {
                    obj["output"] = JsonValue::String(o.clone());
                }
                if let Some(e) = error {
                    obj["error"] = JsonValue::String(e.clone());
                }
                obj
            }
            _ => serde_json::json!({}),
        }).collect::<Vec<_>>(),
        "time": msg.time,
    })
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

// ─── Server ────────────────────────────────────────────────────────────────

pub struct DcpMcpServer {
    config: Arc<Config>,
}

impl DcpMcpServer {
    pub fn new() -> anyhow::Result<Self> {
        let config = Config::load_default().unwrap_or_else(|_| Config::default());
        Ok(Self { config: Arc::new(config) })
    }

    fn run_transform(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let messages = match args_json.get("messages") {
            Some(v) => lift_messages(v),
            None => return rmcp::model::CallToolResult::success(vec![]),
        };
        match ContextPruner::new(self.config.as_ref().clone()) {
            Ok(mut pruner) => {
                match pruner.transform_messages(messages) {
                    Ok(pruned) => {
                        let output: Vec<JsonValue> = pruned.iter().map(lower_message).collect();
                        let json_out = serde_json::to_string_pretty(&output).unwrap_or_default();
                        rmcp::model::CallToolResult::success(vec![Content::text(json_out)])
                    }
                    Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!("transform error: {:?}", e))]),
                }
            }
            Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!("init error: {:?}", e))]),
        }
    }

    fn run_compress(&self, args_json: &JsonValue) -> rmcp::model::CallToolResult {
        let messages = match args_json.get("messages") {
            Some(v) => lift_messages(v),
            None => return rmcp::model::CallToolResult::success(vec![]),
        };
        let topic = args_json
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();
        let ranges: Vec<RangeEntry> = args_json
            .get("ranges")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|r| {
                    let obj = r.as_object()?;
                    Some(RangeEntry {
                        start_id: obj.get("start_id")?.as_str()?.to_string(),
                        end_id: obj.get("end_id")?.as_str()?.to_string(),
                        summary: obj.get("summary").and_then(|v| v.as_str()).map(String::from).unwrap_or_default(),
                    })
                }).collect()
            })
            .unwrap_or_default();
        let args = CompressArgs::Range { topic, content: ranges };
        match ContextPruner::new(self.config.as_ref().clone()) {
            Ok(mut pruner) => {
                match pruner.handle_compress(args, &messages) {
                    Ok(result) => {
                        let json_out = serde_json::to_string_pretty(&result).unwrap_or_default();
                        rmcp::model::CallToolResult::success(vec![Content::text(json_out)])
                    }
                    Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!("compress error: {:?}", e))]),
                }
            }
            Err(e) => rmcp::model::CallToolResult::error(vec![Content::text(format!("init error: {:?}", e))]),
        }
    }

    fn run_stats(&self) -> rmcp::model::CallToolResult {
        let stats = serde_json::json!({
            "enabled": self.config.enabled,
            "debug": self.config.debug,
            "cache_stability_mode": format!("{:?}", self.config.cache_stability_mode),
        });
        rmcp::model::CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stats).unwrap_or_default(),
        )])
    }
}

impl Service<RoleServer> for DcpMcpServer {
    fn handle_request(
        &self,
        request: rmcp::model::ClientRequest,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<
        Output = std::result::Result<rmcp::model::ServerResult, rmcp::ErrorData>,
    > + Send + '_ {
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
                                .with_description("MCP server exposing dynamic context pruning transform"),
                        )
                        .with_instructions(
                            "Use dcp_transform to compress message history. \
                             Use dcp_stats to inspect the current configuration.",
                        );
                    Ok(rmcp::model::ServerResult::InitializeResult(result))
                }
                rmcp::model::ClientRequest::ListToolsRequest(_req) => {
                    let tools = vec![
                        Tool::new(
                            "dcp_transform",
                            "Transform message history via DCP",
                            make_input_schema(&[("messages", "array")]),
                        ),
                        Tool::new(
                            "dcp_compress_range",
                            "Compress message ranges via DCP",
                            make_input_schema(&[
                                ("messages", "array"),
                                ("topic", "string"),
                                ("ranges", "array"),
                            ]),
                        ),
                        Tool::new(
                            "dcp_stats",
                            "Return DCP session statistics",
                            make_input_schema(&[]),
                        ),
                    ];
                    Ok(rmcp::model::ServerResult::ListToolsResult(
                        ListToolsResult::with_all_items(tools),
                    ))
                }
                rmcp::model::ClientRequest::CallToolRequest(req) => {
                    // req.params.arguments is a JsonObject (serde_json::Map<String, Value>)
                    let args_json: JsonValue = req.params.arguments.unwrap_or_else(|| rmcp::model::JsonObject::new().into()).into();
                    let result = match req.params.name.as_ref() {
                        "dcp_transform" => self.run_transform(&args_json),
                        "dcp_compress_range" => self.run_compress(&args_json),
                        "dcp_stats" => self.run_stats(),
                        _ => rmcp::model::CallToolResult::error(vec![Content::text(format!("unknown tool: {}", req.params.name))]),
                    };
                    Ok(rmcp::model::ServerResult::CallToolResult(result))
                }
                rmcp::model::ClientRequest::ListResourcesRequest(_req) => {
                    // List resources that dcp-mcp exposes
                    use rmcp::model::{RawResource, Resource};
                    let raw_res = RawResource::new(
                        "dcp://config",
                        "DCP Configuration",
                    )
                    .with_description("Current effective DCP configuration as JSON")
                    .with_mime_type("application/json");
                    let res = Resource::new(raw_res, None);
                    Ok(rmcp::model::ServerResult::ListResourcesResult(
                        ListResourcesResult::with_all_items(vec![res]),
                    ))
                }
                rmcp::model::ClientRequest::ReadResourceRequest(req) => {
                    let text = match req.params.uri.as_str() {
                        "dcp://config" => serde_json::to_string_pretty(&self.config).unwrap_or_default(),
                        _ => format!("unknown resource: {}", req.params.uri),
                    };
                    let contents = vec![ResourceContents::text(text, req.params.uri)];
                    Ok(rmcp::model::ServerResult::ReadResourceResult(
                        ReadResourceResult::new(contents),
                    ))
                }
                _ => Err(rmcp::ErrorData::method_not_found::<rmcp::model::CallToolRequestMethod>()),
            }
        }
    }

    fn handle_notification(
        &self,
        _notification: <RoleServer as rmcp::service::ServiceRole>::PeerNot,
        _context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<(), rmcp::ErrorData>> + Send + '_ {
        async move { Ok(()) }
    }

    fn get_info(&self) -> ServerInfo {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        InitializeResult::new(caps)
            .with_server_info(
                Implementation::new("dcp-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Dynamic Context Pruning MCP Server")
                    .with_description("MCP server exposing dynamic context pruning transform"),
            )
    }
}

// ─── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = DcpMcpServer::new()?;
    let (stdin, stdout) = make_stdio_transport();
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
    Ok(())
}
