//! Per-workspace GitHub App installation routes (spec #222 Slice 3b —
//! moved from stiglab).
//!
//! Endpoints:
//! - `POST /api/workspaces/:workspace_id/github-installations` — manual
//!   register (caller supplies install_id + optional webhook_secret).
//! - `GET  /api/workspaces/:workspace_id/github-installations` — list.
//! - `DELETE /api/workspaces/:workspace_id/github-installations/:install_row_id`
//!   — unlink (blocked if projects still reference it).
//! - `GET /api/workspaces/:workspace_id/github-installations/:install_row_id/accessible-repos`
//!   — list repos this install can access (powers the "Add Project"
//!   dropdown).
//! - `GET /api/workspaces/:workspace_id/github-installations/:install_row_id/repos/:owner/:repo/labels`
//!   — list labels (powers the workflow trigger label combobox).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use onsager_github::api::app as gh_app;
use serde::Deserialize;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::auth::{encrypt_credential, AuthUser};
use crate::installation::{GitHubAccountType, GitHubAppInstallation};
use crate::installation_db;
use crate::state::AppState;

/// Authorize an `AuthUser` against a target workspace. Mirrors the
/// helper in `handlers/credentials.rs` — duplicated here intentionally
/// because each slice landed its own copy. A follow-up will lift the
/// shared shape into a portal-wide access helper once the route
/// migrations under #222 settle.
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
    match installation_db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
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

/// Mint a per-installation token from the row UUID. Returns `None`
/// when the App is not configured or the installation row is missing.
pub(crate) async fn installation_token_for(
    pool: &PgPool,
    install_row_id: &str,
) -> anyhow::Result<Option<gh_app::InstallationToken>> {
    let Some(cfg) = gh_app::AppConfig::from_env() else {
        return Ok(None);
    };
    let Some(install) = installation_db::get_installation(pool, install_row_id).await? else {
        return Ok(None);
    };
    let jwt = gh_app::mint_app_jwt(&cfg)?;
    let token = gh_app::mint_installation_token(&jwt, install.install_id).await?;
    Ok(Some(token))
}

#[derive(Debug, Deserialize)]
pub struct RegisterInstallationBody {
    pub install_id: i64,
    pub account_login: String,
    pub account_type: GitHubAccountType,
    /// Webhook shared-secret for signature verification. Stored
    /// encrypted at rest using the configured credential key. Optional
    /// so a workspace can register an installation before wiring up
    /// webhooks.
    pub webhook_secret: Option<String>,
}

/// POST /api/workspaces/:workspace_id/github-installations — Manual
/// register (the OAuth callback path mints these automatically; this
/// is the fallback for workspaces self-hosting the install).
pub async fn register_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
    Json(body): Json<RegisterInstallationBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }

    let account_login = body.account_login.trim();
    if account_login.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "account_login is required" })),
        )
            .into_response();
    }

    let secret_cipher = match body.webhook_secret.as_deref() {
        None | Some("") => None,
        Some(plaintext) => {
            let Some(ref key) = state.config.credential_key else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": "credential storage not configured (set ONSAGER_CREDENTIAL_KEY)"
                    })),
                )
                    .into_response();
            };
            match encrypt_credential(key, plaintext) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::error!("failed to encrypt webhook secret: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "encryption failed")
                        .into_response();
                }
            }
        }
    };

    let install = GitHubAppInstallation {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.clone(),
        install_id: body.install_id,
        account_login: account_login.to_string(),
        account_type: body.account_type,
        created_at: Utc::now(),
    };

    if let Err(e) =
        installation_db::insert_installation(&state.pool, &install, secret_cipher.as_deref()).await
    {
        tracing::error!("failed to insert installation: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to register installation (install_id may already be linked)"
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "installation": install })),
    )
        .into_response()
}

/// GET /api/workspaces/:workspace_id/github-installations
pub async fn list_installations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match installation_db::list_installations_for_workspace(&state.pool, &workspace_id).await {
        Ok(installations) => {
            Json(serde_json::json!({ "installations": installations })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to list installations: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// DELETE /api/workspaces/:workspace_id/github-installations/:install_row_id
/// — Unlink. Blocked with 409 when projects still reference it.
pub async fn delete_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_row_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }

    match installation_db::get_installation(&state.pool, &install_row_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    match installation_db::count_projects_for_installation(&state.pool, &install_row_id).await {
        Ok(n) if n > 0 => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!(
                        "cannot unlink installation: {n} project(s) still reference it. \
                         Delete the projects first."
                    )
                })),
            )
                .into_response();
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("failed to count projects for installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    if let Err(e) = installation_db::delete_installation(&state.pool, &install_row_id).await {
        tracing::error!("failed to delete installation: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// GET /api/workspaces/:workspace_id/github-installations/:install_row_id/accessible-repos
pub async fn list_accessible_repos(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_row_id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match installation_db::get_installation(&state.pool, &install_row_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    match installation_token_for(&state.pool, &install_row_id).await {
        Ok(Some(token)) => match gh_app::list_installation_repos(&token).await {
            Ok(repos) => Json(serde_json::json!({ "repos": repos })).into_response(),
            Err(e) => {
                tracing::warn!("list_installation_repos failed: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "GitHub API request failed" })),
                )
                    .into_response()
            }
        },
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "GitHub App is not configured on this server"
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("failed to mint installation token: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub App auth failed" })),
            )
                .into_response()
        }
    }
}

/// GET /api/workspaces/:workspace_id/github-installations/:install_row_id/repos/:owner/:repo/labels
pub async fn list_repo_labels(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_row_id, owner, repo)): Path<(String, String, String, String)>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match installation_db::get_installation(&state.pool, &install_row_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    match installation_token_for(&state.pool, &install_row_id).await {
        Ok(Some(token)) => match gh_app::list_repo_labels(&token, &owner, &repo).await {
            Ok(labels) => Json(serde_json::json!({ "labels": labels })).into_response(),
            Err(e) => {
                tracing::warn!("list_repo_labels failed: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "GitHub API request failed" })),
                )
                    .into_response()
            }
        },
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "GitHub App is not configured on this server"
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("failed to mint installation token: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "GitHub App auth failed" })),
            )
                .into_response()
        }
    }
}
