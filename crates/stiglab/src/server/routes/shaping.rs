//! `POST /api/shaping` — accept a Forge shaping request, dispatch to an agent,
//! and return immediately with a status URL the caller can poll.
//!
//! Issue #31: the previous implementation kept the HTTP request open for up
//! to 5 minutes while polling the database. This was the proximate cause of
//! issue #28 (Forge's tick held the pipeline write lock across that whole
//! window, freezing the dashboard).
//!
//! New shape:
//!
//! - `POST /api/shaping`           → 202 Accepted, body `{ session_id, status_url, request_id }`
//! - `GET  /api/shaping/{id}`      → 200 with `ShapingResult` if terminal, else 202 with current state
//! - `GET  /api/shaping/{id}?wait=Ns` → same, but blocks up to N seconds waiting for terminal state
//!
//! `wait` is capped at [`MAX_WAIT_SECS`] so individual HTTP roundtrips stay
//! friendly to load balancers, proxies and mobile networks. Forge's
//! dispatcher loops until the overall shaping deadline elapses.
//!
//! `Idempotency-Key` header (defaulting to `request_id`) makes POST safe to
//! retry: the second call returns the original session id instead of
//! dispatching a second agent.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::core::adapter;
use crate::core::{ServerMessage, Session, SessionState};

use crate::server::db;
use crate::server::state::AppState;

/// Hard cap on the per-request `wait` window. The full shaping deadline can
/// still be longer; the caller is expected to loop.
const MAX_WAIT_SECS: u64 = 30;

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    /// e.g. "20s" or "1500ms" or a bare seconds integer.
    #[serde(default)]
    pub wait: Option<String>,
}

/// `POST /api/shaping` — accept a shaping request and return immediately.
pub async fn create_shaping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<onsager_spine::ShapingRequest>,
) -> impl IntoResponse {
    // Idempotency: prefer the explicit header so callers can override, fall
    // back to request_id which Forge already promises is stable across retries
    // (forge invariant #6).
    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| req.request_id.clone());

    // Fast path: if a session for this key already exists, return it. The
    // definitive check is performed on insert (ON CONFLICT DO NOTHING) to
    // close the lookup/insert race. Echo the session's original request_id
    // (persisted as task_id) rather than the caller's — a retry with a
    // different request_id but the same Idempotency-Key should return the
    // original request's identity.
    if !idempotency_key.is_empty() {
        match db::find_session_by_idempotency_key(&state.db, &idempotency_key).await {
            Ok(Some(existing)) => {
                let rid = existing.task_id.clone();
                return accepted_response(&existing, &rid);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("idempotency lookup failed: {e}");
                // Fall through and treat as a new request.
            }
        }
    }

    let task = adapter::shaping_request_to_task(&req);

    // Find target node (auto-assign to least loaded).
    let target_node = match db::find_least_loaded_node(&state.db).await {
        Ok(Some(node)) => node,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "no available nodes for dispatch" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("failed to find node: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let mut session = Session {
        id: Uuid::new_v4().to_string(),
        task_id: task.id.clone(),
        node_id: target_node.id.clone(),
        state: SessionState::Pending,
        prompt: task.prompt.clone(),
        output: None,
        working_dir: task.working_dir.clone(),
        artifact_id: Some(req.artifact_id.to_string()),
        artifact_version: Some(req.target_version as i32),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    if idempotency_key.is_empty() {
        if let Err(e) = db::insert_session(&state.db, &session).await {
            tracing::error!("failed to insert session: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    } else {
        match db::insert_session_with_idempotency_key(&state.db, &session, &idempotency_key).await {
            Ok(true) => { /* new session inserted */ }
            Ok(false) => {
                // Concurrent POST with the same key won the race. Load and
                // return whichever session is actually persisted so the
                // caller converges on a single session id.
                match db::find_session_by_idempotency_key(&state.db, &idempotency_key).await {
                    Ok(Some(existing)) => {
                        let rid = existing.task_id.clone();
                        return accepted_response(&existing, &rid);
                    }
                    Ok(None) | Err(_) => {
                        return (
                            StatusCode::CONFLICT,
                            Json(serde_json::json!({
                                "error": "idempotency conflict but no session visible"
                            })),
                        )
                            .into_response();
                    }
                }
            }
            Err(e) => {
                tracing::error!("failed to insert session: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    }

    // Dispatch to agent via WebSocket.
    {
        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&target_node.id) {
            let msg = ServerMessage::DispatchTask {
                task: Box::new(task.clone()),
                session_id: session.id.clone(),
                credentials: None,
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = agent
                    .sender
                    .send(axum::extract::ws::Message::Text(json.into()));
            }
            let _ =
                db::update_session_state(&state.db, &session.id, SessionState::Dispatched).await;
            session.state = SessionState::Dispatched;
        } else {
            tracing::warn!(
                "agent for node {} not connected, session stays pending",
                target_node.id
            );
        }
    }

    // Emit session started event to spine.
    if let Some(ref spine) = state.spine {
        let _ = spine
            .emit_session_started(&session.id, &req.request_id, &target_node.id)
            .await;
    }

    accepted_response(&session, &req.request_id)
}

/// `GET /api/shaping/{session_id}?wait=Ns` — return the current shaping
/// status. With `wait`, blocks up to N seconds (capped at [`MAX_WAIT_SECS`])
/// for a terminal transition, using an in-process broadcast channel so the
/// database is queried at most once per call.
pub async fn get_shaping_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<StatusQuery>,
) -> impl IntoResponse {
    let session = match db::get_session(&state.db, &session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("failed to load session: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if is_terminal(session.state) {
        return terminal_response(&session).await;
    }

    let Some(wait_str) = query.wait.as_deref() else {
        return pending_response(&session);
    };

    let Some(wait) = parse_wait(wait_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "wait must be a duration like '20s', '1500ms', or a bare seconds integer"
            })),
        )
            .into_response();
    };

    // Subscribe before re-checking the DB to avoid a race where the session
    // becomes terminal between the check above and the subscription below.
    let mut rx = state.session_completion_tx.subscribe();
    if let Ok(Some(s)) = db::get_session(&state.db, &session_id).await {
        if is_terminal(s.state) {
            return terminal_response(&s).await;
        }
    }

    let target = session_id.clone();
    let signaled = tokio::time::timeout(wait, async move {
        loop {
            match rx.recv().await {
                Ok(id) if id == target => return true,
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    if !signaled {
        // Timed out — return current pending state without re-querying twice.
        match db::get_session(&state.db, &session_id).await {
            Ok(Some(s)) if is_terminal(s.state) => terminal_response(&s).await,
            Ok(Some(s)) => pending_response(&s),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        // A broadcast fired for this session_id; re-read the DB to get the
        // authoritative state. The notifier only fires on a successful state
        // update so this should be terminal, but verify before returning
        // 200 (a spurious broadcast must not promote a non-terminal session
        // to a terminal response).
        match db::get_session(&state.db, &session_id).await {
            Ok(Some(s)) if is_terminal(s.state) => terminal_response(&s).await,
            Ok(Some(s)) => pending_response(&s),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "session not found" })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    }
}

fn accepted_response(session: &Session, request_id: &str) -> axum::response::Response {
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": request_id,
            "session_id": session.id,
            "status_url": format!("/api/shaping/{}", session.id),
            "state": session.state.to_string(),
        })),
    )
        .into_response()
}

fn pending_response(session: &Session) -> axum::response::Response {
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "session_id": session.id,
            "state": session.state.to_string(),
            "status_url": format!("/api/shaping/{}", session.id),
        })),
    )
        .into_response()
}

async fn terminal_response(session: &Session) -> axum::response::Response {
    // Reconstruct a ShapingRequest-shaped envelope from the session's stored
    // artifact link so the adapter can build a ShapingResult identical to
    // what the legacy long-poll endpoint used to return.
    //
    // This handler does NOT emit spine events — the agent message handler
    // (`crate::server::handler::handle_agent_message`) is the single source
    // of terminal transition events. Emitting from GET as well would make
    // the status endpoint non-idempotent and spam duplicates for every
    // poll after a session terminates.
    let artifact_id_str = session.artifact_id.clone().unwrap_or_default();
    let synthesized_req = onsager_spine::ShapingRequest {
        request_id: session.task_id.clone(),
        artifact_id: onsager_spine::artifact::ArtifactId::new(&artifact_id_str),
        target_version: session.artifact_version.unwrap_or(0).max(0) as u32,
        shaping_intent: serde_json::json!({}),
        inputs: vec![],
        constraints: vec![],
        deadline: None,
    };

    let duration_ms = session
        .updated_at
        .signed_duration_since(session.created_at)
        .num_milliseconds()
        .max(0) as u64;

    let result = adapter::session_to_shaping_result(&synthesized_req, session, duration_ms);

    (StatusCode::OK, Json(result)).into_response()
}

fn is_terminal(state: SessionState) -> bool {
    matches!(state, SessionState::Done | SessionState::Failed)
}

/// Parse `"30s"`, `"1500ms"`, or a bare seconds integer like `"15"`. Returns
/// `None` on parse failure. The result is clamped to [`MAX_WAIT_SECS`].
fn parse_wait(raw: &str) -> Option<Duration> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let dur = if let Some(ms_str) = raw.strip_suffix("ms") {
        let ms: u64 = ms_str.trim().parse().ok()?;
        Duration::from_millis(ms)
    } else if let Some(s_str) = raw.strip_suffix('s') {
        let s: u64 = s_str.trim().parse().ok()?;
        Duration::from_secs(s)
    } else {
        let s: u64 = raw.parse().ok()?;
        Duration::from_secs(s)
    };
    Some(dur.min(Duration::from_secs(MAX_WAIT_SECS)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wait_accepts_seconds_with_suffix() {
        assert_eq!(parse_wait("20s"), Some(Duration::from_secs(20)));
        assert_eq!(parse_wait("0s"), Some(Duration::from_secs(0)));
    }

    #[test]
    fn parse_wait_accepts_milliseconds() {
        assert_eq!(parse_wait("500ms"), Some(Duration::from_millis(500)));
    }

    #[test]
    fn parse_wait_accepts_bare_seconds_integer() {
        assert_eq!(parse_wait("15"), Some(Duration::from_secs(15)));
    }

    #[test]
    fn parse_wait_caps_at_max_wait() {
        assert_eq!(
            parse_wait("3600s"),
            Some(Duration::from_secs(MAX_WAIT_SECS))
        );
        assert_eq!(
            parse_wait("999999"),
            Some(Duration::from_secs(MAX_WAIT_SECS))
        );
    }

    #[test]
    fn parse_wait_rejects_garbage() {
        assert!(parse_wait("abc").is_none());
        assert!(parse_wait("").is_none());
        assert!(parse_wait("  ").is_none());
    }

    #[test]
    fn is_terminal_matches_done_and_failed() {
        assert!(is_terminal(SessionState::Done));
        assert!(is_terminal(SessionState::Failed));
        assert!(!is_terminal(SessionState::Running));
        assert!(!is_terminal(SessionState::Pending));
        assert!(!is_terminal(SessionState::Dispatched));
    }
}
