use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

use crate::server::auth::AuthUser;
use crate::server::db;
use crate::server::state::AppState;

use super::require_workspace_access;

/// Required `?workspace=` filter for every workspace-scoped list endpoint
/// (issue #164). A missing param returns 400 — explicit is better than
/// implicit, and a default-to-everything would hide cross-workspace
/// leaks behind a default value.
#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace: String,
}

#[allow(clippy::result_large_err)]
fn require_workspace_param(q: &WorkspaceQuery) -> Result<&str, Response> {
    let ws = q.workspace.trim();
    if ws.is_empty() {
        Err(missing_workspace())
    } else {
        Ok(ws)
    }
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

pub(super) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "session not found" })),
    )
        .into_response()
}

pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<WorkspaceQuery>,
) -> Response {
    let workspace_id = match require_workspace_param(&q) {
        Ok(w) => w.to_string(),
        Err(r) => return r,
    };

    // The synthetic anonymous principal has no membership rows, so the
    // membership-check helper would 404 every request in
    // auth_enabled = false mode. Skip the check there but still scope
    // the query to the requested workspace.
    let result = if auth_user.user_id == "anonymous" {
        db::list_sessions_in_workspace(&state.db, &workspace_id).await
    } else {
        if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
            return r;
        }
        db::list_sessions_for_user_in_workspace(&state.db, &auth_user.user_id, &workspace_id).await
    };

    match result {
        Ok(sessions) => Json(serde_json::json!({ "sessions": sessions })).into_response(),
        Err(e) => {
            tracing::error!("failed to list sessions: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// Verify the caller may read `session_id`. Returns the session's
/// resolved workspace_id on success (Some when the session is
/// workspace-scoped, None for legacy personal sessions). Per the spec,
/// non-members get a flat 404 — leaking 403 would let callers enumerate
/// session IDs across workspaces.
#[allow(clippy::result_large_err)]
async fn authorize_session_read(
    state: &AppState,
    auth_user: &AuthUser,
    session_id: &str,
) -> Result<Option<String>, Response> {
    if auth_user.user_id == "anonymous" {
        return Ok(None);
    }

    let workspace_id = match db::get_session_workspace(&state.db, session_id).await {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("failed to look up session workspace: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
        }
    };

    if let Some(ref ws) = workspace_id {
        // 404 (not 403) on workspace mismatch — see require_workspace_access.
        require_workspace_access(&state.db, auth_user, ws)
            .await
            .map_err(rewrite_workspace_404_to_session)?;
        return Ok(Some(ws.clone()));
    }

    // Legacy personal session (pre-#164, no workspace_id). Fall back to
    // the original owner check so the same user keeps access.
    match db::get_session_owner(&state.db, session_id).await {
        Ok(Some(owner)) if owner != auth_user.user_id => Err(not_found()),
        Ok(_) => Ok(None),
        Err(e) => {
            tracing::error!("failed to verify session ownership: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
    }
}

/// `require_workspace_access` returns "workspace not found" on a 404. For
/// session detail/log endpoints the caller is asking after a session, so
/// rewrite the body to the session's not-found shape — a reader peeking
/// at the body shouldn't be able to tell whether the workspace or the
/// session was the missing piece.
fn rewrite_workspace_404_to_session(resp: Response) -> Response {
    if resp.status() == StatusCode::NOT_FOUND {
        return not_found();
    }
    resp
}

pub async fn get_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(r) = authorize_session_read(&state, &auth_user, &session_id).await {
        return r;
    }

    match db::get_session(&state.db, &session_id).await {
        Ok(Some(session)) => {
            // Aggregate output from log chunks
            let output = match db::get_session_logs(&state.db, &session.id).await {
                Ok(chunks) => {
                    if chunks.is_empty() {
                        session.output
                    } else {
                        Some(chunks.into_iter().map(|c| c.chunk).collect::<String>())
                    }
                }
                Err(_) => session.output,
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

pub async fn session_logs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = authorize_session_read(&state, &auth_user, &session_id).await {
        return Err((r.status(), Json(serde_json::json!({ "error": "see body" }))));
    }

    // Verify session exists
    match db::get_session(&state.db, &session_id).await {
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            ));
        }
        Ok(Some(_)) => {}
    }

    // SSE stream: send only new chunks since last poll (cursor-based)
    let initial_state = (state, session_id, 0i64);
    let sse_stream = stream::unfold(initial_state, |(state, session_id, last_seq)| async move {
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Get session state
        let session = db::get_session(&state.db, &session_id).await.ok()??;

        // Get only new log chunks since last_seq
        let new_chunks = db::get_session_logs_after(&state.db, &session_id, last_seq)
            .await
            .ok()?;

        let new_last_seq = new_chunks.last().map(|c| c.seq).unwrap_or(last_seq);
        let chunks_data: Vec<serde_json::Value> = new_chunks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "chunk": c.chunk,
                    "stream": c.stream,
                })
            })
            .collect();

        let event = Event::default()
            .json_data(serde_json::json!({
                "state": session.state,
                "chunks": chunks_data,
            }))
            .ok()?;

        // Stop streaming if session is in a terminal state and no new chunks
        let is_terminal = matches!(
            session.state,
            crate::core::SessionState::Done | crate::core::SessionState::Failed
        );
        if is_terminal && chunks_data.is_empty() {
            return None;
        }

        Some((
            Ok::<_, Infallible>(event),
            (state, session_id, new_last_seq),
        ))
    });

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}
