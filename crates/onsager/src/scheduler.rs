//! The in-process scheduler (ADR 0029): a fire is a function call that
//! creates a run row and a supervised tokio task. Execution iterates
//! the workflow's stage list; every step is recorded on the events
//! table (the audit record). The v1 plan compiler, plan store, and
//! event-dispatch loop collapse into this module.

use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::random_token;
use crate::domain::{StageKind, Workflow};
use crate::events::record;
use crate::sessions::{SessionRequest, run_agent_session, session_delivered};
use crate::state::AppState;

/// Fire a workflow: insert the run row, spawn the execution task,
/// return the run id immediately.
pub async fn fire(state: &AppState, workflow: Workflow) -> anyhow::Result<String> {
    let run_id = format!("run_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO runs (id, workspace_id, workflow_id, status) VALUES ($1, $2, $3, 'running')",
    )
    .bind(&run_id)
    .bind(&workflow.workspace_id)
    .bind(&workflow.id)
    .execute(&state.pool)
    .await?;
    record(
        &state.pool,
        &workflow.workspace_id,
        Some(&run_id),
        "run.started",
        serde_json::json!({ "workflow_id": workflow.id, "workflow_name": workflow.name }),
    )
    .await;

    let state = state.clone();
    let id = run_id.clone();
    tokio::spawn(async move {
        let workspace_id = workflow.workspace_id.clone();
        match execute(&state, &workflow, &id).await {
            Ok(()) => {
                finish(&state.pool, &id, "completed", None).await;
                record(
                    &state.pool,
                    &workspace_id,
                    Some(&id),
                    "run.completed",
                    serde_json::json!({ "workflow_id": workflow.id }),
                )
                .await;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!(run_id = %id, "run failed: {msg}");
                finish(&state.pool, &id, "failed", Some(&msg)).await;
                record(
                    &state.pool,
                    &workspace_id,
                    Some(&id),
                    "run.failed",
                    serde_json::json!({ "workflow_id": workflow.id, "error": msg }),
                )
                .await;
            }
        }
    });
    Ok(run_id)
}

async fn finish(pool: &PgPool, run_id: &str, status: &str, error: Option<&str>) {
    let result =
        sqlx::query("UPDATE runs SET status = $2, error = $3, finished_at = NOW() WHERE id = $1")
            .bind(run_id)
            .bind(status)
            .bind(error)
            .execute(pool)
            .await;
    if let Err(e) = result {
        tracing::error!(run_id, "failed to finish run row: {e}");
    }
}

/// Iterate the stage list. The first failing stage fails the run.
async fn execute(state: &AppState, workflow: &Workflow, run_id: &str) -> anyhow::Result<()> {
    for (index, stage) in workflow.definition.iter().enumerate() {
        match stage {
            StageKind::Agent {
                model,
                system_prompt,
                user_prompt,
            } => {
                run_agent_stage(
                    state,
                    workflow,
                    run_id,
                    index,
                    model.clone(),
                    system_prompt.clone(),
                    user_prompt,
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn run_agent_stage(
    state: &AppState,
    workflow: &Workflow,
    run_id: &str,
    stage_index: usize,
    model: Option<String>,
    system_prompt: Option<String>,
    user_prompt: &str,
) -> anyhow::Result<()> {
    let session_id = format!("sess_{}", Uuid::new_v4());
    let token = random_token();
    sqlx::query(
        "INSERT INTO sessions (id, workspace_id, run_id, token, status) \
         VALUES ($1, $2, $3, $4, 'running')",
    )
    .bind(&session_id)
    .bind(&workflow.workspace_id)
    .bind(run_id)
    .bind(&token)
    .execute(&state.pool)
    .await?;
    record(
        &state.pool,
        &workflow.workspace_id,
        Some(run_id),
        "session.started",
        serde_json::json!({ "session_id": session_id, "stage": stage_index }),
    )
    .await;

    // Workspace credentials become the agent's env (CLAUDE_CODE_OAUTH_TOKEN
    // etc.) — decrypted here, passed in-memory; no payload crypto hop.
    let env = match state.config.credential_key.as_deref() {
        Some(key) => decrypted_credentials(&state.pool, key, &workflow.workspace_id).await?,
        None => Vec::new(),
    };

    // Repo access (#624): a write-scoped installation token for the
    // trigger repo, minted per run, in-memory only.
    let repos = crate::github::repo_access_for(&state.pool, workflow).await;

    let outcome = run_agent_session(SessionRequest {
        session_id: session_id.clone(),
        token,
        model,
        system_prompt,
        user_prompt: user_prompt.to_string(),
        mcp_url: state.config.mcp_url(),
        env,
        repos,
    })
    .await;

    match outcome {
        Ok(out) => {
            sqlx::query(
                "UPDATE sessions SET status = 'completed', finished_at = NOW() WHERE id = $1",
            )
            .bind(&session_id)
            .execute(&state.pool)
            .await?;
            record(
                &state.pool,
                &workflow.workspace_id,
                Some(run_id),
                "session.completed",
                serde_json::json!({
                    "session_id": session_id,
                    "output": out.output.chars().take(2000).collect::<String>(),
                }),
            )
            .await;
            // Liveness, v2 shape: delivered or declared, else fail.
            if !session_delivered(&state.pool, &session_id).await? {
                anyhow::bail!(
                    "session {session_id} ended with no emitted artifact and no \
                     declare_no_output"
                );
            }
            Ok(())
        }
        Err(e) => {
            sqlx::query(
                "UPDATE sessions SET status = 'failed', error = $2, finished_at = NOW() \
                 WHERE id = $1",
            )
            .bind(&session_id)
            .bind(e.to_string())
            .execute(&state.pool)
            .await?;
            record(
                &state.pool,
                &workflow.workspace_id,
                Some(run_id),
                "session.failed",
                serde_json::json!({ "session_id": session_id, "error": e.to_string() }),
            )
            .await;
            Err(anyhow::anyhow!("agent stage {stage_index} failed: {e}"))
        }
    }
}

/// Decrypt every credential for a workspace — the env-injection source
/// for agent sessions. Not exposed over HTTP.
async fn decrypted_credentials(
    pool: &PgPool,
    key: &str,
    workspace_id: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, value_cipher FROM credentials WHERE workspace_id = $1")
            .bind(workspace_id)
            .fetch_all(pool)
            .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (name, cipher) in rows {
        out.push((name, crate::crypto::decrypt(key, &cipher)?));
    }
    Ok(out)
}
