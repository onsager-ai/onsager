//! The agent-facing MCP server: `POST /mcp/messages`, JSON-RPC 2.0.
//!
//! Protocol skeleton ported from `legacy/crates/onsager-portal/src/mcp/`
//! cut to the load-bearing surface (decision on #620): the principal is
//! a **session token** (minted per agent session by the scheduler, never
//! a user cookie), and the tool set is the output channel only —
//! `emit_artifact` and `declare_no_output`. Everything else the legacy
//! server offered (diagnostics, workflow CRUD, chat) did not port.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::digest;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::Provenance;
use crate::events::record;
use crate::state::AppState;

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_err(id: Option<Value>, code: i32, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    })
}

/// The session behind a bearer token — the only MCP principal.
#[derive(Debug, sqlx::FromRow)]
struct SessionPrincipal {
    id: String,
    workspace_id: String,
    run_id: String,
}

pub async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        return Json(rpc_err(req.id, -32001, "missing bearer token")).into_response();
    };
    let principal: Option<SessionPrincipal> =
        match sqlx::query_as("SELECT id, workspace_id, run_id FROM sessions WHERE token = $1")
            .bind(token)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("mcp session lookup failed: {e}");
                return Json(rpc_err(req.id, -32603, "internal error")).into_response();
            }
        };
    let Some(session) = principal else {
        return Json(rpc_err(req.id, -32001, "invalid session token")).into_response();
    };

    let id = req.id.clone();
    let result = match req.method.as_str() {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "serverInfo": { "name": "onsager", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": { "listChanged": false } }
        })),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => Ok(tools_list()),
        "tools/call" => tools_call(&state, &session, req.params).await,
        other => Err(format!("unknown method: {other}")),
    };
    match result {
        Ok(value) => Json(rpc_ok(id, value)).into_response(),
        Err(msg) => Json(rpc_err(id, -32602, &msg)).into_response(),
    }
}

fn tools_list() -> Value {
    serde_json::json!({ "tools": [
        {
            "name": "emit_artifact",
            "description": "Record a deliverable this session produced. Persists the \
                content (inline, sha256-checksummed) as an artifact row attributed to \
                this session's run. Call once per deliverable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "artifact kind, e.g. document" },
                    "summary": { "type": "string", "description": "one-line name for the artifact" },
                    "content": { "type": "string", "description": "the deliverable's content" }
                },
                "required": ["kind", "summary", "content"]
            }
        },
        {
            "name": "declare_no_output",
            "description": "Declare that this session intentionally produced no \
                deliverable, with a reason. Distinguishes 'delivered nothing on \
                purpose' from 'delivered nothing' (which fails the run).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": { "type": "string" }
                },
                "required": ["reason"]
            }
        }
    ] })
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn tools_call(
    state: &AppState,
    session: &SessionPrincipal,
    params: Value,
) -> Result<Value, String> {
    let parsed: ToolsCallParams =
        serde_json::from_value(params).map_err(|e| format!("invalid tools/call params: {e}"))?;
    let outcome = match parsed.name.as_str() {
        "emit_artifact" => emit_artifact(state, session, parsed.arguments).await,
        "declare_no_output" => declare_no_output(state, session, parsed.arguments).await,
        other => return Err(format!("unknown tool: {other}")),
    };
    // Domain failures ride back inside the tools/call result with
    // isError so the agent can read them; only bad params bubble to
    // the JSON-RPC error envelope (ported convention).
    Ok(match outcome {
        Ok(value) => serde_json::json!({
            "content": [ { "type": "text", "text": value.to_string() } ],
            "structuredContent": value,
            "isError": false,
        }),
        Err(msg) => serde_json::json!({
            "content": [ { "type": "text", "text": msg } ],
            "isError": true,
        }),
    })
}

#[derive(Debug, Deserialize)]
struct EmitArtifactArgs {
    kind: String,
    summary: String,
    content: String,
}

async fn emit_artifact(
    state: &AppState,
    session: &SessionPrincipal,
    args: Value,
) -> Result<Value, String> {
    let args: EmitArtifactArgs =
        serde_json::from_value(args).map_err(|e| format!("invalid emit_artifact args: {e}"))?;
    let artifact_id = format!("art_{}", Uuid::new_v4());
    // Inline content storage (first cut, ported convention): base64
    // data URI + sha256 checksum.
    let content_uri = format!("inline:base64,{}", BASE64.encode(args.content.as_bytes()));
    let checksum = hex::encode(digest::digest(&digest::SHA256, args.content.as_bytes()));
    let provenance = serde_json::to_value(Provenance::Uncertain {
        source: "agent".into(),
    })
    .expect("provenance serializes");

    sqlx::query(
        "INSERT INTO artifacts \
            (id, workspace_id, run_id, session_id, kind, name, content_uri, \
             content_checksum, provenance) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&artifact_id)
    .bind(&session.workspace_id)
    .bind(&session.run_id)
    .bind(&session.id)
    .bind(&args.kind)
    .bind(&args.summary)
    .bind(&content_uri)
    .bind(&checksum)
    .bind(&provenance)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("emit_artifact insert failed: {e}");
        "failed to record artifact".to_string()
    })?;
    record(
        &state.pool,
        &session.workspace_id,
        Some(&session.run_id),
        "artifact.emitted",
        serde_json::json!({
            "artifact_id": artifact_id,
            "session_id": session.id,
            "kind": args.kind,
            "name": args.summary,
        }),
    )
    .await;
    Ok(serde_json::json!({
        "artifact_id": artifact_id,
        "checksum": checksum,
    }))
}

#[derive(Debug, Deserialize)]
struct DeclareNoOutputArgs {
    reason: String,
}

async fn declare_no_output(
    state: &AppState,
    session: &SessionPrincipal,
    args: Value,
) -> Result<Value, String> {
    let args: DeclareNoOutputArgs =
        serde_json::from_value(args).map_err(|e| format!("invalid declare_no_output args: {e}"))?;
    sqlx::query("UPDATE sessions SET declared_no_output_reason = $2 WHERE id = $1")
        .bind(&session.id)
        .bind(&args.reason)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("declare_no_output update failed: {e}");
            "failed to record declaration".to_string()
        })?;
    record(
        &state.pool,
        &session.workspace_id,
        Some(&session.run_id),
        "session.declared_no_output",
        serde_json::json!({ "session_id": session.id, "reason": args.reason }),
    )
    .await;
    Ok(serde_json::json!({ "ok": true }))
}
