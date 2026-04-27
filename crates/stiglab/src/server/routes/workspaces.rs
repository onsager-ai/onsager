//! Workspace, membership, GitHub App installation, and project CRUD
//! routes (issue #59 — Phase 0; renamed from "tenant" → "workspace" in
//! issue #163).
//!
//! All workspace-scoped endpoints go through [`require_workspace_member`]
//! which returns **404 (not 403)** for non-members. Matching GitHub's
//! private-resource behaviour means the workspace-enumeration surface
//! stays private — no invite-acceptance UI needed for v1.
//!
//! Project deletion blocks with a clear error when live sessions reference
//! the project; there is no cascade and no soft-delete in v1.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use sqlx::AnyPool;
use uuid::Uuid;

use crate::core::{GitHubAccountType, GitHubAppInstallation, Project, Workspace, WorkspaceMember};
use crate::server::auth::{encrypt_credential, AuthUser};
use crate::server::db;
use crate::server::github_app;
use crate::server::state::AppState;

// ── Auth helper ──

/// Ensure the authenticated user is a member of the workspace. Returns
/// **404** (not 403) for non-members — callers get the same response for
/// "workspace doesn't exist" and "you're not a member", so workspace IDs
/// can't be enumerated.
///
/// PAT principals carry an optional `workspace_id` scope (issue #143);
/// when set, the request must target that exact workspace. A mismatch
/// returns **403** (not 404) — the caller already proved membership at
/// PAT-mint time, so hiding the workspace's existence is moot, and a
/// distinct error makes the scope-violation observable in client tooling.
#[allow(clippy::result_large_err)]
async fn require_workspace_member(
    pool: &AnyPool,
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
    match db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(not_found("workspace not found")),
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

#[allow(clippy::result_large_err)]
fn require_auth_user(auth_user: &AuthUser) -> Result<&str, Response> {
    if auth_user.user_id == "anonymous" {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response())
    } else {
        Ok(auth_user.user_id.as_str())
    }
}

// ── Workspace CRUD ──

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceBody {
    pub slug: String,
    pub name: String,
}

/// POST /api/workspaces — Create a workspace. Creator is auto-inserted as
/// a member (no role column in v1).
pub async fn create_workspace(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateWorkspaceBody>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };

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

    // Transactional so a failed member insert can't leave an orphan
    // workspace row that permanently consumes the slug.
    if let Err(e) = db::insert_workspace_with_creator(&state.db, &workspace, &member).await {
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
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match db::list_workspaces_for_user(&state.db, user_id).await {
        Ok(workspaces) => Json(serde_json::json!({ "workspaces": workspaces })).into_response(),
        Err(e) => {
            tracing::error!("failed to list workspaces: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/workspaces/:id — Fetch a workspace. 404 for non-members.
pub async fn get_workspace(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::get_workspace(&state.db, &workspace_id).await {
        Ok(Some(w)) => Json(serde_json::json!({ "workspace": w })).into_response(),
        Ok(None) => not_found("workspace not found"),
        Err(e) => {
            tracing::error!("failed to get workspace: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/workspaces/:id/members — List members (read-only in v1).
pub async fn list_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::list_workspace_members_with_users(&state.db, &workspace_id).await {
        Ok(members) => Json(serde_json::json!({ "members": members })).into_response(),
        Err(e) => {
            tracing::error!("failed to list members: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

// ── GitHub App installations ──

#[derive(Debug, Deserialize)]
pub struct RegisterInstallationBody {
    pub install_id: i64,
    pub account_login: String,
    pub account_type: GitHubAccountType,
    /// Webhook shared-secret for signature verification. Stored encrypted
    /// at rest using the existing credential key. Optional so workspaces can
    /// register an installation before wiring up webhooks.
    pub webhook_secret: Option<String>,
}

/// POST /api/workspaces/:id/github-installations — Register a GitHub App
/// installation linked to this workspace.
///
/// In Phase 0 this is a manual-entry endpoint (caller supplies the
/// installation ID and webhook secret). The full OAuth App-callback flow
/// that mints these automatically is a follow-up spec; the data model and
/// API shape are frozen here so the callback is purely additive.
pub async fn register_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
    Json(body): Json<RegisterInstallationBody>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
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
                        "error": "credential storage not configured (set STIGLAB_CREDENTIAL_KEY)"
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
        db::insert_github_app_installation(&state.db, &install, secret_cipher.as_deref()).await
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

/// GET /api/workspaces/:id/github-installations — List installations
/// linked to a workspace.
pub async fn list_installations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::list_github_app_installations_for_workspace(&state.db, &workspace_id).await {
        Ok(installations) => {
            Json(serde_json::json!({ "installations": installations })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to list installations: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// DELETE /api/workspaces/:id/github-installations/:install_id — Unlink
/// an installation. Blocked with 409 when projects still reference it
/// (app-layer check — the tables do not declare FK constraints, in
/// keeping with the rest of stiglab's schema). Callers must delete the
/// projects first in v1.
pub async fn delete_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_id)): Path<(String, String)>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }

    match db::get_github_app_installation(&state.db, &install_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    match db::count_projects_for_installation(&state.db, &install_id).await {
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
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    if let Err(e) = db::delete_github_app_installation(&state.db, &install_id).await {
        tracing::error!("failed to delete installation: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

// ── Projects ──

#[derive(Debug, Deserialize)]
pub struct AddProjectBody {
    pub github_app_installation_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    /// Optional. Inferring from GitHub at create-time requires an
    /// installation access token the Phase 0 stub can't mint; callers may
    /// supply a branch or let the server fall back to `"main"`.
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
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }

    // Validate the installation belongs to this workspace.
    match db::get_github_app_installation(&state.db, &body.github_app_installation_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
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

    // If the caller supplied a branch, trust it. Otherwise try the GitHub
    // API (when the App is configured), then fall back to "main" on any
    // failure — onboarding must never block on a network hiccup.
    let supplied = body
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let default_branch = match supplied {
        Some(b) => b,
        None => match installation_token_for(&state.db, &body.github_app_installation_id).await {
            Ok(Some(token)) => {
                match github_app::get_repo_default_branch(&token, repo_owner, repo_name).await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(
                            "default_branch inference failed for {repo_owner}/{repo_name}: {e}"
                        );
                        "main".to_string()
                    }
                }
            }
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

    if let Err(e) = db::insert_project(&state.db, &project).await {
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
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::list_projects_for_workspace(&state.db, &workspace_id).await {
        Ok(projects) => Json(serde_json::json!({ "projects": projects })).into_response(),
        Err(e) => {
            tracing::error!("failed to list projects: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
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
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match db::list_projects_for_user(&state.db, user_id).await {
        Ok(projects) => Json(serde_json::json!({ "projects": projects })).into_response(),
        Err(e) => {
            tracing::error!("failed to list projects: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
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
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let project = match db::get_project(&state.db, &project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("project not found"),
        Err(e) => {
            tracing::error!("failed to get project: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &project.workspace_id).await {
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
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let project = match db::get_project(&state.db, &project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("project not found"),
        Err(e) => {
            tracing::error!("failed to get project: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &project.workspace_id).await {
        return r;
    }

    match db::count_live_sessions_for_project(&state.db, &project_id).await {
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
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    if let Err(e) = db::delete_project(&state.db, &project_id).await {
        tracing::error!("failed to delete project: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Public helper re-exported so `routes::tasks` can reuse the same 404
/// semantics (and PAT scope check) when creating a session scoped to a
/// project.
#[allow(clippy::result_large_err)]
pub async fn assert_workspace_member(
    pool: &AnyPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    require_workspace_member(pool, auth_user, workspace_id).await
}

// ── Accessible-repos picker + GitHub App install flow ──
//
// These close the remaining Phase 0 / #59 items: the OAuth callback
// (modal workspace picker) and the "Add Project" dropdown scoped to the
// installation's accessible repos. The App flow is opt-in via env — when
// not configured, the pre-existing manual-entry path still works.

/// Mint a per-installation access token from DB metadata. Returns `None`
/// when the App is not configured or the installation row is missing.
async fn installation_token_for(
    pool: &AnyPool,
    onsager_install_id: &str,
) -> anyhow::Result<Option<github_app::InstallationToken>> {
    let Some(cfg) = github_app::AppConfig::from_env() else {
        return Ok(None);
    };
    let Some(install) = db::get_github_app_installation(pool, onsager_install_id).await? else {
        return Ok(None);
    };
    let jwt = github_app::mint_app_jwt(&cfg)?;
    let token = github_app::mint_installation_token(&jwt, install.install_id).await?;
    Ok(Some(token))
}

/// GET /api/workspaces/:id/github-installations/:install_id/accessible-repos —
/// list repos this installation can access, so "Add Project" can show a
/// dropdown instead of free-text `repo_owner/repo_name` inputs.
pub async fn list_accessible_repos(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_id)): Path<(String, String)>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    // Confirm the install row belongs to this workspace before burning a token.
    match db::get_github_app_installation(&state.db, &install_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    match installation_token_for(&state.db, &install_id).await {
        Ok(Some(token)) => match github_app::list_installation_repos(&token).await {
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

/// GET /api/workspaces/:id/github-installations/:install_id/repos/:owner/:repo/labels
/// — list labels defined on a repo, so workflow triggers can offer a
/// combobox of existing labels instead of free-text entry.
pub async fn list_repo_labels(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((workspace_id, install_id, owner, repo)): Path<(String, String, String, String)>,
) -> Response {
    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }
    match db::get_github_app_installation(&state.db, &install_id).await {
        Ok(Some(inst)) if inst.workspace_id == workspace_id => {}
        Ok(_) => return not_found("installation not found"),
        Err(e) => {
            tracing::error!("failed to get installation: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    match installation_token_for(&state.db, &install_id).await {
        Ok(Some(token)) => match github_app::list_repo_labels(&token, &owner, &repo).await {
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

// ── Install-flow routes ──

#[derive(Debug, Deserialize)]
pub struct InstallStartQuery {
    pub workspace_id: String,
}

/// GET /api/github-app/install-start?workspace_id=... — Redirect the user
/// to GitHub's App installation page, carrying the target workspace in
/// the OAuth `state` param. The callback will re-read it to link the new
/// installation to the workspace without a separate modal round-trip.
pub async fn github_app_install_start(
    State(state): State<AppState>,
    auth_user: AuthUser,
    axum::extract::Query(query): axum::extract::Query<InstallStartQuery>,
) -> Response {
    use axum::http::header;
    use axum::response::Redirect;

    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &query.workspace_id).await {
        return r;
    }

    let Some(cfg) = github_app::AppConfig::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "GitHub App is not configured on this server"
            })),
        )
            .into_response();
    };

    // state = "{workspace_id}.{csrf_random}" — cookie stores the same thing
    // so the callback can verify it came from this browser session.
    let csrf = crate::server::auth::generate_state();
    let state_param = format!("{}.{}", query.workspace_id, csrf);
    let sec = if state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "stiglab_github_app_state={state_param}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}"
    );
    let url = format!(
        "https://github.com/apps/{slug}/installations/new?state={state_param}",
        slug = cfg.slug,
    );
    ([(header::SET_COOKIE, cookie)], Redirect::temporary(&url)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct InstallCallbackQuery {
    pub installation_id: i64,
    pub setup_action: Option<String>,
    pub state: Option<String>,
}

/// GET /api/github-app/callback?installation_id=N&setup_action=install&state=...
///
/// GitHub redirects here after the user installs the App (this path is the
/// App's Setup URL on GitHub). We verify the state cookie, mint an App JWT
/// to look up the install's account, persist the installation row under
/// the originating workspace, and redirect the browser back to
/// `/workspaces?github_app_linked={id}` so `WorkspaceCard`'s useEffect can
/// invalidate the installations query without a manual refresh.
pub async fn github_app_install_callback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: axum::http::HeaderMap,
    axum::extract::Query(query): axum::extract::Query<InstallCallbackQuery>,
) -> Response {
    use axum::http::header;

    let _user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };

    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookie_state = crate::server::auth::parse_cookie(cookie_header, "stiglab_github_app_state");
    let query_state = query.state.as_deref().unwrap_or_default();
    if cookie_state != Some(query_state) || query_state.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
    }
    let workspace_id = match query_state.split_once('.') {
        Some((t, _)) if !t.is_empty() => t.to_string(),
        _ => return (StatusCode::BAD_REQUEST, "malformed state").into_response(),
    };
    if let Err(r) = require_workspace_member(&state.db, &auth_user, &workspace_id).await {
        return r;
    }

    let Some(cfg) = github_app::AppConfig::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub App is not configured on this server",
        )
            .into_response();
    };

    let jwt = match github_app::mint_app_jwt(&cfg) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("mint_app_jwt failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "GitHub App auth failed").into_response();
        }
    };
    let info = match github_app::get_installation(&jwt, query.installation_id).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("get_installation {} failed: {e}", query.installation_id);
            return (StatusCode::BAD_GATEWAY, "GitHub installation lookup failed").into_response();
        }
    };

    // Idempotency: if the user re-runs the install flow (or GitHub
    // redelivers the callback), we must not blind-insert — the numeric
    // `install_id` is UNIQUE. Pre-check and either treat as a no-op
    // (same workspace) or refuse with 409 (different workspace).
    match db::get_github_app_installation_by_install_id(&state.db, query.installation_id).await {
        Ok(Some(existing)) if existing.workspace_id == workspace_id => {
            tracing::info!(
                "GitHub App installation {} already linked to workspace {}; treating callback as idempotent",
                query.installation_id,
                workspace_id
            );
        }
        Ok(Some(existing)) => {
            tracing::warn!(
                "GitHub App installation {} is already linked to workspace {}; requested workspace {}",
                query.installation_id,
                existing.workspace_id,
                workspace_id
            );
            return (
                StatusCode::CONFLICT,
                "GitHub installation is already linked to a different workspace",
            )
                .into_response();
        }
        Ok(None) => {
            let install = GitHubAppInstallation {
                id: Uuid::new_v4().to_string(),
                workspace_id: workspace_id.clone(),
                install_id: query.installation_id,
                account_login: info.account_login,
                account_type: info.account_type,
                created_at: Utc::now(),
            };
            // No webhook secret here — the App-managed shared secret is a
            // server env var (portal reads it); per-install override
            // remains the manual endpoint's job.
            if let Err(e) = db::insert_github_app_installation(&state.db, &install, None).await {
                tracing::error!("insert_github_app_installation failed: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "GitHub installation link failed",
                )
                    .into_response();
            }
        }
        Err(e) => {
            tracing::error!("install_id lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub installation link could not be verified",
            )
                .into_response();
        }
    }

    let sec = if state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    };
    let clear =
        format!("stiglab_github_app_state=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");
    let location = format!(
        "/workspaces?github_app_linked={}&workspace_id={}",
        query.installation_id, workspace_id
    );

    axum::response::Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, clear)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

/// GET /api/github-app/config — Tiny discovery endpoint so the dashboard
/// can decide whether to render the "Install via GitHub App" button or
/// fall back to the manual-entry form.
pub async fn github_app_config() -> Response {
    let enabled = github_app::AppConfig::from_env().is_some();
    let slug = std::env::var("GITHUB_APP_SLUG").ok();
    Json(serde_json::json!({ "enabled": enabled, "slug": slug })).into_response()
}
