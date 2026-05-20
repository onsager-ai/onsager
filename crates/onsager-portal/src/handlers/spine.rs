//! Spine read + write API for the dashboard.
//!
//! Spec #259 (sub-issue of #222) moved this from stiglab to portal so the
//! dashboard's `API_BASE` cutover (#222 Slice 6) can eventually drop the
//! `routes::portal::proxy` shim.
//!
//! Reads land directly against the spine `events_ext` and `artifacts`
//! tables. Writes are control-only — retry/abort/override-gate emit a
//! single event via `EventStore::append_ext`, no synodic side-effects,
//! no GitHub side-effects, no row inserts. (Per #278 artifact creation
//! is exclusively forge's auto-trigger flow; portal does not own a
//! creation endpoint.) Stiglab's `SpineEmitter::emit_raw` wrapper
//! stays in stiglab for the agent-runtime emits; portal calls
//! `state.spine.append_ext` directly.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use onsager_spine::EventMetadata;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use ts_rs::TS;

use crate::auth::AuthUser;
use crate::handlers::workspaces::require_workspace_access;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Required workspace scope (#164). Missing → 400.
    pub workspace: String,
    pub stream_type: Option<String>,
    pub event_type: Option<String>,
    /// Exact-match filter on `stream_id`. Used by per-artifact and
    /// per-workflow views in the dashboard to pull just the events for
    /// one entity without scanning the whole table client-side.
    pub stream_id: Option<String>,
    /// Filter to events tied to a specific run (spec #303). A "run" id
    /// is an artifact_id; events match via any of:
    /// - `stream_id = run_id` (artifact-keyed events), or
    /// - `data->>'artifact_id' = run_id` (events that carry the artifact
    ///   id in their payload), or
    /// - `stream_id IN (SELECT id FROM sessions WHERE artifact_id = run_id)`
    ///   (session-keyed events whose session points back to the artifact).
    pub run_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct SpineEvent {
    pub id: i64,
    pub stream_id: String,
    pub stream_type: String,
    pub event_type: String,
    pub data: serde_json::Value,
    pub actor: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export)]
pub struct SpineArtifact {
    pub id: String,
    pub kind: String,
    /// Provider-authored title for the artifact. NULL for reference-only
    /// external-source artifacts (#170) — clients hydrate via the
    /// `/api/projects/:id/{issues,pulls}` proxy.
    pub name: Option<String>,
    pub state: String,
    /// Provider-authored author/login. Same NULL convention as `name`.
    pub owner: Option<String>,
    pub current_version: i32,
    pub consumers: serde_json::Value,
    /// External-system handle (e.g. `github:project:abc:issue:42`) used to
    /// join skeleton rows with live proxy responses.
    pub external_ref: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Last webhook touch — used to render "last seen N min ago" placeholders
    /// when the proxy is rate-limited (#170 fail-open).
    pub last_observed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, FromRow, TS)]
#[ts(export, rename = "ArtifactVersion")]
pub struct ArtifactVersionRow {
    pub version: i32,
    pub content_ref_uri: String,
    pub content_ref_checksum: Option<String>,
    pub change_summary: String,
    pub created_by_session: String,
    pub parent_version: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct VerticalLineageRow {
    pub version: i32,
    pub session_id: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct HorizontalLineageRow {
    pub source_artifact_id: String,
    pub source_version: i32,
    pub role: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, Default)]
pub struct RetryRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AbortRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OverrideGateRequest {
    /// `allow` (default) or `deny`.
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Standard 400 body for the `?workspace=` requirement on every
/// workspace-scoped list endpoint (#164).
fn missing_workspace() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "workspace query parameter is required",
            "detail": "every workspace-scoped list endpoint requires ?workspace=<id>",
        })),
    )
        .into_response()
}

/// Emit a single spine event, swallowing errors with a warning — every
/// caller in this module treats the emit as best-effort because the
/// row mutation has already happened (or there is none).
async fn emit(
    state: &AppState,
    workspace_id: &str,
    stream_id: &str,
    namespace: &str,
    actor: &str,
    event_type: &str,
    data: serde_json::Value,
) {
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: actor.to_string(),
    };
    if let Err(e) = state
        .spine
        .append_ext(
            workspace_id,
            stream_id,
            namespace,
            event_type,
            data,
            &metadata,
            None,
        )
        .await
    {
        tracing::warn!("failed to emit {event_type} event: {e}");
    }
}

/// GET /api/spine/events?workspace=W — query the events_ext table.
pub async fn list_events(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<EventsQuery>,
) -> Response {
    let workspace_id = params.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, workspace_id).await {
        return r;
    }

    let pool = state.spine.pool();
    let limit = params.limit.unwrap_or(50).min(500);

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext WHERE workspace_id = ",
    );
    qb.push_bind(workspace_id.to_string());
    if let Some(st) = params.stream_type.as_deref() {
        qb.push(" AND namespace = ").push_bind(st.to_string());
    }
    if let Some(et) = params.event_type.as_deref() {
        qb.push(" AND event_type = ").push_bind(et.to_string());
    }
    if let Some(sid) = params.stream_id.as_deref() {
        qb.push(" AND stream_id = ").push_bind(sid.to_string());
    }
    // run_id (spec #303): a run is an artifact, but related events may
    // be keyed by either the artifact_id (stream_id) or a session_id
    // whose session row points back to the artifact. The OR covers
    // both shapes without forcing the dashboard to issue two queries.
    if let Some(rid) = params.run_id.as_deref() {
        qb.push(" AND (stream_id = ");
        qb.push_bind(rid.to_string());
        qb.push(" OR data->>'artifact_id' = ");
        qb.push_bind(rid.to_string());
        qb.push(" OR stream_id IN (SELECT id FROM sessions WHERE artifact_id = ");
        qb.push_bind(rid.to_string());
        qb.push("))");
    }
    qb.push(" ORDER BY id DESC LIMIT ").push_bind(limit);

    match qb.build_query_as::<SpineEvent>().fetch_all(pool).await {
        Ok(events) => Json(serde_json::json!({ "events": events })).into_response(),
        Err(e) => {
            tracing::error!("spine events query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query events" })),
            )
                .into_response()
        }
    }
}

/// Filters for `GET /api/spine/artifacts`.
#[derive(Debug, Deserialize)]
pub struct ListArtifactsQuery {
    /// Required workspace scope (#164). Missing → 400.
    pub workspace: String,
    /// Filter by `kind` discriminator (e.g. `pull_request`, `github_issue`).
    pub kind: Option<String>,
    /// Filter by `metadata->>'project_id'`.
    pub project_id: Option<String>,
}

/// GET /api/spine/artifacts?workspace=W — list artifacts from the spine.
pub async fn list_artifacts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(filters): Query<ListArtifactsQuery>,
) -> Response {
    let workspace_id = filters.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, workspace_id).await {
        return r;
    }

    let pool = state.spine.pool();

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, \
                consumers, external_ref, created_at, updated_at, last_observed_at \
         FROM artifacts WHERE workspace_id = ",
    );
    qb.push_bind(workspace_id.to_string());
    if let Some(kind) = filters.kind.as_deref() {
        qb.push(" AND kind = ").push_bind(kind.to_string());
    }
    if let Some(project_id) = filters.project_id.as_deref() {
        qb.push(" AND metadata->>'project_id' = ")
            .push_bind(project_id.to_string());
    }
    qb.push(" ORDER BY updated_at DESC LIMIT 100");

    match qb.build_query_as::<SpineArtifact>().fetch_all(pool).await {
        Ok(artifacts) => Json(serde_json::json!({ "artifacts": artifacts })).into_response(),
        Err(e) => {
            tracing::error!("spine artifacts query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifacts" })),
            )
                .into_response()
        }
    }
}

/// GET /api/spine/artifacts/:id — single artifact detail with versions
/// and lineage. 404s callers who aren't members of the artifact's
/// workspace (#164) so artifact IDs can't be enumerated cross-workspace.
pub async fn get_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let pool = state.spine.pool();

    let workspace_id: String = match sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("spine artifact workspace lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifact" })),
            )
                .into_response();
        }
    };

    // 404 (not 403) on workspace mismatch via the shared helper —
    // rewrite the body to "artifact not found" so artifact IDs don't
    // leak via the workspace-not-found body.
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    let artifact = sqlx::query_as::<_, SpineArtifact>(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, \
         external_ref, created_at, updated_at, last_observed_at \
         FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await;

    let artifact = match artifact {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("spine artifact query failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifact" })),
            )
                .into_response();
        }
    };

    let versions = match sqlx::query_as::<_, ArtifactVersionRow>(
        "SELECT version, content_ref_uri, content_ref_checksum, change_summary, \
         created_by_session, parent_version, created_at \
         FROM artifact_versions WHERE artifact_id = $1 ORDER BY version DESC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("failed to load versions for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact versions" })),
            )
                .into_response();
        }
    };

    let lineage = match sqlx::query_as::<_, VerticalLineageRow>(
        "SELECT version, session_id, recorded_at \
         FROM vertical_lineage WHERE artifact_id = $1 ORDER BY version DESC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to load vertical lineage for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load vertical lineage" })),
            )
                .into_response();
        }
    };

    let horizontal = match sqlx::query_as::<_, HorizontalLineageRow>(
        "SELECT source_artifact_id, source_version, role, recorded_at \
         FROM horizontal_lineage WHERE artifact_id = $1 \
         ORDER BY recorded_at ASC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to load horizontal lineage for artifact {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load horizontal lineage" })),
            )
                .into_response();
        }
    };

    let created_by: String =
        sqlx::query_scalar("SELECT created_by FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

    let related_events = fetch_related_events(pool, &id).await.unwrap_or_else(|e| {
        tracing::warn!("failed to load related events for artifact {id}: {e}");
        Vec::new()
    });

    Json(serde_json::json!({
        "artifact": {
            "id": artifact.id,
            "kind": artifact.kind,
            "name": artifact.name,
            "state": artifact.state,
            "owner": artifact.owner,
            "current_version": artifact.current_version,
            "created_by": created_by,
            "created_at": artifact.created_at,
            "updated_at": artifact.updated_at,
            "versions": versions,
            "vertical_lineage": lineage,
            "horizontal_lineage": horizontal,
            "related_events": related_events,
        }
    }))
    .into_response()
}

/// Fetch spine events related to an artifact for the per-run DAG (issue #14
/// phase 3). Filters on `stream_id = forge:<artifact_id>` (the convention
/// forge follows) and on session completions whose payload references this
/// artifact.
async fn fetch_related_events(
    pool: &sqlx::PgPool,
    artifact_id: &str,
) -> Result<Vec<SpineEvent>, sqlx::Error> {
    let stream_key = format!("forge:{artifact_id}");
    sqlx::query_as::<_, SpineEvent>(
        "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext \
         WHERE stream_id = $1 \
            OR (event_type IN ('stiglab.session_completed', 'stiglab.session_failed') \
                AND data->>'artifact_id' = $2) \
         ORDER BY id ASC \
         LIMIT 500",
    )
    .bind(&stream_key)
    .bind(artifact_id)
    .fetch_all(pool)
    .await
}

/// POST /api/spine/artifacts/:id/retry
pub async fn retry_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<RetryRequest>,
) -> Response {
    let pool = state.spine.pool();

    let row = match sqlx::query_as::<_, (String, String)>(
        "SELECT workspace_id, state FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!("artifact lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact" })),
            )
                .into_response();
        }
    };
    let Some((workspace_id, state_str)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    if state_str == "archived" || state_str == "released" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("cannot retry artifact in {state_str} state"),
            })),
        )
            .into_response();
    }

    let actor = req.actor.as_deref().unwrap_or("dashboard");
    emit(
        &state,
        &workspace_id,
        &format!("forge:{id}"),
        "forge",
        actor,
        "forge.retry_requested",
        serde_json::json!({
            "artifact_id": id,
            "reason": req.reason,
            "previous_state": state_str,
        }),
    )
    .await;

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "artifact_id": id,
            "action": "retry_requested",
        })),
    )
        .into_response()
}

/// POST /api/spine/artifacts/:id/abort
pub async fn abort_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<AbortRequest>,
) -> Response {
    let pool = state.spine.pool();

    let row = match sqlx::query_as::<_, (String, String)>(
        "SELECT workspace_id, state FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!("artifact lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact" })),
            )
                .into_response();
        }
    };
    let Some((workspace_id, previous_state)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    if previous_state == "archived" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "artifact already archived" })),
        )
            .into_response();
    }

    let reason = req
        .reason
        .clone()
        .unwrap_or_else(|| "aborted via dashboard".to_string());
    let actor = req.actor.as_deref().unwrap_or("dashboard");

    if let Err(e) = sqlx::query(
        "UPDATE artifacts SET state = 'archived', updated_at = NOW() WHERE artifact_id = $1",
    )
    .bind(&id)
    .execute(pool)
    .await
    {
        tracing::error!("failed to archive artifact {id}: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to archive artifact" })),
        )
            .into_response();
    }

    emit(
        &state,
        &workspace_id,
        &format!("forge:{id}"),
        "forge",
        actor,
        "artifact.archived",
        serde_json::json!({
            "artifact_id": id,
            "reason": reason,
            "previous_state": previous_state,
        }),
    )
    .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "artifact_id": id,
            "action": "archived",
            "reason": reason,
        })),
    )
        .into_response()
}

/// POST /api/spine/artifacts/:id/override-gate
pub async fn override_gate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<OverrideGateRequest>,
) -> Response {
    let pool = state.spine.pool();

    let workspace_id: String = match sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("artifact lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to load artifact" })),
            )
                .into_response();
        }
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }

    let verdict = req.verdict.as_deref().unwrap_or("allow").to_lowercase();
    if verdict != "allow" && verdict != "deny" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "verdict must be 'allow' or 'deny'",
            })),
        )
            .into_response();
    }

    let actor = req.actor.as_deref().unwrap_or("dashboard");
    let reason = req
        .reason
        .clone()
        .unwrap_or_else(|| format!("manual {verdict} via dashboard"));
    let escalation_id = format!("esc_{}", ulid::Ulid::new());

    emit(
        &state,
        &workspace_id,
        &format!("forge:{id}"),
        "synodic",
        actor,
        "synodic.escalation_resolved",
        serde_json::json!({
            "escalation_id": escalation_id,
            "artifact_id": id,
            "resolution": {
                "verdict": verdict,
                "resolved_by": actor,
                "reason": reason,
            },
        }),
    )
    .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "artifact_id": id,
            "action": "gate_override",
            "verdict": verdict,
            "escalation_id": escalation_id,
        })),
    )
        .into_response()
}
