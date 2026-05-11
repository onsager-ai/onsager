//! MCP diagnostic tools — read paths that AI clients need to navigate
//! failed runs without dead-ending in "something went wrong" (ADR
//! 0007's first-class diagnostic-surface commitment).
//!
//! Three real tools plus a v1 stub:
//!
//! - `inspect_run` — structured snapshot of an artifact's current
//!   state plus recent spine events touching it.
//! - `get_stage_logs` — ordered log chunks from an agent-session
//!   stage's `sessions` row.
//! - `propose_remediation` — v1 stub: returns the same data
//!   `inspect_run` and `get_stage_logs` would, in a single envelope,
//!   with `suggested_next_tools` pointing the client AI at the next
//!   step. Server-side AI reasoning is deferred to a follow-up.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;

use crate::auth::AuthUser;
use crate::session_db;
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

// =============================================================================
// inspect_run
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InspectRunArgs {
    /// Artifact id flowing through the workflow (one artifact == one
    /// run).
    pub artifact_id: String,
    /// Max recent spine events to include. Clamped to `[1, 200]`.
    /// Defaults to 50.
    #[serde(default)]
    pub event_limit: Option<i64>,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct InspectArtifactRow {
    artifact_id: String,
    workspace_id: String,
    kind: String,
    state: String,
    workflow_id: Option<String>,
    current_stage_index: Option<i32>,
    workflow_parked_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow, serde::Serialize)]
struct RecentEventRow {
    id: i64,
    namespace: String,
    event_type: String,
    actor: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn inspect_run(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: InspectRunArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid inspect_run args: {e}")))?;

    let spine = state.spine.pool();
    let row = sqlx::query_as::<_, InspectArtifactRow>(
        "SELECT artifact_id, workspace_id, kind, state, workflow_id, \
                current_stage_index, workflow_parked_reason, created_at, updated_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&args.artifact_id)
    .fetch_optional(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp inspect_run artifact lookup failed: {e}");
        ToolError::Internal(format!("artifact lookup failed: {e}"))
    })?;

    let Some(row) = row else {
        return Err(ToolError::NotFound(format!(
            "artifact `{}` not found",
            args.artifact_id
        )));
    };
    require_workspace_access(&state.pool, auth_user, &row.workspace_id).await?;

    let limit = args.event_limit.unwrap_or(50).clamp(1, 200);
    let events = sqlx::query_as::<_, RecentEventRow>(
        "SELECT id, namespace, event_type, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext WHERE stream_id = $1 \
         ORDER BY id DESC LIMIT $2",
    )
    .bind(&row.artifact_id)
    .bind(limit)
    .fetch_all(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp inspect_run events query failed: {e}");
        ToolError::Internal(format!("failed to query events: {e}"))
    })?;

    Ok(serde_json::json!({
        "artifact": row,
        "recent_events": events,
    }))
}

// =============================================================================
// get_stage_logs
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetStageLogsArgs {
    /// Session id for the agent-session stage. `inspect_run` surfaces
    /// these via `stiglab.session_*` events on the artifact stream.
    pub session_id: String,
    /// Skip chunks with seq <= `since_seq`. Defaults to 0 (return
    /// everything).
    #[serde(default)]
    pub since_seq: Option<i64>,
}

pub async fn get_stage_logs(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: GetStageLogsArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid get_stage_logs args: {e}")))?;

    // Authorize: walk session → workspace → membership. Sessions
    // without a workspace fall back to owner-equals-caller (legacy
    // personal sessions); MCP requires workspace-scoped sessions to
    // keep the auth check uniform.
    let workspace_id = session_db::get_session_workspace(&state.pool, &args.session_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs session lookup failed: {e}");
            ToolError::Internal(format!("session lookup failed: {e}"))
        })?;
    let Some(workspace_id) = workspace_id else {
        return Err(ToolError::NotFound(format!(
            "session `{}` not found or not workspace-scoped",
            args.session_id
        )));
    };
    require_workspace_access(&state.pool, auth_user, &workspace_id).await?;

    let session = session_db::get_session(&state.pool, &args.session_id)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs get_session failed: {e}");
            ToolError::Internal(format!("session lookup failed: {e}"))
        })?
        .ok_or_else(|| ToolError::NotFound(format!("session `{}` not found", args.session_id)))?;

    let since = args.since_seq.unwrap_or(0);
    let chunks = session_db::get_session_logs_after(&state.pool, &args.session_id, since)
        .await
        .map_err(|e| {
            tracing::error!("mcp get_stage_logs chunk query failed: {e}");
            ToolError::Internal(format!("log chunk query failed: {e}"))
        })?;

    let chunks_json: Vec<Value> = chunks
        .iter()
        .map(|c| {
            serde_json::json!({
                "seq": c.seq,
                "stream": c.stream,
                "chunk": c.chunk,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "session_id": session.id,
        "state": session.state,
        "chunks": chunks_json,
    }))
}

// =============================================================================
// propose_remediation (v1 stub)
// =============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeRemediationArgs {
    /// Artifact id for the failed (or stuck) run.
    pub artifact_id: String,
}

/// Stub remediation tool — returns the same shape `inspect_run` does,
/// plus pointers to log-bearing sessions and a fixed
/// `suggested_next_tools` list. The *client-side* AI reads this and
/// decides what to do; the Full version (server-side prompt design,
/// Anthropic SDK call, cost model) is filed as a follow-up to #288.
///
/// This is intentionally not "AI on the server with no prompt design"
/// — that would lock in cost + provider choices that deserve their own
/// conversation. The stub keeps the architecture honest (every tool
/// in the registry is reachable; the diagnostic surface is complete)
/// without those locks.
pub async fn propose_remediation(
    state: &AppState,
    auth_user: &AuthUser,
    args: Value,
) -> ToolResult {
    let args: ProposeRemediationArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid propose_remediation args: {e}")))?;

    // Reuse inspect_run's payload as the failure_summary. The client AI
    // chains `get_stage_logs` next via the surfaced session_ids.
    let summary = inspect_run(
        state,
        auth_user,
        serde_json::json!({
            "artifact_id": args.artifact_id,
        }),
    )
    .await?;

    // Pull log-pointers (session_ids) from `stiglab.session_*` events
    // on the artifact stream. The client AI uses these as the input to
    // `get_stage_logs`.
    let spine = state.spine.pool();
    #[derive(FromRow)]
    struct SessionPointer {
        session_id: Option<String>,
        event_type: String,
        created_at: chrono::DateTime<chrono::Utc>,
    }
    let pointers = sqlx::query_as::<_, SessionPointer>(
        "SELECT data->>'session_id' AS session_id, event_type, created_at \
         FROM events_ext \
         WHERE stream_id = $1 AND namespace = 'stiglab' \
         ORDER BY id DESC LIMIT 20",
    )
    .bind(&args.artifact_id)
    .fetch_all(spine)
    .await
    .map_err(|e| {
        tracing::error!("mcp propose_remediation pointers query failed: {e}");
        ToolError::Internal(format!("failed to query session pointers: {e}"))
    })?;

    let log_pointers: Vec<Value> = pointers
        .into_iter()
        .filter(|p| p.session_id.is_some())
        .map(|p| {
            serde_json::json!({
                "session_id": p.session_id,
                "event_type": p.event_type,
                "created_at": p.created_at,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "v1_stub": true,
        "stub_reason": "v1 returns log pointers; server-side AI reasoning is a follow-up. See #288 Notes.",
        "failure_summary": summary,
        "log_pointers": log_pointers,
        "suggested_next_tools": [
            "get_stage_logs",
            "get_artifact",
            "inspect_run",
        ],
    }))
}
