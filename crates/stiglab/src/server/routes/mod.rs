pub mod auth;
pub mod credentials;
pub mod governance;
pub mod health;
pub mod nodes;
pub mod pats;
pub mod portal;
pub mod projects;
pub mod sessions;
pub mod shaping;
pub mod spine;
pub mod tasks;
pub mod webhooks;
pub mod workflow_kinds;
pub mod workflows;
pub mod workspaces;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use sqlx::AnyPool;

use crate::server::auth::AuthUser;
use crate::server::db;

/// Authorize an `AuthUser` against a target workspace.
///
/// Three checks, in order:
///
/// 1. **Anonymous bypass.** When auth is disabled the extractor returns
///    a synthetic `anonymous` principal that will never appear in
///    `workspace_members`.  Spec #161 states that `authEnabled=false`
///    mode stays unchanged, so we treat the anonymous principal as
///    blanket-authorized.  Auth-enabled deployments keep the strict
///    checks below.
/// 2. **PAT scope (403 on mismatch).** If the principal is a PAT pinned to
///    a workspace, the request must target that exact workspace. The
///    caller already proved membership at PAT-mint time, so hiding the
///    workspace's existence is moot — surface `pat_workspace_scope_mismatch`
///    so client tooling can react.
/// 3. **Membership (404 on miss).** Otherwise the caller must be a member
///    of the workspace; non-members get a flat 404 to avoid leaking
///    workspace existence.
///
/// Every workspace-scoped route in `crates/stiglab/src/server/routes/`
/// must funnel through this helper — defining a local "just check
/// membership" version skips the PAT scope check and lets a workspace-
/// pinned PAT touch any workspace its user happens to belong to.
#[allow(clippy::result_large_err)]
pub async fn require_workspace_access(
    pool: &AnyPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), Response> {
    if workspace_id.trim().is_empty() {
        return Err(missing_workspace_query());
    }
    if auth_user.user_id == "anonymous" {
        return Ok(());
    }
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

/// 400 response for list endpoints called without `?workspace=`.
///
/// Per #164 the list-endpoint contract is "explicit workspace, always".  A
/// missing or blank `?workspace=` is a client bug, not a "default to
/// everything" — that was the parent #161 leak shape.  This helper keeps
/// the response body shape uniform across every list route so the
/// dashboard can match on `error == "missing_workspace_query"` without a
/// per-route table.
pub fn missing_workspace_query() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "missing_workspace_query",
            "detail": "?workspace= is required",
        })),
    )
        .into_response()
}

/// Standard 404 for "this row exists but you can't see it".  Defined here
/// so detail routes (`GET/PATCH/DELETE /api/.../:id`) emit the same shape
/// when the helper above isn't directly callable (e.g. when the row needs
/// to load *first* and *then* be authz'd against its `workspace_id`).
pub fn workspace_scoped_not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}
