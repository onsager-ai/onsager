//! Agent session spawn — the one launch path (ADR 0029 unified the
//! legacy chat runner and scheduler runner).
//!
//! The CLI invocation, MCP-config wire shape, and env-var contract are
//! ported from `legacy/crates/onsager-agent-spawn` and the legacy
//! engine's `ClaudeCliRunner` (stream-JSON parse, stderr drain — the
//! #601 lesson: CLI errors must never vanish). Dropped relative to
//! legacy: the liveness Stop hook (v2 liveness is a plain run failure,
//! decided from the artifacts table after exit) and `ONSAGER_REPOS`
//! (returns with the GitHub port in M2).

use std::process::Stdio;

use serde::Deserialize;
use sqlx::PgPool;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Env vars the agent process reads — names are the agent-facing
/// contract, unchanged from legacy so existing agent-side docs hold.
pub const SESSION_ID_ENV: &str = "ONSAGER_SESSION_ID";
pub const SESSION_TOKEN_ENV: &str = "ONSAGER_SESSION_TOKEN";
const CLAUDE_BIN_ENV: &str = "ONSAGER_CLAUDE_BIN";

/// Inline `--mcp-config` registering this process's MCP endpoint. The
/// bearer token interpolates from `$ONSAGER_SESSION_TOKEN` at startup
/// (Claude Code expands `${VAR}` in MCP headers); never inlined.
pub fn mcp_config_json(mcp_url: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "onsager": {
                "type": "http",
                "url": mcp_url,
                "headers": {
                    "Authorization": format!("Bearer ${{{SESSION_TOKEN_ENV}}}"),
                },
            }
        }
    })
    .to_string()
}

pub struct SessionRequest {
    pub session_id: String,
    pub token: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    /// This process's MCP endpoint; `None` runs the agent toolless
    /// (it then cannot emit, and the liveness check fails the run).
    pub mcp_url: Option<String>,
    /// Decrypted workspace credentials injected as env vars
    /// (CLAUDE_CODE_OAUTH_TOKEN etc.).
    pub env: Vec<(String, String)>,
}

pub struct SessionOutcome {
    /// Final assistant text from the CLI's Result event.
    pub output: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SessionError(pub String);

/// NDJSON events on the claude CLI's stdout — the subset we consume.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeEvent {
    Result {
        subtype: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// Spawn the claude CLI and wait for its terminal Result event.
pub async fn run_agent_session(req: SessionRequest) -> Result<SessionOutcome, SessionError> {
    let command = std::env::var(CLAUDE_BIN_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let mut cmd = Command::new(&command);

    cmd.args(["--output-format", "stream-json", "--verbose"]);
    // Headless: no human at the TTY.
    cmd.args(["--permission-mode", "bypassPermissions"]);
    if let Some(model) = req.model.as_deref().filter(|m| !m.is_empty()) {
        cmd.args(["--model", model]);
    }
    if let Some(sp) = req.system_prompt.as_deref().filter(|s| !s.is_empty()) {
        cmd.args(["--system-prompt", sp]);
    }
    if let Some(mcp_url) = req.mcp_url.as_deref().filter(|u| !u.is_empty()) {
        cmd.args(["--mcp-config", &mcp_config_json(mcp_url)]);
    }
    cmd.arg("--").arg(&req.user_prompt);

    cmd.env(SESSION_ID_ENV, &req.session_id);
    cmd.env(SESSION_TOKEN_ENV, &req.token);
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| SessionError(format!("spawning `{command}`: {e}")))?;

    // Drain stderr to tracing — a CLI usage error must surface, not
    // fill the pipe buffer (#601).
    if let Some(stderr) = child.stderr.take() {
        let sid = req.session_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(session_id = %sid, "agent stderr: {line}");
            }
        });
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SessionError("child produced no stdout pipe".into()))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut result: Option<Result<String, String>> = None;
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                let _ = child.kill().await;
                return Err(SessionError(format!("reading agent stdout: {e}")));
            }
        };
        match serde_json::from_str::<ClaudeEvent>(&line) {
            Ok(ClaudeEvent::Result {
                subtype,
                result: text,
                error,
            }) => {
                result = Some(match subtype.as_str() {
                    "success" => Ok(text.unwrap_or_default()),
                    _ => Err(error.unwrap_or_else(|| format!("claude exited: {subtype}"))),
                });
            }
            Ok(ClaudeEvent::Other) | Err(_) => {}
        }
    }
    let _ = child.wait().await;

    match result {
        Some(Ok(output)) => Ok(SessionOutcome { output }),
        Some(Err(e)) => Err(SessionError(e)),
        None => Err(SessionError(
            "agent stdout closed without a result event".into(),
        )),
    }
}

/// Liveness, v2 shape: after the session exits, it must have either
/// emitted at least one artifact or declared no output on purpose.
/// Anything else fails the run plainly — no parking, no human gate
/// (returns with an operator surface if needed).
pub async fn session_delivered(pool: &PgPool, session_id: &str) -> anyhow::Result<bool> {
    let emitted: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM artifacts WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await?;
    if emitted.0 > 0 {
        return Ok(true);
    }
    let declared: Option<(String,)> = sqlx::query_as(
        "SELECT declared_no_output_reason FROM sessions \
         WHERE id = $1 AND declared_no_output_reason IS NOT NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(declared.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_registers_http_server_with_interpolated_token() {
        // Ported parity test from legacy onsager-agent-spawn — the
        // agent-facing wire shape must not drift.
        let json = mcp_config_json("http://127.0.0.1:3002/mcp/messages");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let server = &v["mcpServers"]["onsager"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:3002/mcp/messages");
        assert_eq!(
            server["headers"]["Authorization"],
            "Bearer ${ONSAGER_SESSION_TOKEN}"
        );
    }

    #[test]
    fn result_event_parses() {
        let line = r#"{"type":"result","subtype":"success","result":"done"}"#;
        match serde_json::from_str::<ClaudeEvent>(line).unwrap() {
            ClaudeEvent::Result {
                subtype, result, ..
            } => {
                assert_eq!(subtype, "success");
                assert_eq!(result.as_deref(), Some("done"));
            }
            _ => panic!("expected result event"),
        }
    }
}
