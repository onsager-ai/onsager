//! MCP (Model Context Protocol) server for portal — clause 1 of the
//! seam rule as a runtime-agnostic public contract for AI clients
//! (ADR 0007).
//!
//! Transport is HTTP `POST /mcp/messages` carrying JSON-RPC 2.0
//! envelopes. Three methods are supported in v1:
//!
//! - `initialize` — handshake; returns server capabilities and the
//!   negotiated protocol version.
//! - `tools/list` — enumerates the registered tools with their
//!   `schemars`-derived JSON Schema inputs.
//! - `tools/call` — invokes a registered tool by name; arguments are
//!   validated and dispatched to a thin wrapper over the corresponding
//!   portal capability (DB helper, spine emit, etc.).
//!
//! Auth reuses portal's `AuthUser` extractor (PAT or session).
//! Workspace-scoped tools call `handlers::workspaces::require_workspace_access`
//! before delegating.
//!
//! Tool implementations live under [`tools`]. The registry below is
//! the single source of truth for what tools exist and which slot
//! `xtask check-tools-and-skills` checks against.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use schemars::schema::RootSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::AuthUser;
use crate::state::AppState;

pub mod registry;
pub mod tools;

pub use registry::{ToolCategory, ToolDescriptor, registry};

/// MCP server's advertised protocol version. The MCP spec is
/// versioned by date string; we pin to a known-stable revision and
/// renegotiate via the `initialize` handshake.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC envelope. `id` is `null` for notifications.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Error a tool implementation may return. Mapped to a JSON-RPC error
/// for protocol errors (bad input, auth) or an MCP `tools/call` result
/// with `isError: true` for tool-domain failures.
#[derive(Debug)]
pub enum ToolError {
    /// Caller-side problem (bad arguments, missing required fields).
    /// Surfaces as JSON-RPC `-32602 Invalid params`.
    InvalidParams(String),
    /// Caller is not authorized for the requested workspace / artifact.
    /// Surfaces as a `tools/call` result with `isError: true`.
    Forbidden(String),
    /// Resource not found (workflow / run / artifact id).
    NotFound(String),
    /// Tool-domain failure (delegated handler errored). Surfaces as a
    /// `tools/call` result with `isError: true`.
    Internal(String),
}

pub type ToolResult = Result<Value, ToolError>;

/// `POST /mcp/messages` — JSON-RPC entry point.
pub async fn handle_messages(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let id = req.id.clone();
    let resp = dispatch(&state, &auth_user, req).await;
    let resp = match resp {
        Ok(value) => JsonRpcResponse::ok(id, value),
        Err(err) => match err {
            ToolError::InvalidParams(msg) => JsonRpcResponse::err(id, -32602, msg),
            ToolError::Forbidden(msg) => JsonRpcResponse::err(id, -32001, msg),
            ToolError::NotFound(msg) => JsonRpcResponse::err(id, -32002, msg),
            ToolError::Internal(msg) => JsonRpcResponse::err(id, -32000, msg),
        },
    };
    (StatusCode::OK, Json(resp)).into_response()
}

async fn dispatch(
    state: &AppState,
    auth_user: &AuthUser,
    req: JsonRpcRequest,
) -> Result<Value, ToolError> {
    match req.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => tools_call(state, auth_user, req.params).await,
        // Notifications (no id) — silently accept.
        "notifications/initialized" => Ok(Value::Null),
        other => Err(ToolError::InvalidParams(format!("unknown method: {other}"))),
    }
}

fn initialize_result() -> Value {
    serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": "onsager-portal",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "tools": { "listChanged": false }
        }
    })
}

fn tools_list_result() -> Value {
    let tools: Vec<Value> = registry()
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": &t.input_schema,
            })
        })
        .collect();
    serde_json::json!({ "tools": tools })
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn tools_call(
    state: &AppState,
    auth_user: &AuthUser,
    params: Value,
) -> Result<Value, ToolError> {
    let parsed: ToolsCallParams = serde_json::from_value(params)
        .map_err(|e| ToolError::InvalidParams(format!("invalid tools/call params: {e}")))?;

    let desc = registry()
        .iter()
        .find(|t| t.name == parsed.name)
        .ok_or_else(|| ToolError::InvalidParams(format!("unknown tool: {}", parsed.name)))?;

    let outcome = (desc.invoke)(state, auth_user, parsed.arguments).await;

    Ok(format_tool_outcome(outcome))
}

/// MCP wraps tool outputs as `{ content: [{type:"text", text}], isError? }`.
/// Domain failures (Forbidden / NotFound / Internal) ride back as text
/// content with `isError: true`; protocol failures (InvalidParams)
/// bubble up to the JSON-RPC error envelope.
fn format_tool_outcome(outcome: ToolResult) -> Value {
    match outcome {
        Ok(value) => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
            serde_json::json!({
                "content": [ { "type": "text", "text": text } ],
                "structuredContent": value,
                "isError": false,
            })
        }
        Err(ToolError::InvalidParams(msg))
        | Err(ToolError::Forbidden(msg))
        | Err(ToolError::NotFound(msg))
        | Err(ToolError::Internal(msg)) => {
            serde_json::json!({
                "content": [ { "type": "text", "text": msg } ],
                "isError": true,
            })
        }
    }
}

/// Convenience wrapper for `schema_for!` output. Tool descriptors carry
/// a `RootSchema` so the registry can hand it back unchanged in
/// `tools/list`.
pub fn input_schema<T: schemars::JsonSchema>() -> RootSchema {
    schemars::schema_for!(T)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_tool_capabilities() {
        let v = initialize_result();
        assert_eq!(v["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(v["capabilities"]["tools"].is_object());
        assert_eq!(v["serverInfo"]["name"], "onsager-portal");
    }

    #[test]
    fn tools_list_includes_every_registry_entry_with_schema() {
        let v = tools_list_result();
        let tools = v["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), registry().len());
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert!(t["inputSchema"].is_object());
        }
    }

    #[test]
    fn registry_includes_full_v1_tool_surface() {
        let names: Vec<&str> = registry().iter().map(|t| t.name).collect();
        for required in [
            "propose_workflow",
            "run_workflow",
            "edit_workflow",
            "schedule_workflow",
            "list_workflows",
            "list_runs",
            "cancel_run",
            "inspect_run",
            "get_stage_logs",
            "get_artifact",
            "propose_remediation",
        ] {
            assert!(
                names.contains(&required),
                "missing tool `{required}` in registry"
            );
        }
    }

    #[test]
    fn mutation_tools_have_hitl_slot_assignments() {
        // Every mutation tool must declare a non-ReadOnly category so
        // the dashboard HitlCard renderer (and `check-hitl-coverage`
        // when it lands) can pick a slot. Read-only tools render as
        // plain info blocks instead of cards.
        for t in registry() {
            let is_mutation = matches!(
                t.name,
                "propose_workflow"
                    | "run_workflow"
                    | "edit_workflow"
                    | "schedule_workflow"
                    | "cancel_run"
            );
            if is_mutation {
                assert!(
                    !matches!(t.category, registry::ToolCategory::ReadOnly),
                    "mutation tool `{}` is registered as ReadOnly — no HitlCard slot",
                    t.name
                );
            }
        }
    }
}
