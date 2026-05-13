//! `POST /api/chat/completions` — portal-hosted Anthropic relay (spec #318).
//!
//! The dashboard sends the full Anthropic Messages API request body
//! (system, messages, tools, model, max_tokens) plus a `workspace_id`
//! field. Portal resolves the `anthropic` credential for the caller's
//! workspace, decrypts it, and forwards the body verbatim to Anthropic.
//! The API key never reaches the browser.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sqlx::postgres::PgPool;

use crate::anthropic::AnthropicClient;
use crate::auth::{AuthUser, decrypt_credential};
use crate::credential_db;
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

    let client = match AnthropicClient::new(api_key) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to build anthropic client: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    match client.forward(&body.request).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            tracing::warn!("anthropic relay error: {e}");
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
}
