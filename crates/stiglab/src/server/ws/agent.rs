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
        // Reconstruct the dispatch payload from the persisted row. The
        // task id matches the original ShapingRequest.request_id, so a
        // duplicate dispatch is collapsed by the agent's own idempotency
        // (it tracks session_id) — there's no double-shape risk.
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
                "failed to dispatch pending session to agent — channel closed"
            );
            continue;
        }
        if let Err(e) =
            db::update_session_state(&state.db, &session.id, SessionState::Dispatched).await
        {
            tracing::warn!(
                session_id = %session.id,
                "dispatched pending session but failed to update state: {e}"
            );
        }
    }
}
