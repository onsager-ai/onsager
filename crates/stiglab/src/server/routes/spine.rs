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
    pub state: String,
    pub owner: String,
    pub current_version: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
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

    // Build query dynamically based on filters.
    let result = match (params.stream_type.as_deref(), params.event_type.as_deref()) {
        (Some(st), Some(et)) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, stream_type, event_type, data, actor, created_at \
                 FROM events_ext WHERE stream_type = $1 AND event_type = $2 \
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
                "SELECT id, stream_id, stream_type, event_type, data, actor, created_at \
                 FROM events_ext WHERE stream_type = $1 \
                 ORDER BY id DESC LIMIT $2",
            )
            .bind(st)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        (None, Some(et)) => {
            sqlx::query_as::<_, SpineEvent>(
                "SELECT id, stream_id, stream_type, event_type, data, actor, created_at \
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
                "SELECT id, stream_id, stream_type, event_type, data, actor, created_at \
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
        "SELECT id, kind, state, owner, current_version, created_at, updated_at \
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

/// GET /api/spine/artifacts/:id — single artifact detail.
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

    let result = sqlx::query_as::<_, SpineArtifact>(
        "SELECT id, kind, state, owner, current_version, created_at, updated_at \
         FROM artifacts WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await;

    match result {
        Ok(Some(a)) => Json(serde_json::json!({ "artifact": a })).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "artifact not found" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("spine artifact query failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "failed to query artifact" })),
            )
                .into_response()
        }
    }
}
