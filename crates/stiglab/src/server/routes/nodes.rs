use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::server::auth::AuthUser;
use crate::server::db;
use crate::server::routes::sessions::WorkspaceQuery;
use crate::server::state::AppState;

use super::require_workspace_access;

/// GET /api/nodes?workspace=W — list connected agent nodes.
///
/// Nodes are global infrastructure (one set of physical agents serves
/// every workspace), but the list endpoint still requires `?workspace=`
/// so the API surface follows one filter pattern (#164). The
/// membership check 404s callers that aren't members of the requested
/// workspace, so node IDs don't leak across workspaces even though the
/// underlying registry is shared.
pub async fn list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    let workspace_id = q.workspace.trim();
    if workspace_id.is_empty() {
        return crate::server::routes::sessions::missing_workspace();
    }
    if auth_user.user_id != "anonymous" {
        if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
            return r;
        }
    }
    match db::list_nodes(&state.db).await {
        Ok(nodes) => Json(serde_json::json!({ "nodes": nodes })).into_response(),
        Err(e) => {
            tracing::error!("failed to list nodes: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
