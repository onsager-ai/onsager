use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::core::{AgentMessage, Task};

use super::process::{NdjsonResult, SessionProcess};

/// Credentials to inject as environment variables into the subprocess.
pub type Credentials = Option<HashMap<String, String>>;

pub struct SessionManager {
    max_sessions: u32,
    agent_command: String,
    outbound_tx: mpsc::UnboundedSender<AgentMessage>,
    sessions: Arc<RwLock<HashMap<String, SessionProcess>>>,
    active_count: Arc<AtomicU32>,
}

impl SessionManager {
    pub fn new(
        max_sessions: u32,
        agent_command: String,
        outbound_tx: mpsc::UnboundedSender<AgentMessage>,
    ) -> Self {
        SessionManager {
            max_sessions,
            agent_command,
            outbound_tx,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            active_count: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn active_count_handle(&self) -> Arc<AtomicU32> {
        self.active_count.clone()
    }

    pub async fn spawn_session(
        &mut self,
        task: Task,
        session_id: String,
        credentials: Credentials,
    ) {
        let count = self.active_count.load(Ordering::Relaxed);
        if count >= self.max_sessions {
            tracing::warn!(
                "at capacity ({}/{}), rejecting task {}",
                count,
                self.max_sessions,
                task.id
            );
            let _ = self.outbound_tx.send(AgentMessage::SessionFailed {
                session_id,
                error: "node at capacity".to_string(),
            });
            return;
        }

        match SessionProcess::spawn(
            &task,
            &session_id,
            &self.agent_command,
            self.outbound_tx.clone(),
            credentials.as_ref(),
        )
        .await
        {
            Ok(process) => {
                self.active_count.fetch_add(1, Ordering::Relaxed);
                let mut sessions = self.sessions.write().await;
                sessions.insert(session_id.clone(), process);
                drop(sessions);

                // Spawn a task to wait for completion
                let sessions = self.sessions.clone();
                let active_count = self.active_count.clone();
                let outbound_tx = self.outbound_tx.clone();
                let sid = session_id.clone();

                tokio::spawn(async move {
                    // Remove the process from the map first so we don't hold
                    // the lock across long awaits (blocks send_input/cancel).
                    let mut proc = {
                        let mut sessions = sessions.write().await;
                        sessions.remove(&sid)
                    };
                    active_count.fetch_sub(1, Ordering::Relaxed);

                    // Await NDJSON result then reap the child — no lock held.
                    let ndjson_result = if let Some(ref mut p) = proc {
                        let result = p.take_ndjson_result().await;
                        let _ = p.wait().await;
                        result
                    } else {
                        None
                    };

                    // Use NDJSON result; fall back to generic error if unavailable
                    match ndjson_result {
                        Some(NdjsonResult::Success { output }) => {
                            let _ = outbound_tx.send(AgentMessage::SessionCompleted {
                                session_id: sid,
                                output,
                            });
                        }
                        Some(NdjsonResult::Error { error }) => {
                            let _ = outbound_tx.send(AgentMessage::SessionFailed {
                                session_id: sid,
                                error,
                            });
                        }
                        None => {
                            let _ = outbound_tx.send(AgentMessage::SessionFailed {
                                session_id: sid,
                                error: "process exited without producing a result event"
                                    .to_string(),
                            });
                        }
                    }
                });
            }
            Err(e) => {
                tracing::error!("failed to spawn session: {e}");
                let _ = self.outbound_tx.send(AgentMessage::SessionFailed {
                    session_id,
                    error: e.to_string(),
                });
            }
        }
    }

    pub async fn cancel_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(ref mut proc) = sessions.get_mut(session_id) {
            proc.kill();
        }
    }

    pub async fn send_input(&self, session_id: &str, input: &str) {
        let sessions = self.sessions.read().await;
        if let Some(proc) = sessions.get(session_id) {
            if let Err(e) = proc.send_input(input) {
                tracing::error!("failed to send input to session {session_id}: {e}");
            }
        }
    }
}
