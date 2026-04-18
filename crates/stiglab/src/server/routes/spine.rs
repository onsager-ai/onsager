//! Spine API routes — exposes the shared event spine data to the dashboard.
//!
//! These endpoints read from the onsager-spine PostgreSQL tables (events_ext,
//! artifacts) to surface factory-wide activity without requiring each subsystem
//! to expose its own HTTP API.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub stream_type: Option<String>,
    pub event_type: Option<String>,
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
    pub name: String,
    pub state: String,
    pub owner: String,
    pub current_version: i32,
    pub consumers: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
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

#[derive(Debug, Deserialize)]
pub struct RegisterArtifactRequest {
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
    Query(params): Query<EventsQuery>,
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
    let limit = params.limit.unwrap_or(50).min(500);

    // `events_ext` stores the partition key in `namespace` and the actor inside
    // `metadata`. The API surfaces them as `stream_type` / `actor` for clients,
    // so we alias on the way out.
    let result = match (params.stream_type.as_deref(), params.event_type.as_deref()) {
        (Some(st), Some(et)) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                        COALESCE(metadata->>'actor', '') AS actor, created_at \
                 FROM events_ext WHERE namespace = $1 AND event_type = $2 \
                 ORDER BY id DESC LIMIT $3",
            )
            .bind(st)
            .bind(et)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (Some(st), None) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                        COALESCE(metadata->>'actor', '') AS actor, created_at \
                 FROM events_ext WHERE namespace = $1 \
                 ORDER BY id DESC LIMIT $2",
            )
            .bind(st)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (None, Some(et)) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                        COALESCE(metadata->>'actor', '') AS actor, created_at \
                 FROM events_ext WHERE event_type = $1 \
                 ORDER BY id DESC LIMIT $2",
            )
            .bind(et)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, namespace AS stream_type, event_type, data, \
                        COALESCE(metadata->>'actor', '') AS actor, created_at \
                 FROM events_ext ORDER BY id DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    };

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

/// GET /api/spine/artifacts — list artifacts from the spine.
pub async fn list_artifacts(State(state): State<AppState>) -> impl IntoResponse {
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

    let result = sqlx::query_as::<_, SpineArtifact>(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, created_at, updated_at \
         FROM artifacts ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(pool)
    .await;

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
    Json(req): Json<RegisterArtifactRequest>,
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
    let artifact_id = format!("art_{}", ulid::Ulid::new());

    let result = sqlx::query(
        "INSERT INTO artifacts (artifact_id, kind, name, owner, created_by, state, current_version) \
         VALUES ($1, $2, $3, $4, $5, 'draft', 0)",
    )
    .bind(&artifact_id)
    .bind(&req.kind)
    .bind(&req.name)
    .bind(&req.owner)
    .bind("dashboard") // created_by
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
                "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, created_at, updated_at \
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

    let artifact = sqlx::query_as::<_, SpineArtifact>(
        "SELECT artifact_id AS id, kind, name, state, owner, current_version, consumers, created_at, updated_at \
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
