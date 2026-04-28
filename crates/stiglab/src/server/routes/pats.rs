//! Personal Access Token CRUD (issue #143).
//!
//! Tokens are user-owned bearer credentials minted server-side, stored only
//! as a SHA-256 hash, and revealed to the user exactly once on creation.
//! Listing returns prefix-only metadata; the full token is unrecoverable
//! after the create call returns.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::server::auth::{generate_pat_token, AuthUser};
use crate::server::db;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreatePatBody {
    pub name: String,
    /// Workspace the PAT is scoped to.  Every PAT is workspace-pinned
    /// post-#163; requests touching another workspace 403.
    pub workspace_id: String,
    /// Required by the API surface; the dashboard always sends one of the
    /// 7/30/60/90/custom-date choices. `null` is reserved for a future
    /// "never expires" affordance.
    pub expires_at: Option<DateTime<Utc>>,
}

#[allow(clippy::result_large_err)]
fn require_authenticated(auth_user: &AuthUser) -> Result<&str, Response> {
    // Auth is always-on as of #193; the `AuthUser` extractor 401s
    // unauthenticated requests before they reach this helper.
    Ok(auth_user.user_id.as_str())
}

fn pat_summary(pat: &db::UserPat) -> serde_json::Value {
    serde_json::json!({
        "id": pat.id,
        "name": pat.name,
        "workspace_id": pat.workspace_id,
        "token_prefix": pat.token_prefix,
        "expires_at": pat.expires_at,
        "last_used_at": pat.last_used_at,
        "last_used_ip": pat.last_used_ip,
        "last_used_user_agent": pat.last_used_user_agent,
        "created_at": pat.created_at,
        "revoked_at": pat.revoked_at,
    })
}

/// GET /api/pats — List the caller's PATs (no token material).
pub async fn list_pats(State(state): State<AppState>, auth_user: AuthUser) -> Response {
    let user_id = match require_authenticated(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    match db::list_user_pats(&state.db, &user_id).await {
        Ok(pats) => {
            let items: Vec<_> = pats.iter().map(pat_summary).collect();
            Json(serde_json::json!({ "pats": items })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to list pats: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to list pats").into_response()
        }
    }
}

/// POST /api/pats — Mint a new PAT. The full token is returned exactly
/// once; subsequent reads show only the prefix.
pub async fn create_pat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreatePatBody>,
) -> Response {
    let user_id = match require_authenticated(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };

    let name = body.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name is required" })),
        )
            .into_response();
    }
    if name.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name must be at most 100 characters" })),
        )
            .into_response();
    }

    // v1: explicit expiry required. The dashboard always sends one of the
    // 7/30/60/90/custom-date choices; `null` is reserved for a future
    // "never expires" affordance and must not silently mint long-lived
    // tokens today.
    let Some(expires_at) = body.expires_at else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "expires_at is required" })),
        )
            .into_response();
    };
    if expires_at <= Utc::now() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "expires_at must be in the future" })),
        )
            .into_response();
    }

    // The caller must be a member of the workspace they're pinning the
    // PAT to — otherwise the token would be created for a workspace they
    // can't address.
    let workspace_id = body.workspace_id.trim();
    if workspace_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workspace_id is required" })),
        )
            .into_response();
    }
    match db::is_workspace_member(&state.db, workspace_id, &user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "not a member of the requested workspace",
                })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("create_pat: failed to check workspace membership: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    let generated = generate_pat_token();
    let id = Uuid::new_v4().to_string();

    if let Err(e) = db::insert_user_pat(
        &state.db,
        &id,
        &user_id,
        workspace_id,
        name,
        &generated.prefix,
        &generated.hash,
        Some(expires_at),
    )
    .await
    {
        // Translate the unique-name violation into a 409 — every other
        // error is opaque-500 for now.
        let msg = e.to_string();
        if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "a token with this name already exists",
                })),
            )
                .into_response();
        }
        tracing::error!("failed to insert pat: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    let pat = db::UserPat {
        id: id.clone(),
        user_id: user_id.clone(),
        workspace_id: workspace_id.to_string(),
        name: name.to_string(),
        token_prefix: generated.prefix.clone(),
        expires_at: Some(expires_at),
        last_used_at: None,
        last_used_ip: None,
        last_used_user_agent: None,
        created_at: Utc::now(),
        revoked_at: None,
    };

    // The body carries the only copy of the secret token. Tell every
    // intermediary not to cache it — the response is single-use by design.
    let mut response = Json(serde_json::json!({
        "pat": pat_summary(&pat),
        // Returned exactly once. After this response, the only way to
        // recover access is to mint a new token.
        "token": generated.token,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::PRAGMA,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    response
}

/// DELETE /api/pats/{id} — Soft-delete (revoke) a PAT.
pub async fn delete_pat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pat_id): Path<String>,
) -> Response {
    let user_id = match require_authenticated(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    match db::revoke_user_pat(&state.db, &user_id, &pat_id).await {
        Ok(true) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "token not found" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("failed to revoke pat: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}
