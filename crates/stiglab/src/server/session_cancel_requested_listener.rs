//! Spine listener that consumes `portal.session_cancel_requested` events
//! (spec #303) and forwards a `ServerMessage::CancelSession` to the
//! session's agent over the WebSocket.
//!
//! Portal owns `POST /api/sessions/:id/cancel` and emits this event after
//! authorizing the caller; stiglab is the only consumer. Best-effort —
//! if the node isn't connected, or the session is already terminal, the
//! cancel is dropped with a warn log. The agent decides locally what to
//! do (its session_manager already exposes `cancel_session`).

use async_trait::async_trait;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use serde::Deserialize;

use crate::core::{ServerMessage, SessionState};
use crate::server::db;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
struct CancelPayload {
    session_id: String,
    #[serde(default)]
    actor: Option<String>,
}

pub async fn run(store: EventStore, app_state: AppState, since: Option<i64>) -> anyhow::Result<()> {
    let handler = Canceller {
        store: store.clone(),
        app_state,
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct Canceller {
    store: EventStore,
    app_state: AppState,
}

#[async_trait]
impl EventHandler for Canceller {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "portal.session_cancel_requested" {
            return Ok(());
        }
        if notification.table != "events_ext" {
            return Ok(());
        }

        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        let payload: CancelPayload = match serde_json::from_value(row.data) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    id = notification.id,
                    "stiglab: portal.session_cancel_requested parse failed: {e}"
                );
                return Ok(());
            }
        };

        self.dispatch(payload).await
    }
}

impl Canceller {
    async fn dispatch(&self, payload: CancelPayload) -> anyhow::Result<()> {
        let state = &self.app_state;

        let session = match db::get_session(&state.db, &payload.session_id).await? {
            Some(s) => s,
            None => {
                tracing::warn!(
                    session_id = %payload.session_id,
                    "stiglab: cancel — session not found, skipping"
                );
                return Ok(());
            }
        };

        // Idempotency: terminal sessions are no-ops. Pending sessions
        // never dispatched, so just mark them aborted locally.
        if matches!(session.state, SessionState::Done | SessionState::Failed) {
            tracing::debug!(
                session_id = %payload.session_id,
                state = ?session.state,
                "stiglab: cancel — session already terminal, skipping"
            );
            return Ok(());
        }

        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&session.node_id) {
            let msg = ServerMessage::CancelSession {
                session_id: payload.session_id.clone(),
            };
            match serde_json::to_string(&msg) {
                Ok(json) => {
                    if agent
                        .sender
                        .send(axum::extract::ws::Message::Text(json.into()))
                        .is_ok()
                    {
                        tracing::info!(
                            session_id = %payload.session_id,
                            node_id = %session.node_id,
                            actor = ?payload.actor,
                            "stiglab: forwarded CancelSession to agent"
                        );
                    } else {
                        tracing::warn!(
                            session_id = %payload.session_id,
                            node_id = %session.node_id,
                            "stiglab: cancel — agent WS send failed"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %payload.session_id,
                        "stiglab: cancel — serialize failed: {e}"
                    );
                }
            }
        } else {
            tracing::warn!(
                session_id = %payload.session_id,
                node_id = %session.node_id,
                "stiglab: cancel — agent not connected"
            );
        }

        Ok(())
    }
}
