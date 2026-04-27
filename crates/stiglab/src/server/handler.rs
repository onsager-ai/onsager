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
            //
            // `artifact_id` is loaded by the same query so forge's
            // `SessionLinker` can write `vertical_lineage`; without it
            // the workflow detail page's Sessions card stays empty even
            // when sessions run.
            if let Some(spine) = spine {
                let (branch, project_id, artifact_id) =
                    git_context_for_session(pool, &session_id).await;
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
                    .emit_session_completed(
                        &session_id,
                        "",
                        0,
                        artifact_id.as_deref(),
                        None,
                        branch.as_deref(),
                        None,
                    )
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
            // Emit spine event for session failure. Carry artifact_id so
            // forge's workflow signal listener can fail the agent-session
            // gate loudly (issue #156) — without it the artifact stalls
            // at stage 0 and forge re-dispatches every tick forever.
            //
            // Use the DB-only `artifact_id_for_session` lookup rather
            // than `git_context_for_session` — the failure path doesn't
            // need branch/project context, and `git_context_for_session`
            // spawns a `git rev-parse` subprocess we'd be discarding.
            if let Some(spine) = spine {
                let artifact_id = artifact_id_for_session(pool, &session_id).await;
                if let Err(e) = spine
                    .emit_session_failed(&session_id, "", &error, artifact_id.as_deref())
                    .await
                {
                    tracing::warn!("failed to emit session_failed spine event: {e}");
                }
            }
        }
        AgentMessage::Register(_) => {
            // Registration is handled separately (node creation + WS setup)
        }
    }
}

/// DB-only `artifact_id` lookup for a session — used by paths that need
/// the artifact link but not the full git/project context (e.g. the
/// session_failed spine emission, issue #156). Cheaper than
/// `git_context_for_session` because it never spawns `git rev-parse`.
async fn artifact_id_for_session(pool: &AnyPool, session_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT artifact_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

/// Best-effort branch + project + artifact lookup for a session at
/// completion time. Reads `working_dir` from the session row, asks
/// `git rev-parse --abbrev-ref HEAD` for the active branch, joins
/// `sessions.project_id` to surface the owning project (so the portal
/// can scope its branch-link lookup), and returns `artifact_id` so the
/// caller can attach it to the spine event without a second roundtrip.
/// Failures at any step degrade gracefully to `None` — every field is
/// additive context.
async fn git_context_for_session(
    pool: &AnyPool,
    session_id: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let row: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT working_dir, project_id, artifact_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some((working_dir, project_id, artifact_id)) = row else {
        return (None, None, None);
    };
    let Some(working_dir) = working_dir else {
        return (None, project_id, artifact_id);
    };
    // Detached HEAD and empty output both mean "no usable branch name" —
    // record neither, otherwise the portal's branch-match lookup could
    // correlate unrelated sessions. The 5-second timeout keeps session
    // completion responsive against a stuck git invocation (e.g. a broken
    // index lock file) instead of blocking the event loop indefinitely.
    let mut cmd = tokio::process::Command::new("git");
    cmd.kill_on_drop(true);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cmd.args(["-C", &working_dir, "rev-parse", "--abbrev-ref", "HEAD"])
            .output(),
    )
    .await
    .ok()
    .and_then(Result::ok);
    let branch = output.filter(|o| o.status.success()).and_then(|o| {
        let name = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        if name.is_empty() || name == "HEAD" {
            None
        } else {
            Some(name)
        }
    });
    (branch, project_id, artifact_id)
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
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO pr_branch_links (session_id, project_id, branch, recorded_at) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (session_id) DO UPDATE SET branch = EXCLUDED.branch, \
            project_id = EXCLUDED.project_id, recorded_at = EXCLUDED.recorded_at",
    )
    .bind(session_id)
    .bind(project_id)
    .bind(branch)
    .bind(&now)
    .execute(spine_pool)
    .await?;
    Ok(())
}
