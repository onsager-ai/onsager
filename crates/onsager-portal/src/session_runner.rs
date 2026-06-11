//! In-process agent session runner (spec #583, ADR 0027 / #581).
//!
//! Portal owns the whole chat-session lifecycle now that stiglab is
//! retired: `POST /api/tasks` inserts the `sessions` row and calls
//! [`SessionRunner::spawn_session`] directly — no spine round-trip, no
//! WebSocket dispatch, no agent-node fleet. The runner spawns the
//! Claude CLI (`claude --output-format stream-json …`), streams its
//! NDJSON stdout into `session_logs`, and drives the session row through
//! `running → done / failed`, emitting the `session.completed` /
//! `session.failed` lifecycle events portal's own listeners consume
//! (PR-opener, token revoke).
//!
//! The spawn wiring (Stop-hook settings, MCP config, `ONSAGER_REPOS`)
//! is shared with the scheduler's production runner via
//! `onsager-agent-spawn`; the NDJSON parse + completion handling here
//! are ports of stiglab's `agent/session/process.rs` and
//! `server/handler.rs` (folded per #583).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use onsager_agent_spawn::{
    LIVENESS_HOOK_URL_ENV, MCP_URL_ENV, SESSION_TOKEN_ENV, mcp_config_json, repos_env_json,
    stop_hook_settings_json, ClonableRepo, REPOS_ENV,
};
use onsager_spine::protocol::RepoAccess;
use onsager_spine::{EventMetadata, EventStore};
use serde::Deserialize;
use sqlx::postgres::PgPool;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, Semaphore, oneshot};

use crate::auth::decrypt_credential;
use crate::config::Config;
use crate::core::SessionState;
use crate::{credential_db, session_db};

/// Max concurrent agent sessions (semaphore size). Override via
/// `PORTAL_MAX_SESSIONS`; mirrors stiglab's `--max-sessions` default.
const DEFAULT_MAX_SESSIONS: usize = 4;

// ---------------------------------------------------------------------------
// NDJSON event types emitted by `claude --output-format stream-json`
// ---------------------------------------------------------------------------

/// Top-level envelope for each NDJSON line.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeEvent {
    #[serde(rename = "stream_event")]
    StreamEvent { event: StreamEventInner },
    System {
        subtype: String,
        #[serde(default)]
        session_id: Option<String>,
    },
    Result {
        subtype: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Catch-all for event types we don't handle.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct StreamEventInner {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<DeltaPayload>,
    #[serde(default)]
    content_block: Option<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    name: Option<String>,
}

/// Completion signal parsed off the CLI's NDJSON stdout stream. A
/// "session over" marker only — emitted artifacts are recorded
/// out-of-band via portal's MCP `emit_artifact` tools (spec #520 §4a).
#[derive(Debug, PartialEq)]
pub(crate) enum NdjsonResult {
    Success { output: String },
    Error { error: String },
}

/// What one NDJSON line means for the session log + lifecycle.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ParsedLine {
    /// Chunk to append to the session's stdout log, if any.
    pub chunk: Option<String>,
    /// Terminal result, when this line carried the `result` event.
    pub result: Option<NdjsonResult>,
}

/// Pure NDJSON line mapping (port of stiglab's stdout loop). Text
/// deltas accumulate into `accumulated` so a `result` event without a
/// `result` field can fall back to the streamed text.
pub(crate) fn parse_ndjson_line(line: &str, accumulated: &mut String) -> ParsedLine {
    let event: ClaudeEvent = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(_) => {
            // Non-JSON line — surface as raw output defensively.
            return ParsedLine {
                chunk: Some(format!("{line}\n")),
                result: None,
            };
        }
    };

    match event {
        ClaudeEvent::StreamEvent { event: inner } => match inner.event_type.as_str() {
            "content_block_delta" => {
                if let Some(delta) = inner.delta
                    && delta.delta_type == "text_delta"
                    && let Some(text) = delta.text
                {
                    accumulated.push_str(&text);
                    return ParsedLine {
                        chunk: Some(text),
                        result: None,
                    };
                }
                ParsedLine::default()
            }
            "content_block_start" => {
                if let Some(block) = inner.content_block
                    && block.block_type == "tool_use"
                {
                    let tool_name = block.name.as_deref().unwrap_or("unknown");
                    return ParsedLine {
                        chunk: Some(format!("[tool_use: {tool_name}]\n")),
                        result: None,
                    };
                }
                ParsedLine::default()
            }
            _ => ParsedLine::default(), // content_block_stop and others
        },
        ClaudeEvent::System {
            subtype,
            session_id,
        } => {
            if subtype == "session_id"
                && let Some(csid) = session_id
            {
                tracing::debug!("claude session id: {csid}");
            }
            ParsedLine::default()
        }
        ClaudeEvent::Result {
            subtype,
            result,
            error,
        } => {
            let result = match subtype.as_str() {
                "success" => NdjsonResult::Success {
                    output: result.unwrap_or_else(|| accumulated.clone()),
                },
                _ => NdjsonResult::Error {
                    error: error
                        .unwrap_or_else(|| format!("claude exited with subtype: {subtype}")),
                },
            };
            ParsedLine {
                chunk: None,
                result: Some(result),
            }
        }
        ClaudeEvent::Unknown => ParsedLine::default(),
    }
}

/// Terminal session state for an NDJSON completion result.
pub(crate) fn state_for_result(result: &NdjsonResult) -> SessionState {
    match result {
        NdjsonResult::Success { .. } => SessionState::Done,
        NdjsonResult::Error { .. } => SessionState::Failed,
    }
}

// ---------------------------------------------------------------------------
// SessionRunner
// ---------------------------------------------------------------------------

/// Everything `POST /api/tasks` resolves before handing off to the
/// runner. The session token rides **raw** — it never leaves the
/// process, so there is no encrypt→decrypt round-trip (the old
/// `portal.session_requested` wire carried it encrypted because it
/// crossed the spine to stiglab).
pub struct SessionLaunch {
    pub session_id: String,
    pub prompt: String,
    pub working_dir: Option<String>,
    pub max_turns: Option<u32>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub workspace_id: Option<String>,
    pub user_id: String,
    /// Raw workspace-scoped session token (spec #531), injected as
    /// `ONSAGER_SESSION_TOKEN`. `None` for unscoped sessions or when no
    /// credential key is configured.
    pub session_token: Option<String>,
    /// The workspace's bound repo set with encrypted read tokens
    /// (spec #546); decrypted here and surfaced as `ONSAGER_REPOS`.
    pub repos: Vec<RepoAccess>,
}

/// In-process session executor stored on `AppState`.
pub struct SessionRunner {
    semaphore: Arc<Semaphore>,
    /// Cancel handles for queued + running sessions. `Notify` stores a
    /// permit, so a cancel that races session startup still lands.
    cancels: Mutex<HashMap<String, Arc<Notify>>>,
    agent_command: String,
}

impl SessionRunner {
    /// Build from the environment: `PORTAL_MAX_SESSIONS` (default 4)
    /// and `PORTAL_AGENT_COMMAND` (default `claude`; mirrors stiglab's
    /// retired `STIGLAB_AGENT_COMMAND`).
    pub fn from_env() -> Self {
        let max_sessions = std::env::var("PORTAL_MAX_SESSIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_SESSIONS);
        let agent_command = std::env::var("PORTAL_AGENT_COMMAND")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "claude".to_string());
        Self {
            semaphore: Arc::new(Semaphore::new(max_sessions)),
            cancels: Mutex::new(HashMap::new()),
            agent_command,
        }
    }

    /// Launch a session in the background and return immediately. The
    /// spawned task acquires a concurrency permit, runs the CLI to
    /// completion, and persists every transition — the HTTP handler
    /// only needs the inserted `sessions` row.
    pub fn spawn_session(
        self: &Arc<Self>,
        pool: PgPool,
        spine: EventStore,
        config: Arc<Config>,
        launch: SessionLaunch,
    ) {
        let runner = self.clone();
        let cancel = Arc::new(Notify::new());
        runner
            .cancels
            .lock()
            .expect("cancels lock")
            .insert(launch.session_id.clone(), cancel.clone());

        tokio::spawn(async move {
            let session_id = launch.session_id.clone();
            runner
                .run_session(&pool, &spine, &config, launch, cancel)
                .await;
            runner
                .cancels
                .lock()
                .expect("cancels lock")
                .remove(&session_id);
        });
    }

    /// Cancel a queued or running session. Returns `true` when a live
    /// handle was found (the kill itself is asynchronous; the session
    /// reaches `failed` through the normal completion path).
    pub fn cancel(&self, session_id: &str) -> bool {
        let cancels = self.cancels.lock().expect("cancels lock");
        match cancels.get(session_id) {
            Some(notify) => {
                notify.notify_one();
                true
            }
            None => false,
        }
    }

    async fn run_session(
        &self,
        pool: &PgPool,
        spine: &EventStore,
        config: &Config,
        launch: SessionLaunch,
        cancel: Arc<Notify>,
    ) {
        // Queue on the concurrency permit; a cancel while queued fails
        // the session without ever spawning the CLI.
        let _permit = tokio::select! {
            permit = self.semaphore.acquire() => match permit {
                Ok(p) => p,
                Err(_) => return, // semaphore closed — process shutting down
            },
            _ = cancel.notified() => {
                fail_session(pool, &launch.session_id, "session cancelled before start").await;
                emit_session_failed(pool, spine, &launch, "session cancelled before start").await;
                return;
            }
        };

        let env = self.build_session_env(pool, config, &launch).await;

        let mut child = match self.spawn_cli(&launch, &env) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("failed to spawn agent process: {e}");
                tracing::error!(session_id = %launch.session_id, "{msg}");
                fail_session(pool, &launch.session_id, &msg).await;
                emit_session_failed(pool, spine, &launch, &msg).await;
                return;
            }
        };

        if let Err(e) =
            session_db::update_session_state(pool, &launch.session_id, SessionState::Running).await
        {
            tracing::error!("failed to update session state to running: {e}");
        }

        // -- stdout: NDJSON parsing → session_logs ------------------------
        let (completion_tx, completion_rx) = oneshot::channel::<NdjsonResult>();
        if let Some(stdout) = child.stdout.take() {
            let pool = pool.clone();
            let sid = launch.session_id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut completion_tx = Some(completion_tx);
                let mut accumulated = String::new();
                loop {
                    let line = match lines.next_line().await {
                        Ok(Some(line)) => line,
                        Ok(None) => break, // EOF
                        Err(e) => {
                            if let Some(tx) = completion_tx.take() {
                                let _ = tx.send(NdjsonResult::Error {
                                    error: format!("stdout read error: {e}"),
                                });
                            }
                            return;
                        }
                    };
                    let parsed = parse_ndjson_line(&line, &mut accumulated);
                    if let Some(chunk) = parsed.chunk
                        && let Err(e) =
                            session_db::append_session_log(&pool, &sid, &chunk, "stdout").await
                    {
                        tracing::error!("failed to append session log: {e}");
                    }
                    if let Some(result) = parsed.result
                        && let Some(tx) = completion_tx.take()
                    {
                        let _ = tx.send(result);
                    }
                }
                // EOF without a result event → error.
                if let Some(tx) = completion_tx.take() {
                    let _ = tx.send(NdjsonResult::Error {
                        error: "stdout closed without result event".to_string(),
                    });
                }
            });
        }

        // -- stderr → session_logs ----------------------------------------
        if let Some(stderr) = child.stderr.take() {
            let pool = pool.clone();
            let sid = launch.session_id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Skip known noisy warnings that aren't useful to users.
                    if line.contains("no stdin data received") {
                        continue;
                    }
                    if let Err(e) =
                        session_db::append_session_log(&pool, &sid, &format!("{line}\n"), "stderr")
                            .await
                    {
                        tracing::error!("failed to append session log: {e}");
                    }
                }
            });
        }

        // -- await completion, racing the cancel signal --------------------
        tokio::pin!(completion_rx);
        let result = tokio::select! {
            r = &mut completion_rx => r.ok(),
            _ = cancel.notified() => {
                let _ = child.start_kill();
                // Drain the stdout task's terminal signal (EOF → error).
                completion_rx.await.ok()
            }
        };
        let _ = child.wait().await;

        let result = result.unwrap_or(NdjsonResult::Error {
            error: "process exited without producing a result event".to_string(),
        });
        self.handle_completion(pool, spine, &launch, result).await;
    }

    /// Spawn the Claude CLI with the same flags + spawn wiring stiglab's
    /// built-in runner used (`agent/session/process.rs`, folded per #583).
    fn spawn_cli(
        &self,
        launch: &SessionLaunch,
        env: &HashMap<String, String>,
    ) -> anyhow::Result<tokio::process::Child> {
        let mut cmd = tokio::process::Command::new(&self.agent_command);

        cmd.args([
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]);

        // Default to bypassPermissions for non-interactive execution.
        let perm_mode = launch
            .permission_mode
            .as_deref()
            .unwrap_or("bypassPermissions");
        cmd.args(["--permission-mode", perm_mode]);

        if let Some(ref model) = launch.model {
            cmd.args(["--model", model]);
        }
        if let Some(max_turns) = launch.max_turns {
            cmd.args(["--max-turns", &max_turns.to_string()]);
        }
        if let Some(ref system_prompt) = launch.system_prompt {
            cmd.args(["--system-prompt", system_prompt]);
        }

        // Liveness Stop hook (spec #520 §4d / #525): only when a hook
        // endpoint URL is configured and the session is workspace-scoped.
        if let (Ok(hook_url_base), Some(workspace_id)) = (
            std::env::var(LIVENESS_HOOK_URL_ENV),
            launch.workspace_id.as_deref(),
        ) && !hook_url_base.is_empty()
        {
            let settings =
                stop_hook_settings_json(&hook_url_base, workspace_id, &launch.session_id);
            cmd.args(["--settings", &settings]);
        }

        // Portal MCP server (spec #531): only when the MCP URL is
        // configured AND the session token is present — Claude Code
        // fails to parse an MCP config whose `${VAR}` is unset.
        let has_session_token = env.contains_key(SESSION_TOKEN_ENV);
        if let Ok(mcp_url) = std::env::var(MCP_URL_ENV)
            && !mcp_url.is_empty()
            && has_session_token
        {
            let mcp_config = mcp_config_json(&mcp_url);
            cmd.args(["--mcp-config", &mcp_config]);
        }

        cmd.arg("--").arg(&launch.prompt);

        if let Some(ref dir) = launch.working_dir {
            cmd.current_dir(dir);
        }
        for (key, value) in env {
            cmd.env(key, value);
        }

        // No interactive input path — `/api/sessions` has no input
        // route, so stdin is closed rather than piped-and-dangling.
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        Ok(cmd.spawn()?)
    }

    /// Assemble the env-var map handed to the CLI: workspace
    /// credentials, the session token + identity, and `ONSAGER_REPOS`.
    async fn build_session_env(
        &self,
        pool: &PgPool,
        config: &Config,
        launch: &SessionLaunch,
    ) -> HashMap<String, String> {
        let mut env = match launch.workspace_id.as_deref() {
            Some(ws) => fetch_workspace_credentials(pool, config, ws, &launch.user_id).await,
            None => HashMap::new(),
        };

        if let (Some(token), Some(ws)) = (
            launch.session_token.as_deref(),
            launch.workspace_id.as_deref(),
        ) {
            env.insert(SESSION_TOKEN_ENV.to_string(), token.to_string());
            env.insert(
                "ONSAGER_SESSION_ID".to_string(),
                launch.session_id.clone(),
            );
            env.insert("ONSAGER_WORKSPACE_ID".to_string(), ws.to_string());
        }

        if let Some(repos_json) =
            repos_env_from_access(&launch.repos, config.credential_key.as_deref())
        {
            env.insert(REPOS_ENV.to_string(), repos_json);
        }

        env
    }

    /// Port of stiglab's `handle_agent_message` terminal arms: persist
    /// the terminal state, the fallback output, the branch link, and
    /// emit the lifecycle event.
    async fn handle_completion(
        &self,
        pool: &PgPool,
        spine: &EventStore,
        launch: &SessionLaunch,
        result: NdjsonResult,
    ) {
        let state = state_for_result(&result);
        if let Err(e) = session_db::update_session_state(pool, &launch.session_id, state).await {
            tracing::error!("failed to update session terminal state: {e}");
        }

        match result {
            NdjsonResult::Success { output } => {
                // Persist the final output only when nothing streamed —
                // avoids duplicating the whole response in the log.
                if !output.is_empty() {
                    let already_streamed =
                        session_db::get_session_logs(pool, &launch.session_id)
                            .await
                            .map(|logs| !logs.is_empty())
                            .unwrap_or(false);
                    if !already_streamed
                        && let Err(e) = session_db::append_session_log(
                            pool,
                            &launch.session_id,
                            &output,
                            "stdout",
                        )
                        .await
                    {
                        tracing::error!("failed to append final session output: {e}");
                    }
                }

                // Best-effort branch detection (issue #60) so the PR
                // webhook can attach vertical lineage later.
                let (branch, project_id, artifact_id) =
                    git_context_for_session(pool, &launch.session_id).await;
                if let Some(branch_name) = branch.as_deref()
                    && let Err(e) = record_branch_link(
                        pool,
                        &launch.session_id,
                        project_id.as_deref(),
                        branch_name,
                    )
                    .await
                {
                    tracing::warn!("failed to record branch link: {e}");
                }

                let event = onsager_spine::factory_event::FactoryEventKind::SessionCompleted {
                    session_id: launch.session_id.clone(),
                    duration_ms: 0,
                    artifact_id: artifact_id.clone(),
                    token_usage: None,
                    branch,
                    pr_number: None,
                };
                emit_lifecycle_event(spine, launch.workspace_id.as_deref(), event).await;
            }
            NdjsonResult::Error { error } => {
                if let Err(e) =
                    session_db::append_session_log(pool, &launch.session_id, &error, "stderr")
                        .await
                {
                    tracing::error!("failed to append session log: {e}");
                }
                let artifact_id = artifact_id_for_session(pool, &launch.session_id).await;
                let event = onsager_spine::factory_event::FactoryEventKind::SessionFailed {
                    session_id: launch.session_id.clone(),
                    error,
                    artifact_id,
                };
                emit_lifecycle_event(spine, launch.workspace_id.as_deref(), event).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers (ports of stiglab's server/{handler,routes/shaping,repo_env}.rs)
// ---------------------------------------------------------------------------

/// Mark a session failed + log the error (pre-spawn failure paths).
async fn fail_session(pool: &PgPool, session_id: &str, error: &str) {
    if let Err(e) = session_db::update_session_state(pool, session_id, SessionState::Failed).await {
        tracing::error!("failed to update session state to failed: {e}");
    }
    if let Err(e) = session_db::append_session_log(pool, session_id, error, "stderr").await {
        tracing::error!("failed to append session log: {e}");
    }
}

/// Emit `session.failed` for a session that never produced a result.
async fn emit_session_failed(pool: &PgPool, spine: &EventStore, launch: &SessionLaunch, error: &str) {
    let artifact_id = artifact_id_for_session(pool, &launch.session_id).await;
    let event = onsager_spine::factory_event::FactoryEventKind::SessionFailed {
        session_id: launch.session_id.clone(),
        error: error.to_string(),
        artifact_id,
    };
    emit_lifecycle_event(spine, launch.workspace_id.as_deref(), event).await;
}

/// Append a session lifecycle event to the spine under the `portal`
/// namespace. The workspace falls back to `"default"` for unscoped
/// (legacy personal) sessions, matching the tasks handler's convention.
async fn emit_lifecycle_event(
    spine: &EventStore,
    workspace_id: Option<&str>,
    event: onsager_spine::factory_event::FactoryEventKind,
) {
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "portal".to_string(),
    };
    let stream_id = event.stream_id();
    let event_type = event.event_type();
    let data = serde_json::to_value(&event).unwrap_or_default();
    if let Err(e) = spine
        .append_ext(
            workspace_id.unwrap_or("default"),
            &stream_id,
            "portal",
            event_type,
            data,
            &metadata,
            None,
        )
        .await
    {
        tracing::warn!("failed to emit {event_type}: {e}");
    }
}

/// Fetch the credentials a user holds in a workspace, decrypted into an
/// env-var map. Empty when the credential key is unset or there are no
/// credentials (the session then fails loudly via `session.failed`).
async fn fetch_workspace_credentials(
    pool: &PgPool,
    config: &Config,
    workspace_id: &str,
    user_id: &str,
) -> HashMap<String, String> {
    let Some(key) = config.credential_key.as_deref() else {
        return HashMap::new();
    };
    let encrypted = match credential_db::get_all_user_credential_values(pool, workspace_id, user_id)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("failed to load workspace credentials: {e}");
            return HashMap::new();
        }
    };
    let mut out = HashMap::new();
    for (name, encrypted_value) in encrypted {
        match decrypt_credential(key, &encrypted_value) {
            Ok(value) => {
                out.insert(name, value);
            }
            Err(e) => {
                tracing::error!("failed to decrypt credential {name} for user {user_id}: {e}");
            }
        }
    }
    out
}

/// Decrypt the repo set and build the `ONSAGER_REPOS` env value (port
/// of stiglab's `repo_env::repos_env_from_access`). `None` when the set
/// is empty; a token that fails to decrypt degrades to an
/// unauthenticated clone URL rather than dropping the repo.
fn repos_env_from_access(repos: &[RepoAccess], key: Option<&str>) -> Option<String> {
    if repos.is_empty() {
        return None;
    }
    let clonable: Vec<ClonableRepo> = repos
        .iter()
        .map(|r| {
            let token = match (key, r.encrypted_read_token.as_deref()) {
                (Some(k), Some(enc)) => match decrypt_credential(k, enc) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        tracing::warn!(
                            owner = %r.owner,
                            repo = %r.name,
                            "session_runner: failed to decrypt read token; surfacing \
                             unauthenticated clone URL: {e}"
                        );
                        None
                    }
                },
                _ => None,
            };
            ClonableRepo {
                owner: r.owner.clone(),
                name: r.name.clone(),
                default_branch: r.default_branch.clone(),
                token,
            }
        })
        .collect();
    repos_env_json(&clonable)
}

/// DB-only `artifact_id` lookup for a session (failure path — no git
/// subprocess needed).
async fn artifact_id_for_session(pool: &PgPool, session_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT artifact_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

/// Best-effort branch + project + artifact lookup at completion time.
/// Asks `git rev-parse --abbrev-ref HEAD` in the session's working dir;
/// detached HEAD / empty output / a >5s hang all degrade to `None`.
async fn git_context_for_session(
    pool: &PgPool,
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

/// Upsert the `(session_id, branch, project_id)` row the PR-opened
/// webhook handler joins against for vertical lineage.
async fn record_branch_link(
    pool: &PgPool,
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
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_event_success_maps_to_done() {
        let mut acc = String::new();
        let parsed = parse_ndjson_line(
            r#"{"type":"result","subtype":"success","result":"final answer"}"#,
            &mut acc,
        );
        let result = parsed.result.expect("result event parsed");
        assert_eq!(
            result,
            NdjsonResult::Success {
                output: "final answer".into()
            }
        );
        // Mutation guard: flipping Done↔Failed in state_for_result
        // fails one of these two assertions.
        assert_eq!(state_for_result(&result), SessionState::Done);
    }

    #[test]
    fn result_event_error_maps_to_failed() {
        let mut acc = String::new();
        let parsed = parse_ndjson_line(
            r#"{"type":"result","subtype":"error_max_turns","error":"too many turns"}"#,
            &mut acc,
        );
        let result = parsed.result.expect("result event parsed");
        assert_eq!(
            result,
            NdjsonResult::Error {
                error: "too many turns".into()
            }
        );
        assert_eq!(state_for_result(&result), SessionState::Failed);
    }

    #[test]
    fn result_error_without_message_names_the_subtype() {
        let mut acc = String::new();
        let parsed =
            parse_ndjson_line(r#"{"type":"result","subtype":"error_during_run"}"#, &mut acc);
        assert_eq!(
            parsed.result,
            Some(NdjsonResult::Error {
                error: "claude exited with subtype: error_during_run".into()
            })
        );
    }

    #[test]
    fn result_success_without_body_falls_back_to_accumulated_text() {
        let mut acc = String::new();
        // Two text deltas accumulate…
        let p1 = parse_ndjson_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}"#,
            &mut acc,
        );
        assert_eq!(p1.chunk.as_deref(), Some("hel"));
        let p2 = parse_ndjson_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"lo"}}}"#,
            &mut acc,
        );
        assert_eq!(p2.chunk.as_deref(), Some("lo"));
        // …and a result with no `result` field returns them.
        let parsed = parse_ndjson_line(r#"{"type":"result","subtype":"success"}"#, &mut acc);
        assert_eq!(
            parsed.result,
            Some(NdjsonResult::Success {
                output: "hello".into()
            })
        );
    }

    #[test]
    fn non_json_line_passes_through_as_raw_output() {
        let mut acc = String::new();
        let parsed = parse_ndjson_line("plain text warning", &mut acc);
        assert_eq!(parsed.chunk.as_deref(), Some("plain text warning\n"));
        assert!(parsed.result.is_none());
    }

    #[test]
    fn tool_use_block_start_emits_marker() {
        let mut acc = String::new();
        let parsed = parse_ndjson_line(
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"Bash"}}}"#,
            &mut acc,
        );
        assert_eq!(parsed.chunk.as_deref(), Some("[tool_use: Bash]\n"));
        assert!(parsed.result.is_none());
    }

    #[test]
    fn unknown_and_system_events_are_silent() {
        let mut acc = String::new();
        assert_eq!(
            parse_ndjson_line(r#"{"type":"system","subtype":"init"}"#, &mut acc),
            ParsedLine::default()
        );
        assert_eq!(
            parse_ndjson_line(r#"{"type":"some_future_event"}"#, &mut acc),
            ParsedLine::default()
        );
    }

    #[test]
    fn repos_env_empty_set_is_none_and_tokenless_degrades() {
        assert_eq!(repos_env_from_access(&[], Some("00")), None);
        let repos = vec![RepoAccess {
            owner: "acme".into(),
            name: "public".into(),
            default_branch: "main".into(),
            encrypted_read_token: None,
        }];
        let json = repos_env_from_access(&repos, None).expect("repo surfaced");
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr[0]["clone_url"], "https://github.com/acme/public.git");
    }

    #[test]
    fn repos_env_decrypts_read_tokens_into_clone_urls() {
        // 32-byte key (64 hex chars) for the AES-256-GCM credential cipher.
        const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let enc = crate::auth::encrypt_credential(KEY, "tok_a").unwrap();
        let repos = vec![RepoAccess {
            owner: "acme".into(),
            name: "widgets".into(),
            default_branch: "main".into(),
            encrypted_read_token: Some(enc),
        }];
        let json = repos_env_from_access(&repos, Some(KEY)).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            arr[0]["clone_url"],
            "https://x-access-token:tok_a@github.com/acme/widgets.git"
        );
    }
}
