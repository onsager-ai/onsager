//! Shared logic for processing AgentMessage events (used by both the WebSocket
//! handler and the built-in runner).

use crate::core::{AgentMessage, SessionState};
use sqlx::AnyPool;

use crate::server::db;
use crate::server::spine::SpineEmitter;

/// Process an `AgentMessage` by applying the corresponding DB mutations.
/// `node_id` identifies the agent node (used only for heartbeat updates).
/// If `spine` is provided, factory events are emitted on session transitions.
pub async fn handle_agent_message(
    pool: &AnyPool,
    node_id: &str,
    msg: AgentMessage,
    spine: Option<&SpineEmitter>,
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
            if let Err(e) = db::update_session_state(pool, &session_id, *state).await {
                tracing::error!("failed to update session state: {e}");
            }
            // Emit spine event when session transitions to Running
            if *state == SessionState::Running {
                if let Some(spine) = spine {
                    // We don't have request_id here, use task_id from the session as correlation
                    if let Err(e) = spine.emit_session_started(&session_id, "", node_id).await {
                        tracing::warn!("failed to emit session_started spine event: {e}");
                    }
                }
            }
        }
        AgentMessage::SessionOutput {
            session_id,
            chunk,
            stream,
        } => {
            // Normalize: anything that isn't "stderr" is treated as "stdout"
            let stream = if stream == "stderr" { "stderr" } else { "stdout" };
            if let Err(e) = db::append_session_log(pool, &session_id, &chunk, stream).await {
                tracing::error!("failed to append session log: {e}");
            }
        }
        AgentMessage::SessionCompleted { session_id, output } => {
            if let Err(e) = db::update_session_state(pool, &session_id, SessionState::Done).await {
                tracing::error!("failed to update session state to done: {e}");
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
                if let Err(e) = spine.emit_session_completed(&session_id, "", 0).await {
                    tracing::warn!("failed to emit session_completed spine event: {e}");
                }
            }
        }
        AgentMessage::SessionFailed { session_id, error } => {
            if let Err(e) = db::update_session_state(pool, &session_id, SessionState::Failed).await
            {
                tracing::error!("failed to update session state to failed: {e}");
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
