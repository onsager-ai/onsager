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
            // Emit spine event for session completion
            if let Some(spine) = spine {
                if let Err(e) = spine.emit_session_completed(&session_id, "", 0, None).await {
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
