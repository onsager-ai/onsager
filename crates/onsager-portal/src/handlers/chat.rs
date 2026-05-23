//! Chat endpoints:
//!
//! - `POST /api/chat/completions` — portal-hosted Anthropic relay
//!   (spec #318). The dashboard sends the full Anthropic Messages API
//!   request body plus a `workspace_id` field; portal resolves the
//!   workspace `anthropic` credential, decrypts it, and forwards the
//!   body verbatim. The API key never reaches the browser.
//! - `GET /api/chat/thread` — fetch (or implicitly create) the single
//!   rolling thread for `(user_id, workspace_id, scope_type, scope_id)`
//!   plus its messages (spec #470, ADR 0020).
//! - `POST /api/chat/thread/:id/messages` — append a message to a thread
//!   (spec #470).
//! - `GET /api/chat/search` — full-text search across the caller's
//!   threads in a workspace, returning matching messages with their
//!   source scope cited (spec #470).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use ts_rs::TS;

use crate::auth::{AuthUser, decrypt_credential};
use crate::chat_db::{self, ChatMessage, ChatSearchHit, ChatThread};
use crate::credential_db;
use crate::runtime::{AnthropicRelay, ChatRuntime, ChatRuntimeError};
use crate::state::AppState;

async fn require_workspace_access(
    pool: &PgPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    if let Some(pinned) = auth_user.principal.pinned_workspace_id()
        && pinned != workspace_id
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "pat_workspace_scope_mismatch",
                "detail": "PAT is pinned to a different workspace",
            })),
        )
            .into_response());
    }
    match credential_db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "workspace not found" })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("failed to check workspace membership: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Request body for the chat relay. `workspace_id` names the workspace
/// whose `anthropic` credential is used; everything else is forwarded
/// verbatim to `POST https://api.anthropic.com/v1/messages`.
#[derive(Deserialize)]
pub struct ChatRelayBody {
    pub workspace_id: String,
    #[serde(flatten)]
    pub request: serde_json::Value,
}

/// `POST /api/chat/completions` — resolve the workspace `anthropic`
/// credential and forward the Anthropic Messages request.
pub async fn create_chat_completion(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<ChatRelayBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &body.workspace_id).await {
        return r;
    }

    let Some(ref key) = state.config.credential_key else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "credential storage not configured (set ONSAGER_CREDENTIAL_KEY)"
            })),
        )
            .into_response();
    };

    let encrypted = match credential_db::get_user_credential_encrypted(
        &state.pool,
        &body.workspace_id,
        &auth_user.user_id,
        "anthropic",
    )
    .await
    {
        Ok(Some(enc)) => enc,
        Ok(None) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "anthropic_credential_missing",
                    "detail": "Set an `anthropic` credential in workspace Settings → Credentials",
                })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("failed to fetch anthropic credential: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let api_key = match decrypt_credential(key, &encrypted) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("failed to decrypt anthropic credential: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let runtime: Box<dyn ChatRuntime> = match AnthropicRelay::new(api_key) {
        Ok(r) => Box::new(r),
        Err(e) => {
            tracing::error!("failed to build anthropic relay: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Normalize before forwarding:
    // - `stream` must be absent or false — relay always calls `resp.json()`.
    // - `model` is passed through `resolve_model` to reject non-Claude ids
    //   and expand aliases ("sonnet" → full id) consistently.
    let mut req = body.request.clone();
    if let Some(obj) = req.as_object_mut() {
        obj.remove("stream");
        let model_str = obj
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let resolved = crate::anthropic::resolve_model(Some(model_str.as_str())).to_owned();
        obj.insert("model".to_owned(), serde_json::json!(resolved));
    }

    match runtime.chat(&req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(ChatRuntimeError::Upstream { status, body }) => {
            // 4xx = caller error — return the upstream status + body
            // verbatim so the dashboard can surface Anthropic's message.
            // 5xx = provider error — map to 502 Bad Gateway.
            let mapped = if (400..500).contains(&status) {
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST)
            } else {
                StatusCode::BAD_GATEWAY
            };
            (mapped, Json(body)).into_response()
        }
        Err(ChatRuntimeError::Transport(e)) => {
            tracing::warn!("chat runtime transport error: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "anthropic_relay_error",
                    "detail": format!("{e}"),
                })),
            )
                .into_response()
        }
    }
}

// =====================================================================
// Server-side chat storage (spec #470, ADR 0020).
// =====================================================================

/// `scope_type` values accepted by the thread endpoints. Mirrors the
/// CHECK constraint on `chat_threads.scope_type`.
const VALID_SCOPE_TYPES: &[&str] = &["workspace", "workflow", "run", "verdict"];

/// Query string for `GET /api/chat/thread`.
#[derive(Debug, Deserialize)]
pub struct ThreadQuery {
    /// Workspace the request is scoped to. Required.
    pub workspace_id: String,
    /// One of `workspace` / `workflow` / `run` / `verdict`.
    pub scope_type: String,
    /// Required for all `scope_type`s except `workspace`. Empty / missing
    /// for the workspace scope.
    #[serde(default)]
    pub scope_id: Option<String>,
}

/// Response body for `GET /api/chat/thread`.
#[derive(Debug, Serialize, TS)]
#[ts(export, rename = "ChatThreadResponse")]
pub struct ThreadResponse {
    pub thread: ChatThread,
    pub messages: Vec<ChatMessage>,
}

/// Body for `POST /api/chat/thread/:id/messages`. The `tool_call_id` is
/// reserved for assistant tool-call/result messages (per the spec's
/// schema).
#[derive(Debug, Deserialize)]
pub struct AppendMessageBody {
    pub workspace_id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// Query string for `GET /api/chat/search`.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub workspace_id: String,
    pub q: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Response body for `GET /api/chat/search`.
#[derive(Debug, Serialize, TS)]
#[ts(export, rename = "ChatSearchResponse")]
pub struct SearchResponse {
    pub hits: Vec<ChatSearchHit>,
}

fn bad_request(error: &str, detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": error, "detail": detail })),
    )
        .into_response()
}

/// Validate + normalize a `(scope_type, scope_id)` pair. Returns the
/// normalized values on success or an error tag the handler can map to
/// a 400 response. Kept as a tag-returning function (not a `Result<_,
/// Response>`) so the caller controls how the error is rendered; this
/// also keeps the `Result` `Err`-variant small.
fn normalize_scope(
    scope_type: &str,
    scope_id: Option<&str>,
) -> Result<(String, Option<String>), &'static str> {
    if !VALID_SCOPE_TYPES.contains(&scope_type) {
        return Err("scope_type must be one of workspace, workflow, run, verdict");
    }
    // `workspace` scope has no scope_id; every other scope requires a
    // non-empty one. Empty strings collapse to None on the workspace
    // case so the unique constraint binds the right shape.
    let normalized_scope_id = match scope_id {
        Some("") => None,
        other => other.map(|s| s.to_string()),
    };
    if scope_type == "workspace" {
        if normalized_scope_id.is_some() {
            return Err("workspace scope must not include a scope_id");
        }
    } else if normalized_scope_id.is_none() {
        return Err("scope_id is required for non-workspace scope_types");
    }
    Ok((scope_type.to_string(), normalized_scope_id))
}

/// `GET /api/chat/thread?workspace_id=&scope_type=&scope_id=` — fetch or
/// implicitly create the rolling thread for the caller in this scope,
/// plus its messages.
pub async fn get_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ThreadQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &q.workspace_id).await {
        return r;
    }
    let (scope_type, scope_id) = match normalize_scope(&q.scope_type, q.scope_id.as_deref()) {
        Ok(v) => v,
        Err(detail) => return bad_request("invalid_scope", detail),
    };

    let thread = match chat_db::get_or_create_thread(
        &state.pool,
        &auth_user.user_id,
        &q.workspace_id,
        &scope_type,
        scope_id.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("failed to get/create chat thread: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let messages = match chat_db::list_messages(&state.pool, &thread.id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("failed to list chat messages: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    Json(ThreadResponse { thread, messages }).into_response()
}

/// `POST /api/chat/thread/:id/messages` — append a message to a thread.
/// The thread must belong to the caller in the workspace named in the
/// body; otherwise we 404 (no thread leakage across users / workspaces).
pub async fn append_message(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(thread_id): Path<String>,
    Json(body): Json<AppendMessageBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &body.workspace_id).await {
        return r;
    }
    if !matches!(body.role.as_str(), "user" | "assistant" | "tool") {
        return bad_request("invalid_role", "role must be one of user, assistant, tool");
    }

    match chat_db::get_thread_for_user(
        &state.pool,
        &thread_id,
        &auth_user.user_id,
        &body.workspace_id,
    )
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "thread not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("failed to look up chat thread: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    match chat_db::append_message(
        &state.pool,
        &thread_id,
        &body.role,
        &body.content,
        body.tool_call_id.as_deref(),
    )
    .await
    {
        Ok(msg) => Json(msg).into_response(),
        Err(e) => {
            tracing::error!("failed to append chat message: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// `GET /api/chat/search?workspace_id=&q=&limit=` — full-text search
/// across the caller's chat history within a workspace. Returns
/// matching messages plus their source scope so the UI can cite where
/// the answer came from. Workspace-scoped: a user with threads in W1
/// and W2 sees only the threads in the workspace named on the request.
pub async fn search(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<SearchQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &q.workspace_id).await {
        return r;
    }
    let trimmed = q.q.trim();
    if trimmed.is_empty() {
        return bad_request("invalid_query", "q must be a non-empty search string");
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    match chat_db::search_messages(
        &state.pool,
        &auth_user.user_id,
        &q.workspace_id,
        trimmed,
        limit,
    )
    .await
    {
        Ok(hits) => Json(SearchResponse { hits }).into_response(),
        Err(e) => {
            tracing::error!("failed to search chat messages: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `workspace_id` is consumed; all other fields land in `request`.
    #[test]
    fn relay_body_flattens_workspace_id() {
        let raw = serde_json::json!({
            "workspace_id": "ws-123",
            "model": "claude-opus-4-7",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}],
        });
        let body: ChatRelayBody = serde_json::from_value(raw).unwrap();
        assert_eq!(body.workspace_id, "ws-123");
        let req = body.request.as_object().unwrap();
        assert_eq!(req["model"], "claude-opus-4-7");
        assert_eq!(req["max_tokens"], 1024);
        // workspace_id must not be forwarded to Anthropic.
        assert!(!req.contains_key("workspace_id"));
    }

    /// An empty (no-fields) body after workspace_id is also valid —
    /// Anthropic will reject it, but the relay shouldn't panic.
    #[test]
    fn relay_body_workspace_only() {
        let raw = serde_json::json!({ "workspace_id": "ws-xyz" });
        let body: ChatRelayBody = serde_json::from_value(raw).unwrap();
        assert_eq!(body.workspace_id, "ws-xyz");
        assert_eq!(body.request, serde_json::json!({}));
    }

    /// Guardrail: `stream` is stripped before forwarding.
    #[test]
    fn guardrail_strips_stream() {
        let raw = serde_json::json!({
            "workspace_id": "ws-1",
            "model": "claude-opus-4-7",
            "max_tokens": 256,
            "stream": true,
            "messages": [],
        });
        let body: ChatRelayBody = serde_json::from_value(raw).unwrap();
        let mut req = body.request.clone();
        if let Some(obj) = req.as_object_mut() {
            obj.remove("stream");
            let model_str = obj
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let resolved = crate::anthropic::resolve_model(Some(model_str.as_str())).to_owned();
            obj.insert("model".to_owned(), serde_json::json!(resolved));
        }
        let obj = req.as_object().unwrap();
        assert!(!obj.contains_key("stream"), "stream must be stripped");
        assert_eq!(obj["model"], "claude-opus-4-7");
    }

    /// Guardrail: unknown model alias is replaced with the default.
    #[test]
    fn guardrail_normalizes_model() {
        let raw = serde_json::json!({
            "workspace_id": "ws-2",
            "model": "gpt-4o",
            "max_tokens": 512,
            "messages": [],
        });
        let body: ChatRelayBody = serde_json::from_value(raw).unwrap();
        let mut req = body.request.clone();
        if let Some(obj) = req.as_object_mut() {
            obj.remove("stream");
            let model_str = obj
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let resolved = crate::anthropic::resolve_model(Some(model_str.as_str())).to_owned();
            obj.insert("model".to_owned(), serde_json::json!(resolved));
        }
        let obj = req.as_object().unwrap();
        // Non-Claude model id falls back to the default.
        assert_eq!(obj["model"], crate::anthropic::DEFAULT_MODEL);
    }

    /// `workspace` scope normalizes empty scope_id to None.
    #[test]
    fn normalize_scope_workspace_empty_id() {
        let (ty, id) = normalize_scope("workspace", Some("")).unwrap();
        assert_eq!(ty, "workspace");
        assert_eq!(id, None);
        // `None` and `Some("")` are equivalent for the workspace scope.
        let (_, id) = normalize_scope("workspace", None).unwrap();
        assert_eq!(id, None);
    }

    /// `workspace` scope rejects a non-empty scope_id.
    #[test]
    fn normalize_scope_workspace_rejects_scope_id() {
        let r = normalize_scope("workspace", Some("wf-1"));
        assert!(r.is_err());
    }

    /// Non-workspace scope types require a non-empty scope_id.
    #[test]
    fn normalize_scope_workflow_requires_id() {
        assert!(normalize_scope("workflow", None).is_err());
        assert!(normalize_scope("workflow", Some("")).is_err());
        let (ty, id) = normalize_scope("workflow", Some("wf-42")).unwrap();
        assert_eq!(ty, "workflow");
        assert_eq!(id.as_deref(), Some("wf-42"));
    }

    /// Unknown scope types are rejected up front (defense in depth on
    /// top of the DB CHECK).
    #[test]
    fn normalize_scope_rejects_unknown_type() {
        assert!(normalize_scope("nope", Some("x")).is_err());
    }
}
