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
use ts_rs::TS;

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

/// Stage / run status the dashboard renders.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum StageRunStatus {
    Pending,
    Blocked,
    Passed,
    Failed,
}

/// Per-stage projection of a workflow run.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct WorkflowRunStage {
    pub stage_id: String,
    pub status: StageRunStatus,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One execution of a workflow against an artifact. The dashboard treats
/// `id` and `artifact_id` interchangeably — a run is the artifact's
/// flow through its workflow.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub artifact_id: String,
    pub status: StageRunStatus,
    pub stages: Vec<WorkflowRunStage>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Stage-status projection rules shared by both `runs.rs::get_run` and
/// `workflows.rs::list_workflow_runs`. The two call sites carry different
/// row shapes (the per-run row has `workflow_id: Option<String>`, the
/// per-workflow row has it as `String`), but the status math is identical.
pub(crate) fn project_run_stages(
    state: &str,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<&str>,
    updated_at: chrono::DateTime<chrono::Utc>,
    stages: &[crate::workflow::WorkflowStage],
) -> (StageRunStatus, Vec<WorkflowRunStage>) {
    let current_idx = current_stage_index.and_then(|i| usize::try_from(i).ok());
    let archived = state == "archived";
    let released = state == "released";
    let parked = workflow_parked_reason.is_some();

    let stage_entries: Vec<WorkflowRunStage> = stages
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let status = match (released, archived, parked, current_idx) {
                (true, _, _, _) => StageRunStatus::Passed,
                (_, true, _, Some(idx)) if i < idx => StageRunStatus::Passed,
                (_, true, _, Some(idx)) if i == idx => StageRunStatus::Failed,
                (_, true, _, _) => StageRunStatus::Pending,
                (_, _, true, Some(idx)) if i < idx => StageRunStatus::Passed,
                (_, _, true, Some(idx)) if i == idx => StageRunStatus::Blocked,
                (_, _, _, Some(idx)) if i < idx => StageRunStatus::Passed,
                _ => StageRunStatus::Pending,
            };
            WorkflowRunStage {
                stage_id: s.id.clone(),
                status,
                updated_at,
            }
        })
        .collect();

    let run_status = if released {
        StageRunStatus::Passed
    } else if archived {
        StageRunStatus::Failed
    } else if parked {
        StageRunStatus::Blocked
    } else {
        StageRunStatus::Pending
    };

    (run_status, stage_entries)
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

    let (status, stage_entries) = project_run_stages(
        &row.state,
        row.current_stage_index,
        row.workflow_parked_reason.as_deref(),
        row.updated_at,
        &stages,
    );
    let run = WorkflowRun {
        id: row.artifact_id.clone(),
        workflow_id,
        artifact_id: row.artifact_id.clone(),
        status,
        stages: stage_entries,
        started_at: row.created_at,
        updated_at: row.updated_at,
    };
    Json(serde_json::json!({
        "run": run,
        "workflow": workflow,
        "stages": stages,
        "sessions": linked_sessions,
    }))
    .into_response()
}
