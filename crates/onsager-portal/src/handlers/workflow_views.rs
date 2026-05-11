//! Workflow-scoped read views (spec #302 — #289 PR 2b).
//!
//! Dedicated list endpoints that replace the dashboard's derived
//! client-side filters: a workflow's artifacts (formerly a per-run
//! artifact fan-out from `GET /api/spine/artifacts/:id`) and verdicts
//! (formerly the workspace-wide `GET /api/governance/events` filtered
//! on the client). One round-trip per tab, server-side workspace
//! gating, and `check-api-contract`-clean.
//!
//! Same workspace-access model as `handlers/workflows.rs` — caller must
//! be a member of the workflow's workspace (404 otherwise).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;
use crate::workflow_db;

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Default page size + hard cap for the workflow-scoped list endpoints.
/// Matches `GET /api/workflows/:id/runs` so the three workflow tabs poll
/// the same shape.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 500)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct WorkflowArtifactRow {
    id: String,
    kind: String,
    name: Option<String>,
    state: String,
    owner: Option<String>,
    current_version: i32,
    consumers: serde_json::Value,
    external_ref: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /api/workflows/:id/artifacts — artifacts produced by runs of
/// this workflow. Same `SpineArtifact` shape as
/// `GET /api/spine/artifacts`, filtered by `artifacts.workflow_id`.
///
/// `?limit=` (default 50, clamped to 500) keeps the polled response
/// bounded as a workflow accumulates artifacts.
pub async fn list_workflow_artifacts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let spine = state.spine.pool();
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }

    let limit = clamp_limit(q.limit);
    match sqlx::query_as::<_, WorkflowArtifactRow>(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, \
                consumers, external_ref, created_at, updated_at, last_observed_at \
         FROM artifacts \
         WHERE workflow_id = $1 \
         ORDER BY updated_at DESC \
         LIMIT $2",
    )
    .bind(&workflow_id)
    .bind(limit)
    .fetch_all(spine)
    .await
    {
        Ok(artifacts) => Json(serde_json::json!({ "artifacts": artifacts })).into_response(),
        Err(e) => {
            tracing::error!("failed to load workflow artifacts: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifacts" })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct WorkflowVerdictRow {
    id: String,
    event_type: String,
    title: String,
    severity: String,
    source: String,
    metadata: serde_json::Value,
    resolved: bool,
    resolution_notes: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /api/workflows/:id/verdicts — governance verdicts on this
/// workflow's runs. Filters `governance_events` by
/// `metadata->>'artifact_id'` against the artifacts owned by this
/// workflow.
///
/// `?limit=` (default 50, clamped to 500) bounds the polled response;
/// the dashboard hits this every 5s when a governance stage exists.
///
/// Synodic and portal share the spine Postgres in production
/// (`deploy/docker-compose.yml`) and via the `migrate` service in
/// `just dev`, so this is a direct SQL read — same pattern as
/// portal's other shared-table reads (`artifacts`, `events_ext`,
/// `sessions`).
pub async fn list_workflow_verdicts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(workflow_id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let spine = state.spine.pool();
    let workflow = match workflow_db::get_workflow(spine, &workflow_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return not_found("workflow not found"),
        Err(e) => {
            tracing::error!("failed to load workflow: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workflow.workspace_id).await
    {
        return r;
    }

    let limit = clamp_limit(q.limit);
    match sqlx::query_as::<_, WorkflowVerdictRow>(
        "SELECT id, event_type, title, severity, source, metadata, resolved, \
                resolution_notes, created_at, resolved_at \
         FROM governance_events \
         WHERE metadata->>'artifact_id' IN ( \
             SELECT artifact_id FROM artifacts WHERE workflow_id = $1 \
         ) \
         ORDER BY created_at DESC \
         LIMIT $2",
    )
    .bind(&workflow_id)
    .bind(limit)
    .fetch_all(spine)
    .await
    {
        Ok(verdicts) => Json(serde_json::json!({ "verdicts": verdicts })).into_response(),
        Err(e) => {
            tracing::error!("failed to load workflow verdicts: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load verdicts" })),
            )
                .into_response()
        }
    }
}
