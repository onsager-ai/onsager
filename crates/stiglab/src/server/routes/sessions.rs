use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

use crate::server::auth::AuthUser;
use crate::server::db;
use crate::server::state::AppState;

use super::{require_workspace_access, workspace_scoped_not_found};

/// Query string for `GET /api/sessions`.
///
/// Per issue #164, every workspace-scoped list endpoint requires
/// `?workspace=` and rejects missing/blank values with 400 — explicit is
/// better than "default to everything", which was the parent #161 leak
/// shape.  Validation funnels through `require_workspace_access` which
/// also enforces PAT pinning (403) and membership (404).
#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default)]
    pub workspace: Option<String>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(q): Query<ListSessionsQuery>,
) -> impl IntoResponse {
    let workspace_id = match q.workspace.as_deref() {
        Some(w) if !w.trim().is_empty() => w,
        _ => return super::missing_workspace_query(),
    };
    if let Err(r) = require_workspace_access(&state.db, &auth_user, workspace_id).await {
        return r;
    }
    match db::list_sessions_for_workspace(&state.db, workspace_id).await {
        Ok(sessions) => Json(serde_json::json!({ "sessions": sessions })).into_response(),
        Err(e) => {
            tracing::error!("failed to list sessions: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// `GET /api/sessions/:id` — Detail view for a single session.
///
/// Authz contract per issue #164: load the row first, then funnel the
/// session's `workspace_id` through `require_workspace_access` so PAT
/// pinning (403) and membership (404) are enforced uniformly.  Sessions
/// without a workspace (legacy/anonymous direct dispatch) are 404 to
/// any authenticated caller — they have no workspace to authz against.
pub async fn get_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if auth_user.user_id != "anonymous" {
        let workspace_id = match db::get_session_workspace_id(&state.db, &session_id).await {
            Ok(Some(w)) => w,
            Ok(None) => return workspace_scoped_not_found("session not found"),
            Err(e) => {
                tracing::error!("failed to load session workspace: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        };
        if let Err(r) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
            return r;
        }
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
        Ok(None) => workspace_scoped_not_found("session not found"),
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
    if auth_user.user_id != "anonymous" {
        let workspace_id = match db::get_session_workspace_id(&state.db, &session_id).await {
            Ok(Some(w)) => w,
            Ok(None) => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "session not found" })),
                ));
            }
            Err(e) => {
                tracing::error!("failed to load session workspace: {e}");
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                ));
            }
        };
        if let Err(_resp) = require_workspace_access(&state.db, &auth_user, &workspace_id).await {
            // The helper already produced the right status (403 PAT vs
            // 404 non-member); the SSE-typed return forces us to map it
            // back into a JSON tuple.  We err on the side of 404 here
            // for the same existence-leak reason — a caller that can't
            // see the workspace shouldn't see the session id either.
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            ));
        }
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
