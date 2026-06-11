//! Task creation endpoint.
//!
//! Per spec #583 (ADR 0027) portal owns the whole session lifecycle:
//! `POST /api/tasks` validates the request, inserts the session row in
//! Pending state, and hands it to the in-process
//! [`SessionRunner`](crate::session_runner::SessionRunner) — which
//! spawns the Claude CLI directly. There is no node fleet and no spine
//! dispatch hop anymore; `sessions.node_id` is the constant `"portal"`
//! (the column stays NOT NULL for historical rows).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use onsager_spine::EventMetadata;
use serde::Serialize;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::core::{Session, SessionState, Task, TaskRequest};
use crate::handlers::workspaces::require_workspace_access;
use crate::session_db;
use crate::session_runner::SessionLaunch;
use crate::state::AppState;
use crate::workspace_db;

/// The session runner executes in-process now (#583), so `sessions.node_id`
/// records this constant instead of a fleet node id.
pub const PORTAL_NODE_ID: &str = "portal";

/// Diagnostic payload on the `portal.session_requested` timeline event.
///
/// Nothing consumes this event anymore — the runner is called in-process
/// — but the row keeps the dashboard timeline's "a session was asked
/// for" marker. **Secrets are deliberately absent**: the old wire shape
/// carried `encrypted_session_token` + `repos` (with encrypted read
/// tokens) because it crossed the spine to stiglab; the in-process
/// hand-off means no secret material needs to ride a persisted event,
/// so none does.
#[derive(Debug, Serialize)]
pub struct SessionRequestedPayload {
    pub session_id: String,
    pub task_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
        working_dir: request.working_dir.clone(),
        allowed_tools: request.allowed_tools.clone(),
        max_turns: request.max_turns,
        model: request.model.clone(),
        system_prompt: request.system_prompt.clone(),
        permission_mode: request.permission_mode.clone(),
        created_at: Utc::now(),
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
        if let Some(ref explicit) = request.workspace_id
            && explicit != &project.workspace_id
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "workspace_id does not match the project's workspace",
                })),
            )
                .into_response();
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
        node_id: PORTAL_NODE_ID.to_string(),
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

    // Mint a workspace-scoped session token (spec #531) so the agent can
    // reach portal's MCP surface + the liveness Stop hook. The raw token
    // goes straight to the in-process runner — it never crosses the
    // spine, so no encryption round-trip (#583).
    let session_token = match workspace_id.as_deref() {
        Some(ws) => {
            match crate::session_token::mint_for_session(&state.pool, user_id, ws, &session.id)
                .await
            {
                Ok(token) => Some(token),
                Err(e) => {
                    // Non-fatal: the session can still run; it just can't
                    // emit via MCP and the Stop hook will fail open.
                    tracing::error!(
                        session_id = %session.id,
                        "failed to mint session token: {e}"
                    );
                    None
                }
            }
        }
        None => None,
    };

    // Resolve the workspace's bound repo set with read-scoped per-repo
    // credentials (spec #546). Eager read access: the agent gets the whole
    // set and clones on demand. Unscoped sessions and workspaces with no
    // bound repos yield an empty set — the single-repo `working_dir` path
    // is unchanged.
    let repos = match workspace_id.as_deref() {
        Some(ws) => crate::repo_access::build_workspace_repo_access(&state, ws).await,
        None => Vec::new(),
    };

    // Diagnostic timeline marker (secret-free — see the payload doc).
    let diagnostic = SessionRequestedPayload {
        session_id: session.id.clone(),
        task_id: task.id.clone(),
        prompt: task.prompt.clone(),
        working_dir: task.working_dir.clone(),
        model: task.model.clone(),
        workspace_id: workspace_id.clone(),
        user_id: user_id.to_string(),
    };
    let ws_id = workspace_id.as_deref().unwrap_or("default");
    let metadata = EventMetadata {
        correlation_id: None,
        causation_id: None,
        actor: "portal".to_string(),
    };
    if let Err(e) = state
        .spine
        .append_ext(
            ws_id,
            &session.id,
            "portal",
            "portal.session_requested",
            serde_json::to_value(&diagnostic).unwrap_or_default(),
            &metadata,
            None,
        )
        .await
    {
        // Non-fatal: the event is diagnostic-only; the session runs anyway.
        tracing::error!(
            session_id = %session.id,
            "failed to emit portal.session_requested: {e}"
        );
    }

    // Hand the session to the in-process runner (#583). Returns
    // immediately; the runner drives pending → running → done/failed.
    state.session_runner.spawn_session(
        state.pool.clone(),
        state.spine.clone(),
        state.config.clone(),
        SessionLaunch {
            session_id: session.id.clone(),
            prompt: task.prompt.clone(),
            working_dir: task.working_dir.clone(),
            max_turns: task.max_turns,
            model: task.model.clone(),
            system_prompt: task.system_prompt.clone(),
            permission_mode: task.permission_mode.clone(),
            workspace_id: workspace_id.clone(),
            user_id: user_id.to_string(),
            session_token,
            repos,
        },
    );

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "task": task,
            "session": session,
        })),
    )
        .into_response()
}
