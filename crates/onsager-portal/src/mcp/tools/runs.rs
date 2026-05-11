//! MCP tools that list and cancel workflow runs.
//!
//! A "run" is one artifact flowing through a workflow's stage chain
//! (the same shape `GET /api/workflows/:id/runs` projects). `cancel_run`
//! emits a `workflow.cancel_requested` event on the spine — same shape
//! REST's `POST /api/spine/artifacts/:id/abort` produces — and forge's
//! abort listener consumes it on the next tick.

use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;

use crate::auth::AuthUser;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;
use super::workflows::load_workflow;

// =============================================================================
// list_runs
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRunsArgs {
    pub workflow_id: String,
    /// Max runs to return; clamped to `[1, 500]`. Defaults to 50.
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct RunRow {
    artifact_id: String,
    workflow_id: String,
    state: String,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_runs(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: ListRunsArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid list_runs args: {e}")))?;
    let workflow = load_workflow(state, &args.workflow_id).await?;
    require_workspace_access(&state.pool, auth_user, &workflow.workspace_id).await?;

    let limit = args.limit.unwrap_or(50).clamp(1, 500);
    let spine = state.spine.pool();
    let rows = sqlx::query_as::<_, RunRow>(
        "SELECT artifact_id, workflow_id, state, current_stage_index, \
                workflow_parked_reason, created_at, updated_at \
         FROM artifacts \
         WHERE workflow_id = $1 \
         ORDER BY updated_at DESC \
         LIMIT $2",
    )
    .bind(&workflow.id)
    .bind(limit)
    .fetch_all(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp list_runs query failed: {e}");
        ToolError::Internal(format!("failed to query runs: {e}"))
    })?;

    let runs: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let status = if r.state == "released" {
                "passed"
            } else if r.state == "archived" {
                "failed"
            } else if r.workflow_parked_reason.is_some() {
                "blocked"
            } else {
                "pending"
            };
            serde_json::json!({
                "id": r.artifact_id,
                "workflow_id": r.workflow_id,
                "artifact_id": r.artifact_id,
                "status": status,
                "current_stage_index": r.current_stage_index,
                "parked_reason": r.workflow_parked_reason,
                "started_at": r.created_at,
                "updated_at": r.updated_at,
            })
        })
        .collect();

    Ok(serde_json::json!({ "runs": runs }))
}

// =============================================================================
// cancel_run
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CancelRunArgs {
    /// Artifact id flowing through the workflow (one artifact == one
    /// run, per the workflow runtime).
    pub artifact_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn cancel_run(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: CancelRunArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid cancel_run args: {e}")))?;

    // Look up the artifact's workspace and authorize.
    let spine = state.spine.pool();
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT workspace_id, workflow_id FROM artifacts WHERE artifact_id = $1")
            .bind(&args.artifact_id)
            .fetch_optional(spine)
            .await
            .map_err(|e| {
                tracing::error!("mcp cancel_run artifact lookup failed: {e}");
                ToolError::Internal(format!("artifact lookup failed: {e}"))
            })?;

    let Some((workspace_id, workflow_id)) = row else {
        return Err(ToolError::NotFound(format!(
            "artifact `{}` not found",
            args.artifact_id
        )));
    };
    require_workspace_access(&state.pool, auth_user, &workspace_id).await?;

    let now = Utc::now();
    let payload = serde_json::json!({
        "artifact_id": args.artifact_id,
        "workflow_id": workflow_id,
        "workspace_id": workspace_id,
        "reason": args.reason.clone().unwrap_or_else(|| "cancelled via MCP".into()),
        "actor": auth_user.user_id,
        "requested_at": now,
    });

    let metadata = onsager_spine::EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: auth_user.user_id.clone(),
    };
    let event_id = state
        .spine
        .append_ext(
            &workspace_id,
            &args.artifact_id,
            "artifact",
            "artifact.abort_requested",
            payload,
            &metadata,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("mcp cancel_run emit failed: {e}");
            ToolError::Internal(format!("failed to emit abort: {e}"))
        })?;

    Ok(serde_json::json!({
        "artifact_id": args.artifact_id,
        "event_id": event_id,
        "requested_at": now,
    }))
}
