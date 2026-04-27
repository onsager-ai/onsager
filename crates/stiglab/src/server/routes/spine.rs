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

use super::{require_workspace_access, workspace_scoped_not_found};

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Required (issue #164).  The list-endpoint contract is "explicit
    /// workspace, always".  A missing/blank `?workspace=` returns 400.
    #[serde(default)]
    pub workspace: Option<String>,
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
    /// Required (issue #164) — every artifact registered through the
    /// dashboard must be scoped to a specific workspace.
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

/// GET /api/spine/events — query the events_ext table.
pub async fn list_events(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<EventsQuery>,
) -> impl IntoResponse {
    let workspace_id = match params.workspace.as_deref() {
        Some(w) if !w.trim().is_empty() => w.to_string(),
        _ => return super::missing_workspace_query(),
    };
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
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
    // Workspace filter (issue #164): `events_ext` rows do not carry an
    // explicit `workspace_id` column today.  Scope by joining through
    // the spine `artifacts` table.  Forge-emitted events use a
    // namespaced `stream_id` of the form `forge:{artifact_id}` (see
    // `register_artifact` / `fetch_related_events` below), while
    // spine-direct writes use the bare `{artifact_id}` — the JOIN's OR
    // clause matches both.  Cross-workspace event types (e.g. global
    // `agent.*` heartbeats) are intentionally elided from per-workspace
    // dashboards.
    //
    // The proper fix is to add `workspace_id` to `events_ext` directly;
    // tracked as a follow-up so the join goes away.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT e.id, e.stream_id, e.namespace AS stream_type, e.event_type, e.data, \
                COALESCE(e.metadata->>'actor', '') AS actor, e.created_at \
         FROM events_ext e \
         JOIN artifacts a ON e.stream_id = a.artifact_id \
                          OR e.stream_id LIKE '%:' || a.artifact_id \
         WHERE a.workspace_id = ",
    );
    qb.push_bind(workspace_id.clone());
    if let Some(st) = params.stream_type.as_deref() {
        qb.push(" AND e.namespace = ").push_bind(st.to_string());
    }
    if let Some(et) = params.event_type.as_deref() {
        qb.push(" AND e.event_type = ").push_bind(et.to_string());
    }
    if let Some(sid) = params.stream_id.as_deref() {
        qb.push(" AND e.stream_id = ").push_bind(sid.to_string());
    }
    qb.push(" ORDER BY e.id DESC LIMIT ").push_bind(limit);
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

/// Filters for `GET /api/spine/artifacts`. `workspace` is required
/// (issue #164); the rest are optional.
#[derive(Debug, Deserialize)]
pub struct ListArtifactsQuery {
    /// Required workspace filter — issue #164.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Filter by `kind` discriminator (e.g. `pull_request`, `github_issue`).
    pub kind: Option<String>,
    /// Filter by `metadata->>'project_id'`. Used by the dashboard `/issues`
    /// inbox to scope to the workspace's project.
    pub project_id: Option<String>,
}

/// GET /api/spine/artifacts — list artifacts from the spine.
///
/// Supports `?kind=` and `?project_id=` filters so the dashboard's per-page
/// queries (Factory PRs, Issues inbox) can target a single discriminated
/// slice without scanning the whole table client-side.
pub async fn list_artifacts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(filters): Query<ListArtifactsQuery>,
) -> impl IntoResponse {
    let workspace_id = match filters.workspace.as_deref() {
        Some(w) if !w.trim().is_empty() => w.to_string(),
        _ => return super::missing_workspace_query(),
    };
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
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

    // Build the query with optional WHERE clauses. sqlx::Postgres binds use
    // `$N` positional placeholders; we keep this simple by branching on the
    // (kind, project_id) combinations rather than building a string.
    let base = "SELECT artifact_id AS id, kind, name, state, owner, current_version, \
                consumers, external_ref, created_at, updated_at, last_observed_at \
                FROM artifacts";
    let result = match (filters.kind.as_deref(), filters.project_id.as_deref()) {
        (Some(kind), Some(project_id)) => {
            sqlx::query_as::<_, SpineArtifact>(&format!(
                "{base} WHERE workspace_id = $1 AND kind = $2 \
                 AND metadata->>'project_id' = $3 \
                 ORDER BY updated_at DESC LIMIT 100"
            ))
            .bind(&workspace_id)
            .bind(kind)
            .bind(project_id)
            .fetch_all(pool)
            .await
        }
        (Some(kind), None) => {
            sqlx::query_as::<_, SpineArtifact>(&format!(
                "{base} WHERE workspace_id = $1 AND kind = $2 \
                 ORDER BY updated_at DESC LIMIT 100"
            ))
            .bind(&workspace_id)
            .bind(kind)
            .fetch_all(pool)
            .await
        }
        (None, Some(project_id)) => {
            sqlx::query_as::<_, SpineArtifact>(&format!(
                "{base} WHERE workspace_id = $1 AND metadata->>'project_id' = $2 \
                 ORDER BY updated_at DESC LIMIT 100"
            ))
            .bind(&workspace_id)
            .bind(project_id)
            .fetch_all(pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, SpineArtifact>(&format!(
                "{base} WHERE workspace_id = $1 ORDER BY updated_at DESC LIMIT 100"
            ))
            .bind(&workspace_id)
            .fetch_all(pool)
            .await
        }
    };

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
) -> impl IntoResponse {
    if let Err(r) = require_workspace_access(&state.db, &auth_user, &req.workspace_id).await {
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
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, workspace_id) \
         VALUES ($1, $2, $3, $4, $5, 'draft', 0, $6)",
    )
    .bind(&artifact_id)
    .bind(&req.kind)
    .bind(&req.name)
    .bind(&req.owner)
    .bind("dashboard") // created_by
    .bind(&req.workspace_id)
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

/// GET /api/spine/artifacts/:id — single artifact detail with versions and lineage.
pub async fn get_artifact(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> impl IntoResponse {
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

    if let Err(r) = require_artifact_workspace_access(pool, &state, &auth_user, &id).await {
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
        Ok(None) => return workspace_scoped_not_found("artifact not found"),
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

/// Look up `artifacts.workspace_id` for a spine-scoped detail or
/// mutation route, then funnel through `require_workspace_access` so
/// the same authz contract (PAT pin → 403, non-member → 404) applies.
///
/// Missing artifact rows return a flat 404 — same shape as a non-member
/// hit, so the existence of an artifact id can't be probed by a
/// workspace mismatch test.
#[allow(clippy::result_large_err)]
async fn require_artifact_workspace_access(
    pool: &sqlx::PgPool,
    state: &AppState,
    auth_user: &AuthUser,
    artifact_id: &str,
) -> Result<(), Response> {
    let workspace_id = sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM artifacts WHERE artifact_id = $1",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await;
    let workspace_id = match workspace_id {
        Ok(Some(w)) => w,
        Ok(None) => return Err(workspace_scoped_not_found("artifact not found")),
        Err(e) => {
            tracing::error!("artifact workspace lookup failed: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    require_workspace_access(&state.db, auth_user, &workspace_id).await
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
) -> impl IntoResponse {
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
    if let Err(r) = require_artifact_workspace_access(pool, &state, &auth_user, &id).await {
        return r;
    }

    // Confirm the artifact exists and is not archived.
    let current_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let Some(state_str) = current_state else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };

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
) -> impl IntoResponse {
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
    if let Err(r) = require_artifact_workspace_access(pool, &state, &auth_user, &id).await {
        return r;
    }

    let previous_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let Some(previous_state) = previous_state else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
    };

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
) -> impl IntoResponse {
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
    if let Err(r) = require_artifact_workspace_access(pool, &state, &auth_user, &id).await {
        return r;
    }

    let exists: Option<String> =
        sqlx::query_scalar("SELECT artifact_id FROM artifacts WHERE artifact_id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    if exists.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response();
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
