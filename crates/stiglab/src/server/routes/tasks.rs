use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::core::{ServerMessage, Session, SessionState, Task, TaskRequest};

use crate::server::auth::{decrypt_credential, AuthUser};
use crate::server::db;
use crate::server::state::AppState;

pub async fn create_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(request): Json<TaskRequest>,
) -> impl IntoResponse {
    let task = Task {
        id: Uuid::new_v4().to_string(),
        prompt: request.prompt.clone(),
        node_id: request.node_id.clone(),
        working_dir: request.working_dir.clone(),
        allowed_tools: request.allowed_tools.clone(),
        max_turns: request.max_turns,
        model: request.model.clone(),
        system_prompt: request.system_prompt.clone(),
        permission_mode: request.permission_mode.clone(),
        created_at: Utc::now(),
    };

    // Find target node
    let target_node = if let Some(ref node_id) = request.node_id {
        match db::get_node(&state.db, node_id).await {
            Ok(Some(node)) => {
                if node.status != crate::core::NodeStatus::Online {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("node {} is not online", node_id) })),
                    )
                        .into_response();
                }
                if node.active_sessions >= node.max_sessions {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("node {} is at capacity", node_id) })),
                    )
                        .into_response();
                }
                node
            }
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": format!("node {} not found", node_id) })),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!("failed to get node: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        }
    } else {
        // Auto-assign to least loaded node
        match db::find_least_loaded_node(&state.db).await {
            Ok(Some(node)) => node,
            Ok(None) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({ "error": "no available nodes for dispatch" })),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!("failed to find node: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        }
    };

    // Create session
    let mut session = Session {
        id: Uuid::new_v4().to_string(),
        task_id: task.id.clone(),
        node_id: target_node.id.clone(),
        state: SessionState::Pending,
        prompt: request.prompt.clone(),
        output: None,
        working_dir: request.working_dir.clone(),
        artifact_id: None,
        artifact_version: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let user_id = if auth_user.user_id == "anonymous" {
        None
    } else {
        Some(auth_user.user_id.as_str())
    };

    // Validate project membership if the caller scoped the session to a
    // tenant-owned project (issue #59). Non-members get 404 via
    // `assert_tenant_member` so project IDs can't be enumerated.
    if let Some(ref project_id) = request.project_id {
        if user_id.is_none() {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "authentication required to scope a session to a project"
                })),
            )
                .into_response();
        };
        let project = match db::get_project(&state.db, project_id).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "project not found" })),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::error!("failed to look up project: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
        };
        if let Err(r) = crate::server::routes::tenants::assert_tenant_member(
            &state.db,
            &auth_user,
            &project.tenant_id,
        )
        .await
        {
            return r;
        }
    }

    if let Err(e) = db::insert_session_with_user_and_project(
        &state.db,
        &session,
        user_id,
        request.project_id.as_deref(),
    )
    .await
    {
        tracing::error!("failed to insert session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    // Fetch user credentials to pass to the agent
    let credentials = if let Some(uid) = user_id {
        fetch_user_credentials(&state, uid).await
    } else {
        None
    };

    // Dispatch to agent via WebSocket
    let agents = state.agents.read().await;
    if let Some(agent) = agents.get(&target_node.id) {
        let msg = ServerMessage::DispatchTask {
            task: Box::new(task.clone()),
            session_id: session.id.clone(),
            credentials,
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = agent
                .sender
                .send(axum::extract::ws::Message::Text(json.into()));
        }
        // Update session state to dispatched
        let _ = db::update_session_state(&state.db, &session.id, SessionState::Dispatched).await;
        session.state = SessionState::Dispatched;
        session.updated_at = Utc::now();
    } else {
        tracing::warn!(
            "agent for node {} not connected, session stays pending",
            target_node.id
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "task": task,
            "session": session,
        })),
    )
        .into_response()
}

/// Fetch all user credentials, decrypt them, and return as a HashMap.
async fn fetch_user_credentials(
    state: &AppState,
    user_id: &str,
) -> Option<HashMap<String, String>> {
    let key = state.config.credential_key.as_deref()?;

    let encrypted_creds = db::get_all_user_credential_values(&state.db, user_id)
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

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
