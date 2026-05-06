//! Session read endpoints (spec #222 Follow-up 3).
//!
//! Moved from `crates/stiglab/src/server/routes/sessions.rs`. All DB
//! access goes through `crate::session_db` (PgPool).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

use crate::auth::AuthUser;
use crate::core::SessionState;
use crate::handlers::workspaces::require_workspace_access;
use crate::session_db;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace: String,
}

pub(super) fn missing_workspace() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "workspace query parameter is required",
            "detail": "every workspace-scoped list endpoint requires ?workspace=<id>",
        })),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "session not found" })),
    )
        .into_response()
}

/// Verify the caller may read `session_id`. Returns the session's
/// workspace_id on success (Some for workspace-scoped, None for legacy
/// personal sessions). Per the spec, non-members get a flat 404.
#[allow(clippy::result_large_err)]
async fn authorize_session_read(
    state: &AppState,
    auth_user: &AuthUser,
    session_id: &str,
) -> Result<Option<String>, Response> {
    let workspace_id = match session_db::get_session_workspace(&state.pool, session_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("failed to look up session workspace: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
        }
    };

    if let Some(ref ws) = workspace_id {
        require_workspace_access(&state.pool, auth_user, ws)
            .await
            .map_err(rewrite_workspace_404_to_session)?;
        return Ok(Some(ws.clone()));
    }

    // Legacy personal session path — fall back to owner check.
    match session_db::get_session_owner(&state.pool, session_id).await {
        Ok(Some(owner)) if owner != auth_user.user_id => Err(not_found()),
        Ok(_) => Ok(None),
        Err(e) => {
            tracing::error!("failed to verify session ownership: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
    }
}

fn rewrite_workspace_404_to_session(resp: Response) -> Response {
    if resp.status() == StatusCode::NOT_FOUND {
        return not_found();
    }
    resp
}

/// GET /api/sessions?workspace=W
pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    let workspace_id = q.workspace.trim().to_string();
    if workspace_id.is_empty() {
        return missing_workspace();
    }
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }
    match session_db::list_sessions_for_user_in_workspace(
        &state.pool,
        &auth_user.user_id,
        &workspace_id,
    )
    .await
    {
        Ok(sessions) => Json(serde_json::json!({ "sessions": sessions })).into_response(),
        Err(e) => {
            tracing::error!("failed to list sessions: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/sessions/:id
pub async fn get_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(r) = authorize_session_read(&state, &auth_user, &session_id).await {
        return r;
    }

    match session_db::get_session(&state.pool, &session_id).await {
        Ok(Some(session)) => {
            let output = match session_db::get_session_logs(&state.pool, &session.id).await {
                Ok(chunks) if !chunks.is_empty() => {
                    Some(chunks.into_iter().map(|c| c.chunk).collect::<String>())
                }
                _ => session.output,
            };
            Json(serde_json::json!({
                "session": {
                    "id": session.id,
                    "task_id": session.task_id,
                    "node_id": session.node_id,
                    "state": session.state,
                    "prompt": session.prompt,
                    "output": output,
                    "working_dir": session.working_dir,
                    "created_at": session.created_at,
                    "updated_at": session.updated_at,
                }
            }))
            .into_response()
        }
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("failed to get session: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// GET /api/sessions/:id/logs — SSE stream of log chunks.
pub async fn session_logs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(r) = authorize_session_read(&state, &auth_user, &session_id).await {
        return r;
    }

    match session_db::get_session(&state.pool, &session_id).await {
        Ok(None) => return not_found(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        Ok(Some(_)) => {}
    }

    let initial = (state, session_id.to_string(), 0i64);
    let sse_stream = stream::unfold(initial, |(state, session_id, last_seq)| async move {
        tokio::time::sleep(Duration::from_millis(500)).await;

        let session = session_db::get_session(&state.pool, &session_id)
            .await
            .ok()??;
        let new_chunks = session_db::get_session_logs_after(&state.pool, &session_id, last_seq)
            .await
            .ok()?;

        let new_last_seq = new_chunks.last().map(|c| c.seq).unwrap_or(last_seq);
        let chunks_data: Vec<serde_json::Value> = new_chunks
            .iter()
            .map(|c| serde_json::json!({ "chunk": c.chunk, "stream": c.stream }))
            .collect();

        let event = Event::default()
            .json_data(serde_json::json!({
                "state": session.state,
                "chunks": chunks_data,
            }))
            .ok()?;

        let is_terminal = matches!(session.state, SessionState::Done | SessionState::Failed);
        if is_terminal && chunks_data.is_empty() {
            return None;
        }

        Some((
            Ok::<_, Infallible>(event),
            (state, session_id, new_last_seq),
        ))
    });

    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
