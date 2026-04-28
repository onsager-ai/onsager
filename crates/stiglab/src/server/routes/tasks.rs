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

    // Auth is always-on as of #193 — every request carries a real user.
    let user_id: &str = auth_user.user_id.as_str();

    // Validate project membership if the caller scoped the session to a
    // workspace-owned project (issue #59). Non-members get 404 via
    // `assert_workspace_member` so project IDs can't be enumerated.
    // The project also resolves the workspace the session is launched
    // into — credentials, listing, and detail-access checks all key on
    // that workspace_id (issue #164).
    //
    // Resolution order for the session's workspace:
    //   1. project_id  → use that project's workspace (project owns scope)
    //   2. workspace_id explicit → use it after a membership check
    //   3. neither     → personal session, NULL workspace_id (legacy path)
    //
    // When both are supplied and disagree, 400 — the dashboard should
    // pick one rather than the server silently preferring one over the
    // other.
    let workspace_id: Option<String> = if let Some(ref project_id) = request.project_id {
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
        if let Some(ref explicit) = request.workspace_id {
            if explicit != &project.workspace_id {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "workspace_id does not match the project's workspace",
                    })),
                )
                    .into_response();
            }
        }
        if let Err(r) = crate::server::routes::workspaces::assert_workspace_member(
            &state.db,
            &auth_user,
            &project.workspace_id,
        )
        .await
        {
            return r;
        }
        Some(project.workspace_id)
    } else if let Some(ref explicit) = request.workspace_id {
        // 404 (not 403) on non-membership via the shared helper —
        // matches every other workspace-scoped surface so a caller
        // can't enumerate workspaces by probing this endpoint.
        if let Err(r) =
            crate::server::routes::require_workspace_access(&state.db, &auth_user, explicit).await
        {
            return r;
        }
        Some(explicit.clone())
    } else {
        None
    };

    if let Err(e) = db::insert_session_with_user_project_workspace(
        &state.db,
        &session,
        Some(user_id),
        request.project_id.as_deref(),
        workspace_id.as_deref(),
    )
    .await
    {
        tracing::error!("failed to insert session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    // Fetch user credentials scoped to the session's workspace. Personal
    // sessions (no project, no workspace) get no credentials — the resulting
    // session_failed event surfaces the broken state via forge's listener
    // rather than silently launching an unauthenticated agent.
    let credentials = match workspace_id.as_deref() {
        Some(ws) => fetch_workspace_credentials(&state, ws, user_id).await,
        None => None,
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

/// Fetch the credentials a user holds in a specific workspace, decrypt
/// them, and return as a HashMap of env-var name → plaintext value.
///
/// Used by both `POST /api/tasks` and the spine-dispatched
/// `forge.shaping_dispatched` listener (`crate::server::shaping_listener`).
/// The workspace argument is mandatory post-#164 — credentials are
/// per-workspace and a session in W1 must never be launched with a
/// W2 token.
pub(super) async fn fetch_workspace_credentials(
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

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
