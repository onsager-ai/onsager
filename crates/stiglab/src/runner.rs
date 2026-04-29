//! Built-in runner: embeds an agent's SessionManager directly in the server process.
//!
//! The runner registers itself as a regular node in the DB and appears in the
//! agents map like any WebSocket-connected agent. Task dispatch flows through
//! the same code path — the server doesn't know the difference.

use std::sync::atomic::Ordering;

use axum::extract::ws::Message;
use chrono::Utc;
use tokio::sync::mpsc;
use uuid::Uuid;

use stiglab::agent::session::manager::SessionManager;
use stiglab::core::{AgentMessage, Node, NodeStatus, ServerMessage};
use stiglab::server::db;
use stiglab::server::handler;
use stiglab::server::state::{AgentConnection, AppState};
use stiglab::server::AnyPool;

pub async fn start_built_in_runner(
    state: &AppState,
    pool: &AnyPool,
    node_name: &str,
    max_sessions: u32,
    agent_command: &str,
) -> anyhow::Result<()> {
    // Register (or reuse) the node in the database
    let existing = db::find_node_by_name(pool, node_name).await.ok().flatten();
    let node_id = existing
        .as_ref()
        .map(|n| n.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "localhost".to_string());

    let node = Node {
        id: node_id.clone(),
        name: node_name.to_string(),
        hostname,
        status: NodeStatus::Online,
        max_sessions,
        active_sessions: 0,
        last_heartbeat: Utc::now(),
        registered_at: existing.map(|n| n.registered_at).unwrap_or_else(Utc::now),
    };
    db::upsert_node(pool, &node).await?;
    tracing::info!("built-in runner registered as node: {node_name} ({node_id})");

    // Channel for server → runner (dispatched tasks arrive here)
    let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel::<Message>();

    // Insert into the agents map so the server dispatches to us like any other agent
    {
        let mut agents = state.agents.write().await;
        agents.insert(
            node_id.clone(),
            AgentConnection {
                node_id: node_id.clone(),
                sender: dispatch_tx,
            },
        );
    }

    // Channel for runner → server (session events flow back here)
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<AgentMessage>();

    let mut session_manager =
        SessionManager::new(max_sessions, agent_command.to_string(), outbound_tx.clone());

    // Heartbeat: keep the node alive in the DB
    let heartbeat_pool = pool.clone();
    let heartbeat_node_id = node_id.clone();
    let active_count = session_manager.active_count_handle();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let count = active_count.load(Ordering::Relaxed);
            if let Err(e) =
                db::update_node_heartbeat(&heartbeat_pool, &heartbeat_node_id, count).await
            {
                tracing::warn!(
                    node_id = %heartbeat_node_id,
                    active_sessions = count,
                    error = ?e,
                    "built-in runner: failed to update heartbeat"
                );
            }
        }
    });

    // Task: receive dispatched tasks from the server and feed them to SessionManager
    tokio::spawn(async move {
        while let Some(msg) = dispatch_rx.recv().await {
            let Message::Text(text) = msg else {
                continue;
            };

            let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) else {
                tracing::warn!("built-in runner: invalid message: {text}");
                continue;
            };

            match server_msg {
                ServerMessage::DispatchTask {
                    task,
                    session_id,
                    credentials,
                } => {
                    tracing::info!(
                        "built-in runner: received task {} (session: {})",
                        task.id,
                        session_id
                    );
                    session_manager
                        .spawn_session(*task, session_id, credentials)
                        .await;
                }
                ServerMessage::CancelSession { session_id } => {
                    tracing::info!("built-in runner: cancelling session {session_id}");
                    session_manager.cancel_session(&session_id).await;
                }
                ServerMessage::SendInput { session_id, input } => {
                    session_manager.send_input(&session_id, &input).await;
                }
                ServerMessage::Registered { .. } => {}
            }
        }
    });

    // Task: process outbound messages from SessionManager using the shared handler
    let event_pool = pool.clone();
    let runner_spine = state.spine.clone();
    tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            handler::handle_agent_message(&event_pool, &node_id, msg, runner_spine.as_ref()).await;
        }
    });

    Ok(())
}
