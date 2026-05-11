//! Run-detail route (spec #303).
//!
//! `GET /api/runs/:id` is the hub endpoint for the dashboard's
//! `RunDetailPage`. A "run" is one artifact flowing through a workflow:
//! `runs.id == artifacts.artifact_id`. The shape combines:
//!
//! - the projected run (status + per-stage status), reusing the same
//!   projection helper that `GET /api/workflows/:id/runs` returns, and
//! - the parent `Workflow` (with its ordered stages), so the frontend
//!   doesn't need a second roundtrip to render stage names / gate kinds, and
//! - linked agent `Session` ids (the `sessions` table joins back via
//!   `artifact_id`), so the Stages tab can deep-link into session logs.
//!
//! Auth: workspace-scoped through the artifact's `workspace_id`, mirroring
//! `handlers/workflows.rs` (non-members get a flat 404).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::AuthUser;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;
use crate::workflow_db;

#[derive(Debug, sqlx::FromRow)]
struct RunRow {
    artifact_id: String,
    workspace_id: String,
    workflow_id: Option<String>,
    state: String,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct LinkedSession {
    id: String,
    state: String,
    node_id: String,
    created_at: String,
    updated_at: String,
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

/// GET /api/runs/:id — combined run + workflow + linked sessions view.
pub async fn get_run(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(run_id): Path<String>,
) -> Response {
    let spine = state.spine.pool();

    let row = match sqlx::query_as::<_, RunRow>(
        "SELECT artifact_id, workspace_id, workflow_id, state, current_stage_index, \
                workflow_parked_reason, created_at, updated_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&run_id)
    .fetch_optional(spine)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("run not found"),
        Err(e) => {
            tracing::error!("failed to load run row: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load run" })),
            )
                .into_response();
        }
    };

    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &row.workspace_id).await {
        return r;
    }

    let workflow_id = match row.workflow_id.clone() {
        Some(id) => id,
        // Artifacts without a workflow_id are not "runs" — the runs index
        // only surfaces artifacts that are flowing through a workflow.
        None => return not_found("run is not associated with a workflow"),
    };

    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    let stages = match workflow_db::list_stages_for_workflow(spine, &workflow_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to load stages: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let linked_sessions: Vec<LinkedSession> = match sqlx::query_as::<_, LinkedSession>(
        "SELECT id, state, node_id, created_at, updated_at FROM sessions \
         WHERE artifact_id = $1 ORDER BY created_at ASC",
    )
    .bind(&run_id)
    .fetch_all(&state.pool)
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to load linked sessions: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load linked sessions" })),
            )
                .into_response();
        }
    };

    let run = project_run(&row, &stages);
    Json(serde_json::json!({
        "run": run,
        "workflow": workflow,
        "stages": stages,
        "sessions": linked_sessions,
    }))
    .into_response()
}

/// Same projection rules as `handlers::workflows::project_run`. Kept
/// inline because the source struct shape (`RunRow` vs `ArtifactRunRow`)
/// differs by one nullable field; collapsing them into a shared helper
/// would force the workflow handler to handle `Option<String>` for the
/// `workflow_id` it always has.
fn project_run(row: &RunRow, stages: &[crate::workflow::WorkflowStage]) -> serde_json::Value {
    let current_idx = row
        .current_stage_index
        .and_then(|i| usize::try_from(i).ok());

    let archived = row.state == "archived";
    let released = row.state == "released";
    let parked = row.workflow_parked_reason.is_some();

    let stage_entries: Vec<serde_json::Value> = stages
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let status = match (released, archived, parked, current_idx) {
                (true, _, _, _) => "passed",
                (_, true, _, Some(idx)) if i < idx => "passed",
                (_, true, _, Some(idx)) if i == idx => "failed",
                (_, true, _, _) => "pending",
                (_, _, true, Some(idx)) if i < idx => "passed",
                (_, _, true, Some(idx)) if i == idx => "blocked",
                (_, _, _, Some(idx)) if i < idx => "passed",
                _ => "pending",
            };
            serde_json::json!({
                "stage_id": s.id,
                "status": status,
                "updated_at": row.updated_at,
            })
        })
        .collect();

    let run_status = if released {
        "passed"
    } else if archived {
        "failed"
    } else if parked {
        "blocked"
    } else {
        "pending"
    };

    serde_json::json!({
        "id": row.artifact_id,
        "workflow_id": row.workflow_id,
        "artifact_id": row.artifact_id,
        "status": run_status,
        "stages": stage_entries,
        "started_at": row.created_at,
        "updated_at": row.updated_at,
    })
}
