//! Workspace + member CRUD (spec #222 Slice 3a — moved from stiglab).
//!
//! These routes funnel through `require_workspace_access` for the
//! 404-not-403 membership check and the PAT-scope guardrail (matching
//! GitHub's private-resource pattern — non-members can't enumerate
//! workspaces via 403 vs 404 differentiation).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::core::{Workspace, WorkspaceMember};
use crate::state::AppState;
use crate::workspace_db;

/// Authorize an `AuthUser` against a target workspace. Two checks, in
/// order:
///
/// 1. **PAT scope (403 on mismatch).** If the principal is a PAT pinned
///    to a workspace, the request must target that exact workspace.
/// 2. **Membership (404 on miss).** Otherwise the caller must be a
///    member of the workspace; non-members get a flat 404 to avoid
///    leaking workspace existence.
pub(crate) async fn require_workspace_access(
    pool: &PgPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    if let Some(pinned) = auth_user.principal.pinned_workspace_id() {
        if pinned != workspace_id {
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "pat_workspace_scope_mismatch",
                    "detail": "PAT is pinned to a different workspace",
                })),
            )
                .into_response());
        }
    }
    match workspace_db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
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

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceBody {
    pub slug: String,
    pub name: String,
}

/// POST /api/workspaces — Create a workspace. Creator is auto-inserted
/// as a member (no role column in v1).
pub async fn create_workspace(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateWorkspaceBody>,
) -> Response {
    let user_id = auth_user.user_id.clone();

    let slug = body.slug.trim();
    let name = body.name.trim();
    if slug.is_empty() || name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "slug and name are required" })),
        )
            .into_response();
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "slug must be lowercase alphanumeric with hyphens"
            })),
        )
            .into_response();
    }

    let now = Utc::now();
    let workspace = Workspace {
        id: Uuid::new_v4().to_string(),
        slug: slug.to_string(),
        name: name.to_string(),
        created_by: user_id.clone(),
        created_at: now,
    };
    let member = WorkspaceMember {
        workspace_id: workspace.id.clone(),
        user_id: user_id.clone(),
        joined_at: now,
    };

    if let Err(e) =
        workspace_db::insert_workspace_with_creator(&state.pool, &workspace, &member).await
    {
        tracing::error!("failed to insert workspace + creator-member: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to create workspace (slug may already exist)"
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "workspace": workspace })),
    )
        .into_response()
}

/// GET /api/workspaces — List workspaces the current user belongs to.
pub async fn list_workspaces(State(state): State<AppState>, auth_user: AuthUser) -> Response {
    match workspace_db::list_workspaces_for_user(&state.pool, &auth_user.user_id).await {
        Ok(workspaces) => Json(serde_json::json!({ "workspaces": workspaces })).into_response(),
        Err(e) => {
            tracing::error!("failed to list workspaces: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// GET /api/workspaces/:id — Fetch a workspace. 404 for non-members.
pub async fn get_workspace(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match workspace_db::get_workspace(&state.pool, &workspace_id).await {
        Ok(Some(w)) => Json(serde_json::json!({ "workspace": w })).into_response(),
        Ok(None) => not_found("workspace not found"),
        Err(e) => {
            tracing::error!("failed to get workspace: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// GET /api/workspaces/:id/members — List members (read-only in v1).
pub async fn list_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match workspace_db::list_workspace_members_with_users(&state.pool, &workspace_id).await {
        Ok(members) => Json(serde_json::json!({ "members": members })).into_response(),
        Err(e) => {
            tracing::error!("failed to list members: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}
