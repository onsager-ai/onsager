//! Spine listener that consumes `portal.session_requested` events and
//! dispatches the session to an agent via WebSocket (spec #222 Follow-up 3).
//!
//! Portal owns `POST /api/tasks` and creates the session row in `Pending`
//! state, then emits this event. Stiglab listens, fetches the workspace
//! credential set, and sends `ServerMessage::DispatchTask` to the agent
//! node — the same dispatch path `routes::tasks::create_task` used before
//! it moved to portal.

use std::collections::HashMap;

use async_trait::async_trait;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use serde::Deserialize;

use crate::core::{ServerMessage, SessionState, Task};
use crate::server::auth::decrypt_credential;
use crate::server::db;
use crate::server::state::AppState;

/// Wire payload from `portal.session_requested`.
#[derive(Debug, Deserialize)]
struct TaskDispatchPayload {
    session_id: String,
    node_id: String,
    task_id: String,
    prompt: String,
    #[serde(default)]
    working_dir: Option<String>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    max_turns: Option<u32>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    user_id: String,
}

pub async fn run(store: EventStore, app_state: AppState, since: Option<i64>) -> anyhow::Result<()> {
    let handler = Dispatcher {
        store: store.clone(),
        app_state,
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct Dispatcher {
    store: EventStore,
    app_state: AppState,
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "portal.session_requested" {
            return Ok(());
        }

        // The event lives in events_ext (portal uses append_ext).
        if notification.table != "events_ext" {
            return Ok(());
        }

        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        let payload: TaskDispatchPayload = match serde_json::from_value(row.data) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    id = notification.id,
                    "stiglab: portal.session_requested parse failed: {e}"
                );
                return Ok(());
            }
        };

        self.dispatch(payload).await
    }
}

impl Dispatcher {
    async fn dispatch(&self, payload: TaskDispatchPayload) -> anyhow::Result<()> {
        let state = &self.app_state;

        let session = match db::get_session(&state.db, &payload.session_id).await? {
            Some(s) => s,
            None => {
                tracing::warn!(
                    session_id = %payload.session_id,
                    "stiglab: portal.session_requested — session not found, skipping"
                );
                return Ok(());
            }
        };

        // Idempotency: only dispatch Pending sessions.
        if session.state != SessionState::Pending {
            tracing::debug!(
                session_id = %payload.session_id,
                state = ?session.state,
                "stiglab: portal.session_requested — session not pending, skipping"
            );
            return Ok(());
        }

        let credentials = match payload.workspace_id.as_deref() {
            Some(ws) => fetch_workspace_credentials(state, ws, &payload.user_id).await,
            None => None,
        };

        let task = Task {
            id: payload.task_id.clone(),
            prompt: payload.prompt.clone(),
            node_id: Some(payload.node_id.clone()),
            working_dir: payload.working_dir.clone(),
            allowed_tools: payload.allowed_tools.clone(),
            max_turns: payload.max_turns,
            model: payload.model.clone(),
            system_prompt: payload.system_prompt.clone(),
            permission_mode: payload.permission_mode.clone(),
            created_at: session.created_at,
        };

        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&payload.node_id) {
            let msg = ServerMessage::DispatchTask {
                task: Box::new(task),
                session_id: payload.session_id.clone(),
                credentials,
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = agent
                    .sender
                    .send(axum::extract::ws::Message::Text(json.into()));
            }
            if let Err(e) =
                db::update_session_state(&state.db, &payload.session_id, SessionState::Dispatched)
                    .await
            {
                tracing::error!(
                    session_id = %payload.session_id,
                    "stiglab: portal.session_requested — failed to update state: {e}"
                );
            }
            if let Some(ref spine) = state.spine {
                let _ = spine
                    .emit_session_started(&payload.session_id, "", &payload.node_id)
                    .await;
            }
            tracing::info!(
                session_id = %payload.session_id,
                node_id = %payload.node_id,
                "stiglab: dispatched session from portal.session_requested"
            );
        } else {
            tracing::warn!(
                session_id = %payload.session_id,
                node_id = %payload.node_id,
                "stiglab: portal.session_requested — agent not connected, session stays pending"
            );
        }

        Ok(())
    }
}

async fn fetch_workspace_credentials(
    state: &AppState,
    workspace_id: &str,
    user_id: &str,
) -> Option<HashMap<String, String>> {
    let key = state.config.credential_key.as_deref()?;

    let encrypted_creds = db::get_all_user_credential_values(&state.db, workspace_id, user_id)
        .await
        .ok()?;

    if encrypted_creds.is_empty() {
        return None;
    }

    let mut result = HashMap::new();
    for (name, encrypted_value) in encrypted_creds {
        match decrypt_credential(key, &encrypted_value) {
            Ok(value) => {
                result.insert(name, value);
            }
            Err(e) => {
                tracing::error!("failed to decrypt credential {name} for user {user_id}: {e}");
            }
        }
    }

    if result.is_empty() { None } else { Some(result) }
}
