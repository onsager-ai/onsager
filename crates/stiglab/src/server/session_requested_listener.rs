//! Spine listener that consumes `portal.session_requested` events and
//! dispatches the session to an agent via WebSocket (spec #222 Follow-up 3).
//!
//! Portal owns `POST /api/tasks` and creates the session row in `Pending`
//! state, then emits this event. Stiglab listens, fetches the workspace
//! credential set, and sends `ServerMessage::DispatchTask` to the agent
//! node — the same dispatch path `routes::tasks::create_task` used before
//! it moved to portal.

use async_trait::async_trait;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use serde::Deserialize;

use crate::core::{ServerMessage, SessionState, Task};
use crate::server::db;
use crate::server::routes::shaping::fetch_workspace_credentials;
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
    /// Workspace-scoped session token (spec #531), encrypted with the
    /// shared credential key. Decrypted here and injected into the agent
    /// env as `ONSAGER_SESSION_TOKEN`.
    #[serde(default)]
    encrypted_session_token: Option<String>,
    /// The workspace's bound repo set with read-scoped per-repo
    /// credentials (spec #546). Decrypted here and surfaced to the agent
    /// as `ONSAGER_REPOS` so it can clone on demand. Empty for an unscoped
    /// session or a workspace with no bound repos — the single-repo
    /// `working_dir` path is unchanged.
    #[serde(default)]
    repos: Vec<onsager_spine::protocol::RepoAccess>,
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

        let mut credentials = match payload.workspace_id.as_deref() {
            Some(ws) => fetch_workspace_credentials(state, ws, &payload.user_id).await,
            None => None,
        };

        // Inject the workspace-scoped session token + the session's own
        // identity (spec #531) so the agent can authenticate to portal's
        // MCP surface and the liveness Stop hook, and pass its
        // `(workspace_id, session_id)` to the emit tools. The token rides
        // the event encrypted; decrypt it with the shared credential key.
        if let (Some(enc), Some(ws), Some(key)) = (
            payload.encrypted_session_token.as_deref(),
            payload.workspace_id.as_deref(),
            state.config.credential_key.as_deref(),
        ) {
            match crate::server::auth::decrypt_credential(key, enc) {
                Ok(token) => {
                    let creds = credentials.get_or_insert_with(std::collections::HashMap::new);
                    creds.insert("ONSAGER_SESSION_TOKEN".to_string(), token);
                    creds.insert("ONSAGER_SESSION_ID".to_string(), payload.session_id.clone());
                    creds.insert("ONSAGER_WORKSPACE_ID".to_string(), ws.to_string());
                }
                Err(e) => {
                    // Non-fatal: session proceeds without MCP auth; the
                    // Stop hook then fails open and the authoritative gate
                    // (#523) still applies.
                    tracing::error!(
                        session_id = %payload.session_id,
                        "failed to decrypt session token: {e}"
                    );
                }
            }
        }

        // Surface the workspace's bound repo set to the agent (spec #546).
        // Decrypt each read token and expose the set as `ONSAGER_REPOS` so
        // the agent can clone whichever repos it needs beneath its
        // `working_dir`. Empty set → nothing injected (single-repo path
        // unchanged).
        if let Some(repos_json) = crate::server::repo_env::repos_env_from_access(
            &payload.repos,
            state.config.credential_key.as_deref(),
        ) {
            let creds = credentials.get_or_insert_with(std::collections::HashMap::new);
            creds.insert(
                crate::server::repo_env::REPOS_ENV.to_string(),
                repos_json,
            );
        }

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
            workspace_id: payload.workspace_id.clone(),
            created_at: session.created_at,
        };

        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&payload.node_id) {
            let msg = ServerMessage::DispatchTask {
                task: Box::new(task),
                session_id: payload.session_id.clone(),
                credentials,
            };
            let sent = match serde_json::to_string(&msg) {
                Ok(json) => agent
                    .sender
                    .send(axum::extract::ws::Message::Text(json.into()))
                    .is_ok(),
                Err(e) => {
                    tracing::error!(
                        session_id = %payload.session_id,
                        "stiglab: portal.session_requested — serialize failed: {e}"
                    );
                    false
                }
            };

            if sent {
                if let Err(e) = db::update_session_state(
                    &state.db,
                    &payload.session_id,
                    SessionState::Dispatched,
                )
                .await
                {
                    tracing::error!(
                        session_id = %payload.session_id,
                        "stiglab: portal.session_requested — failed to update state: {e}"
                    );
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
                    "stiglab: portal.session_requested — WS send failed, session stays pending"
                );
            }
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
