//! Node list endpoint (spec #222 Follow-up 3).
//!
//! Moved from `crates/stiglab/src/server/routes/nodes.rs`.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::auth::AuthUser;
use crate::handlers::sessions::WorkspaceQuery;
use crate::handlers::sessions::missing_workspace;
use crate::handlers::workspaces::require_workspace_access;
use crate::session_db;
use crate::state::AppState;

/// GET /api/nodes?workspace=W — list connected agent nodes.
///
/// Nodes are global but the endpoint requires `?workspace=` so the API
/// surface follows one filter pattern (#164). The membership check
/// 404s non-members, preventing them from accessing the nodes listing.
pub async fn list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    let workspace_id = q.workspace.trim().to_string();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if auth_user.user_id != "anonymous" {
        if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
            return r;
        }
    }
    match session_db::list_nodes(&state.pool).await {
        Ok(nodes) => Json(serde_json::json!({ "nodes": nodes })).into_response(),
        Err(e) => {
            tracing::error!("failed to list nodes: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
