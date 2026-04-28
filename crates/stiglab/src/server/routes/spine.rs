//! Spine API routes — exposes the shared event spine data to the dashboard.
//!
//! These endpoints read from the onsager-spine PostgreSQL tables (events_ext,
//! artifacts) to surface factory-wide activity without requiring each subsystem
//! to expose its own HTTP API.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::server::auth::AuthUser;
use crate::server::state::AppState;

use super::require_workspace_access;

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

#[derive(Debug, Serialize, FromRow)]
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

#[derive(Debug, Serialize, FromRow)]
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

#[derive(Debug, Deserialize)]
pub struct RegisterArtifactRequest {
    /// Workspace the new artifact belongs to (#164). Caller must be a
    /// member; the value is written into `artifacts.workspace_id` so
    /// later list/detail queries scope it correctly.
    pub workspace_id: String,
    pub kind: String,
    pub name: String,
    pub owner: String,
    pub description: Option<String>,
    pub working_dir: Option<String>,
}

/// POST /api/spine/artifacts/:id/retry — request re-shaping of an artifact.
///
/// Emits a `forge.retry_requested` event and bumps the artifact back to
/// `in_progress` if it was stuck in `under_review`. Forge picks it up on
/// the next tick.
#[derive(Debug, Deserialize, Default)]
pub struct RetryRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// POST /api/spine/artifacts/:id/abort — archive an artifact.
///
/// Flips state to `archived` and emits `artifact.archived`. Irreversible;
/// the dashboard asks for confirmation.
#[derive(Debug, Deserialize, Default)]
pub struct AbortRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
}

/// POST /api/spine/artifacts/:id/override-gate — record a manual gate override.
///
/// Emits `synodic.escalation_resolved` with the chosen verdict so Forge's
/// next tick honors it. This is the dashboard's counterpart to a human
/// resolving an escalated gate.
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

/// GET /api/spine/events?workspace=W — query the events_ext table.
///
/// Filters event rows whose payload references the requested workspace
/// (matched on `data->>'workspace_id'`). The `?workspace=` query param
/// is required (#164) — a missing value returns 400 so a caller can't
/// silently scan every workspace's stream.
pub async fn list_events(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<EventsQuery>,
) -> Response {
    let workspace_id = params.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
        return r;
    }

    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();
    let limit = params.limit.unwrap_or(50).min(500);

    // `events_ext` stores the partition key in `namespace` and the actor inside
    // `metadata`. The API surfaces them as `stream_type` / `actor` for clients,
    // so we alias on the way out.
    //
    // Workspace scope is matched on `data->>'workspace_id'`; events that
    // don't carry the field (legacy, infrastructure-level) are excluded
    // so a workspace listing never includes another workspace's rows.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                COALESCE(metadata->>'actor', '') AS actor, created_at \
         FROM events_ext WHERE data->>'workspace_id' = ",
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
    qb.push(" ORDER BY id DESC LIMIT ").push_bind(limit);
    let result = qb.build_query_as::<SpineEvent>().fetch_all(pool).await;

    match result {
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

/// Standard 400 body for the `?workspace=` requirement on every
/// workspace-scoped list endpoint (#164). Mirrors the helper in
/// `routes/sessions.rs` — kept duplicated to avoid a fragile module
/// dependency.
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

/// Filters for `GET /api/spine/artifacts`.
#[derive(Debug, Deserialize)]
pub struct ListArtifactsQuery {
    /// Required workspace scope (#164). Missing → 400.
    pub workspace: String,
    /// Filter by `kind` discriminator (e.g. `pull_request`, `github_issue`).
    pub kind: Option<String>,
    /// Filter by `metadata->>'project_id'`. Used by the dashboard `/issues`
    /// inbox to scope to the workspace's project.
    pub project_id: Option<String>,
}

/// GET /api/spine/artifacts?workspace=W — list artifacts from the spine.
///
/// Filters by `workspace_id` (required). Optional `?kind=` and
/// `?project_id=` further narrow the listing.
pub async fn list_artifacts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(filters): Query<ListArtifactsQuery>,
) -> Response {
    let workspace_id = filters.workspace.trim();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
        return r;
    }

    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();

    // The base query always filters by workspace_id (#162 added the
    // column on the spine `artifacts` table).  Optional kind/project
    // filters tack onto the same WHERE clause via QueryBuilder so a
    // mis-set bind index can't shift between branches.
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
    let result = qb.build_query_as::<SpineArtifact>().fetch_all(pool).await;

    match result {
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

/// POST /api/spine/artifacts — register a new artifact in Draft state.
pub async fn register_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<RegisterArtifactRequest>,
) -> Response {
    let workspace_id = req.workspace_id.trim();
    if workspace_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workspace_id is required" })),
        )
            .into_response();
    }
    if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
        return r;
    }

    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();
    let artifact_id = format!("art_{}", ulid::Ulid::new());

    let result = sqlx::query(
        "INSERT INTO artifacts (artifact_id, kind, name, owner, created_by, state, current_version, workspace_id) \
         VALUES ($1, $2, $3, $4, $5, 'draft', 0, $6)",
    )
    .bind(&artifact_id)
    .bind(&req.kind)
    .bind(&req.name)
    .bind(&req.owner)
    .bind("dashboard") // created_by
    .bind(workspace_id)
    .execute(pool)
    .await;

    match result {
        Ok(_) => {
            // Emit a spine event for the registration.
            if let Some(ref spine) = state.spine {
                let data = serde_json::json!({
                    "artifact_id": artifact_id,
                    "kind": req.kind,
                    "name": req.name,
                    "owner": req.owner,
                    "description": req.description,
                    "working_dir": req.working_dir,
                });
                if let Err(e) = spine
                    .emit_raw(
                        &format!("forge:{artifact_id}"),
                        "forge",
                        "dashboard",
                        "artifact.registered",
                        &data,
                    )
                    .await
                {
                    tracing::warn!("failed to emit artifact.registered event: {e}");
                }
            }

            // Query back the inserted artifact.
            let artifact = sqlx::query_as::<_, SpineArtifact>(
                "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, \
                 external_ref, created_at, updated_at, last_observed_at \
                 FROM artifacts WHERE artifact_id = $1",
            )
            .bind(&artifact_id)
            .fetch_one(pool)
            .await;

            match artifact {
                Ok(a) => (
                    StatusCode::CREATED,
                    Json(serde_json::json!({ "artifact": a })),
                )
                    .into_response(),
                Err(e) => {
                    tracing::error!("failed to read back artifact: {e}");
                    (
                        StatusCode::CREATED,
                        Json(serde_json::json!({ "artifact": { "id": artifact_id } })),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            tracing::error!("failed to register artifact: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to register artifact" })),
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
    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();

    // Resolve the artifact's workspace first so the membership check
    // 404s before any further data is read; the artifact body itself
    // is fetched separately to keep the row type a clean SpineArtifact.
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
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
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
                .into_response()
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

    // Fetch versions
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

    // Fetch vertical lineage
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

    // Fetch horizontal lineage — which other artifacts were used as
    // inputs when shaping this one (artifact-model §4, e.g. PR
    // `closes_issue` link). Empty for artifacts with no cross-kind
    // references; we still surface the (possibly empty) field so the UI
    // doesn't have to special-case undefined.
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

    // Fetch created_by from artifacts table
    let created_by: String =
        sqlx::query_scalar("SELECT created_by FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

    // Fetch related spine events for the per-run DAG (issue #14 phase 3).
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
    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();

    // Resolve workspace + state in one round-trip; the membership check
    // comes before any mutation so a non-member can't probe artifact
    // state via the side-effect.
    let row: Option<(String, String)> = match sqlx::query_as::<_, (String, String)>(
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
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
        if r.status() == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "artifact not found" })),
            )
                .into_response();
        }
        return r;
    }
    let _ = workspace_id;

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
    let data = serde_json::json!({
        "artifact_id": id,
        "reason": req.reason,
        "previous_state": state_str,
    });
    if let Err(e) = spine
        .emit_raw(
            &format!("forge:{id}"),
            "forge",
            actor,
            "forge.retry_requested",
            &data,
        )
        .await
    {
        tracing::warn!("failed to emit forge.retry_requested event: {e}");
    }

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
    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();

    let row: Option<(String, String)> = match sqlx::query_as::<_, (String, String)>(
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
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
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

    // Flip state to archived. The factory pipeline treats archived as terminal.
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

    let data = serde_json::json!({
        "artifact_id": id,
        "reason": reason,
        "previous_state": previous_state,
    });
    if let Err(e) = spine
        .emit_raw(
            &format!("forge:{id}"),
            "forge",
            actor,
            "artifact.archived",
            &data,
        )
        .await
    {
        tracing::warn!("failed to emit artifact.archived event: {e}");
    }

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
    let spine = match &state.spine {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "spine not connected" })),
            )
                .into_response()
        }
    };

    let pool = spine.pool();

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
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
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

    let data = serde_json::json!({
        "escalation_id": escalation_id,
        "artifact_id": id,
        "resolution": {
            "verdict": verdict,
            "resolved_by": actor,
            "reason": reason,
        },
    });
    if let Err(e) = spine
        .emit_raw(
            &format!("forge:{id}"),
            "synodic",
            actor,
            "synodic.escalation_resolved",
            &data,
        )
        .await
    {
        tracing::warn!("failed to emit synodic.escalation_resolved event: {e}");
    }

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
