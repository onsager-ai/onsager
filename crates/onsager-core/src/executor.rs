use onsager_events::{CoreEvent, EventMetadata, EventStore};
use uuid::Uuid;

use crate::node::Node;
use crate::process::{NdjsonResult, ProcessConfig, ProcessOutput, SessionProcess};
use crate::task::TaskRequest;

/// Callback for session events — allows the CLI to react in real time.
pub type EventCallback = Box<dyn Fn(&CoreEvent) + Send>;

/// Executes a session end-to-end: creates task, spawns Claude, emits events.
pub struct SessionExecutor {
    store: EventStore,
    config: ProcessConfig,
}

/// Result of a completed session execution.
pub struct ExecutionResult {
    pub session_id: String,
    pub task_id: String,
    pub output: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

impl SessionExecutor {
    pub fn new(store: EventStore, config: ProcessConfig) -> Self {
        Self { store, config }
    }

    /// Run a session from a task request. Emits events at each lifecycle stage.
    /// The `on_event` callback is invoked synchronously for each event, enabling
    /// real-time terminal output.
    pub async fn run(
        &self,
        request: &TaskRequest,
        on_event: Option<EventCallback>,
        policy_eval: Option<&dyn PolicyEvaluator>,
    ) -> anyhow::Result<ExecutionResult> {
        let task_id = Uuid::new_v4().to_string();
        let session_id = Uuid::new_v4().to_string();
        let node = Node::local("local-0".to_string(), "local".to_string());
        let meta = EventMetadata {
            actor: "cli".to_string(),
            ..Default::default()
        };

        // 1. Task created
        let task_event = CoreEvent::TaskCreated {
            task_id: task_id.clone(),
            prompt: request.prompt.clone(),
            working_dir: request.working_dir.clone(),
            model: request.model.clone(),
            max_turns: request.max_turns,
            system_prompt: request.system_prompt.clone(),
            permission_mode: request.permission_mode.clone(),
        };
        self.emit(&task_event, &meta, &on_event).await?;

        // 2. Node registered (idempotent)
        let node_event = CoreEvent::NodeRegistered {
            node_id: node.id.clone(),
            name: node.name.clone(),
            hostname: node.hostname.clone(),
        };
        self.emit(&node_event, &meta, &on_event).await?;

        // 3. Session created
        let create_event = CoreEvent::SessionCreated {
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            node_id: node.id.clone(),
        };
        self.emit(&create_event, &meta, &on_event).await?;

        // 4. Session dispatched
        let dispatch_event = CoreEvent::SessionDispatched {
            session_id: session_id.clone(),
        };
        self.emit(&dispatch_event, &meta, &on_event).await?;

        // 5. Spawn Claude subprocess
        let mut process =
            match SessionProcess::spawn(&request.prompt, &request.working_dir, &self.config) {
                Ok(p) => p,
                Err(e) => {
                    let fail_event = CoreEvent::SessionFailed {
                        session_id: session_id.clone(),
                        error: format!("failed to spawn agent: {e}"),
                    };
                    self.emit(&fail_event, &meta, &on_event).await?;
                    return Ok(ExecutionResult {
                        session_id,
                        task_id,
                        output: None,
                        success: false,
                        error: Some(format!("failed to spawn agent: {e}")),
                    });
                }
            };

        // 6. Session running
        let run_event = CoreEvent::SessionRunning {
            session_id: session_id.clone(),
        };
        self.emit(&run_event, &meta, &on_event).await?;

        // 7. Process NDJSON output
        let mut accumulated_text = String::new();

        while let Some(output) = process.recv().await {
            match output {
                ProcessOutput::Text(text) => {
                    accumulated_text.push_str(&text);
                    let text_event = CoreEvent::SessionOutput {
                        session_id: session_id.clone(),
                        chunk: text,
                    };
                    self.emit(&text_event, &meta, &on_event).await?;
                }
                ProcessOutput::ToolUse { name, input } => {
                    let tool_event = CoreEvent::SessionToolUse {
                        session_id: session_id.clone(),
                        tool_name: name.clone(),
                        tool_input: input.clone(),
                    };
                    let event_id = self.emit(&tool_event, &meta, &on_event).await?;

                    // Evaluate against policy engine (observational)
                    if let Some(evaluator) = policy_eval {
                        evaluator
                            .evaluate(&self.store, &session_id, event_id, &name, &input, &meta)
                            .await?;
                    }
                }
                ProcessOutput::Completed(_) => {
                    // The result will come from take_result
                }
                ProcessOutput::Stderr(line) => {
                    tracing::debug!("[stderr] {}", line);
                }
            }
        }

        // 8. Get final result
        let result = process.take_result().await;
        match result {
            Some(NdjsonResult::Success { output }) => {
                let final_output = output.or(if accumulated_text.is_empty() {
                    None
                } else {
                    Some(accumulated_text)
                });
                let complete_event = CoreEvent::SessionCompleted {
                    session_id: session_id.clone(),
                    output: final_output.clone(),
                };
                self.emit(&complete_event, &meta, &on_event).await?;
                Ok(ExecutionResult {
                    session_id,
                    task_id,
                    output: final_output,
                    success: true,
                    error: None,
                })
            }
            Some(NdjsonResult::Error { message }) => {
                let fail_event = CoreEvent::SessionFailed {
                    session_id: session_id.clone(),
                    error: message.clone(),
                };
                self.emit(&fail_event, &meta, &on_event).await?;
                Ok(ExecutionResult {
                    session_id,
                    task_id,
                    output: None,
                    success: false,
                    error: Some(message),
                })
            }
            None => {
                let fail_event = CoreEvent::SessionFailed {
                    session_id: session_id.clone(),
                    error: "agent exited without result".to_string(),
                };
                self.emit(&fail_event, &meta, &on_event).await?;
                Ok(ExecutionResult {
                    session_id,
                    task_id,
                    output: None,
                    success: false,
                    error: Some("agent exited without result".to_string()),
                })
            }
        }
    }

    /// Emit an event: persist to store and invoke callback.
    async fn emit(
        &self,
        event: &CoreEvent,
        metadata: &EventMetadata,
        on_event: &Option<EventCallback>,
    ) -> anyhow::Result<i64> {
        let id = self.store.append(event, metadata).await?;
        if let Some(cb) = on_event {
            cb(event);
        }
        Ok(id)
    }
}

/// Trait for policy evaluation — decouples the executor from onsager-synodic.
#[async_trait::async_trait]
pub trait PolicyEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        store: &EventStore,
        session_id: &str,
        ref_event_id: i64,
        tool_name: &str,
        tool_input: &serde_json::Value,
        metadata: &EventMetadata,
    ) -> anyhow::Result<()>;
}
