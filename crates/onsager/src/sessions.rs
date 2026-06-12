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
pub const REPOS_ENV: &str = "ONSAGER_REPOS";
const CLAUDE_BIN_ENV: &str = "ONSAGER_CLAUDE_BIN";

/// One repo surfaced to the agent (token already minted). Ported from
/// legacy `onsager-agent-spawn` (#624) — the `ONSAGER_REPOS` array is
/// the agent-facing contract and must not drift.
#[derive(Debug, Clone, PartialEq)]
pub struct ClonableRepo {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub token: Option<String>,
}

/// Build the `ONSAGER_REPOS` JSON array; `None` for an empty set so
/// the caller injects nothing.
pub fn repos_env_json(repos: &[ClonableRepo]) -> Option<String> {
    if repos.is_empty() {
        return None;
    }
    let entries: Vec<serde_json::Value> = repos
        .iter()
        .map(|r| {
            serde_json::json!({
                "owner": r.owner,
                "name": r.name,
                "default_branch": r.default_branch,
                "clone_url": clone_url(&r.owner, &r.name, r.token.as_deref()),
            })
        })
        .collect();
    serde_json::to_string(&entries).ok()
}

/// HTTPS clone URL, authenticated via the `x-access-token` convention
/// when a token is present; the token is percent-encoded defensively.
fn clone_url(owner: &str, name: &str, token: Option<&str>) -> String {
    match token {
        Some(t) => format!(
            "https://x-access-token:{}@github.com/{owner}/{name}.git",
            urlencode(t)
        ),
        None => format!("https://github.com/{owner}/{name}.git"),
    }
}

/// Minimal percent-encoding for URL userinfo / query values (ported).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

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
    /// The run's repo-access set, surfaced as `ONSAGER_REPOS` (#624).
    pub repos: Vec<ClonableRepo>,
    /// Written with the child's pid (== its pgid) after spawn so the
    /// abort endpoint can kill the whole process tree (M3 #625).
    pub pgid_slot: std::sync::Arc<std::sync::Mutex<Option<i32>>>,
    /// Sink for normalized `agent.*` session events (#632).
    pub pool: PgPool,
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
    if let Some(repos) = repos_env_json(&req.repos) {
        cmd.env(REPOS_ENV, repos);
    }
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        // Abort (#625) drops this future; the child must die with it.
        // The child leads its own process group so abort can SIGKILL
        // the whole tree (claude spawns subprocess trees).
        .kill_on_drop(true)
        .process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| SessionError(format!("spawning `{command}`: {e}")))?;
    if let Some(pid) = child.id() {
        *req.pgid_slot.lock().expect("pgid slot lock") = Some(pid as i32);
    }

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
    let mut normalizer = crate::agent_events::Normalizer::new();
    let mut seq: i32 = 0;
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                let _ = child.kill().await;
                return Err(SessionError(format!("reading agent stdout: {e}")));
            }
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Normalize + persist the session feed (#632). A failed insert
        // is logged, never fatal — the feed must not fail the work.
        for event in normalizer.handle(&value) {
            seq += 1;
            let inserted = sqlx::query(
                "INSERT INTO session_events (session_id, seq, kind, payload)                  VALUES ($1, $2, $3, $4)",
            )
            .bind(&req.session_id)
            .bind(seq)
            .bind(event.kind)
            .bind(&event.payload)
            .execute(&req.pool)
            .await;
            if let Err(e) = inserted {
                tracing::warn!(session_id = %req.session_id, "session event insert failed: {e}");
            }
        }
        if let Ok(ClaudeEvent::Result {
            subtype,
            result: text,
            error,
        }) = serde_json::from_value::<ClaudeEvent>(value)
        {
            result = Some(match subtype.as_str() {
                "success" => Ok(text.unwrap_or_default()),
                _ => Err(error.unwrap_or_else(|| format!("claude exited: {subtype}"))),
            });
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
    fn repos_env_json_builds_authenticated_entries() {
        // Ported parity tests from legacy onsager-agent-spawn (#624).
        let repos = vec![
            ClonableRepo {
                owner: "acme".into(),
                name: "widgets".into(),
                default_branch: "main".into(),
                token: Some("tok_a".into()),
            },
            ClonableRepo {
                owner: "acme".into(),
                name: "public".into(),
                default_branch: "main".into(),
                token: None,
            },
        ];
        let json = repos_env_json(&repos).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 2);
        assert_eq!(
            arr[0]["clone_url"],
            "https://x-access-token:tok_a@github.com/acme/widgets.git"
        );
        assert_eq!(arr[1]["clone_url"], "https://github.com/acme/public.git");
        assert_eq!(repos_env_json(&[]), None);
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
