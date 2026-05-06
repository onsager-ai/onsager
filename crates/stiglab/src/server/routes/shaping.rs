//! Shaping dispatch core — used by `shaping_listener` to spawn an
//! agent session in response to `forge.shaping_dispatched`.
//!
//! Phase 5 of Lever C (#148) deleted the legacy `POST /api/shaping`
//! entrypoint, and #209 deleted the dashboard-facing
//! `GET /api/shaping/{session_id}` long-poll status endpoint
//! (the dashboard reads session state via `GET /api/sessions/{id}`
//! and the spine event feed). What's left here is the shared
//! dispatch core called by the spine listener.

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::core::adapter;
use crate::core::{ServerMessage, Session, SessionState};

use crate::server::auth::decrypt_credential;
use crate::server::db;
use crate::server::state::AppState;

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
        (Some(uid), Some(ws)) => fetch_workspace_credentials(state, ws, uid).await,
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

/// Fetch the credentials a user holds in a specific workspace, decrypt
/// them, and return as a HashMap of env-var name → plaintext value.
/// Returns `None` when the credential key is unset or there are no creds.
pub(crate) async fn fetch_workspace_credentials(
    state: &AppState,
    workspace_id: &str,
    user_id: &str,
) -> Option<HashMap<String, String>> {
    let key = state.config.credential_key.as_deref()?;

    let encrypted_creds = db::get_all_user_credential_values(&state.db, workspace_id, user_id)
        .await
        .ok()?;

    if encrypted_creds.is_empty() {
        return None;
    }

    let mut result = HashMap::new();
    for (name, encrypted_value) in encrypted_creds {
        match decrypt_credential(key, &encrypted_value) {
            Ok(value) => {
                result.insert(name, value);
            }
            Err(e) => {
                tracing::error!("failed to decrypt credential {name} for user {user_id}: {e}");
            }
        }
    }

    if result.is_empty() { None } else { Some(result) }
}
