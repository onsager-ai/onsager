//! Project CRUD (spec #222 Slice 3a — moved from stiglab).
//!
//! These routes link a workspace to a `(repo_owner, repo_name)` pair.
//! Membership is checked via `super::workspaces::require_workspace_access`
//! (404 not 403 for non-members; PAT scope guardrail).
//!
//! Project deletion blocks with a clear error when live sessions
//! reference the project; there is no cascade and no soft-delete in v1.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use onsager_github::api::app as gh_app;
use serde::Deserialize;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::core::Project;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;
use crate::workspace_db;

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

/// Mint a per-installation token from the row UUID. Returns `None` when
/// the App is not configured or the installation row is missing — the
/// caller falls back to a default.
async fn installation_token_for(
    pool: &PgPool,
    install_row_id: &str,
) -> anyhow::Result<Option<gh_app::InstallationToken>> {
    let Some(cfg) = gh_app::AppConfig::from_env() else {
        return Ok(None);
    };
    let Some(lookup) = workspace_db::get_installation_lookup(pool, install_row_id).await? else {
        return Ok(None);
    };
    let jwt = gh_app::mint_app_jwt(&cfg)?;
    let token = gh_app::mint_installation_token(&jwt, lookup.install_id).await?;
    Ok(Some(token))
}

#[derive(Debug, Deserialize)]
pub struct AddProjectBody {
    pub github_app_installation_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    /// Optional. If absent we try the GitHub API (when the App is
    /// configured) and fall back to `"main"` on any failure.
    pub default_branch: Option<String>,
}

/// POST /api/workspaces/:id/projects — Add a project (opt-in per repo;
/// no auto-mirroring).
pub async fn add_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
    Json(body): Json<AddProjectBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }

    // Validate the installation belongs to this workspace.
    match workspace_db::get_installation_lookup(&state.pool, &body.github_app_installation_id).await
    {
        Ok(Some(l)) if l.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    let repo_owner = body.repo_owner.trim();
    let repo_name = body.repo_name.trim();
    if repo_owner.is_empty() || repo_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "repo_owner and repo_name are required" })),
        )
            .into_response();
    }

    // If the caller supplied a branch, trust it. Otherwise try the
    // GitHub API (when the App is configured), then fall back to
    // "main" on any failure — onboarding must never block on a
    // network hiccup.
    let supplied = body
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let default_branch = match supplied {
        Some(b) => b,
        None => match installation_token_for(&state.pool, &body.github_app_installation_id).await {
            Ok(Some(token)) => match gh_app::get_repo_default_branch(
                &token,
                repo_owner,
                repo_name,
            )
            .await
            {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        "default_branch inference failed for {repo_owner}/{repo_name}: {e}"
                    );
                    "main".to_string()
                }
            },
            Ok(None) => "main".to_string(),
            Err(e) => {
                tracing::warn!(
                    "installation token lookup failed for default_branch inference on {repo_owner}/{repo_name}: {e}"
                );
                "main".to_string()
            }
        },
    };

    let project = Project {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.clone(),
        github_app_installation_id: body.github_app_installation_id.clone(),
        repo_owner: repo_owner.to_string(),
        repo_name: repo_name.to_string(),
        default_branch,
        created_at: Utc::now(),
    };

    if let Err(e) = workspace_db::insert_project(&state.pool, &project).await {
        tracing::error!("failed to insert project: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to add project (repo may already be onboarded)"
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "project": project })),
    )
        .into_response()
}

/// GET /api/workspaces/:id/projects — List projects in a workspace.
pub async fn list_projects(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match workspace_db::list_projects_for_workspace(&state.pool, &workspace_id).await {
        Ok(projects) => Json(serde_json::json!({ "projects": projects })).into_response(),
        Err(e) => {
            tracing::error!("failed to list projects: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// GET /api/projects — List every project the current user can access,
/// across all their workspaces. Powers the cross-workspace project
/// selector in `CreateSessionSheet`.
pub async fn list_all_projects_for_user(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Response {
    match workspace_db::list_projects_for_user(&state.pool, &auth_user.user_id).await {
        Ok(projects) => Json(serde_json::json!({ "projects": projects })).into_response(),
        Err(e) => {
            tracing::error!("failed to list projects: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// GET /api/projects/:id — Fetch a project by ID. 404 for users who are
/// not members of the owning workspace.
pub async fn get_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
) -> Response {
    let project = match workspace_db::get_project(&state.pool, &project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("project not found"),
        Err(e) => {
            tracing::error!("failed to get project: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &project.workspace_id).await {
        return r;
    }
    Json(serde_json::json!({ "project": project })).into_response()
}

/// DELETE /api/projects/:id — Delete a project. Blocks with a clear
/// error when any attached session is not in a terminal state (no
/// cascade, no soft-delete in v1).
pub async fn delete_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
) -> Response {
    let project = match workspace_db::get_project(&state.pool, &project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("project not found"),
        Err(e) => {
            tracing::error!("failed to get project: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &project.workspace_id).await {
        return r;
    }

    match workspace_db::count_live_sessions_for_project(&state.pool, &project_id).await {
        Ok(n) if n > 0 => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!(
                        "cannot delete project: {n} live session(s) still reference it. \
                         Wait for them to finish or abort them first."
                    )
                })),
            )
                .into_response();
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("failed to count live sessions: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    if let Err(e) = workspace_db::delete_project(&state.pool, &project_id).await {
        tracing::error!("failed to delete project: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}
