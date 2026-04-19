//! Tenant / workspace, membership, GitHub App installation, and project
//! CRUD routes (issue #59 — Phase 0).
//!
//! All tenant-scoped endpoints go through [`require_tenant_member`] which
//! returns **404 (not 403)** for non-members. Matching GitHub's private-
//! resource behaviour means the tenant-enumeration surface stays private —
//! no invite-acceptance UI needed for v1.
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

use crate::core::{GitHubAccountType, GitHubAppInstallation, Project, Tenant, TenantMember};
use crate::server::auth::{encrypt_credential, AuthUser};
use crate::server::db;
use crate::server::state::AppState;

// ── Auth helper ──

/// Ensure the authenticated user is a member of the tenant. Returns
/// **404** (not 403) for non-members — callers get the same response for
/// "tenant doesn't exist" and "you're not a member", so tenant IDs can't be
/// enumerated.
#[allow(clippy::result_large_err)]
async fn require_tenant_member(
    pool: &AnyPool,
    user_id: &str,
    tenant_id: &str,
) -> Result<(), Response> {
    match db::is_tenant_member(pool, tenant_id, user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(not_found("tenant not found")),
        Err(e) => {
            tracing::error!("failed to check tenant membership: {e}");
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

// ── Tenant CRUD ──

#[derive(Debug, Deserialize)]
pub struct CreateTenantBody {
    pub slug: String,
    pub name: String,
}

/// POST /api/tenants — Create a workspace. Creator is auto-inserted as a
/// member (no role column in v1).
pub async fn create_tenant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateTenantBody>,
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
    let tenant = Tenant {
        id: Uuid::new_v4().to_string(),
        slug: slug.to_string(),
        name: name.to_string(),
        created_by: user_id.clone(),
        created_at: now,
    };
    let member = TenantMember {
        tenant_id: tenant.id.clone(),
        user_id: user_id.clone(),
        joined_at: now,
    };

    // Transactional so a failed member insert can't leave an orphan
    // tenant row that permanently consumes the slug.
    if let Err(e) = db::insert_tenant_with_creator(&state.db, &tenant, &member).await {
        tracing::error!("failed to insert tenant + creator-member: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": "failed to create tenant (slug may already exist)" }),
            ),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "tenant": tenant })),
    )
        .into_response()
}

/// GET /api/tenants — List workspaces the current user belongs to.
pub async fn list_tenants(State(state): State<AppState>, auth_user: AuthUser) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match db::list_tenants_for_user(&state.db, user_id).await {
        Ok(tenants) => Json(serde_json::json!({ "tenants": tenants })).into_response(),
        Err(e) => {
            tracing::error!("failed to list tenants: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/tenants/:id — Fetch a workspace. 404 for non-members.
pub async fn get_tenant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }
    match db::get_tenant(&state.db, &tenant_id).await {
        Ok(Some(t)) => Json(serde_json::json!({ "tenant": t })).into_response(),
        Ok(None) => not_found("tenant not found"),
        Err(e) => {
            tracing::error!("failed to get tenant: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/tenants/:id/members — List members (read-only in v1).
pub async fn list_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }
    match db::list_tenant_members(&state.db, &tenant_id).await {
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
    /// at rest using the existing credential key. Optional so tenants can
    /// register an installation before wiring up webhooks.
    pub webhook_secret: Option<String>,
}

/// POST /api/tenants/:id/github-installations — Register a GitHub App
/// installation linked to this tenant.
///
/// In Phase 0 this is a manual-entry endpoint (caller supplies the
/// installation ID and webhook secret). The full OAuth App-callback flow
/// that mints these automatically is a follow-up spec; the data model and
/// API shape are frozen here so the callback is purely additive.
pub async fn register_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
    Json(body): Json<RegisterInstallationBody>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id.to_string(),
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, &user_id, &tenant_id).await {
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
        tenant_id: tenant_id.clone(),
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

/// GET /api/tenants/:id/github-installations — List installations linked
/// to a tenant.
pub async fn list_installations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }
    match db::list_github_app_installations_for_tenant(&state.db, &tenant_id).await {
        Ok(installations) => {
            Json(serde_json::json!({ "installations": installations })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to list installations: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// DELETE /api/tenants/:id/github-installations/:install_id — Unlink an
/// installation. Blocked with 409 when projects still reference it
/// (app-layer check — the tables do not declare FK constraints, in
/// keeping with the rest of stiglab's schema). Callers must delete the
/// projects first in v1.
pub async fn delete_installation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((tenant_id, install_id)): Path<(String, String)>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }

    match db::get_github_app_installation(&state.db, &install_id).await {
        Ok(Some(inst)) if inst.tenant_id == tenant_id => {}
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

/// POST /api/tenants/:id/projects — Add a project (opt-in per repo; no
/// auto-mirroring).
pub async fn add_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
    Json(body): Json<AddProjectBody>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }

    // Validate the installation belongs to this tenant.
    match db::get_github_app_installation(&state.db, &body.github_app_installation_id).await {
        Ok(Some(inst)) if inst.tenant_id == tenant_id => {}
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

    let default_branch = body
        .default_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_string();

    let project = Project {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.clone(),
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

/// GET /api/tenants/:id/projects — List projects in a workspace.
pub async fn list_projects(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(tenant_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(r) = require_tenant_member(&state.db, user_id, &tenant_id).await {
        return r;
    }
    match db::list_projects_for_tenant(&state.db, &tenant_id).await {
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
/// not members of the owning tenant.
pub async fn get_project(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(project_id): Path<String>,
) -> Response {
    let user_id = match require_auth_user(&auth_user) {
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
    if let Err(r) = require_tenant_member(&state.db, user_id, &project.tenant_id).await {
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
    let user_id = match require_auth_user(&auth_user) {
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
    if let Err(r) = require_tenant_member(&state.db, user_id, &project.tenant_id).await {
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
/// semantics when creating a session scoped to a project.
#[allow(clippy::result_large_err)]
pub async fn assert_tenant_member(
    pool: &AnyPool,
    user_id: &str,
    tenant_id: &str,
) -> Result<(), Response> {
    require_tenant_member(pool, user_id, tenant_id).await
}
