use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::core::{AgentMessage, SessionState, Task};

// ---------------------------------------------------------------------------
// Liveness Stop hook + MCP spawn wiring (spec #520 §4d / #525, #531)
// ---------------------------------------------------------------------------
//
// The env-var constants and JSON builders that register the liveness
// Stop hook and portal's MCP server were authored here inline (#530/
// #532) and were extracted to the shared `onsager-agent-spawn` crate
// (spec #535) so the scheduler-side production runner can reuse the
// exact same wiring across the subsystem seam. Stiglab now consumes
// them rather than owning a private copy.
use onsager_agent_spawn::{
    LIVENESS_HOOK_URL_ENV, MCP_URL_ENV, SESSION_TOKEN_ENV, mcp_config_json, stop_hook_settings_json,
};

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

/// Completion signal parsed off the `claude` CLI's NDJSON stdout
/// stream.
///
/// Per spec #520 §4b this stays a "session over" signal — it carries
/// only the final assistant `output`, **not** the artifacts the session
/// emitted. The agent records outputs out-of-band by calling the portal
/// MCP `emit_artifact` / `declare_no_output` tools (§4a), which write
/// the `events_ext` `session:<id>` manifest. That manifest is the
/// authoritative source of "what came out": the substrate
/// `AgentExecutor` reads it back directly to run the liveness gate (§4c)
/// rather than trusting anything parsed from this stdout stream. The
/// optimization channel for surfacing emitted refs lives on
/// `onsager_nodes::AgentResponse.emitted` — populated by a runner that
/// can cheaply observe them, left empty here because the NDJSON stream
/// genuinely does not see MCP tool side effects.
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

        // Portal MCP server (spec #531). Registered only when the MCP URL
        // is configured AND the session token is present in the env —
        // Claude Code fails to parse an MCP config whose `${VAR}` is unset,
        // so a tokenless registration would break the session rather than
        // degrade. Absent either, the agent runs without MCP (no emit
        // path); the authoritative gate (#523) still covers liveness.
        let has_session_token = credentials.is_some_and(|c| c.contains_key(SESSION_TOKEN_ENV));
        if let Ok(mcp_url) = std::env::var(MCP_URL_ENV)
            && !mcp_url.is_empty()
            && has_session_token
        {
            let mcp_config = mcp_config_json(&mcp_url);
            cmd.args(["--mcp-config", &mcp_config]);
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

// The Stop-hook / MCP-config builder unit tests moved with their
// functions to `onsager-agent-spawn` (spec #535). Stiglab's own session
// tests live alongside the session lifecycle code; the spawn-wiring
// shape is asserted once, in the crate that owns it.
