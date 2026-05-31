use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::core::{AgentMessage, SessionState, Task};

// ---------------------------------------------------------------------------
// Liveness Stop hook (spec #520 §4d / #525)
// ---------------------------------------------------------------------------

/// Env var (read on the agent node) holding the base URL of portal's
/// liveness endpoint, e.g.
/// `https://app.example.com/api/internal/session-liveness`. Unset → no
/// Stop hook is configured (fail-safe default for local dev and tests);
/// the authoritative gate (#523) alone then covers liveness.
const LIVENESS_HOOK_URL_ENV: &str = "ONSAGER_LIVENESS_HOOK_URL";

/// Env var holding the session's workspace-scoped bearer token. The
/// Stop hook interpolates it into the `Authorization` header at call
/// time (kept out of the on-disk settings via Claude Code's
/// `allowedEnvVars`). Provisioning this token into the session env is a
/// sibling of #520's deferred MCP-config wiring; until it lands the
/// hook's request is unauthorized → `401` → the hook fails open, which
/// is the desired degrade-to-authoritative-gate behavior.
const SESSION_TOKEN_ENV: &str = "ONSAGER_SESSION_TOKEN";

/// Build the inline `--settings` JSON that registers the liveness Stop
/// hook. Pure (no env, no IO) so the wire shape is unit-testable.
///
/// The hook is an **HTTP** hook (Claude Code `>= 2.1.x`): on every stop
/// attempt it POSTs to the manifest evaluator, which returns allow
/// (empty 2xx) or `{ "decision": "block", "reason": ... }`. Claude Code
/// fails the hook open on any non-2xx / timeout. Consecutive blocks are
/// capped by `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP` (default 8); the decision
/// is recomputed from the manifest on every call, so a session that is
/// still empty stays blocked until the cap — we never short-circuit on
/// a `stop_hook_active`-style flag.
///
/// Our stiglab `session_id` + `workspace_id` are baked into the URL
/// query string (the POST body carries Claude Code's *own* session id,
/// which is unrelated to our manifest stream key).
fn stop_hook_settings_json(hook_url_base: &str, workspace_id: &str, session_id: &str) -> String {
    // Choose `?` vs `&` so a base URL that already carries a query
    // string (or a trailing `?`) stays a valid URL rather than
    // producing a double `?`.
    let sep = if hook_url_base.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!(
        "{hook_url_base}{sep}workspace_id={}&session_id={}",
        urlencode(workspace_id),
        urlencode(session_id),
    );
    let token_ref = format!("Bearer ${SESSION_TOKEN_ENV}");
    serde_json::json!({
        "hooks": {
            "Stop": [
                {
                    "hooks": [
                        {
                            "type": "http",
                            "url": url,
                            // Short timeout: a network blip should fail
                            // the hook open promptly, not stall the stop
                            // for the 600s HTTP-hook default.
                            "timeout": 10,
                            "headers": { "Authorization": token_ref },
                            "allowedEnvVars": [SESSION_TOKEN_ENV],
                        }
                    ]
                }
            ]
        }
    })
    .to_string()
}

/// Minimal percent-encoding for query-string values. Server-minted ids
/// (`ws_<ulid>`, ULID/UUID session ids) are already URL-safe; this is
/// defensive against any reserved character sneaking in.
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

// ---------------------------------------------------------------------------
// Completion result forwarded from the stdout parser to the manager
// ---------------------------------------------------------------------------

pub enum NdjsonResult {
    Success { output: String },
    Error { error: String },
}

// ---------------------------------------------------------------------------
// SessionProcess
// ---------------------------------------------------------------------------

pub struct SessionProcess {
    child: Child,
    stdin_tx: Option<mpsc::UnboundedSender<String>>,
    completion_rx: Option<oneshot::Receiver<NdjsonResult>>,
}

impl SessionProcess {
    pub async fn spawn(
        task: &Task,
        session_id: &str,
        agent_command: &str,
        outbound_tx: mpsc::UnboundedSender<AgentMessage>,
        credentials: Option<&HashMap<String, String>>,
    ) -> Result<Self> {
        let mut cmd = Command::new(agent_command);

        // Stream-JSON flags
        cmd.args([
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]);

        // Permission mode — default to bypassPermissions for non-interactive execution
        let perm_mode = task
            .permission_mode
            .as_deref()
            .unwrap_or("bypassPermissions");
        cmd.args(["--permission-mode", perm_mode]);

        // Optional execution config
        if let Some(ref model) = task.model {
            cmd.args(["--model", model]);
        }
        if let Some(max_turns) = task.max_turns {
            cmd.args(["--max-turns", &max_turns.to_string()]);
        }
        if let Some(ref system_prompt) = task.system_prompt {
            cmd.args(["--system-prompt", system_prompt]);
        }

        // Liveness Stop hook (spec #520 §4d / #525). Configured only when
        // a hook endpoint URL is set and the session is workspace-scoped
        // (the manifest is addressed by `(workspace_id, session_id)`).
        // Absent either → no hook, and the authoritative gate (#523)
        // alone covers liveness.
        if let (Ok(hook_url_base), Some(workspace_id)) = (
            std::env::var(LIVENESS_HOOK_URL_ENV),
            task.workspace_id.as_deref(),
        ) && !hook_url_base.is_empty()
        {
            let settings = stop_hook_settings_json(&hook_url_base, workspace_id, session_id);
            cmd.args(["--settings", &settings]);
        }

        // Separator + prompt
        cmd.arg("--").arg(&task.prompt);

        if let Some(ref dir) = task.working_dir {
            cmd.current_dir(dir);
        }

        // Inject per-user credentials as environment variables
        if let Some(creds) = credentials {
            for (key, value) in creds {
                cmd.env(key, value);
            }
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        let mut child = cmd.spawn()?;

        let session_id = session_id.to_string();

        // -- stdin forwarding ------------------------------------------------
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        if let Some(mut stdin) = child.stdin.take() {
            tokio::spawn(async move {
                while let Some(input) = stdin_rx.recv().await {
                    if stdin.write_all(input.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdin.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
            });
        }

        // -- stdout: NDJSON parsing ------------------------------------------
        let (completion_tx, completion_rx) = oneshot::channel::<NdjsonResult>();

        if let Some(stdout) = child.stdout.take() {
            let tx = outbound_tx.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                let mut completion_tx = Some(completion_tx);
                let mut accumulated_output = String::new();

                loop {
                    let line = match lines.next_line().await {
                        Ok(Some(line)) => line,
                        Ok(None) => break, // EOF
                        Err(e) => {
                            tracing::error!("stdout read error for session {}: {e}", sid);
                            if let Some(ctx) = completion_tx.take() {
                                let _ = ctx.send(NdjsonResult::Error {
                                    error: format!("stdout read error: {e}"),
                                });
                            }
                            return;
                        }
                    };

                    let event: ClaudeEvent = match serde_json::from_str(&line) {
                        Ok(e) => e,
                        Err(_) => {
                            // Non-JSON line — emit as raw output defensively
                            let _ = tx.send(AgentMessage::SessionOutput {
                                session_id: sid.clone(),
                                chunk: format!("{line}\n"),
                                stream: "stdout".to_string(),
                            });
                            continue;
                        }
                    };

                    match event {
                        ClaudeEvent::StreamEvent { event: inner } => {
                            match inner.event_type.as_str() {
                                "content_block_delta" => {
                                    if let Some(delta) = inner.delta
                                        && delta.delta_type == "text_delta"
                                        && let Some(ref text) = delta.text
                                    {
                                        accumulated_output.push_str(text);
                                        let _ = tx.send(AgentMessage::SessionOutput {
                                            session_id: sid.clone(),
                                            chunk: text.clone(),
                                            stream: "stdout".to_string(),
                                        });
                                    }
                                }
                                "content_block_start" => {
                                    if let Some(block) = inner.content_block
                                        && block.block_type == "tool_use"
                                    {
                                        let tool_name = block.name.as_deref().unwrap_or("unknown");
                                        let _ = tx.send(AgentMessage::SessionOutput {
                                            session_id: sid.clone(),
                                            chunk: format!("[tool_use: {tool_name}]\n"),
                                            stream: "stdout".to_string(),
                                        });
                                    }
                                }
                                _ => {} // content_block_stop and others — no action needed
                            }
                        }
                        ClaudeEvent::System {
                            subtype,
                            session_id: claude_sid,
                        } => {
                            if subtype == "session_id"
                                && let Some(ref csid) = claude_sid
                            {
                                tracing::info!("claude session id for {}: {}", sid, csid);
                            }
                        }
                        ClaudeEvent::Result {
                            subtype,
                            result,
                            error,
                        } => {
                            if let Some(ctx) = completion_tx.take() {
                                match subtype.as_str() {
                                    "success" => {
                                        let _ = ctx.send(NdjsonResult::Success {
                                            output: result.unwrap_or(accumulated_output.clone()),
                                        });
                                    }
                                    _ => {
                                        let _ = ctx.send(NdjsonResult::Error {
                                            error: error.unwrap_or_else(|| {
                                                format!("claude exited with subtype: {subtype}")
                                            }),
                                        });
                                    }
                                }
                            }
                        }
                        ClaudeEvent::Unknown => {}
                    }
                }

                // If stdout closes without a result event, signal error
                if let Some(ctx) = completion_tx.take() {
                    let _ = ctx.send(NdjsonResult::Error {
                        error: "stdout closed without result event".to_string(),
                    });
                }
            });
        }

        // -- stderr ----------------------------------------------------------
        if let Some(stderr) = child.stderr.take() {
            let tx = outbound_tx.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Skip known noisy warnings that aren't useful to users
                    if line.contains("no stdin data received") {
                        continue;
                    }
                    let _ = tx.send(AgentMessage::SessionOutput {
                        session_id: sid.clone(),
                        chunk: format!("{line}\n"),
                        stream: "stderr".to_string(),
                    });
                }
            });
        }

        // Notify running state
        let _ = outbound_tx.send(AgentMessage::SessionStateChanged {
            session_id: session_id.clone(),
            state: SessionState::Running,
        });

        Ok(SessionProcess {
            child,
            stdin_tx: Some(stdin_tx),
            completion_rx: Some(completion_rx),
        })
    }

    pub fn send_input(&self, input: &str) -> Result<()> {
        if let Some(ref tx) = self.stdin_tx {
            tx.send(input.to_string())?;
        }
        Ok(())
    }

    /// Consume the NDJSON completion result. Call before `wait()`.
    pub async fn take_ndjson_result(&mut self) -> Option<NdjsonResult> {
        match self.completion_rx.take() {
            Some(rx) => rx.await.ok(),
            _ => None,
        }
    }

    pub async fn wait(&mut self) -> Result<bool> {
        let status = self.child.wait().await?;
        Ok(status.success())
    }

    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_hook_settings_have_http_hook_shape() {
        let json = stop_hook_settings_json(
            "https://app.example.com/api/internal/session-liveness",
            "ws_123",
            "sess_abc",
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let hook = &v["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(hook["type"], "http");
        // Identity rides on the URL query string, not the POST body
        // (the body carries Claude Code's own session id).
        let url = hook["url"].as_str().unwrap();
        assert!(url.starts_with("https://app.example.com/api/internal/session-liveness?"));
        assert!(url.contains("workspace_id=ws_123"));
        assert!(url.contains("session_id=sess_abc"));
        // Token is interpolated from env at call time, never inlined.
        assert_eq!(
            hook["headers"]["Authorization"],
            "Bearer $ONSAGER_SESSION_TOKEN"
        );
        assert_eq!(hook["allowedEnvVars"][0], "ONSAGER_SESSION_TOKEN");
        // Short timeout so a blip fails open promptly.
        assert_eq!(hook["timeout"], 10);
    }

    #[test]
    fn stop_hook_url_uses_amp_when_base_already_has_query() {
        // A base URL carrying its own query string must stay valid —
        // append with `&`, not a second `?`.
        let json = stop_hook_settings_json("https://h.example/live?env=prod", "ws_1", "s_1");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let url = v["hooks"]["Stop"][0]["hooks"][0]["url"].as_str().unwrap();
        assert_eq!(
            url,
            "https://h.example/live?env=prod&workspace_id=ws_1&session_id=s_1"
        );
        // Exactly one `?` in the final URL.
        assert_eq!(url.matches('?').count(), 1);
    }

    #[test]
    fn urlencode_passes_safe_ids_and_escapes_reserved() {
        assert_eq!(urlencode("ws_01HZX-abc.def~9"), "ws_01HZX-abc.def~9");
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
    }
}
