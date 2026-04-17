use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::core::adapter;
use crate::core::{ServerMessage, Session, SessionState};

use crate::server::db;
use crate::server::state::AppState;

/// POST /api/shaping — accept an Onsager ShapingRequest, dispatch it, and
/// return a ShapingResult when the session reaches a terminal state.
pub async fn create_shaping(
    State(state): State<AppState>,
    Json(req): Json<onsager_spine::ShapingRequest>,
) -> impl IntoResponse {
    let task = adapter::shaping_request_to_task(&req);

    // Find target node (auto-assign to least loaded)
    let target_node = match db::find_least_loaded_node(&state.db).await {
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Create session
    let mut session = Session {
        id: Uuid::new_v4().to_string(),
        task_id: task.id.clone(),
        node_id: target_node.id.clone(),
        state: SessionState::Pending,
        prompt: task.prompt.clone(),
        output: None,
        working_dir: task.working_dir.clone(),
        artifact_id: Some(req.artifact_id.to_string()),
        artifact_version: Some(req.target_version as i32),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    if let Err(e) = db::insert_session(&state.db, &session).await {
        tracing::error!("failed to insert session: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // Dispatch to agent via WebSocket
    {
        let agents = state.agents.read().await;
        if let Some(agent) = agents.get(&target_node.id) {
            let msg = ServerMessage::DispatchTask {
                task: Box::new(task.clone()),
                session_id: session.id.clone(),
                credentials: None,
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = agent
                    .sender
                    .send(axum::extract::ws::Message::Text(json.into()));
            }
            let _ =
                db::update_session_state(&state.db, &session.id, SessionState::Dispatched).await;
            session.state = SessionState::Dispatched;
        } else {
            tracing::warn!(
                "agent for node {} not connected, session stays pending",
                target_node.id
            );
        }
    }

    // Emit session started event to spine
    if let Some(ref spine) = state.spine {
        let _ = spine
            .emit_session_started(&session.id, &req.request_id, &target_node.id)
            .await;
    }

    let start = std::time::Instant::now();

    // Compute timeout: use request deadline or default to 300 seconds
    let timeout_duration = req
        .deadline
        .map(|d| {
            let remaining = d.signed_duration_since(Utc::now());
            std::time::Duration::from_millis(remaining.num_milliseconds().max(0) as u64)
        })
        .unwrap_or(std::time::Duration::from_secs(300));

    let deadline = tokio::time::Instant::now() + timeout_duration;

    // Poll loop: check session state every 500ms
    let final_session = loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if tokio::time::Instant::now() >= deadline {
            // Timeout — mark as failed
            let _ = db::update_session_state(&state.db, &session.id, SessionState::Failed).await;
            let timed_out = Session {
                state: SessionState::Failed,
                output: Some("shaping request timed out".to_string()),
                ..session.clone()
            };
            break timed_out;
        }

        match db::get_session(&state.db, &session.id).await {
            Ok(Some(s)) if s.state == SessionState::Done || s.state == SessionState::Failed => {
                break s;
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                tracing::error!("session {} disappeared from DB", session.id);
                let lost = Session {
                    state: SessionState::Failed,
                    output: Some("session lost".to_string()),
                    ..session.clone()
                };
                break lost;
            }
            Err(e) => {
                tracing::error!("failed to query session: {e}");
                continue;
            }
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Emit terminal event to spine
    if let Some(ref spine) = state.spine {
        match final_session.state {
            SessionState::Done => {
                let artifact_id = req.artifact_id.to_string();
                let _ = spine
                    .emit_session_completed(
                        &session.id,
                        &req.request_id,
                        duration_ms,
                        Some(&artifact_id),
                    )
                    .await;
            }
            SessionState::Failed => {
                let error_msg = final_session.output.as_deref().unwrap_or("unknown error");
                let _ = spine
                    .emit_session_failed(&session.id, &req.request_id, error_msg)
                    .await;
            }
            _ => {}
        }
    }

    let result = adapter::session_to_shaping_result(&req, &final_session, duration_ms);

    (StatusCode::OK, Json(result)).into_response()
}
