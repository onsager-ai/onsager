//! [`ClaudeCliRunner`] — the production [`AgentRunner`] (spec #535,
//! Plan item 1 of #533).
//!
//! [`onsager_nodes::AgentExecutor`] dispatches its session through an
//! `Arc<dyn AgentRunner>`; the only workspace impls before this were
//! `UnconfiguredRunner` (errors) and `StubAgentRunner` (tests). This is
//! the first runner that actually runs an agent. Per #533 the backend
//! is the subprocess `claude` CLI — maximal parity with stiglab, whose
//! `SessionProcess::spawn` (`crates/stiglab/src/agent/session/process.rs`)
//! this mirrors. It lives host-side in the scheduler binary's crate so
//! `onsager-nodes` stays free of a process-spawn dependency, and reuses
//! — never forks — the spawn-wiring constants and JSON builders through
//! the shared `onsager-agent-spawn` crate.
//!
//! ## What the runner spawns
//!
//! `claude --output-format stream-json --verbose --include-partial-messages
//! --permission-mode bypassPermissions [--model M] [--system-prompt P]
//! [--mcp-config …] [--settings …] -- <prompt>` with `ONSAGER_SESSION_ID`
//! threaded into the child env from [`AgentRequest::session_id`]. The MCP
//! config (spec #531) and liveness Stop hook (spec #520 §4d) are wired
//! only when their env is present, mirroring stiglab's gating exactly:
//! Claude Code fails to parse an MCP config whose `${VAR}` is unset, so a
//! tokenless registration would break the session rather than degrade.
//!
//! ## What it returns
//!
//! It parses the NDJSON stdout to the terminal `Result` event and
//! returns [`AgentResponse`] `{ output, emitted: vec![] }`. `emitted`
//! stays empty by design (§4c): the agent records outputs out-of-band
//! through portal's MCP `emit_artifact` / `declare_no_output` tools,
//! which write the authoritative `events_ext` manifest the executor
//! reads back. The NDJSON stream genuinely does not see those MCP side
//! effects, so the runner reports "session over" only and the manifest
//! remains the single source of truth for "what came out".

use std::process::Stdio;

use async_trait::async_trait;
use onsager_agent_spawn::{
    LIVENESS_HOOK_URL_ENV, MCP_URL_ENV, SESSION_TOKEN_ENV, mcp_config_json, stop_hook_settings_json,
};
use onsager_nodes::{AgentRequest, AgentResponse, AgentRunError, AgentRunner};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Env var holding the session's workspace id. Threaded into the child
/// env so the liveness Stop hook (#520 §4d) can address the manifest by
/// `(workspace_id, session_id)`. Unset → no Stop hook is wired, the same
/// fail-safe stiglab uses when a session is not workspace-scoped.
const WORKSPACE_ID_ENV: &str = "ONSAGER_WORKSPACE_ID";

/// Env var the agent's execution environment reads to attribute its MCP
/// `emit_artifact` / `declare_no_output` calls to the right manifest
/// stream. The executor mints the id and the runner threads it here
/// (spec #520 §4b), mirroring stiglab's `session_requested_listener`.
const SESSION_ID_ENV: &str = "ONSAGER_SESSION_ID";

/// Production [`AgentRunner`] backed by the subprocess `claude` CLI.
///
/// Holds only the binary name/path to invoke (default `claude`, on
/// PATH). Everything else comes off the per-call [`AgentRequest`] and
/// the process env, matching the host-side configuration model:
/// `AgentExecutor` carries the workflow template's config (model,
/// prompt, tools), the runner carries the runtime backend.
#[derive(Debug, Clone)]
pub struct ClaudeCliRunner {
    /// The `claude` binary to spawn. Defaults to `"claude"` (resolved on
    /// PATH); overridable for tests (a fixture script) and for pinning a
    /// specific install in production.
    command: String,
}

impl Default for ClaudeCliRunner {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
        }
    }
}

impl ClaudeCliRunner {
    /// Runner spawning the default `claude` binary on PATH.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runner spawning a specific binary path / name. Used by tests to
    /// point at a fixture script and by deployments pinning an install.
    pub fn with_command(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

#[async_trait]
impl AgentRunner for ClaudeCliRunner {
    async fn run(&self, request: AgentRequest) -> Result<AgentResponse, AgentRunError> {
        let mut cmd = Command::new(&self.command);

        // Stream-JSON flags — identical to stiglab's SessionProcess.
        cmd.args([
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]);

        // Non-interactive execution: bypass permission prompts. The
        // scheduler runs agents headlessly with no human at the TTY.
        cmd.args(["--permission-mode", "bypassPermissions"]);

        if !request.model.is_empty() {
            cmd.args(["--model", &request.model]);
        }
        if !request.system_prompt.is_empty() {
            cmd.args(["--system-prompt", &request.system_prompt]);
        }

        // Liveness Stop hook (spec #520 §4d). Wired only when both the
        // hook endpoint URL and the session's workspace id are present
        // (the manifest is addressed by `(workspace_id, session_id)`).
        // Absent either → no hook; the authoritative gate (#523) alone
        // covers liveness. Same gating stiglab applies.
        if let (Ok(hook_url_base), Ok(workspace_id)) = (
            std::env::var(LIVENESS_HOOK_URL_ENV),
            std::env::var(WORKSPACE_ID_ENV),
        ) && !hook_url_base.is_empty()
            && !workspace_id.is_empty()
        {
            let settings =
                stop_hook_settings_json(&hook_url_base, &workspace_id, &request.session_id);
            cmd.args(["--settings", &settings]);
        }

        // Portal MCP server (spec #531). Registered only when the MCP
        // URL is configured AND the session token is present in the env —
        // Claude Code fails to parse an MCP config whose `${VAR}` is
        // unset, so a tokenless registration would break the session
        // rather than degrade. Absent either → the agent runs without
        // MCP (no emit path); the authoritative gate (#523) still covers
        // liveness. Same gating stiglab applies.
        let has_session_token = std::env::var(SESSION_TOKEN_ENV).is_ok_and(|t| !t.is_empty());
        if let Ok(mcp_url) = std::env::var(MCP_URL_ENV)
            && !mcp_url.is_empty()
            && has_session_token
        {
            let mcp_config = mcp_config_json(&mcp_url);
            cmd.args(["--mcp-config", &mcp_config]);
        }

        // Separator + prompt body assembled by the executor.
        cmd.arg("--").arg(&request.user_prompt);

        // Thread the executor-minted session id so the agent attributes
        // its MCP emits to the same `session:<id>` manifest stream the
        // executor reads back authoritatively (spec #520 §4b/§4c).
        cmd.env(SESSION_ID_ENV, &request.session_id);

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| AgentRunError::new(format!("spawning `{}`: {e}", self.command)))?;

        // Drain stdout, parsing NDJSON to the terminal `Result` event.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AgentRunError::new("child process produced no stdout pipe"))?;
        let mut lines = BufReader::new(stdout).lines();

        let mut accumulated = String::new();
        let mut result: Option<Result<String, String>> = None;

        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break, // EOF
                Err(e) => {
                    let _ = child.kill().await;
                    return Err(AgentRunError::new(format!("reading agent stdout: {e}")));
                }
            };
            let event: ClaudeEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                // Non-JSON line — defensively accumulate as raw output so
                // a session whose final text leaks outside the Result
                // event is not silently dropped.
                Err(_) => {
                    accumulated.push_str(&line);
                    accumulated.push('\n');
                    continue;
                }
            };
            match event {
                ClaudeEvent::StreamEvent { event: inner } => {
                    if inner.event_type == "content_block_delta"
                        && let Some(delta) = inner.delta
                        && delta.delta_type == "text_delta"
                        && let Some(text) = delta.text
                    {
                        accumulated.push_str(&text);
                    }
                }
                ClaudeEvent::Result {
                    subtype,
                    result: text,
                    error,
                } => {
                    result = Some(match subtype.as_str() {
                        "success" => Ok(text.unwrap_or_else(|| accumulated.clone())),
                        _ => Err(error
                            .unwrap_or_else(|| format!("claude exited with subtype: {subtype}"))),
                    });
                }
                ClaudeEvent::Unknown => {}
            }
        }

        // Reap the child so we don't leak a zombie. A non-zero exit with
        // a parsed success Result still counts as success (the CLI can
        // exit non-zero on a Stop-hook block cap, for instance); a
        // missing Result is the real failure surfaced below.
        let _ = child.wait().await;

        match result {
            Some(Ok(output)) => Ok(AgentResponse {
                output,
                // Empty by design: emits flow out-of-band through MCP and
                // are read back from the authoritative manifest (§4c).
                emitted: Vec::new(),
            }),
            Some(Err(error)) => Err(AgentRunError::new(error)),
            None => Err(AgentRunError::new(
                "agent stdout closed without a result event",
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// NDJSON event types emitted by `claude --output-format stream-json`.
//
// A focused subset of stiglab's `ClaudeEvent` — the runner only needs
// the final-text deltas and the terminal `Result`. System / tool_use
// events stiglab forwards as UI chunks have no consumer here (the agent
// runs headlessly under the scheduler), so they fold into `Unknown`.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeEvent {
    #[serde(rename = "stream_event")]
    StreamEvent { event: StreamEventInner },
    Result {
        subtype: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct StreamEventInner {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<DeltaPayload>,
}

#[derive(Debug, Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Serializes every test that spawns the runner. `Command::spawn`
    /// reads the process-global env, and the env-mutating tests below
    /// call `set_var` / `remove_var`; without this lock those mutations
    /// would race a concurrent spawn (a genuine data race, not just a
    /// logical one). Every spawning test holds it across its `run`. A
    /// `tokio::sync::Mutex` (not `std`) so the guard can be held across
    /// the `.await` without tripping `clippy::await_holding_lock`.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Write an executable fake `claude` script that records its argv +
    /// env to `capture_path` and emits `body` on stdout. Returns the
    /// temp dir (kept alive) and the script path.
    fn fake_claude(body: &str, capture_path: &std::path::Path) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-claude.sh");
        let mut f = std::fs::File::create(&script).unwrap();
        // Record the full argv and the env vars the assertions care
        // about, then print the canned NDJSON body.
        write!(
            f,
            "#!/usr/bin/env bash\n\
             {{\n\
               echo \"ARGS: $*\"\n\
               echo \"SESSION_ID: ${{ONSAGER_SESSION_ID:-}}\"\n\
             }} >> \"{capture}\"\n\
             cat <<'NDJSON'\n{body}\nNDJSON\n",
            capture = capture_path.display(),
            body = body,
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        (dir, script.to_string_lossy().into_owned())
    }

    fn request(prompt: &str, session_id: &str) -> AgentRequest {
        AgentRequest {
            model: "claude-sonnet-4-6".into(),
            system_prompt: "you are helpful".into(),
            tools: Vec::new(),
            credential_ref: None,
            user_prompt: prompt.into(),
            session_id: session_id.into(),
        }
    }

    #[tokio::test]
    async fn parses_success_result_into_output() {
        let _guard = ENV_LOCK.lock().await;
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = r#"{"type":"result","subtype":"success","result":"final answer"}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        let runner = ClaudeCliRunner::with_command(script);
        let resp = runner.run(request("do the thing", "sess_1")).await.unwrap();
        assert_eq!(resp.output, "final answer");
        // emitted is always empty — manifest is authoritative (§4c).
        assert!(resp.emitted.is_empty());
    }

    #[tokio::test]
    async fn spawned_argv_and_env_carry_model_and_session_id() {
        let _guard = ENV_LOCK.lock().await;
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = r#"{"type":"result","subtype":"success","result":"ok"}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        // No MCP URL / session token / workspace id in env → no
        // --mcp-config, no --settings; we assert the always-on flags.
        let runner = ClaudeCliRunner::with_command(script);
        runner.run(request("the prompt", "sess_xyz")).await.unwrap();

        let recorded = std::fs::read_to_string(&cap_file).unwrap();
        assert!(
            recorded.contains("--model claude-sonnet-4-6"),
            "argv should carry --model, got: {recorded}"
        );
        assert!(
            recorded.contains("--output-format stream-json"),
            "argv should carry stream-json flags, got: {recorded}"
        );
        assert!(
            recorded.contains("--permission-mode bypassPermissions"),
            "argv should bypass permissions, got: {recorded}"
        );
        assert!(
            recorded.contains("SESSION_ID: sess_xyz"),
            "ONSAGER_SESSION_ID must be threaded into the child env, got: {recorded}"
        );
    }

    #[tokio::test]
    async fn accumulates_text_deltas_when_result_lacks_result_field() {
        let _guard = ENV_LOCK.lock().await;
        // A Result event with no `result` field falls back to the
        // accumulated content_block_delta text.
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = concat!(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello "}}}"#,
            "\n",
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"world"}}}"#,
            "\n",
            r#"{"type":"result","subtype":"success"}"#,
        );
        let (_dir, script) = fake_claude(body, &cap_file);

        let runner = ClaudeCliRunner::with_command(script);
        let resp = runner.run(request("p", "s")).await.unwrap();
        assert_eq!(resp.output, "Hello world");
    }

    #[tokio::test]
    async fn error_result_surfaces_as_run_error() {
        let _guard = ENV_LOCK.lock().await;
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body =
            r#"{"type":"result","subtype":"error_during_execution","error":"model refused"}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        let runner = ClaudeCliRunner::with_command(script);
        let err = runner.run(request("p", "s")).await.unwrap_err();
        assert!(
            err.to_string().contains("model refused"),
            "runner should surface the CLI error, got: {err}"
        );
    }

    #[tokio::test]
    async fn missing_result_event_is_an_error() {
        let _guard = ENV_LOCK.lock().await;
        // stdout closes with only deltas, no terminal Result.
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"partial"}}}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        let runner = ClaudeCliRunner::with_command(script);
        let err = runner.run(request("p", "s")).await.unwrap_err();
        assert!(
            err.to_string().contains("without a result event"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn spawn_failure_for_missing_binary_is_an_error() {
        let _guard = ENV_LOCK.lock().await;
        let runner = ClaudeCliRunner::with_command("/nonexistent/definitely-not-claude");
        let err = runner.run(request("p", "s")).await.unwrap_err();
        assert!(err.to_string().contains("spawning"), "got: {err}");
    }

    #[tokio::test]
    async fn mcp_config_wired_when_url_and_token_present() {
        let _guard = ENV_LOCK.lock().await;
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = r#"{"type":"result","subtype":"success","result":"ok"}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        // SAFETY: env mutation is serialized by ENV_LOCK; vars are
        // cleared before the guard drops so no other test observes them.
        unsafe {
            std::env::set_var(MCP_URL_ENV, "https://app.example.com/mcp/messages");
            std::env::set_var(SESSION_TOKEN_ENV, "tok_secret");
        }

        let runner = ClaudeCliRunner::with_command(script);
        runner.run(request("p", "sess_mcp")).await.unwrap();

        unsafe {
            std::env::remove_var(MCP_URL_ENV);
            std::env::remove_var(SESSION_TOKEN_ENV);
        }

        let recorded = std::fs::read_to_string(&cap_file).unwrap();
        assert!(
            recorded.contains("--mcp-config"),
            "argv should carry --mcp-config when MCP URL + token are set, got: {recorded}"
        );
        // The portal MCP server URL rides inside the inline JSON config.
        assert!(
            recorded.contains("https://app.example.com/mcp/messages"),
            "the MCP config JSON should carry the portal URL, got: {recorded}"
        );
    }

    #[tokio::test]
    async fn mcp_config_omitted_when_token_absent() {
        let _guard = ENV_LOCK.lock().await;
        let capture = tempfile::tempdir().unwrap();
        let cap_file = capture.path().join("argv.txt");
        let body = r#"{"type":"result","subtype":"success","result":"ok"}"#;
        let (_dir, script) = fake_claude(body, &cap_file);

        // MCP URL set but NO session token → Claude Code would fail to
        // parse a tokenless config, so the runner must omit it (mirrors
        // stiglab's gating). Mutation guard: dropping the
        // `has_session_token` check here makes this assertion fail.
        unsafe {
            std::env::set_var(MCP_URL_ENV, "https://app.example.com/mcp/messages");
            std::env::remove_var(SESSION_TOKEN_ENV);
        }

        let runner = ClaudeCliRunner::with_command(script);
        runner.run(request("p", "sess_no_tok")).await.unwrap();

        unsafe {
            std::env::remove_var(MCP_URL_ENV);
        }

        let recorded = std::fs::read_to_string(&cap_file).unwrap();
        assert!(
            !recorded.contains("--mcp-config"),
            "argv must omit --mcp-config when the session token is absent, got: {recorded}"
        );
    }
}
