//! Shared logic for processing AgentMessage events (used by both the WebSocket
//! handler and the built-in runner).

use crate::core::{adapter, AgentMessage, SessionState};
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

                // Emit `stiglab.shaping_result_ready` carrying the full
                // `ShapingResult` so Forge's pipeline can resume the
                // parked decision (spec #131 / ADR 0004 Lever C, phase 3).
                // The lifecycle event above stays for dashboard / node
                // telemetry; this event is the actionable signal Forge
                // consumes via `shaping_result_listener`. Sessions
                // without an artifact link are direct task POSTs that
                // never produced a shaping request — skip them so we
                // don't fabricate result events with empty fields.
                if let Some(artifact_id) = artifact_id.as_deref() {
                    if let Err(e) =
                        emit_shaping_result_for_session(pool, spine, &session_id, artifact_id).await
                    {
                        tracing::warn!("failed to emit shaping_result_ready spine event: {e}");
                    }
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

/// Build a `ShapingResult` from a terminal session row and emit it as
/// `stiglab.shaping_result_ready` (spec #131 / ADR 0004 Lever C, phase 3).
///
/// We synthesize a minimal `ShapingRequest` envelope from the session's
/// stored `task_id` (= original request_id) and artifact link so the
/// existing `session_to_shaping_result` adapter produces the same
/// `ShapingResult` shape Forge previously consumed as the synchronous
/// `POST /api/shaping` response body. Fields not stored on the session
/// row (inputs, constraints, deadline) default to empty — Forge's
/// pipeline never reads them off the result, only the request, so the
/// adapter stays consistent with the HTTP path it replaces.
async fn emit_shaping_result_for_session(
    pool: &AnyPool,
    spine: &SpineEmitter,
    session_id: &str,
    artifact_id: &str,
) -> anyhow::Result<()> {
    let Some(session) = db::get_session(pool, session_id).await? else {
        // Session was deleted between message handling and this call —
        // nothing to emit. Logged at debug to avoid noise on race.
        tracing::debug!(
            session_id,
            "shaping_result emit: session row vanished before lookup"
        );
        return Ok(());
    };

    let synthesized_req = onsager_spine::protocol::ShapingRequest {
        request_id: session.task_id.clone(),
        artifact_id: onsager_artifact::ArtifactId::new(artifact_id),
        target_version: session.artifact_version.unwrap_or(0).max(0) as u32,
        shaping_intent: serde_json::json!({}),
        inputs: vec![],
        constraints: vec![],
        deadline: None,
        // Owner identity isn't persisted on the session row; not used
        // by `session_to_shaping_result`, kept here to satisfy the
        // struct shape.
        created_by: None,
    };

    let duration_ms = session
        .updated_at
        .signed_duration_since(session.created_at)
        .num_milliseconds()
        .max(0) as u64;

    let result = adapter::session_to_shaping_result(&synthesized_req, &session, duration_ms);

    spine
        .emit_shaping_result_ready(onsager_artifact::ArtifactId::new(artifact_id), result)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(())
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
