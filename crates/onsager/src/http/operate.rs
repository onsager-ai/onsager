//! Operator surfaces (M3 #625): the activity feed and run abort.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::auth::{AuthUser, require_workspace_access};
use crate::events::record;
use crate::http::workflows::WorkspaceQuery;
use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SessionEventRow {
    pub seq: i32,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// `GET /api/sessions/:id/events` — the normalized agent feed for one
/// session (#632). The run page polls this while the session runs.
pub async fn session_events(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let session: Option<(String, String)> =
        match sqlx::query_as("SELECT workspace_id, status FROM sessions WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("session lookup failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    let Some((workspace_id, status)) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }
    let rows: Result<Vec<SessionEventRow>, _> = sqlx::query_as(
        "SELECT seq, kind, payload, created_at FROM session_events          WHERE session_id = $1 ORDER BY seq",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(events) => {
            Json(serde_json::json!({ "status": status, "events": events })).into_response()
        }
        Err(e) => {
            tracing::error!("session events query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/sessions/:id/events/stream` — the live feed (#634).
/// Server-sent events: one JSON message per durable `agent.*` event
/// plus transient `agent.text_delta` drafts. The stream ends when the
/// session does (the scheduler drops the channel). A session that has
/// already ended gets an immediately-closing stream — the client's
/// initial fetch already has the durable rows.
pub async fn session_events_stream(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let session: Option<(String,)> =
        match sqlx::query_as("SELECT workspace_id FROM sessions WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("session lookup failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    let Some((workspace_id,)) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "session not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }

    let receiver = state
        .feeds
        .lock()
        .expect("feeds lock")
        .get(&id)
        .map(|sender| sender.subscribe());
    let live = BroadcastStream::new(match receiver {
        Some(r) => r,
        None => {
            // Session already ended: empty channel whose sender drops
            // here, closing the stream immediately.
            let (sender, receiver) = tokio::sync::broadcast::channel(1);
            drop(sender);
            receiver
        }
    });
    let stream = live.filter_map(|item| match item {
        Ok(value) => Some(Ok::<Event, std::convert::Infallible>(
            Event::default().data(value.to_string()),
        )),
        // Lagged subscriber: skip — the durable rows are the truth.
        Err(_) => None,
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ActivityRow {
    pub id: i64,
    pub run_id: Option<String>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// `GET /api/activity?workspace=` — the workspace's audit feed, newest
/// first. The dashboard polls this; rows deep-link via `run_id`.
pub async fn activity(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &user, &q.workspace).await {
        return r;
    }
    let rows: Result<Vec<ActivityRow>, _> = sqlx::query_as(
        "SELECT id, run_id, kind, payload, created_at FROM events \
         WHERE workspace_id = $1 ORDER BY id DESC LIMIT 100",
    )
    .bind(&q.workspace)
    .fetch_all(&state.pool)
    .await;
    match rows {
        Ok(events) => Json(serde_json::json!({ "events": events })).into_response(),
        Err(e) => {
            tracing::error!("activity query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `POST /api/runs/:id/abort` — abort an in-flight run. The scheduler
/// task is aborted (its agent child dies via `kill_on_drop`); this
/// handler then settles the rows the dead task can no longer settle.
pub async fn abort_run(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Response {
    let run: Option<(String, String)> =
        match sqlx::query_as("SELECT workspace_id, status FROM runs WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("abort run lookup failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    let Some((workspace_id, status)) = run else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response();
    };
    if let Err(r) = require_workspace_access(&state.pool, &user, &workspace_id).await {
        return r;
    }
    if status != "running" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("run is {status}, not running") })),
        )
            .into_response();
    }

    let handle = state.running.lock().expect("registry lock").remove(&id);
    let was_live = handle.is_some();
    if let Some(handle) = handle {
        handle.abort.abort();
        // kill_on_drop reaches the direct child; the session's process
        // GROUP catches the tree the agent spawned under it.
        let pgid = *handle.pgid.lock().expect("pgid lock");
        if let Some(pgid) = pgid
            && pgid > 1
        {
            // SAFETY: plain syscall; negative pid addresses the group.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }

    // If any of the run's sessions are running on a fleet machine, the
    // local pgid is empty — route a kill frame to the owning machine
    // (ADR 0030). Best-effort; the row settle below is authoritative.
    if was_live {
        let session_ids: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM sessions WHERE run_id = $1 AND status = 'running'")
                .bind(&id)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();
        for (sid,) in &session_ids {
            crate::fleet::abort_remote_session(&state, sid);
        }
    }

    // Settle the rows the aborted task can no longer settle.
    if let Err(e) = sqlx::query(
        "UPDATE runs SET status = 'aborted', error = $2, finished_at = NOW() WHERE id = $1",
    )
    .bind(&id)
    .bind(format!("aborted by {}", user.github_login))
    .execute(&state.pool)
    .await
    {
        tracing::error!("abort run update failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(e) = sqlx::query(
        "UPDATE sessions SET status = 'failed', error = 'run aborted', finished_at = NOW() \
         WHERE run_id = $1 AND status = 'running'",
    )
    .bind(&id)
    .execute(&state.pool)
    .await
    {
        tracing::error!("abort session settle failed: {e}");
    }
    record(
        &state.pool,
        &workspace_id,
        Some(&id),
        "run.aborted",
        serde_json::json!({ "by": user.github_login, "task_was_live": was_live }),
    )
    .await;
    Json(serde_json::json!({ "ok": true, "run_id": id, "status": "aborted" })).into_response()
}
