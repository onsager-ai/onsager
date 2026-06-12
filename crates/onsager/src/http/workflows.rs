//! Workflow CRUD + fire, and the run/artifact read surfaces.
//!
//! A workflow is exactly what the engine executes — trigger + stage
//! list on one row, derived nowhere else (ADR 0029 / #620). Activation
//! is a flag flip: a manual trigger has nothing to verify upstream
//! (the legacy GitHub gate returns with the GitHub kinds in M2).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{AuthUser, require_workspace_access};
use crate::domain::{Artifact, Run, StageKind, TriggerKind, Workflow};
use crate::state::AppState;

fn internal(e: impl std::fmt::Display, what: &str) -> Response {
    tracing::error!("{what}: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn bad_request(msg: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg.into() })),
    )
        .into_response()
}

fn not_found(what: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": format!("{what} not found") })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace: String,
}

const WORKFLOW_COLS: &str =
    "id, workspace_id, name, trigger, definition, active, created_by, created_at, updated_at";

async fn load_workflow(state: &AppState, id: &str) -> Result<Workflow, Response> {
    let row: Option<Workflow> = sqlx::query_as(&format!(
        "SELECT {WORKFLOW_COLS} FROM workflows WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| internal(e, "load workflow"))?;
    row.ok_or_else(|| not_found("workflow"))
}

// ── Workflows ──

pub async fn list_workflows(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &q.workspace).await {
        return r;
    }
    let rows: Result<Vec<Workflow>, _> = sqlx::query_as(&format!(
        "SELECT {WORKFLOW_COLS} FROM workflows WHERE workspace_id = $1 ORDER BY created_at DESC"
    ))
    .bind(&q.workspace)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(workflows) => Json(serde_json::json!({ "workflows": workflows })).into_response(),
        Err(e) => internal(e, "list workflows"),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkflowBody {
    pub workspace_id: String,
    pub name: String,
    pub trigger: TriggerKind,
    pub definition: Vec<StageKind>,
}

pub async fn create_workflow(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateWorkflowBody>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &body.workspace_id).await {
        return r;
    }
    if body.name.trim().is_empty() {
        return bad_request("name is required");
    }
    if body.definition.is_empty() {
        return bad_request("definition needs at least one stage");
    }
    for (i, stage) in body.definition.iter().enumerate() {
        let StageKind::Agent { user_prompt, .. } = stage;
        if user_prompt.trim().is_empty() {
            return bad_request(format!("stage {i}: user_prompt is required"));
        }
    }
    let id = format!("wf_{}", Uuid::new_v4());
    let result = sqlx::query(
        "INSERT INTO workflows (id, workspace_id, name, trigger, definition, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(&body.workspace_id)
    .bind(body.name.trim())
    .bind(serde_json::to_value(&body.trigger).expect("trigger serializes"))
    .bind(serde_json::to_value(&body.definition).expect("definition serializes"))
    .bind(&user.user_id)
    .execute(&state.pool)
    .await;
    if let Err(e) = result {
        return internal(e, "create workflow");
    }
    match load_workflow(&state, &id).await {
        Ok(wf) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "workflow": wf })),
        )
            .into_response(),
        Err(r) => r,
    }
}

pub async fn get_workflow(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let wf = match load_workflow(&state, &id).await {
        Ok(wf) => wf,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &wf.workspace_id).await {
        return r;
    }
    Json(serde_json::json!({ "workflow": wf })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PatchWorkflowBody {
    pub active: Option<bool>,
}

pub async fn patch_workflow(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PatchWorkflowBody>,
) -> Response {
    let wf = match load_workflow(&state, &id).await {
        Ok(wf) => wf,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &wf.workspace_id).await {
        return r;
    }
    let Some(active) = body.active else {
        return bad_request("no patchable fields provided");
    };
    if let Err(e) =
        sqlx::query("UPDATE workflows SET active = $2, updated_at = NOW() WHERE id = $1")
            .bind(&id)
            .bind(active)
            .execute(&state.pool)
            .await
    {
        return internal(e, "patch workflow");
    }
    match load_workflow(&state, &id).await {
        Ok(wf) => Json(serde_json::json!({ "workflow": wf })).into_response(),
        Err(r) => r,
    }
}

/// `POST /api/workflows/:id/fire` — the manual trigger. The fire is a
/// function call into the in-process scheduler; the run id comes back
/// immediately.
pub async fn fire_workflow(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let wf = match load_workflow(&state, &id).await {
        Ok(wf) => wf,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &wf.workspace_id).await {
        return r;
    }
    let TriggerKind::Manual { .. } = &wf.trigger;
    if !wf.active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "workflow_inactive",
                "detail": "activate the workflow before firing",
            })),
        )
            .into_response();
    }
    match crate::scheduler::fire(&state, wf).await {
        Ok(run_id) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "run_id": run_id })),
        )
            .into_response(),
        Err(e) => internal(e, "fire workflow"),
    }
}

// ── Runs ──

pub async fn list_runs(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let wf = match load_workflow(&state, &id).await {
        Ok(wf) => wf,
        Err(r) => return r,
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &wf.workspace_id).await {
        return r;
    }
    let rows: Result<Vec<Run>, _> = sqlx::query_as(
        "SELECT id, workspace_id, workflow_id, status, error, started_at, finished_at \
         FROM runs WHERE workflow_id = $1 ORDER BY started_at DESC LIMIT 100",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(runs) => Json(serde_json::json!({ "runs": runs })).into_response(),
        Err(e) => internal(e, "list runs"),
    }
}

pub async fn get_run(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let run: Option<Run> = match sqlx::query_as(
        "SELECT id, workspace_id, workflow_id, status, error, started_at, finished_at \
         FROM runs WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return internal(e, "load run"),
    };
    let Some(run) = run else {
        return not_found("run");
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &run.workspace_id).await {
        return r;
    }
    let timeline = match crate::events::for_run(&state.pool, &id).await {
        Ok(t) => t,
        Err(e) => return internal(e, "load run timeline"),
    };
    let artifacts: Vec<Artifact> = match sqlx::query_as(
        "SELECT id, workspace_id, run_id, session_id, kind, name, content_uri, \
                content_checksum, external_ref, provenance, created_at \
         FROM artifacts WHERE run_id = $1 ORDER BY created_at",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await
    {
        Ok(a) => a,
        Err(e) => return internal(e, "load run artifacts"),
    };
    Json(serde_json::json!({ "run": run, "timeline": timeline, "artifacts": artifacts }))
        .into_response()
}

// ── Artifacts ──

pub async fn list_artifacts(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &q.workspace).await {
        return r;
    }
    let rows: Result<Vec<Artifact>, _> = sqlx::query_as(
        "SELECT id, workspace_id, run_id, session_id, kind, name, content_uri, \
                content_checksum, external_ref, provenance, created_at \
         FROM artifacts WHERE workspace_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(&q.workspace)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(artifacts) => Json(serde_json::json!({ "artifacts": artifacts })).into_response(),
        Err(e) => internal(e, "list artifacts"),
    }
}

pub async fn get_artifact(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let row: Option<Artifact> = match sqlx::query_as(
        "SELECT id, workspace_id, run_id, session_id, kind, name, content_uri, \
                content_checksum, external_ref, provenance, created_at \
         FROM artifacts WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(a) => a,
        Err(e) => return internal(e, "load artifact"),
    };
    let Some(artifact) = row else {
        return not_found("artifact");
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &artifact.workspace_id).await {
        return r;
    }
    Json(serde_json::json!({ "artifact": artifact })).into_response()
}
