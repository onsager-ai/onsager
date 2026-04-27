//! Per-workspace credential routes (issue #164).
//!
//! All routes nest under `/api/workspaces/:workspace/credentials` so each
//! credential row is bound to exactly one workspace. The unscoped
//! `/api/credentials` surface that this module used to host has been
//! removed, not aliased — per CLAUDE.md "no dangling wires" / "compat
//! aliases that ossify". Dashboards and CLIs must address the
//! workspace-scoped path.
//!
//! Authorization is funnelled through the shared `require_workspace_access`
//! helper, which handles both PAT scope (403 with
//! `pat_workspace_scope_mismatch`) and membership (404). PAT principals
//! retain the destructive-action block (PUT-overwrite + DELETE map to
//! `pat_destructive_blocked`) — those are still session-only operations.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::server::auth::{encrypt_credential, AuthUser, RequestPrincipal};
use crate::server::db;
use crate::server::state::AppState;

use super::require_workspace_access;

/// Standard 403 body for the PAT destructive-credential guardrail (issue
/// #143). PATs may read and create credentials, but deleting an existing
/// credential or overwriting one requires a real browser session — those
/// actions take a refresh of the secret out of the user's hands and into
/// any system that holds the token, which is the explicit non-goal here.
fn pat_destructive_blocked() -> axum::response::Response {
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

/// GET /api/workspaces/:workspace/credentials — list credential names
/// (no values) bound to this workspace for the current user.
pub async fn list_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::get_user_credentials(&state.db, &auth_user.user_id, &workspace_id).await {
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

/// PUT /api/workspaces/:workspace/credentials/:name — set or update a
/// credential bound to this workspace.
pub async fn set_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
    Json(body): Json<SetCredentialBody>,
) -> impl IntoResponse {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    // PAT principals can create new credentials but not overwrite existing
    // ones — silently rotating a secret over the API while the dashboard
    // still shows the old one would be confusing and a footgun.
    if matches!(auth_user.principal, RequestPrincipal::Pat { .. }) {
        match db::user_credential_exists(&state.db, &auth_user.user_id, &workspace_id, &name).await
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
        &auth_user.user_id,
        &workspace_id,
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

/// DELETE /api/workspaces/:workspace/credentials/:name — delete a
/// workspace-scoped credential.
pub async fn delete_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, name)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    // The destructive guard is unconditional for PATs — deletion is always
    // a session-only operation regardless of whether the credential exists.
    if matches!(auth_user.principal, RequestPrincipal::Pat { .. }) {
        return pat_destructive_blocked();
    }
    match db::delete_user_credential(&state.db, &auth_user.user_id, &workspace_id, &name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            tracing::error!("failed to delete credential: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
