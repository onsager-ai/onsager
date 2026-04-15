use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::core::{AgentMessage, NodeInfo, ServerMessage};

use crate::agent::config::AgentConfig;
use crate::agent::session::manager::SessionManager;

pub async fn connect_and_run(config: AgentConfig) -> Result<()> {
    let node_name = config.node_name();
    tracing::info!("connecting to server: {}", config.server);

    let (ws_stream, _) = connect_async(&config.server).await?;
    tracing::info!("connected to server");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<AgentMessage>();

    // Send Register message
    let register = AgentMessage::Register(NodeInfo {
        name: node_name.clone(),
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        max_sessions: config.max_sessions,
    });

    let json = serde_json::to_string(&register)?;
    ws_sender.send(Message::Text(json)).await?;

    let mut session_manager = SessionManager::new(
        config.max_sessions,
        config.agent_command.clone(),
        outbound_tx.clone(),
    );

    // Spawn outbound message forwarder
    let send_task = tokio::spawn(async move {
        while let Some(msg) = outbound_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Spawn heartbeat loop
    let heartbeat_tx = outbound_tx.clone();
    let heartbeat_manager = session_manager.active_count_handle();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let count = heartbeat_manager.load(std::sync::atomic::Ordering::Relaxed);
            let _ = heartbeat_tx.send(AgentMessage::Heartbeat {
                active_sessions: count,
            });
        }
    });

    // Process incoming messages from server
    while let Some(Ok(msg)) = ws_receiver.next().await {
        let Message::Text(text) = msg else {
            continue;
        };

        let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) else {
            tracing::warn!("invalid message from server: {text}");
            continue;
        };

        match server_msg {
            ServerMessage::Registered { node_id } => {
                tracing::info!("registered as node: {node_id}");
            }
            ServerMessage::DispatchTask {
                task,
                session_id,
                credentials,
            } => {
                tracing::info!(
                    "received task: {} (session: {}) - {}",
                    task.id,
                    session_id,
                    &task.prompt[..task.prompt.len().min(50)]
                );
                session_manager
                    .spawn_session(*task, session_id, credentials)
                    .await;
            }
            ServerMessage::CancelSession { session_id } => {
                tracing::info!("cancelling session: {session_id}");
                session_manager.cancel_session(&session_id).await;
            }
            ServerMessage::SendInput { session_id, input } => {
                tracing::info!("sending input to session: {session_id}");
                session_manager.send_input(&session_id, &input).await;
            }
        }
    }

    tracing::warn!("disconnected from server");
    heartbeat_task.abort();
    send_task.abort();

    Ok(())
}
