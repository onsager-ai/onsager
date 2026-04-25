use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::core::{AgentMessage, Node, NodeStatus, ServerMessage, SessionState, Task};

use crate::server::db;
use crate::server::handler;
use crate::server::state::{AgentConnection, AppState, WsSender};

pub async fn agent_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_agent_connection(socket, state))
}

async fn handle_agent_connection(socket: WebSocket, state: AppState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Spawn task to forward messages from channel to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut node_id: Option<String> = None;

    // Process incoming messages
    while let Some(Ok(msg)) = ws_receiver.next().await {
        let Message::Text(text) = msg else {
            continue;
        };

        let Ok(agent_msg) = serde_json::from_str::<AgentMessage>(&text) else {
            tracing::warn!("invalid message from agent: {text}");
            continue;
        };

        match agent_msg {
            AgentMessage::Register(info) => {
                // Reuse existing node ID if a node with this name already exists
                let existing = db::find_node_by_name(&state.db, &info.name)
                    .await
                    .ok()
                    .flatten();
                let id = existing
                    .as_ref()
                    .map(|n| n.id.clone())
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                let node = Node {
                    id: id.clone(),
                    name: info.name.clone(),
                    hostname: info.hostname,
                    status: NodeStatus::Online,
                    max_sessions: info.max_sessions,
                    active_sessions: 0,
                    last_heartbeat: Utc::now(),
                    registered_at: existing.map(|n| n.registered_at).unwrap_or_else(Utc::now),
                };

                if let Err(e) = db::upsert_node(&state.db, &node).await {
                    tracing::error!("failed to register node: {e}");
                    continue;
                }

                // Store agent connection
                {
                    let mut agents = state.agents.write().await;
                    agents.insert(
                        id.clone(),
                        AgentConnection {
                            node_id: id.clone(),
                            sender: tx.clone(),
                        },
                    );
                }

                node_id = Some(id.clone());
                tracing::info!("node registered: {} ({})", info.name, id);

                // Send confirmation
                let response = ServerMessage::Registered {
                    node_id: id.clone(),
                };
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = tx.send(Message::Text(json.into()));
                }

                // Drain any sessions that were created and assigned to
                // this node while the agent was disconnected. Without
                // this, a session created by `POST /api/shaping` during
                // a brief disconnect (or before the agent has registered
                // for the first time) sits in `pending` forever even
                // though the spine's `stiglab.session_created` event
                // says one was scheduled.
                dispatch_pending_for_node(&state, &id, &tx).await;
            }

            other => {
                if let Some(ref nid) = node_id {
                    handler::handle_agent_message(
                        &state.db,
                        nid,
                        other,
                        state.spine.as_ref(),
                        Some(&state.session_completion_tx),
                    )
                    .await;
                }
            }
        }
    }

    // Clean up on disconnect
    if let Some(ref nid) = node_id {
        tracing::info!("node disconnected: {nid}");
        let _ = db::update_node_status(&state.db, nid, NodeStatus::Offline).await;
        let mut agents = state.agents.write().await;
        agents.remove(nid);
    }

    send_task.abort();
}

/// Look up sessions that were assigned to this node but never dispatched
/// (state stuck in `pending`) and dispatch them now over the freshly
/// registered WebSocket. Best-effort: a send failure logs but doesn't
/// abort registration — the next reconnect will retry.
///
/// Replay is gated on `artifact_id.is_some()` so we only redispatch
/// sessions that came from `POST /api/shaping`. The `sessions` row
/// persists `prompt` / `working_dir` / `artifact_id` / `artifact_version`
/// but NOT the task-level fields (`allowed_tools`, `max_turns`, `model`,
/// `system_prompt`, `permission_mode`, `credentials`). For shaping
/// sessions those are always `None` (see `adapter::shaping_request_to_task`),
/// so a lossless replay is safe. Direct `POST /api/tasks` sessions can
/// carry user-set values that aren't on the row, so replaying them with
/// defaults would silently change the task — skip + log instead.
async fn dispatch_pending_for_node(state: &AppState, node_id: &str, tx: &WsSender) {
    let pending = match db::list_pending_sessions_for_node(&state.db, node_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(node_id, "failed to list pending sessions on register: {e}");
            return;
        }
    };
    if pending.is_empty() {
        return;
    }
    tracing::info!(
        node_id,
        count = pending.len(),
        "draining pending sessions for newly registered agent"
    );

    for session in pending {
        if session.artifact_id.is_none() {
            // Direct-task session: the persisted row is missing
            // task-level fields a user may have set on the original
            // request. Re-dispatching with defaults would silently
            // change the task; surface the orphan instead.
            tracing::warn!(
                session_id = %session.id,
                task_id = %session.task_id,
                node_id,
                "skipping pending direct-task session replay — persisted row \
                 doesn't carry the full original task payload"
            );
            continue;
        }

        // Atomically claim the session before sending. If another path
        // (e.g. a parallel reconnect or the create-time dispatch that
        // raced with our list) already promoted this row past `pending`,
        // the conditional UPDATE returns 0 affected rows and we skip.
        // This closes the window where Copilot review #137 worried
        // about duplicate local processes per session_id.
        let claimed =
            match db::claim_pending_session(&state.db, &session.id, SessionState::Dispatched).await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        session_id = %session.id,
                        "failed to claim pending session for redispatch: {e}"
                    );
                    continue;
                }
            };
        if !claimed {
            tracing::debug!(
                session_id = %session.id,
                "pending session already claimed by another path — skipping replay"
            );
            continue;
        }

        // Shaping sessions: all task-level fields are known to be `None`
        // for the original request (see shaping_request_to_task), so the
        // reconstruction below is lossless.
        let task = Task {
            id: session.task_id.clone(),
            prompt: session.prompt.clone(),
            node_id: Some(node_id.to_string()),
            working_dir: session.working_dir.clone(),
            allowed_tools: None,
            max_turns: None,
            model: None,
            system_prompt: None,
            permission_mode: None,
            created_at: session.created_at,
        };
        let msg = ServerMessage::DispatchTask {
            task: Box::new(task),
            session_id: session.id.clone(),
            credentials: None,
        };
        let Ok(json) = serde_json::to_string(&msg) else {
            continue;
        };
        if tx.send(Message::Text(json.into())).is_err() {
            tracing::warn!(
                session_id = %session.id,
                "failed to dispatch claimed session to agent — channel closed; \
                 leaving session in dispatched state for the next reconnect"
            );
        }
    }
}
