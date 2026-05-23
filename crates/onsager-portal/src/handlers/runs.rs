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
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
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
    let artifact_id = row.artifact_id;
    let run = WorkflowRun {
        id: artifact_id.clone(),
        workflow_id,
        artifact_id,
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

/// Status filter for `GET /api/workspaces/:id/runs`. Mirrors
/// `StageRunStatus` — the projection above is total, so each variant
/// has a complementary SQL predicate against `state` +
/// `workflow_parked_reason`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatusFilter {
    Pending,
    Blocked,
    Passed,
    Failed,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRunsQuery {
    #[serde(default)]
    pub workflow_id: Option<String>,
    #[serde(default)]
    pub status: Option<RunStatusFilter>,
    /// Inclusive lower bound on `updated_at` (ISO-8601). Filters at the
    /// SQL layer alongside `until`; ordering still uses `updated_at`.
    #[serde(default)]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    /// Inclusive upper bound on `updated_at` (ISO-8601).
    #[serde(default)]
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct WorkspaceRunRow {
    pub artifact_id: String,
    pub workflow_id: String,
    pub state: String,
    pub current_stage_index: Option<i32>,
    pub workflow_parked_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// SQL half of `list_workspace_runs`. Pulled out so integration tests
/// can exercise the `status` / `since` / `until` filter combinations
/// without spinning up the Axum app — the projection layer above is
/// already covered by the per-stage status table elsewhere.
pub async fn fetch_workspace_run_rows(
    spine: &sqlx::PgPool,
    workspace_id: &str,
    q: &WorkspaceRunsQuery,
) -> Result<Vec<WorkspaceRunRow>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT artifact_id, workflow_id, state, current_stage_index, \
                workflow_parked_reason, created_at, updated_at \
         FROM artifacts WHERE workflow_id IS NOT NULL AND workspace_id = ",
    );
    qb.push_bind(workspace_id);
    if let Some(wf_id) = q.workflow_id.as_deref() {
        qb.push(" AND workflow_id = ").push_bind(wf_id);
    }
    match q.status {
        Some(RunStatusFilter::Passed) => {
            qb.push(" AND state = 'released'");
        }
        Some(RunStatusFilter::Failed) => {
            qb.push(" AND state = 'archived'");
        }
        Some(RunStatusFilter::Blocked) => {
            qb.push(
                " AND state NOT IN ('released', 'archived') \
                  AND workflow_parked_reason IS NOT NULL",
            );
        }
        Some(RunStatusFilter::Pending) => {
            qb.push(
                " AND state NOT IN ('released', 'archived') \
                  AND workflow_parked_reason IS NULL",
            );
        }
        None => {}
    }
    if let Some(since) = q.since {
        qb.push(" AND updated_at >= ").push_bind(since);
    }
    if let Some(until) = q.until {
        qb.push(" AND updated_at <= ").push_bind(until);
    }
    qb.push(" ORDER BY updated_at DESC LIMIT ").push_bind(limit);
    qb.build_query_as::<WorkspaceRunRow>()
        .fetch_all(spine)
        .await
}

/// GET /api/workspaces/:id/runs — workspace-scoped cross-workflow runs
/// list. Each row is one artifact flowing through a workflow, projected
/// the same way as `GET /api/workflows/:id/runs`. Optional query params:
/// `workflow_id` (scope to a single workflow), `status`
/// (pending | blocked | passed | failed — filtered at the SQL layer
/// rather than post-projection), `since` / `until` (ISO-8601 bounds on
/// `updated_at`).
pub async fn list_workspace_runs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workspace_id): Path<String>,
    Query(q): Query<WorkspaceRunsQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    let spine = state.spine.pool();

    let rows = match fetch_workspace_run_rows(spine, &workspace_id, &q).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to list workspace runs: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load runs" })),
            )
                .into_response();
        }
    };

    // Group rows by workflow_id so we load each workflow's stage chain
    // once, then zip the projection per-row. Single-workflow filter case
    // skips the loop overhead but still falls through this path.
    let mut stages_by_workflow: std::collections::HashMap<
        String,
        Vec<crate::workflow::WorkflowStage>,
    > = std::collections::HashMap::new();
    let mut runs: Vec<WorkflowRun> = Vec::with_capacity(rows.len());
    for row in rows {
        let stages = match stages_by_workflow.get(&row.workflow_id) {
            Some(s) => s.clone(),
            None => {
                match crate::workflow_db::list_stages_for_workflow(spine, &row.workflow_id).await {
                    Ok(s) => {
                        stages_by_workflow.insert(row.workflow_id.clone(), s.clone());
                        s
                    }
                    Err(e) => {
                        tracing::error!("failed to load stages for {}: {e}", row.workflow_id);
                        Vec::new()
                    }
                }
            }
        };
        let (status, stage_entries) = project_run_stages(
            &row.state,
            row.current_stage_index,
            row.workflow_parked_reason.as_deref(),
            row.updated_at,
            &stages,
        );
        runs.push(WorkflowRun {
            id: row.artifact_id.clone(),
            workflow_id: row.workflow_id,
            artifact_id: row.artifact_id,
            status,
            stages: stage_entries,
            started_at: row.created_at,
            updated_at: row.updated_at,
        });
    }
    Json(serde_json::json!({ "runs": runs })).into_response()
}
