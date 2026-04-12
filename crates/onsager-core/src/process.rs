use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

/// Top-level NDJSON event envelope from Claude's `--output-format stream-json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeEvent {
    /// System metadata (session IDs, etc.)
    #[serde(alias = "system")]
    System {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Message start
    #[serde(alias = "message_start")]
    MessageStart {
        #[serde(default)]
        message: Option<serde_json::Value>,
    },
    /// Content block start — signals tool_use or text block beginning
    #[serde(alias = "content_block_start")]
    ContentBlockStart {
        #[serde(default)]
        index: u32,
        #[serde(default)]
        content_block: Option<ContentBlock>,
    },
    /// Content block delta — streaming text or tool input
    #[serde(alias = "content_block_delta")]
    ContentBlockDelta {
        #[serde(default)]
        index: u32,
        #[serde(default)]
        delta: Option<DeltaPayload>,
    },
    /// Content block stop
    #[serde(alias = "content_block_stop")]
    ContentBlockStop {
        #[serde(default)]
        index: u32,
    },
    /// Final result
    #[serde(alias = "result")]
    Result {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(default)]
        cost_usd: Option<f64>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        duration_api_ms: Option<u64>,
    },
    /// Catch-all for unrecognized event types
    #[serde(other)]
    Unknown,
}

/// Content block metadata — identifies tool_use blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Delta payload — streaming text or input_json_delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaPayload {
    #[serde(rename = "type")]
    pub delta_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Result from NDJSON parsing — either success with output or an error.
#[derive(Debug)]
pub enum NdjsonResult {
    Success { output: Option<String> },
    Error { message: String },
}

/// Configuration for spawning a Claude subprocess.
#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub agent_command: String,
    pub permission_mode: String,
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub system_prompt: Option<String>,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            agent_command: "claude".to_string(),
            permission_mode: "auto".to_string(),
            model: None,
            max_turns: None,
            system_prompt: None,
        }
    }
}

/// Parsed output from the Claude subprocess stream.
#[derive(Debug)]
pub enum ProcessOutput {
    /// Streaming text output
    Text(String),
    /// Tool use detected
    ToolUse {
        name: String,
        input: serde_json::Value,
    },
    /// Session completed
    Completed(NdjsonResult),
    /// stderr line
    Stderr(String),
}

/// Manages a Claude agent subprocess.
pub struct SessionProcess {
    child: Child,
    stdin_tx: mpsc::UnboundedSender<String>,
    output_rx: mpsc::UnboundedReceiver<ProcessOutput>,
    result_rx: Option<oneshot::Receiver<NdjsonResult>>,
}

impl SessionProcess {
    /// Spawn a Claude subprocess with the given prompt and config.
    pub fn spawn(prompt: &str, working_dir: &str, config: &ProcessConfig) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.agent_command);
        cmd.arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--print-input-tokens")
            .arg("--print-output-tokens");

        cmd.arg("--permission-mode").arg(&config.permission_mode);

        if let Some(model) = &config.model {
            cmd.arg("--model").arg(model);
        }

        if let Some(max_turns) = config.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }

        if let Some(sys_prompt) = &config.system_prompt {
            cmd.arg("--system-prompt").arg(sys_prompt);
        }

        cmd.arg("--prompt").arg(prompt);

        cmd.current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");

        // Stdin forwarding channel
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let mut stdin = stdin;
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

        // Output channel
        let (output_tx, output_rx) = mpsc::unbounded_channel::<ProcessOutput>();
        let (result_tx, result_rx) = oneshot::channel::<NdjsonResult>();

        // Stdout NDJSON parser
        let out_tx = output_tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut accumulated_output = String::new();
            let mut current_tool_name: Option<String> = None;
            let mut current_tool_input = String::new();
            let mut final_result: Option<NdjsonResult> = None;

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                match serde_json::from_str::<ClaudeEvent>(&line) {
                    Ok(event) => match event {
                        ClaudeEvent::ContentBlockStart {
                            content_block: Some(block),
                            ..
                        } => {
                            if block.block_type == "tool_use" {
                                current_tool_name = block.name;
                                current_tool_input.clear();
                            }
                        }
                        ClaudeEvent::ContentBlockDelta {
                            delta: Some(delta), ..
                        } => {
                            if delta.delta_type == "text_delta" {
                                if let Some(text) = &delta.text {
                                    accumulated_output.push_str(text);
                                    let _ = out_tx.send(ProcessOutput::Text(text.clone()));
                                }
                            } else if delta.delta_type == "input_json_delta" {
                                if let Some(text) = &delta.text {
                                    current_tool_input.push_str(text);
                                }
                            }
                        }
                        ClaudeEvent::ContentBlockStop { .. } => {
                            if let Some(name) = current_tool_name.take() {
                                let input: serde_json::Value =
                                    serde_json::from_str(&current_tool_input)
                                        .unwrap_or(serde_json::Value::Null);
                                let _ = out_tx.send(ProcessOutput::ToolUse { name, input });
                                current_tool_input.clear();
                            }
                        }
                        ClaudeEvent::Result {
                            result, is_error, ..
                        } => {
                            final_result = Some(if is_error == Some(true) {
                                NdjsonResult::Error {
                                    message: result.unwrap_or_else(|| "unknown error".to_string()),
                                }
                            } else {
                                NdjsonResult::Success {
                                    output: result.or_else(|| {
                                        if accumulated_output.is_empty() {
                                            None
                                        } else {
                                            Some(accumulated_output.clone())
                                        }
                                    }),
                                }
                            });
                        }
                        _ => {}
                    },
                    Err(_) => {
                        // Malformed JSON — emit as raw text
                        let _ = out_tx.send(ProcessOutput::Text(line));
                    }
                }
            }

            let result = final_result.unwrap_or(NdjsonResult::Success {
                output: if accumulated_output.is_empty() {
                    None
                } else {
                    Some(accumulated_output)
                },
            });

            let _ = out_tx.send(ProcessOutput::Completed(NdjsonResult::Success {
                output: None,
            }));
            let _ = result_tx.send(result);
        });

        // Stderr reader
        let err_tx = output_tx;
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = err_tx.send(ProcessOutput::Stderr(line));
            }
        });

        Ok(Self {
            child,
            stdin_tx,
            output_rx,
            result_rx: Some(result_rx),
        })
    }

    /// Send input to the subprocess stdin.
    pub fn send_input(&self, input: &str) -> anyhow::Result<()> {
        self.stdin_tx
            .send(input.to_string())
            .map_err(|_| anyhow::anyhow!("subprocess stdin channel closed"))
    }

    /// Receive the next output from the subprocess.
    pub async fn recv(&mut self) -> Option<ProcessOutput> {
        self.output_rx.recv().await
    }

    /// Take the final NDJSON result (can only be called once).
    pub async fn take_result(mut self) -> Option<NdjsonResult> {
        if let Some(rx) = self.result_rx.take() {
            rx.await.ok()
        } else {
            None
        }
    }

    /// Kill the subprocess.
    pub async fn kill(&mut self) -> anyhow::Result<()> {
        self.child.kill().await?;
        Ok(())
    }

    /// Wait for the subprocess to exit.
    pub async fn wait(&mut self) -> anyhow::Result<std::process::ExitStatus> {
        let status = self.child.wait().await?;
        Ok(status)
    }
}
