//! Shaping HTTP endpoints (the dashboard's read side).
//!
//! Phase 5 of Lever C (#148) deleted the legacy `POST /api/shaping`
//! entrypoint that forge used; spawn-side dispatch now flows through
//! the spine via `crates/stiglab/src/server/shaping_listener.rs`,
//! which calls [`dispatch_shaping_inner`] (still defined here so the
//! HTTP and listener paths share one dispatch core if a future use
//! case re-introduces an HTTP wrapper).
//!
//! What remains:
//!
//! - `GET  /api/shaping/{id}`      → 200 with `ShapingResult` if terminal, else 202 with current state
//! - `GET  /api/shaping/{id}?wait=Ns` → same, but blocks up to N seconds waiting for terminal state
//!
//! `wait` is capped at [`MAX_WAIT_SECS`] so individual HTTP roundtrips stay
//! friendly to load balancers, proxies and mobile networks.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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

/// Outcome of [`dispatch_shaping_inner`] — distinguishes a freshly created
/// session from one returned via the idempotency-key fast path.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// A new session was inserted and dispatched. The `Session` is the
    /// version this server holds in memory (state may have been advanced
    /// to `Dispatched` after the WS send).
    Created(Session),
    /// A previous request with the same idempotency key already produced
    /// a session. The caller should return this session's identity rather
    /// than spawning a duplicate.
    Idempotent(Session),
    /// No connected agent could pick up the dispatch — caller decides
    /// whether to error (HTTP 503) or retry on the next event.
    NoAvailableNode,
}

/// Core shaping-dispatch logic. After Lever C phase 5 the only
/// production caller is `crate::server::shaping_listener` consuming
/// `forge.shaping_dispatched`; the HTTP `POST /api/shaping` wrapper
/// is gone. The function stays factored out so a future caller (HTTP
/// re-introduction, tests) gets a single dispatch core.
///
/// **Trust decisions on request fields stay with the caller.**
/// `req.created_by` drives credential lookup and the spawned agent's
/// `CLAUDE_CODE_OAUTH_TOKEN` injection; the caller is responsible for
/// ensuring that value came from a trusted source before calling here.
/// The current caller (`shaping_listener`) gates `created_by` behind a
/// metadata `actor == "forge"` check on the spine event row.
/// Anything that doesn't pass its trust check must strip `created_by`
/// to `None` before reaching this helper.
///
/// `idempotency_key` is the durable handle the caller uses to collapse
/// retries onto a single session. The listener path uses the outer
/// envelope's `request_id` after validating it matches the embedded
/// payload.
pub async fn dispatch_shaping_inner(
    state: &AppState,
    req: &onsager_spine::protocol::ShapingRequest,
    idempotency_key: &str,
) -> Result<DispatchOutcome, anyhow::Error> {
    // Idempotency fast path — the definitive check is the conflict-aware
    // insert below; this lookup just lets the common case skip the work.
    if !idempotency_key.is_empty() {
        match db::find_session_by_idempotency_key(&state.db, idempotency_key).await {
            Ok(Some(existing)) => return Ok(DispatchOutcome::Idempotent(existing)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("idempotency lookup failed: {e}");
                // Fall through and treat as a new request.
            }
        }
    }

    let task = adapter::shaping_request_to_task(req);

    let target_node = match db::find_least_loaded_node(&state.db).await? {
        Some(node) => node,
        None => return Ok(DispatchOutcome::NoAvailableNode),
    };

    // Resolve the workspace this dispatch targets from the spine
    // artifact row (#164).  The session's workspace pins which
    // credential set the agent runner reaches for and which `?workspace=`
    // listing surfaces it. Falls back to None when the spine isn't wired
    // (dev / tests without spine), in which case credentials are skipped
    // — same loud-fail behaviour as the no-`created_by` path.
    let workspace_id = resolve_workspace_for_artifact(state, req.artifact_id.as_str()).await;

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
        db::insert_session_with_user_project_workspace(
            &state.db,
            &session,
            req.created_by.as_deref(),
            None,
            workspace_id.as_deref(),
        )
        .await?;
    } else {
        match insert_session_with_idempotency_and_workspace(
            &state.db,
            &session,
            req.created_by.as_deref(),
            workspace_id.as_deref(),
            idempotency_key,
        )
        .await?
        {
            true => { /* new session inserted */ }
            false => {
                // Concurrent insert with the same key won the race; load
                // and return the persisted row so callers converge.
                if let Some(existing) =
                    db::find_session_by_idempotency_key(&state.db, idempotency_key).await?
                {
                    return Ok(DispatchOutcome::Idempotent(existing));
                }
                return Err(anyhow::anyhow!(
                    "idempotency conflict on {idempotency_key} but no session visible"
                ));
            }
        }
    }

    // Fetch credentials scoped to the session's workspace (#164). A
    // request without both `created_by` and a resolvable workspace gets
    // `None` — the resulting session_failed event surfaces the broken
    // state via forge's signal listener rather than dispatching with the
    // wrong workspace's secrets.
    let credentials = match (req.created_by.as_deref(), workspace_id.as_deref()) {
        (Some(uid), Some(ws)) => super::tasks::fetch_workspace_credentials(state, ws, uid).await,
        _ => None,
    };

    // Dispatch to the agent over WebSocket.
    {
        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&target_node.id) {
            let msg = ServerMessage::DispatchTask {
                task: Box::new(task.clone()),
                session_id: session.id.clone(),
                credentials,
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = agent
                    .sender
                    .send(axum::extract::ws::Message::Text(json.into()));
            }
            db::update_session_state(&state.db, &session.id, SessionState::Dispatched).await?;
            session.state = SessionState::Dispatched;
        } else {
            tracing::warn!(
                "agent for node {} not connected, session stays pending",
                target_node.id
            );
        }
    }

    // Emit session-started spine event.
    if let Some(ref spine) = state.spine {
        let _ = spine
            .emit_session_started(&session.id, &req.request_id, &target_node.id)
            .await;
    }

    Ok(DispatchOutcome::Created(session))
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
    let synthesized_req = onsager_spine::protocol::ShapingRequest {
        request_id: session.task_id.clone(),
        artifact_id: onsager_artifact::ArtifactId::new(&artifact_id_str),
        target_version: session.artifact_version.unwrap_or(0).max(0) as u32,
        shaping_intent: serde_json::json!({}),
        inputs: vec![],
        constraints: vec![],
        deadline: None,
        // Synthesised for adapter shape-matching only; not dispatched
        // anywhere. Owner identity isn't part of the persisted session
        // row, and the adapter doesn't read this field anyway.
        created_by: None,
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

/// Resolve the workspace an artifact belongs to by querying the spine
/// `artifacts.workspace_id` column added in #162. Returns `None` when
/// the spine isn't connected, the artifact row is missing, or the
/// query fails — every failure mode degrades gracefully and the
/// caller treats a missing workspace as "no credentials, dispatch and
/// fail loudly".
async fn resolve_workspace_for_artifact(state: &AppState, artifact_id: &str) -> Option<String> {
    let spine = state.spine.as_ref()?;
    sqlx::query_scalar::<_, String>("SELECT workspace_id FROM artifacts WHERE artifact_id = $1")
        .bind(artifact_id)
        .fetch_optional(spine.pool())
        .await
        .ok()
        .flatten()
}

/// Insert a session bound to an idempotency key with full workspace +
/// user context. Mirrors `db::insert_session_with_idempotency_key` but
/// also writes `user_id` and `workspace_id` so the session row has the
/// full provenance the new list/credential paths need (#164).
async fn insert_session_with_idempotency_and_workspace(
    pool: &sqlx::AnyPool,
    session: &Session,
    user_id: Option<&str>,
    workspace_id: Option<&str>,
    idempotency_key: &str,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, workspace_id, artifact_id, artifact_version, \
                               idempotency_key, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(user_id)
    .bind(workspace_id)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(idempotency_key)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
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
