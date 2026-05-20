//! `/api/pats*` route handlers.
//!
//! Personal Access Token CRUD (issue #143). Tokens are user-owned bearer
//! credentials minted server-side, stored only as a SHA-256 hash, and
//! revealed to the user exactly once on creation. Listing returns
//! prefix-only metadata; the full token is unrecoverable after the create
//! call returns.
//!
//! Spec #222 Slice 2b moved this surface from stiglab to portal so the
//! external HTTP boundary is owned by the edge subsystem (clause 1 of the
//! seam rule). Stiglab proxies `/api/pats*` to portal via
//! `routes::portal::proxy` so the dashboard's API_BASE cutover (Slice 6)
//! can land independently.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::auth::{AuthUser, generate_pat_token};
use crate::pat_db::{self, UserPat};
use crate::state::AppState;

/// Wire-shape projection of [`UserPat`] returned to the dashboard.
/// Token material never appears here; the prefix is the only identifier
/// surfaced after creation.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Pat {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub token_prefix: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_ip: Option<String>,
    pub last_used_user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<&UserPat> for Pat {
    fn from(p: &UserPat) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            workspace_id: p.workspace_id.clone(),
            token_prefix: p.token_prefix.clone(),
            expires_at: p.expires_at,
            last_used_at: p.last_used_at,
            last_used_ip: p.last_used_ip.clone(),
            last_used_user_agent: p.last_used_user_agent.clone(),
            created_at: p.created_at,
            revoked_at: p.revoked_at,
        }
    }
}

/// Wire-shape envelope for `POST /api/pats`. The full `token` field is
/// returned exactly once and is unrecoverable thereafter.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CreatePatResponse {
    pub pat: Pat,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePatBody {
    pub name: String,
    /// Workspace the PAT is scoped to. Every PAT is workspace-pinned
    /// post-#163; requests touching another workspace 403.
    pub workspace_id: String,
    /// Required by the API surface; the dashboard always sends one of the
    /// 7/30/60/90/custom-date choices. `null` is reserved for a future
    /// "never expires" affordance.
    pub expires_at: Option<DateTime<Utc>>,
}

fn pat_summary(pat: &UserPat) -> Pat {
    Pat::from(pat)
}

/// GET /api/pats — list the caller's PATs (no token material).
pub async fn list_pats(State(state): State<AppState>, auth_user: AuthUser) -> Response {
    match pat_db::list_user_pats(&state.pool, &auth_user.user_id).await {
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

/// POST /api/pats — mint a new PAT. The full token is returned exactly
/// once; subsequent reads show only the prefix.
pub async fn create_pat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreatePatBody>,
) -> Response {
    let user_id = auth_user.user_id.clone();

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
    match pat_db::is_workspace_member(&state.pool, workspace_id, &user_id).await {
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

    if let Err(e) = pat_db::insert_user_pat(
        &state.pool,
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

    let pat = UserPat {
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
    // Returned exactly once. After this response, the only way to
    // recover access is to mint a new token.
    let mut response = Json(CreatePatResponse {
        pat: pat_summary(&pat),
        token: generated.token,
    })
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

/// DELETE /api/pats/{id} — soft-delete (revoke) a PAT.
pub async fn delete_pat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pat_id): Path<String>,
) -> Response {
    match pat_db::revoke_user_pat(&state.pool, &auth_user.user_id, &pat_id).await {
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
