pub mod auth;
pub mod credentials;
pub mod governance;
pub mod health;
pub mod nodes;
pub mod pats;
pub mod portal;
pub mod projects;
pub mod registry_events;
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
/// Two checks, in order:
///
/// 1. **PAT scope (403 on mismatch).** If the principal is a PAT pinned to
///    a workspace, the request must target that exact workspace. The
///    caller already proved membership at PAT-mint time, so hiding the
///    workspace's existence is moot — surface `pat_workspace_scope_mismatch`
///    so client tooling can react.
/// 2. **Membership (404 on miss).** Otherwise the caller must be a member
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
