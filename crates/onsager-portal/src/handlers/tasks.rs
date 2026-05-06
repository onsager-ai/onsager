//! Task creation endpoint (spec #222 Follow-up 3).
//!
//! Moved from `crates/stiglab/src/server/routes/tasks.rs`. The key
//! difference is that portal cannot dispatch directly to an agent
//! WebSocket — stiglab owns those connections. Instead, portal:
//!
//! 1. Validates the request (node availability, workspace membership).
//! 2. Inserts the session row in Pending state.
//! 3. Emits `portal.session_requested` onto the spine.
//! 4. Returns immediately. Stiglab's `session_requested_listener` picks up
//!    the event, fetches credentials, and dispatches to the agent WebSocket.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use onsager_spine::EventMetadata;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::core::{Session, SessionState, Task, TaskRequest};
use crate::handlers::workspaces::require_workspace_access;
use crate::session_db;
use crate::state::AppState;
use crate::workspace_db;

/// Wire payload embedded in every `portal.session_requested` spine event.
/// Stiglab's `session_requested_listener` deserializes this to dispatch the
/// session to the correct agent node.
#[derive(Debug, Serialize)]
pub struct TaskDispatchPayload {
    pub session_id: String,
    pub node_id: String,
    pub task_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub user_id: String,
}

/// POST /api/tasks — create a task and its initial session.
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

    // Find target node.
    let target_node = if let Some(ref node_id) = request.node_id {
        match session_db::get_node(&state.pool, node_id).await {
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
        match session_db::find_least_loaded_node(&state.pool).await {
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

    // Resolve workspace (project wins over explicit, explicit wins over None).
    let workspace_id: Option<String> = if let Some(ref project_id) = request.project_id {
        let project = match workspace_db::get_project(&state.pool, project_id).await {
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
        if let Err(r) =
            require_workspace_access(&state.pool, &auth_user, &project.workspace_id).await
        {
            return r;
        }
        Some(project.workspace_id)
    } else if let Some(ref explicit) = request.workspace_id {
        if let Err(r) = require_workspace_access(&state.pool, &auth_user, explicit).await {
            return r;
        }
        Some(explicit.clone())
    } else {
        None
    };

    let session = Session {
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

    let user_id: &str = auth_user.user_id.as_str();

    if let Err(e) = session_db::insert_session_with_user_project_workspace(
        &state.pool,
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

    // Emit portal.session_requested so stiglab's listener can pick up the
    // session, fetch credentials, and dispatch to the agent WebSocket.
    let dispatch = TaskDispatchPayload {
        session_id: session.id.clone(),
        node_id: target_node.id.clone(),
        task_id: task.id.clone(),
        prompt: task.prompt.clone(),
        working_dir: task.working_dir.clone(),
        allowed_tools: task.allowed_tools.clone(),
        max_turns: task.max_turns,
        model: task.model.clone(),
        system_prompt: task.system_prompt.clone(),
        permission_mode: task.permission_mode.clone(),
        workspace_id: workspace_id.clone(),
        user_id: user_id.to_string(),
    };

    let ws_id = workspace_id.as_deref().unwrap_or("default");
    let stream_id = session.id.clone();
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "portal".to_string(),
    };
    let data = serde_json::to_value(&dispatch).unwrap_or_default();

    if let Err(e) = state
        .spine
        .append_ext(
            ws_id,
            &stream_id,
            "portal",
            "portal.session_requested",
            data,
            &metadata,
            None,
        )
        .await
    {
        // Non-fatal: the session is in DB; stiglab's WS drain will pick it up
        // on reconnect. Log loudly so operators can correlate with stalled dispatches.
        tracing::error!(
            session_id = %session.id,
            "failed to emit portal.session_requested: {e}"
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
