//! Shared logic for processing AgentMessage events (used by both the WebSocket
//! handler and the built-in runner).

use crate::core::{AgentMessage, SessionState};
use sqlx::AnyPool;
use tokio::sync::broadcast;

use crate::server::db;
use crate::server::spine::SpineEmitter;

/// Notify in-process waiters that a session reached a terminal state.
///
/// `tx` is `None` for the runner code path that doesn't carry an `AppState`
/// — those callers don't have HTTP `wait` clients to notify, so the no-op
/// is safe.
fn notify_terminal(tx: Option<&broadcast::Sender<String>>, session_id: &str) {
    if let Some(tx) = tx {
        // Errors are expected when there are zero subscribers; ignore.
        let _ = tx.send(session_id.to_string());
    }
}

/// Process an `AgentMessage` by applying the corresponding DB mutations.
/// `node_id` identifies the agent node (used only for heartbeat updates).
/// If `spine` is provided, factory events are emitted on session transitions.
/// If `completion_tx` is provided, in-process waiters subscribed to it are
/// notified when a session reaches Done or Failed (issue #31).
pub async fn handle_agent_message(
    pool: &AnyPool,
    node_id: &str,
    msg: AgentMessage,
    spine: Option<&SpineEmitter>,
    completion_tx: Option<&broadcast::Sender<String>>,
) {
    match msg {
        AgentMessage::Heartbeat { active_sessions } => {
            if let Err(e) = db::update_node_heartbeat(pool, node_id, active_sessions).await {
                tracing::warn!(
                    node_id = %node_id,
                    active_sessions,
                    error = ?e,
                    "failed to update heartbeat"
                );
            }
        }
        AgentMessage::SessionStateChanged {
            session_id,
            ref state,
        } => {
            let persisted = match db::update_session_state(pool, &session_id, *state).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!("failed to update session state: {e}");
                    false
                }
            };
            // Emit spine event when session transitions to Running
            if *state == SessionState::Running {
                if let Some(spine) = spine {
                    // We don't have request_id here, use task_id from the session as correlation
                    if let Err(e) = spine.emit_session_started(&session_id, "", node_id).await {
                        tracing::warn!("failed to emit session_started spine event: {e}");
                    }
                }
            }
            if persisted && matches!(*state, SessionState::Done | SessionState::Failed) {
                notify_terminal(completion_tx, &session_id);
            }
        }
        AgentMessage::SessionOutput {
            session_id,
            chunk,
            stream,
        } => {
            // Normalize: anything that isn't "stderr" is treated as "stdout"
            let stream = if stream == "stderr" {
                "stderr"
            } else {
                "stdout"
            };
            if let Err(e) = db::append_session_log(pool, &session_id, &chunk, stream).await {
                tracing::error!("failed to append session log: {e}");
            }
        }
        AgentMessage::SessionCompleted { session_id, output } => {
            let persisted =
                match db::update_session_state(pool, &session_id, SessionState::Done).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::error!("failed to update session state to done: {e}");
                        false
                    }
                };
            if persisted {
                notify_terminal(completion_tx, &session_id);
            }
            // Only persist the final output as a fallback when no chunks were
            // already streamed, to avoid duplicating the entire response.
            if !output.is_empty() {
                let already_streamed = db::get_session_logs(pool, &session_id)
                    .await
                    .map(|logs| !logs.is_empty())
                    .unwrap_or(false);
                if !already_streamed {
                    if let Err(e) =
                        db::append_session_log(pool, &session_id, &output, "stdout").await
                    {
                        tracing::error!("failed to append final session output: {e}");
                    }
                }
            }
            // Emit spine event for session completion. Best-effort branch
            // detection (issue #60): if the session has a working_dir, ask
            // git for the current branch so the portal can attach
            // vertical_lineage when the matching PR webhook arrives.
            if let Some(spine) = spine {
                let (branch, project_id) = git_context_for_session(pool, &session_id).await;
                if let Some(branch_name) = branch.as_deref() {
                    let spine_pool = spine.pool().clone();
                    if let Err(e) = record_branch_link(
                        &spine_pool,
                        &session_id,
                        project_id.as_deref(),
                        branch_name,
                    )
                    .await
                    {
                        tracing::warn!("failed to record branch link: {e}");
                    }
                }
                if let Err(e) = spine
                    .emit_session_completed(&session_id, "", 0, None, None, branch.as_deref(), None)
                    .await
                {
                    tracing::warn!("failed to emit session_completed spine event: {e}");
                }
            }
        }
        AgentMessage::SessionFailed { session_id, error } => {
            let persisted =
                match db::update_session_state(pool, &session_id, SessionState::Failed).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::error!("failed to update session state to failed: {e}");
                        false
                    }
                };
            if persisted {
                notify_terminal(completion_tx, &session_id);
            }
            if let Err(e) = db::append_session_log(pool, &session_id, &error, "stderr").await {
                tracing::error!("failed to append session log: {e}");
            }
            // Emit spine event for session failure
            if let Some(spine) = spine {
                if let Err(e) = spine.emit_session_failed(&session_id, "", &error).await {
                    tracing::warn!("failed to emit session_failed spine event: {e}");
                }
            }
        }
        AgentMessage::Register(_) => {
            // Registration is handled separately (node creation + WS setup)
        }
    }
}

/// Best-effort branch + project lookup for a session at completion time.
/// Reads `working_dir` from the session row, asks `git rev-parse --abbrev-ref HEAD`
/// for the active branch, and joins `sessions.project_id` to surface the
/// owning project (so the portal can scope its branch-link lookup). Failures
/// at any step degrade gracefully to `None` — branch/PR detection is purely
/// additive context.
async fn git_context_for_session(
    pool: &AnyPool,
    session_id: &str,
) -> (Option<String>, Option<String>) {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT working_dir, project_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some((working_dir, project_id)) = row else {
        return (None, None);
    };
    let Some(working_dir) = working_dir else {
        return (None, project_id);
    };
    let branch = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .args(["-C", &working_dir, "rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    })
    .await
    .ok()
    .flatten();
    (branch, project_id)
}

/// Persist a `(session_id, branch, project_id)` row in the portal-managed
/// `pr_branch_links` table so the portal's PR-opened handler can resolve
/// vertical lineage on webhook arrival. Stiglab writes the row via the spine
/// pool so the portal sees it on the same Postgres instance — without a
/// stiglab→portal HTTP call.
async fn record_branch_link(
    spine_pool: &sqlx::PgPool,
    session_id: &str,
    project_id: Option<&str>,
    branch: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pr_branch_links (session_id, project_id, branch, recorded_at) \
         VALUES ($1, $2, $3, NOW()) \
         ON CONFLICT (session_id) DO UPDATE SET branch = EXCLUDED.branch, \
            project_id = EXCLUDED.project_id, recorded_at = NOW()",
    )
    .bind(session_id)
    .bind(project_id)
    .bind(branch)
    .execute(spine_pool)
    .await?;
    Ok(())
}
