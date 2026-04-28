//! Per-workspace credential CRUD (issue #164, child C of #161).
//!
//! Credentials live under `/api/workspaces/:workspace/credentials` so a
//! user holding two workspaces gets two independent secret stores —
//! launching a session in W1 will never reach for a token registered in
//! W2. Every route funnels through `require_workspace_access` for the
//! 404-not-403 membership check + PAT scope guardrail.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::server::auth::{encrypt_credential, AuthUser, RequestPrincipal};
use crate::server::db;
use crate::server::state::AppState;

use super::require_workspace_access;

/// Standard 403 body for the PAT destructive-credential guardrail (issue
/// #143). PATs may read and create credentials, but deleting an existing
/// credential or overwriting one requires a real browser session.
fn pat_destructive_blocked() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": "pat_destructive_blocked",
            "detail": "Use the dashboard to delete or overwrite credentials",
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SetCredentialBody {
    pub value: String,
}

/// GET /api/workspaces/:workspace/credentials — list credential names for
/// the current user in this workspace (no values).
pub async fn list_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::get_user_credentials(&state.db, &workspace_id, &auth_user.user_id).await {
        Ok(creds) => {
            let items: Vec<_> = creds
                .into_iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "created_at": c.created_at,
                        "updated_at": c.updated_at,
                    })
                })
                .collect();
            Json(serde_json::json!({ "credentials": items })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to list credentials: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// PUT /api/workspaces/:workspace/credentials/{name} — set or update a credential.
pub async fn set_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
    Json(body): Json<SetCredentialBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }

    // PAT principals can create new credentials but not overwrite existing
    // ones — silently rotating a secret over the API while the dashboard
    // still shows the old one would be confusing and a footgun.
    if matches!(auth_user.principal, RequestPrincipal::Pat { .. }) {
        match db::user_credential_exists(&state.db, &workspace_id, &auth_user.user_id, &name).await
        {
            Ok(true) => return pat_destructive_blocked(),
            Ok(false) => {}
            Err(e) => {
                tracing::error!("failed to check credential existence: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        }
    }

    let Some(ref key) = state.config.credential_key else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "credential storage not configured (set STIGLAB_CREDENTIAL_KEY)" })),
        )
            .into_response();
    };

    let encrypted = match encrypt_credential(key, &body.value) {
        Ok(enc) => enc,
        Err(e) => {
            tracing::error!("failed to encrypt credential: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "encryption failed").into_response();
        }
    };

    match db::set_user_credential(
        &state.db,
        &workspace_id,
        &auth_user.user_id,
        &name,
        &encrypted,
    )
    .await
    {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::error!("failed to set credential: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// DELETE /api/workspaces/:workspace/credentials/{name} — delete a credential.
pub async fn delete_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    // The destructive guard is unconditional for PATs — deletion is always
    // a session-only operation regardless of whether the credential exists.
    if matches!(auth_user.principal, RequestPrincipal::Pat { .. }) {
        return pat_destructive_blocked();
    }
    match db::delete_user_credential(&state.db, &workspace_id, &auth_user.user_id, &name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::error!("failed to delete credential: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
