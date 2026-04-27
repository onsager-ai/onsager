//! Node listing — `GET /api/nodes` (issue #59 / #164).
//!
//! Nodes are platform-wide infrastructure (the agent fleet), not
//! workspace data per se, but the listing endpoint still requires
//! `?workspace=` (issue #164) so:
//!   1. The authz check funnels through the same helper as every other
//!      list endpoint (no per-route variation), and
//!   2. PAT scope (`pat_workspace_scope_mismatch`) and membership (404)
//!      are enforced uniformly — a PAT pinned to W1 cannot use a
//!      workspace W2 the user is also a member of as a back door to the
//!      node fleet.
//!
//! The result set is unfiltered (every member of any workspace sees the
//! same fleet status) — node capacity is shared, not partitioned.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::server::auth::AuthUser;
use crate::server::db;
use crate::server::state::AppState;

use super::require_workspace_access;

#[derive(Debug, Deserialize)]
pub struct ListNodesQuery {
    #[serde(default)]
    pub workspace: Option<String>,
}

pub async fn list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ListNodesQuery>,
) -> impl IntoResponse {
    let workspace_id = match q.workspace.as_deref() {
        Some(w) if !w.trim().is_empty() => w,
        _ => return super::missing_workspace_query(),
    };
    if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
        return r;
    }
    match db::list_nodes(&state.db).await {
        Ok(nodes) => Json(serde_json::json!({ "nodes": nodes })).into_response(),
        Err(e) => {
            tracing::error!("failed to list nodes: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
