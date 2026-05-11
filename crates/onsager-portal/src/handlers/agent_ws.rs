//! Transparent WebSocket proxy for the agent control plane.
//!
//! Portal accepts the externally-reachable `/agent/ws` upgrade (per
//! ADR 0008) and opens a backend WebSocket on loopback to stiglab at
//! `ws://127.0.0.1:3000/agent/ws-internal`. Two `tokio` tasks forward
//! bytes bidirectionally; portal does not parse the agent protocol.
//!
//! v1 inherits stiglab's existing auth model — the WebSocket itself is
//! unauthenticated today, and this move is intentionally structural
//! per ADR 0008's out-of-scope list. Tightening to PAT/session auth is
//! filed as a follow-up.

use std::borrow::Cow;
use std::env;

use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TungsteniteCloseFrame;

/// `GET /agent/ws` — accept the upgrade and proxy bytes to stiglab
/// on loopback. The backend dial happens inside `on_upgrade`, so the
/// HTTP 101 response has already been sent by the time we know
/// whether stiglab is reachable; a failed dial closes the just-
/// upgraded socket and the agent CLI's reconnect loop retries.
pub async fn proxy_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    let backend_url = env::var("STIGLAB_INTERNAL_WS_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:3000/agent/ws-internal".to_string());
    ws.on_upgrade(move |socket| proxy(socket, backend_url))
}

async fn proxy(mut client: WebSocket, backend_url: String) {
    let (backend_stream, _resp) = match connect_async(&backend_url).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(error = %e, url = %backend_url, "agent_ws: backend dial failed");
            // Close the client side so the agent CLI retries via its
            // existing reconnect loop.
            let _ = client.close().await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client.split();
    let (mut backend_tx, mut backend_rx) = backend_stream.split();

    // Client → backend.
    let c2b = async move {
        while let Some(msg) = client_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!(error = %e, "agent_ws: client recv error");
                    break;
                }
            };
            let forwarded = axum_to_tungstenite(msg);
            let is_close = matches!(forwarded, TungsteniteMessage::Close(_));
            if let Err(e) = backend_tx.send(forwarded).await {
                tracing::debug!(error = %e, "agent_ws: backend send error");
                break;
            }
            if is_close {
                break;
            }
        }
        let _ = backend_tx.close().await;
    };

    // Backend → client.
    let b2c = async move {
        while let Some(msg) = backend_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!(error = %e, "agent_ws: backend recv error");
                    break;
                }
            };
            let forwarded = match tungstenite_to_axum(msg) {
                Some(m) => m,
                None => continue,
            };
            let is_close = matches!(forwarded, AxumMessage::Close(_));
            if let Err(e) = client_tx.send(forwarded).await {
                tracing::debug!(error = %e, "agent_ws: client send error");
                break;
            }
            if is_close {
                break;
            }
        }
        let _ = client_tx.close().await;
    };

    // Run both directions concurrently; either side terminating closes
    // the other implicitly via the `close()` calls above.
    tokio::join!(c2b, b2c);
}

fn axum_to_tungstenite(msg: AxumMessage) -> TungsteniteMessage {
    match msg {
        AxumMessage::Text(t) => TungsteniteMessage::Text(String::from(t.as_str())),
        AxumMessage::Binary(b) => TungsteniteMessage::Binary(b.to_vec()),
        AxumMessage::Ping(p) => TungsteniteMessage::Ping(p.to_vec()),
        AxumMessage::Pong(p) => TungsteniteMessage::Pong(p.to_vec()),
        AxumMessage::Close(frame) => {
            TungsteniteMessage::Close(frame.map(|f| TungsteniteCloseFrame {
                code: f.code.into(),
                reason: Cow::Owned(String::from(f.reason.as_str())),
            }))
        }
    }
}

fn tungstenite_to_axum(msg: TungsteniteMessage) -> Option<AxumMessage> {
    match msg {
        TungsteniteMessage::Text(t) => Some(AxumMessage::Text(t.into())),
        TungsteniteMessage::Binary(b) => Some(AxumMessage::Binary(b.into())),
        TungsteniteMessage::Ping(p) => Some(AxumMessage::Ping(p.into())),
        TungsteniteMessage::Pong(p) => Some(AxumMessage::Pong(p.into())),
        TungsteniteMessage::Close(frame) => {
            Some(AxumMessage::Close(frame.map(|f| AxumCloseFrame {
                code: f.code.into(),
                reason: f.reason.into_owned().into(),
            })))
        }
        // tungstenite-only raw frame; never produced by a peer that
        // speaks the high-level message API.
        TungsteniteMessage::Frame(_) => None,
    }
}
