//! MCP tool implementations, grouped by domain.
//!
//! Each submodule owns a slice of the tool registry (`super::registry`)
//! and the argument types those tools consume. Argument types derive
//! `JsonSchema` so the registry can hand the schema back unchanged in
//! `tools/list` — there is no hand-written JSON Schema in the tree.
//!
//! Tools delegate to the same DB helpers (`workflow_db`,
//! `workspace_db`, `session_db`, `spine`) that the REST handlers use.
//! No new business logic — the tool surface is a typed wrapper around
//! existing capabilities.

use sqlx::PgPool;

use crate::auth::AuthUser;
use crate::workspace_db;

use super::ToolError;

pub mod artifacts;
pub mod diagnostics;
pub mod runs;
pub mod workflows;

/// MCP-side workspace authorization. Mirrors
/// `handlers::workspaces::require_workspace_access` but returns a
/// `ToolError` instead of an axum `Response`.
///
/// Two checks, in order:
///
/// 1. PAT scope: if the principal is a PAT pinned to a workspace, the
///    request must target that exact workspace.
/// 2. Membership: caller must be a member; non-members get a flat
///    "not found" so workspace IDs aren't enumerable.
pub(crate) async fn require_workspace_access(
    pool: &PgPool,
    auth_user: &AuthUser,
    workspace_id: &str,
) -> Result<(), ToolError> {
    if let Some(pinned) = auth_user.principal.pinned_workspace_id()
        && pinned != workspace_id
    {
        return Err(ToolError::Forbidden(
            "PAT is pinned to a different workspace".into(),
        ));
    }
    match workspace_db::is_workspace_member(pool, workspace_id, &auth_user.user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(ToolError::NotFound("workspace not found".into())),
        Err(e) => {
            tracing::error!("mcp workspace membership check failed: {e}");
            Err(ToolError::Internal("workspace lookup failed".into()))
        }
    }
}
