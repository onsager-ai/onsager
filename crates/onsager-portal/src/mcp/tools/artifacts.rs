//! MCP tools that read spine artifacts.
//!
//! Mirrors `GET /api/spine/artifacts/:id` minus the full
//! version+lineage payload — the diagnostic flow that consumes this
//! tool only needs the artifact row itself; deeper inspection can come
//! through `inspect_run` or REST.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;

use crate::auth::AuthUser;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetArtifactArgs {
    pub artifact_id: String,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct ArtifactRow {
    id: String,
    workspace_id: String,
    kind: String,
    name: Option<String>,
    state: String,
    owner: Option<String>,
    current_version: i32,
    consumers: Value,
    external_ref: Option<String>,
    workflow_id: Option<String>,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn get_artifact(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetArtifactArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_artifact args: {e}")))?;

    let spine = state.spine.pool();
    let row = sqlx::query_as::<_, ArtifactRow>(
        "SELECT artifact_id AS id, workspace_id, kind, name, state, owner, current_version, \
                consumers, external_ref, workflow_id, current_stage_index, \
                workflow_parked_reason, created_at, updated_at, last_observed_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&args.artifact_id)
    .fetch_optional(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp get_artifact query failed: {e}");
        ToolError::Internal(format!("failed to query artifact: {e}"))
    })?;

    let Some(row) = row else {
        return Err(ToolError::NotFound(format!(
            "artifact `{}` not found",
            args.artifact_id
        )));
    };

    // 404 on workspace mismatch — flat shape, no workspace existence leak.
    if let Err(err) = require_workspace_access(&state.pool, auth_user, &row.workspace_id).await
        && matches!(err, ToolError::NotFound(_))
    {
        return Err(ToolError::NotFound(format!(
            "artifact `{}` not found",
            args.artifact_id
        )));
    }

    let value = serde_json::to_value(&row)
        .map_err(|e| ToolError::Internal(format!("response serialization failed: {e}")))?;
    Ok(serde_json::json!({ "artifact": value }))
}
